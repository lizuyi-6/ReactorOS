use chrono::Utc;
use reactor_edge_daemon::{
    db::{Db, ProductResult},
    optimizer::recommend,
    state::SensorSnapshot,
};
use rusqlite::{params, Connection};
use serde_json::json;
use std::sync::Arc;

fn sample(index: usize) -> SensorSnapshot {
    SensorSnapshot {
        temperature_c: 170.0 + index as f64,
        pressure_mpa: 0.2,
        stirrer_rpm: 450.0,
        shake_speed_cpm: 30.0,
        tilt_state: (index % 2) as u8,
        tilt_angle_deg: 12.5,
        flow_rate_l_min: 2.5,
        product_concentration_percent: 62.4,
        ph: 7.18,
        captured_at: Utc::now(),
    }
}

#[test]
fn batch_result_and_recommendation_round_trip() {
    let db = Db::open_memory().unwrap();
    let batch = db.create_batch("test", 70.0, 400.0, 45.0, 60.0).unwrap();
    db.insert_product_result(&ProductResult {
        batch_id: batch.id,
        yield_percent: 72.5,
        product_ratio: 0.82,
        notes: "ok".to_string(),
    })
    .unwrap();

    let outcomes = db.batch_outcomes().unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].batch_id, batch.id);

    let rec = recommend(
        &reactor_edge_daemon::config::OptimizerBounds {
            min_temperature_c: 35.0,
            max_temperature_c: 140.0,
            min_stirrer_rpm: 100.0,
            max_stirrer_rpm: 1000.0,
            min_heating_minutes: 15.0,
            max_heating_minutes: 240.0,
            min_stirring_minutes: 15.0,
            max_stirring_minutes: 240.0,
        },
        &outcomes,
    );
    db.insert_recommendation(&rec).unwrap();

    let latest = db.latest_recommendation().unwrap().unwrap();
    assert_eq!(latest.based_on_batch_count, 1);
}

#[test]
fn persists_extended_sensor_sample() {
    let db = Db::open_memory().unwrap();
    let batch = db.create_batch("esp32", 70.0, 400.0, 45.0, 60.0).unwrap();
    db.insert_sample(
        Some(batch.id),
        &SensorSnapshot {
            temperature_c: 175.4,
            pressure_mpa: 0.2102,
            stirrer_rpm: 450.0,
            shake_speed_cpm: 30.0,
            tilt_state: 1,
            tilt_angle_deg: 12.5,
            flow_rate_l_min: 2.5,
            product_concentration_percent: 62.4,
            ph: 7.18,
            captured_at: Utc::now(),
        },
    )
    .unwrap();

    let sample = db.recent_samples(1).unwrap().pop().unwrap();
    assert_eq!(sample.shake_speed_cpm, 30.0);
    assert_eq!(sample.tilt_state, 1);
    assert_eq!(sample.tilt_angle_deg, 12.5);
}

#[test]
fn file_database_allows_parallel_reads_while_writing_samples() {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Db::open(dir.path().join("reactor.sqlite3")).unwrap());

    let writer = {
        let db = Arc::clone(&db);
        std::thread::spawn(move || {
            for index in 0..80 {
                db.insert_sample(None, &sample(index)).unwrap();
            }
        })
    };

    let readers: Vec<_> = (0..4)
        .map(|_| {
            let db = Arc::clone(&db);
            std::thread::spawn(move || {
                for _ in 0..40 {
                    let _ = db.recent_sample_records(20).unwrap();
                    let _ = db.recent_samples(20).unwrap();
                }
            })
        })
        .collect();

    writer.join().unwrap();
    for reader in readers {
        reader.join().unwrap();
    }

    assert_eq!(db.recent_sample_records(100).unwrap().len(), 80);
}

#[test]
fn migration_creates_indexes_for_hot_history_queries() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();
    let indexes = db.index_names_for_diagnostics().unwrap();

    assert!(indexes.contains(&"idx_sensor_samples_captured_id".to_string()));
    assert!(indexes.contains(&"idx_sensor_samples_batch_id_id".to_string()));
    assert!(indexes.contains(&"idx_control_events_batch_id_id".to_string()));
    assert!(indexes.contains(&"idx_control_events_hashed_id".to_string()));
}

#[test]
fn recent_windows_return_oldest_to_newest_within_the_limited_window() {
    let db = Db::open_memory().unwrap();
    let mut batch_ids = Vec::new();
    for index in 0..4 {
        let batch = db
            .create_batch(
                &format!("batch {index}"),
                60.0 + index as f64,
                300.0 + index as f64,
                30.0,
                60.0,
            )
            .unwrap();
        db.insert_product_result(&ProductResult {
            batch_id: batch.id,
            yield_percent: 70.0 + index as f64,
            product_ratio: 0.8,
            notes: format!("outcome {index}"),
        })
        .unwrap();
        db.insert_control_event(
            Some(batch.id),
            "recent_window_probe",
            None,
            &format!("event {index}"),
        )
        .unwrap();
        batch_ids.push(batch.id);
    }

    let recent_batches = db.recent_batches(2).unwrap();
    assert_eq!(
        recent_batches
            .iter()
            .map(|batch| batch.id)
            .collect::<Vec<_>>(),
        vec![batch_ids[2], batch_ids[3]]
    );

    let recent_outcomes = db.recent_batch_outcomes(2).unwrap();
    assert_eq!(
        recent_outcomes
            .iter()
            .map(|outcome| outcome.batch_id)
            .collect::<Vec<_>>(),
        vec![batch_ids[2], batch_ids[3]]
    );

    let recent_events = db.recent_control_events(2).unwrap();
    assert_eq!(
        recent_events
            .iter()
            .map(|event| event.reason.as_str())
            .collect::<Vec<_>>(),
        vec!["event 2", "event 3"]
    );

    db.insert_control_event(
        Some(batch_ids[3]),
        "recent_window_probe",
        None,
        "batch event 1",
    )
    .unwrap();
    db.insert_control_event(
        Some(batch_ids[3]),
        "recent_window_probe",
        None,
        "batch event 2",
    )
    .unwrap();
    db.insert_control_event(
        Some(batch_ids[3]),
        "recent_window_probe",
        None,
        "batch event 3",
    )
    .unwrap();
    let batch_events = db.control_events_for_batch(batch_ids[3], 2).unwrap();
    assert_eq!(
        batch_events
            .iter()
            .map(|event| event.reason.as_str())
            .collect::<Vec<_>>(),
        vec!["batch event 2", "batch event 3"]
    );
}

#[test]
fn audit_chain_status_uses_bounded_window_without_claiming_full_validity() {
    let db = Db::open_memory().unwrap();
    for index in 0..10_001 {
        db.insert_control_event(None, "audit_window_probe", None, &format!("event {index}"))
            .unwrap();
    }

    let status = db.audit_chain_status().unwrap();

    assert_eq!(status.total_hashed_events, 10_001);
    assert_eq!(status.checked_events, 10_000);
    assert!(status.window_valid);
    assert!(!status.valid);
    assert!(status.verification_truncated);
    assert_eq!(status.checked_from_event_id, Some(2));
    assert_eq!(status.checked_to_event_id, Some(10_001));

    let full = db.full_audit_chain_status_for_diagnostics().unwrap();
    assert_eq!(full.total_hashed_events, 10_001);
    assert_eq!(full.checked_events, 10_001);
    assert!(full.valid);
    assert!(!full.verification_truncated);
    assert_eq!(full.checked_from_event_id, Some(1));
    assert_eq!(full.checked_to_event_id, Some(10_001));
}

#[tokio::test]
async fn async_file_database_audit_count_uses_sqlx_pool() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();

    db.insert_control_event(None, "sqlx_probe", None, "event 1")
        .unwrap();
    db.insert_control_event(None, "sqlx_probe", None, "event 2")
        .unwrap();
    db.insert_control_event(None, "manual_probe", None, "event 3")
        .unwrap();

    assert_eq!(db.audit_event_count_sqlx(None).await.unwrap(), 3);
    assert_eq!(
        db.audit_event_count_sqlx(Some("sqlx_probe")).await.unwrap(),
        2
    );
    assert_eq!(
        db.audit_event_count_sqlx(Some("missing_probe"))
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn async_file_database_audit_events_use_sqlx_pool() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();

    for index in 0..5 {
        let event_type = if index % 2 == 0 {
            "sqlx_list_probe"
        } else {
            "other_probe"
        };
        db.insert_control_event(None, event_type, None, &format!("event {index}"))
            .unwrap();
    }

    let first_page = db.audit_events_sqlx(2, 0, None).await.unwrap();
    assert_eq!(
        first_page
            .iter()
            .map(|event| event.reason.as_str())
            .collect::<Vec<_>>(),
        vec!["event 4", "event 3"]
    );

    let second_page = db.audit_events_sqlx(2, 2, None).await.unwrap();
    assert_eq!(
        second_page
            .iter()
            .map(|event| event.reason.as_str())
            .collect::<Vec<_>>(),
        vec!["event 2", "event 1"]
    );

    let filtered = db
        .audit_events_sqlx(10, 0, Some("sqlx_list_probe"))
        .await
        .unwrap();
    assert_eq!(
        filtered
            .iter()
            .map(|event| event.reason.as_str())
            .collect::<Vec<_>>(),
        vec!["event 4", "event 2", "event 0"]
    );
}

#[tokio::test]
async fn async_file_database_audit_chain_status_uses_sqlx_pool() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();

    for index in 0..4 {
        db.insert_control_event(None, "sqlx_chain_probe", None, &format!("event {index}"))
            .unwrap();
    }

    let status = db.audit_chain_status_sqlx().await.unwrap();
    assert_eq!(status.total_hashed_events, 4);
    assert_eq!(status.checked_events, 4);
    assert_eq!(status.chained_events, 4);
    assert_eq!(status.broken_events, 0);
    assert!(status.window_valid);
    assert!(status.valid);
    assert_eq!(status.checked_from_event_id, Some(1));
    assert_eq!(status.checked_to_event_id, Some(4));
    assert!(!status.verification_truncated);
}

#[tokio::test]
async fn async_file_database_recent_batches_and_outcomes_use_sqlx_pool() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();
    let mut batch_ids = Vec::new();
    for index in 0..4 {
        let batch = db
            .create_batch(
                &format!("sqlx batch {index}"),
                60.0 + index as f64,
                300.0 + index as f64,
                30.0,
                60.0,
            )
            .unwrap();
        db.insert_product_result(&ProductResult {
            batch_id: batch.id,
            yield_percent: 70.0 + index as f64,
            product_ratio: 0.8 + (index as f64 * 0.01),
            notes: format!("outcome {index}"),
        })
        .unwrap();
        batch_ids.push(batch.id);
    }

    let batches = db.recent_batches_sqlx(2).await.unwrap();
    assert_eq!(
        batches.iter().map(|batch| batch.id).collect::<Vec<_>>(),
        vec![batch_ids[2], batch_ids[3]]
    );

    let outcomes = db.recent_batch_outcomes_sqlx(2).await.unwrap();
    assert_eq!(
        outcomes
            .iter()
            .map(|outcome| outcome.batch_id)
            .collect::<Vec<_>>(),
        vec![batch_ids[2], batch_ids[3]]
    );
    assert_eq!(outcomes[1].yield_percent, 73.0);
}

#[tokio::test]
async fn async_file_database_batch_lifecycle_writes_use_sqlx_pool() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();
    let process = db
        .create_process("sqlx lifecycle process", "batch lifecycle parent")
        .unwrap();

    let batch = db
        .create_batch_for_process_sqlx(Some(process.id), "sqlx lifecycle", 82.5, 440.0, 36.0, 58.0)
        .await
        .unwrap();
    assert_eq!(batch.process_id, Some(process.id));
    assert!(batch.finished_at.is_none());

    db.finish_batch_sqlx(batch.id).await.unwrap();

    let batches = db.recent_batches_sqlx(1).await.unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].id, batch.id);
    assert_eq!(batches[0].process_id, Some(process.id));
    assert_eq!(batches[0].target_temperature_c, 82.5);
    assert!(batches[0].finished_at.is_some());
}

#[tokio::test]
async fn async_file_database_all_batch_outcomes_use_sqlx_pool() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();
    for index in 0..3 {
        let batch = db
            .create_batch(
                &format!("sqlx outcome {index}"),
                55.0 + index as f64,
                250.0 + index as f64,
                25.0,
                50.0,
            )
            .unwrap();
        db.insert_product_result(&ProductResult {
            batch_id: batch.id,
            yield_percent: 68.0 + index as f64,
            product_ratio: 0.75 + (index as f64 * 0.02),
            notes: format!("outcome {index}"),
        })
        .unwrap();
    }

    let outcomes = db.batch_outcomes_sqlx().await.unwrap();
    assert_eq!(
        outcomes
            .iter()
            .map(|outcome| outcome.yield_percent)
            .collect::<Vec<_>>(),
        vec![68.0, 69.0, 70.0]
    );

    let rec = recommend(
        &reactor_edge_daemon::config::OptimizerBounds {
            min_temperature_c: 35.0,
            max_temperature_c: 140.0,
            min_stirrer_rpm: 100.0,
            max_stirrer_rpm: 1000.0,
            min_heating_minutes: 15.0,
            max_heating_minutes: 240.0,
            min_stirring_minutes: 15.0,
            max_stirring_minutes: 240.0,
        },
        &outcomes,
    );
    assert_eq!(rec.based_on_batch_count, 3);
}

#[tokio::test]
async fn async_file_database_sensor_history_reads_use_sqlx_pool() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();
    let batch = db
        .create_batch("sqlx samples", 70.0, 400.0, 45.0, 60.0)
        .unwrap();
    let base_time = Utc::now();
    for index in 0..5 {
        let mut sample = sample(index);
        sample.captured_at = base_time + chrono::Duration::seconds(index as i64);
        db.insert_sample(Some(batch.id), &sample).unwrap();
    }

    let recent = db.recent_sample_records_sqlx(2).await.unwrap();
    assert_eq!(
        recent
            .iter()
            .map(|record| record.sample.temperature_c)
            .collect::<Vec<_>>(),
        vec![173.0, 174.0]
    );
    assert_eq!(recent[1].batch_id, Some(batch.id));
    assert_eq!(recent[1].sample.tilt_state, 0);

    let batch_samples = db.sample_records_for_batch_sqlx(batch.id, 3).await.unwrap();
    assert_eq!(
        batch_samples
            .iter()
            .map(|record| record.sample.temperature_c)
            .collect::<Vec<_>>(),
        vec![172.0, 173.0, 174.0]
    );

    let ranged = db
        .samples_between_sqlx(
            base_time + chrono::Duration::seconds(1),
            base_time + chrono::Duration::seconds(4),
            2,
            1,
        )
        .await
        .unwrap();
    assert_eq!(
        ranged
            .iter()
            .map(|record| record.sample.temperature_c)
            .collect::<Vec<_>>(),
        vec![172.0, 173.0]
    );
}

#[tokio::test]
async fn async_file_database_sensor_sample_writes_use_sqlx_pool() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();
    let batch = db
        .create_batch("sqlx sample write", 72.0, 410.0, 42.0, 63.0)
        .unwrap();
    let mut written = sample(9);
    written.temperature_c = 181.25;
    written.pressure_mpa = 0.33;
    written.tilt_state = 1;

    db.insert_sample_sqlx(Some(batch.id), &written)
        .await
        .unwrap();

    let samples = db.recent_sample_records_sqlx(1).await.unwrap();
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].batch_id, Some(batch.id));
    assert_eq!(samples[0].sample.temperature_c, 181.25);
    assert_eq!(samples[0].sample.pressure_mpa, 0.33);
    assert_eq!(samples[0].sample.tilt_state, 1);
}

#[tokio::test]
async fn async_file_database_latest_recommendation_uses_sqlx_pool() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();

    assert!(db.latest_recommendation_sqlx().await.unwrap().is_none());

    db.insert_recommendation(&reactor_edge_daemon::optimizer::Recommendation {
        based_on_batch_count: 1,
        target_temperature_c: 65.0,
        target_stirrer_rpm: 350.0,
        heating_minutes: 30.0,
        stirring_minutes: 45.0,
        expected_score: 80.0,
        rationale: "local first".to_string(),
    })
    .unwrap();
    db.insert_recommendation(&reactor_edge_daemon::optimizer::Recommendation {
        based_on_batch_count: 2,
        target_temperature_c: 72.0,
        target_stirrer_rpm: 420.0,
        heating_minutes: 35.0,
        stirring_minutes: 55.0,
        expected_score: 86.0,
        rationale: "local latest".to_string(),
    })
    .unwrap();

    let recommendation = db.latest_recommendation_sqlx().await.unwrap().unwrap();
    assert_eq!(recommendation.based_on_batch_count, 2);
    assert_eq!(recommendation.target_temperature_c, 72.0);
    assert_eq!(recommendation.rationale, "local latest");
}

#[tokio::test]
async fn async_file_database_recommendation_writes_use_sqlx_pool() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();

    db.insert_recommendation_sqlx(&reactor_edge_daemon::optimizer::Recommendation {
        based_on_batch_count: 4,
        target_temperature_c: 68.0,
        target_stirrer_rpm: 360.0,
        heating_minutes: 38.0,
        stirring_minutes: 52.0,
        expected_score: 84.5,
        rationale: "sqlx recommendation write".to_string(),
    })
    .await
    .unwrap();

    let recommendation = db.latest_recommendation_sqlx().await.unwrap().unwrap();
    assert_eq!(recommendation.based_on_batch_count, 4);
    assert_eq!(recommendation.target_temperature_c, 68.0);
    assert_eq!(recommendation.rationale, "sqlx recommendation write");
}

#[tokio::test]
async fn async_file_database_product_result_writes_use_sqlx_pool() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();
    let batch = db
        .create_batch("sqlx outcome write", 77.0, 430.0, 41.0, 62.0)
        .unwrap();

    db.insert_product_result_sqlx(&ProductResult {
        batch_id: batch.id,
        yield_percent: 71.5,
        product_ratio: 0.76,
        notes: "first result".to_string(),
    })
    .await
    .unwrap();
    db.insert_product_result_sqlx(&ProductResult {
        batch_id: batch.id,
        yield_percent: 74.25,
        product_ratio: 0.81,
        notes: "updated result".to_string(),
    })
    .await
    .unwrap();

    let outcomes = db.batch_outcomes_sqlx().await.unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].batch_id, batch.id);
    assert_eq!(outcomes[0].yield_percent, 74.25);
    assert_eq!(outcomes[0].product_ratio, 0.81);
}

#[test]
fn integration_task_payloads_encrypt_at_rest_when_key_is_configured() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactor.sqlite3");
    let db = Db::open_with_encryption_key(&path, [7_u8; 32]).unwrap();
    let request = json!({
        "action": "set_targets",
        "external_task_id": "secure-001",
        "reason": "sensitive AINAS payload",
        "target_temperature_c": 62.5
    });
    let task = db
        .create_integration_task("ainas", Some("secure-001"), "set_targets", &request)
        .unwrap();
    let response = json!({
        "code": 0,
        "message": "success",
        "data": { "receipt": "private-third-party-receipt" }
    });
    db.update_integration_task(task.id, "executed", &response)
        .unwrap();

    let raw = raw_task_payloads(&path, task.id);
    assert!(raw.0.starts_with("xingshu:v1:aes256gcm:"));
    assert!(raw.1.starts_with("xingshu:v1:aes256gcm:"));
    assert!(!raw.0.contains("sensitive AINAS payload"));
    assert!(!raw.1.contains("private-third-party-receipt"));

    let stored = db.integration_task(task.id).unwrap().unwrap();
    assert_eq!(stored.request["reason"], "sensitive AINAS payload");
    assert_eq!(
        stored.response["data"]["receipt"],
        "private-third-party-receipt"
    );

    let status = db.encryption_status();
    assert!(status.enabled);
    assert_eq!(status.algorithm, "AES-256-GCM");
    assert!(status
        .encrypted_fields
        .contains(&"integration_tasks.request_json"));
    assert!(status
        .encrypted_fields
        .contains(&"integration_tasks.response_json"));
}

#[tokio::test]
async fn async_file_database_integration_task_reads_use_sqlx_pool_with_encryption() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactor.sqlite3");
    let db = Db::open_with_encryption_key(&path, [11_u8; 32]).unwrap();
    let task = db
        .create_integration_task(
            "ainas",
            Some("sqlx-secure-001"),
            "set_targets",
            &json!({ "action": "set_targets", "reason": "sqlx encrypted payload" }),
        )
        .unwrap();
    db.update_integration_task(
        task.id,
        "executed",
        &json!({ "code": 0, "message": "sqlx encrypted response" }),
    )
    .unwrap();

    let by_id = db.integration_task_sqlx(task.id).await.unwrap().unwrap();
    assert_eq!(by_id.request["reason"], "sqlx encrypted payload");
    assert_eq!(by_id.response["message"], "sqlx encrypted response");

    let tasks = db.integration_tasks_sqlx(Some("ainas"), 10).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].external_task_id.as_deref(),
        Some("sqlx-secure-001")
    );
    assert_eq!(tasks[0].request["reason"], "sqlx encrypted payload");

    let raw = raw_task_payloads(&path, task.id);
    assert!(raw.0.starts_with("xingshu:v1:aes256gcm:"));
    assert!(!raw.0.contains("sqlx encrypted payload"));
}

#[tokio::test]
async fn async_file_database_integration_task_writes_use_sqlx_pool_with_encryption() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactor.sqlite3");
    let db = Db::open_with_encryption_key(&path, [12_u8; 32]).unwrap();
    let task = db
        .create_integration_task_sqlx(
            "mqtt",
            Some("sqlx-write-001"),
            "set_targets",
            &json!({ "action": "set_targets", "reason": "sqlx write secret" }),
        )
        .await
        .unwrap();
    assert_eq!(task.status, "received");
    assert_eq!(task.request["reason"], "sqlx write secret");
    assert!(task.response.is_null());

    let updated = db
        .update_integration_task_sqlx(
            task.id,
            "executed",
            &json!({ "code": 0, "message": "sqlx write response" }),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.status, "executed");
    assert_eq!(updated.response["message"], "sqlx write response");

    let raw = raw_task_payloads(&path, task.id);
    assert!(raw.0.starts_with("xingshu:v1:aes256gcm:"));
    assert!(raw.1.starts_with("xingshu:v1:aes256gcm:"));
    assert!(!raw.0.contains("sqlx write secret"));
    assert!(!raw.1.contains("sqlx write response"));
}

#[test]
fn integration_task_reader_keeps_plaintext_rows_compatible() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactor.sqlite3");
    let db = Db::open_with_encryption_key(&path, [9_u8; 32]).unwrap();
    let now = Utc::now().to_rfc3339();
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            r#"
            INSERT INTO integration_tasks
                (external_task_id, source, action, status, request_json, response_json, created_at, updated_at)
            VALUES (?1, 'ainas', 'set_targets', 'executed', ?2, ?3, ?4, ?4)
            "#,
            params![
                "legacy-001",
                json!({ "action": "set_targets", "reason": "legacy plaintext payload" }).to_string(),
                json!({ "code": 0, "message": "legacy plaintext response" }).to_string(),
                now
            ],
        )
        .unwrap();
    }

    let legacy = db.integration_tasks(Some("ainas"), 1).unwrap();
    assert_eq!(legacy[0].request["reason"], "legacy plaintext payload");
    assert_eq!(legacy[0].response["message"], "legacy plaintext response");
}

#[tokio::test]
async fn async_file_database_integration_task_sqlx_reader_keeps_plaintext_rows_compatible() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactor.sqlite3");
    let db = Db::open_with_encryption_key(&path, [13_u8; 32]).unwrap();
    let now = Utc::now().to_rfc3339();
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            r#"
            INSERT INTO integration_tasks
                (external_task_id, source, action, status, request_json, response_json, created_at, updated_at)
            VALUES (?1, 'ainas', 'set_targets', 'executed', ?2, ?3, ?4, ?4)
            "#,
            params![
                "legacy-sqlx-001",
                json!({ "action": "set_targets", "reason": "legacy sqlx plaintext" }).to_string(),
                json!({ "code": 0, "message": "legacy sqlx response" }).to_string(),
                now
            ],
        )
        .unwrap();
    }

    let legacy = db.integration_tasks_sqlx(Some("ainas"), 1).await.unwrap();
    assert_eq!(legacy[0].request["reason"], "legacy sqlx plaintext");
    assert_eq!(legacy[0].response["message"], "legacy sqlx response");
}

fn raw_task_payloads(path: &std::path::Path, task_id: i64) -> (String, String) {
    let conn = Connection::open(path).unwrap();
    conn.query_row(
        "SELECT request_json, response_json FROM integration_tasks WHERE id = ?1",
        params![task_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap()
}
