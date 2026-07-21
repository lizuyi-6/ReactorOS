use chrono::Utc;
use reactor_edge_daemon::{
    control::SafeCommand,
    db::{Db, NewProcessStep, ProductResult},
    optimizer::{recommend, Recommendation},
    state::SensorSnapshot,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::Path;
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

fn sample_process_step(name: &str) -> NewProcessStep {
    NewProcessStep {
        name: name.to_string(),
        target_temperature_c: 72.0,
        ramp_rate_c_min: 2.5,
        duration_minutes: 12.0,
        target_stirrer_rpm: 320.0,
        target_shake_speed_cpm: 18.0,
        target_pressure_mpa: 0.4,
        cooling_mode: "natural".to_string(),
    }
}

fn sample_recommendation(rationale: &str) -> Recommendation {
    Recommendation {
        based_on_batch_count: 3,
        target_temperature_c: 72.0,
        target_stirrer_rpm: 420.0,
        heating_minutes: 35.0,
        stirring_minutes: 55.0,
        expected_score: 86.0,
        rationale: rationale.to_string(),
    }
}

fn sample_command(reason: &str) -> SafeCommand {
    SafeCommand {
        target_temperature_c: 72.0,
        heat_time_s: 1800.0,
        hold_time_s: 2700.0,
        cool_time_s: 300.0,
        target_stirrer_rpm: 420.0,
        target_shake_speed_cpm: 18.0,
        target_pressure_mpa: 0.5,
        reason: reason.to_string(),
    }
}

#[test]
fn batch_result_and_recommendation_round_trip() {
    let db = Db::open_memory().unwrap();
    let batch = db.create_batch("test", 70.0, 400.0, 45.0, 60.0).unwrap();
    db.finish_batch(batch.id).unwrap();
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
fn rejects_invalid_product_result_before_sync_insert() {
    let db = Db::open_memory().unwrap();
    let batch = db
        .create_batch("invalid result", 70.0, 400.0, 45.0, 60.0)
        .unwrap();
    db.finish_batch(batch.id).unwrap();

    let err = db
        .insert_product_result(&ProductResult {
            batch_id: batch.id,
            yield_percent: 101.0,
            product_ratio: 0.82,
            notes: "bad yield".to_string(),
        })
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid product result rejected before DB insert")
            && message.contains("yield_percent must be between 0 and 100"),
        "unexpected product result rejection: {message}"
    );
    assert!(db.batch_outcomes().unwrap().is_empty());
    assert!(db.latest_recommendation().unwrap().is_none());
}

#[test]
fn rejects_invalid_product_result_before_audit_transaction() {
    let db = Db::open_memory().unwrap();
    let batch = db
        .create_batch("invalid audited result", 70.0, 400.0, 45.0, 60.0)
        .unwrap();
    db.finish_batch(batch.id).unwrap();

    let err = db
        .insert_product_result_with_audit(
            &ProductResult {
                batch_id: batch.id,
                yield_percent: 72.5,
                product_ratio: f64::NAN,
                notes: "bad ratio".to_string(),
            },
            "product_result_recorded",
            "must not audit invalid product result",
        )
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid product result rejected before DB insert")
            && message.contains("product_ratio must be between 0 and 1"),
        "unexpected audited product result rejection: {message}"
    );
    assert!(db.batch_outcomes().unwrap().is_empty());
    assert!(db.recent_control_events(10).unwrap().is_empty());
}

#[test]
fn skips_legacy_invalid_product_results_when_reading_outcomes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactor.sqlite3");
    let db = Db::open(&path).unwrap();
    let valid = db
        .create_batch("valid legacy outcome", 70.0, 400.0, 45.0, 60.0)
        .unwrap();
    db.finish_batch(valid.id).unwrap();
    db.insert_product_result(&ProductResult {
        batch_id: valid.id,
        yield_percent: 72.5,
        product_ratio: 0.82,
        notes: "valid legacy outcome".to_string(),
    })
    .unwrap();
    let invalid = db
        .create_batch("invalid legacy outcome", 71.0, 410.0, 46.0, 61.0)
        .unwrap();
    db.finish_batch(invalid.id).unwrap();
    db.insert_product_result(&ProductResult {
        batch_id: invalid.id,
        yield_percent: 73.5,
        product_ratio: 0.83,
        notes: "will be corrupted".to_string(),
    })
    .unwrap();

    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE product_results SET yield_percent = ?1 WHERE batch_id = ?2",
        params![101.0, invalid.id],
    )
    .unwrap();

    let outcomes = db.batch_outcomes().unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].batch_id, valid.id);
    let recent = db.recent_batch_outcomes(10).unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].batch_id, valid.id);
    assert!(db.batch_outcome_by_id(invalid.id).unwrap().is_none());

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
    assert_eq!(rec.based_on_batch_count, 1);
}

#[test]
fn unfinished_batches_are_not_product_outcomes_even_if_result_row_exists() {
    let db = Db::open_memory().unwrap();
    let unfinished = db
        .create_batch("unfinished with stray result", 70.0, 400.0, 45.0, 60.0)
        .unwrap();
    db.insert_product_result(&ProductResult {
        batch_id: unfinished.id,
        yield_percent: 72.5,
        product_ratio: 0.82,
        notes: "stray result must not train AI".to_string(),
    })
    .unwrap();

    assert!(db.batch_outcomes().unwrap().is_empty());
    assert!(db.batch_outcome_by_id(unfinished.id).unwrap().is_none());
}

#[test]
fn product_result_with_audit_is_atomic_when_audit_insert_fails() {
    let db = Db::open_memory().unwrap();
    let batch = db
        .create_batch("audit atomic result", 70.0, 400.0, 45.0, 60.0)
        .unwrap();
    db.finish_batch(batch.id).unwrap();
    db.break_control_events_for_tests().unwrap();

    let err = db
        .insert_product_result_with_audit(
            &ProductResult {
                batch_id: batch.id,
                yield_percent: 72.5,
                product_ratio: 0.82,
                notes: "must roll back with audit".to_string(),
            },
            "product_result_recorded",
            "product result saved; recommendation regeneration queued",
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("audit"),
        "unexpected product result audit failure: {err}"
    );
    assert!(db.batch_outcome_by_id(batch.id).unwrap().is_none());
}

#[test]
fn recommendation_with_audit_rolls_back_when_audit_fails() {
    let db = Db::open_memory().unwrap();
    db.break_control_events_for_tests().unwrap();

    let err = db
        .insert_recommendation_with_audit(
            &sample_recommendation("must roll back with audit"),
            "recommendation_generated",
            "operator regenerated latest AI recommendation",
        )
        .unwrap_err();

    assert!(
        err.to_string().contains("audit"),
        "unexpected recommendation audit failure: {err}"
    );
    assert!(db.latest_recommendation().unwrap().is_none());
}

#[test]
fn rejects_invalid_recommendation_before_sync_insert() {
    let db = Db::open_memory().unwrap();
    let mut recommendation = sample_recommendation("invalid recommendation");
    recommendation.expected_score = 100.01;

    let err = db.insert_recommendation(&recommendation).unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid AI recommendation rejected before DB insert")
            && message.contains("expected_score must be between 0 and 100"),
        "unexpected recommendation rejection: {message}"
    );
    assert!(db.latest_recommendation().unwrap().is_none());
}

#[test]
fn rejects_invalid_recommendation_before_audit_transaction() {
    let db = Db::open_memory().unwrap();
    let mut recommendation = sample_recommendation("invalid audited recommendation");
    recommendation.based_on_batch_count = -1;

    let err = db
        .insert_recommendation_with_audit(
            &recommendation,
            "recommendation_generated",
            "must not audit invalid recommendation",
        )
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid AI recommendation rejected before DB insert")
            && message.contains("based_on_batch_count must be >= 0"),
        "unexpected audited recommendation rejection: {message}"
    );
    assert!(db.latest_recommendation().unwrap().is_none());
    assert!(db.recent_control_events(10).unwrap().is_empty());
}

#[test]
fn rejects_invalid_batch_targets_before_sync_insert() {
    let db = Db::open_memory().unwrap();

    let err = db
        .create_batch("invalid batch target", -0.01, 400.0, 45.0, 60.0)
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid batch target rejected before DB insert")
            && message.contains("target_temperature_c must be between 0 and 500"),
        "unexpected batch target rejection: {message}"
    );
    assert!(db.recent_batches(10).unwrap().is_empty());
}

#[test]
fn rejects_invalid_process_step_before_sync_insert_or_update() {
    let db = Db::open_memory().unwrap();
    let process = db
        .create_process("process guard", "DB input guard")
        .unwrap();
    let mut invalid = sample_process_step("Invalid");
    invalid.target_shake_speed_cpm = 60.01;

    let err = db.add_process_step(process.id, &invalid).unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid process step rejected before DB insert")
            && message.contains("target_shake_speed_cpm must be between 0 and 60"),
        "unexpected process step rejection: {message}"
    );
    assert!(db
        .process_detail(process.id)
        .unwrap()
        .unwrap()
        .steps
        .is_empty());

    let step = db
        .add_process_step(process.id, &sample_process_step("Valid"))
        .unwrap()
        .unwrap();
    invalid.target_shake_speed_cpm = 30.0;
    invalid.duration_minutes = 0.0;
    let err = db
        .update_process_step_with_audit(
            process.id,
            step.id,
            &invalid,
            "process_step_updated",
            "must not audit invalid step",
        )
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid process step rejected before DB insert")
            && message.contains("duration_minutes must be between 1 and 1440"),
        "unexpected process step update rejection: {message}"
    );
    let detail = db.process_detail(process.id).unwrap().unwrap();
    assert_eq!(detail.steps.len(), 1);
    assert_eq!(detail.steps[0].name, "Valid");
    assert!(db.recent_control_events(10).unwrap().is_empty());
}

#[test]
fn rejects_legacy_invalid_process_step_when_reading_detail() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactor.sqlite3");
    let db = Db::open(&path).unwrap();
    let process = db
        .create_process("legacy process step guard", "DB input guard")
        .unwrap();
    let step = db
        .add_process_step(process.id, &sample_process_step("Restored Step"))
        .unwrap()
        .unwrap();

    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE process_steps SET target_pressure_mpa = ?1 WHERE id = ?2",
        params![10.01, step.id],
    )
    .unwrap();

    let err = db.process_detail(process.id).unwrap_err();
    assert!(
        format!("{err:#}").contains("invalid process step in database"),
        "unexpected process detail read error: {err:#}"
    );
}

#[test]
fn ignores_legacy_invalid_latest_recommendation_cache() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactor.sqlite3");
    let db = Db::open(&path).unwrap();
    db.insert_recommendation(&sample_recommendation("valid cached recommendation"))
        .unwrap();

    let conn = Connection::open(&path).unwrap();
    conn.execute(
        r#"
        INSERT INTO ai_recommendations
            (based_on_batch_count, target_temperature_c, target_stirrer_rpm,
             heating_minutes, stirring_minutes, expected_score, rationale, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
        params![
            2_i64,
            72.0,
            420.0,
            35.0,
            55.0,
            101.0,
            "legacy invalid cached recommendation",
            Utc::now().to_rfc3339()
        ],
    )
    .unwrap();

    assert!(db.latest_recommendation().unwrap().is_none());
}

#[test]
fn process_create_with_audit_rolls_back_when_audit_fails() {
    let db = Db::open_memory().unwrap();
    db.break_control_events_for_tests().unwrap();

    let err = db
        .create_process_with_audit(
            "audit atomic process",
            "must roll back with audit",
            "process_created",
            "operator created process",
        )
        .unwrap_err();

    assert!(
        err.to_string().contains("audit"),
        "unexpected process create audit failure: {err}"
    );
    assert!(db.list_processes().unwrap().is_empty());
}

#[test]
fn process_update_with_audit_rolls_back_when_audit_fails() {
    let db = Db::open_memory().unwrap();
    let process = db
        .create_process("audit update process", "original")
        .unwrap();
    db.break_control_events_for_tests().unwrap();

    let err = db
        .update_process_with_audit(
            process.id,
            "changed",
            "changed description",
            "ready",
            "process_updated",
            "operator updated process",
        )
        .unwrap_err();

    assert!(
        err.to_string().contains("audit"),
        "unexpected process update audit failure: {err}"
    );
    let detail = db.process_detail(process.id).unwrap().unwrap();
    assert_eq!(detail.process.name, "audit update process");
    assert_eq!(detail.process.description, "original");
    assert_eq!(detail.process.status, "draft");
    assert_eq!(detail.process.version, 1);
}

#[test]
fn process_step_add_with_audit_rolls_back_when_audit_fails() {
    let db = Db::open_memory().unwrap();
    let process = db
        .create_process("audit add step process", "original")
        .unwrap();
    db.break_control_events_for_tests().unwrap();

    let err = db
        .add_process_step_with_audit(
            process.id,
            &sample_process_step("Heat"),
            "process_step_added",
            "operator added process step",
        )
        .unwrap_err();

    assert!(
        err.to_string().contains("audit"),
        "unexpected process step add audit failure: {err}"
    );
    let detail = db.process_detail(process.id).unwrap().unwrap();
    assert!(detail.steps.is_empty());
}

#[test]
fn process_step_update_with_audit_rolls_back_when_audit_fails() {
    let db = Db::open_memory().unwrap();
    let process = db
        .create_process("audit update step process", "original")
        .unwrap();
    let step = db
        .add_process_step(process.id, &sample_process_step("Heat"))
        .unwrap()
        .unwrap();
    db.break_control_events_for_tests().unwrap();

    let err = db
        .update_process_step_with_audit(
            process.id,
            step.id,
            &sample_process_step("Hold"),
            "process_step_updated",
            "operator updated process step",
        )
        .unwrap_err();

    assert!(
        err.to_string().contains("audit"),
        "unexpected process step update audit failure: {err}"
    );
    let detail = db.process_detail(process.id).unwrap().unwrap();
    assert_eq!(detail.steps.len(), 1);
    assert_eq!(detail.steps[0].name, "Heat");
}

#[tokio::test]
async fn sqlx_process_writes_with_audit_roll_back_when_audit_fails() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();

    db.fail_control_events_after_successes_for_tests(0);
    db.create_process_with_audit_sqlx(
        "sqlx audit atomic process",
        "must roll back with audit",
        "process_created",
        "operator created process",
    )
    .await
    .unwrap_err();
    assert!(db.list_processes_sqlx().await.unwrap().is_empty());

    let process = db
        .create_process_sqlx("sqlx audit update process", "original")
        .await
        .unwrap();
    db.fail_control_events_after_successes_for_tests(0);
    db.update_process_with_audit_sqlx(
        process.id,
        "changed",
        "changed description",
        "ready",
        "process_updated",
        "operator updated process",
    )
    .await
    .unwrap_err();
    let detail = db.process_detail_sqlx(process.id).await.unwrap().unwrap();
    assert_eq!(detail.process.name, "sqlx audit update process");
    assert_eq!(detail.process.description, "original");
    assert_eq!(detail.process.status, "draft");
    assert_eq!(detail.process.version, 1);

    db.fail_control_events_after_successes_for_tests(0);
    db.add_process_step_with_audit_sqlx(
        process.id,
        &sample_process_step("Heat"),
        "process_step_added",
        "operator added process step",
    )
    .await
    .unwrap_err();
    let detail = db.process_detail_sqlx(process.id).await.unwrap().unwrap();
    assert!(detail.steps.is_empty());

    let step = db
        .add_process_step_sqlx(process.id, &sample_process_step("Heat"))
        .await
        .unwrap()
        .unwrap();
    db.fail_control_events_after_successes_for_tests(0);
    db.update_process_step_with_audit_sqlx(
        process.id,
        step.id,
        &sample_process_step("Hold"),
        "process_step_updated",
        "operator updated process step",
    )
    .await
    .unwrap_err();
    let detail = db.process_detail_sqlx(process.id).await.unwrap().unwrap();
    assert_eq!(detail.steps.len(), 1);
    assert_eq!(detail.steps[0].name, "Heat");
}

#[tokio::test]
async fn sqlx_recommendation_with_audit_rolls_back_when_audit_fails() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();
    db.fail_control_events_after_successes_for_tests(0);

    db.insert_recommendation_with_audit_sqlx(
        &sample_recommendation("sqlx must roll back with audit"),
        "recommendation_generated",
        "operator regenerated latest AI recommendation",
    )
    .await
    .unwrap_err();

    assert!(db.latest_recommendation_sqlx().await.unwrap().is_none());
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
fn rejects_physically_invalid_sensor_sample_before_sync_insert() {
    let db = Db::open_memory().unwrap();
    let mut invalid = sample(1);
    invalid.pressure_mpa = -0.01;

    let err = db.insert_sample(None, &invalid).unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid sensor sample rejected before DB insert")
            && message.contains("pressure_mpa must be between 0 and 10"),
        "unexpected DB sample rejection: {message}"
    );
    assert!(db.recent_samples(10).unwrap().is_empty());
}

#[test]
fn skips_legacy_invalid_sensor_samples_when_reading_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactor.sqlite3");
    let db = Db::open(&path).unwrap();
    db.insert_sample(None, &sample(1)).unwrap();

    let conn = Connection::open(&path).unwrap();
    conn.execute(
        r#"
        INSERT INTO sensor_samples
            (batch_id, temperature_c, pressure_mpa, stirrer_rpm,
             shake_speed_cpm, tilt_state, tilt_angle_deg, flow_rate_l_min,
             product_concentration_percent, ph, captured_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
        params![
            Option::<i64>::None,
            55.0,
            -0.01,
            240.0,
            24.0,
            1,
            12.5,
            2.2,
            10.0,
            6.8,
            Utc::now().to_rfc3339()
        ],
    )
    .unwrap();
    conn.execute(
        r#"
        INSERT INTO sensor_samples
            (batch_id, temperature_c, pressure_mpa, stirrer_rpm,
             shake_speed_cpm, tilt_state, tilt_angle_deg, flow_rate_l_min,
             product_concentration_percent, ph, captured_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
        params![
            Option::<i64>::None,
            56.0,
            0.2,
            240.0,
            24.0,
            300,
            12.5,
            2.2,
            10.0,
            6.8,
            Utc::now().to_rfc3339()
        ],
    )
    .unwrap();

    let recent = db.recent_samples(10).unwrap();
    assert_eq!(recent.len(), 1);
    assert!(recent.iter().all(|sample| sample.pressure_mpa >= 0.0));
    let records = db.recent_sample_records(10).unwrap();
    assert_eq!(records.len(), 1);
    assert!(records
        .iter()
        .all(|record| record.sample.pressure_mpa >= 0.0));
    let ranged = db
        .samples_between(
            Utc::now() - chrono::Duration::minutes(1),
            Utc::now() + chrono::Duration::minutes(1),
            10,
            0,
        )
        .unwrap();
    assert_eq!(ranged.len(), 1);
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
    assert!(indexes.contains(&"idx_integration_tasks_unique_active_external_task_id".to_string()));
}

#[test]
fn migration_adds_legacy_integration_columns_before_indexes_and_preserves_batches() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy-reactor.sqlite3");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE batches (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            started_at TEXT NOT NULL,
            finished_at TEXT,
            target_temperature_c REAL NOT NULL,
            target_stirrer_rpm REAL NOT NULL,
            heating_minutes REAL NOT NULL,
            stirring_minutes REAL NOT NULL
        );
        INSERT INTO batches
            (name, started_at, finished_at, target_temperature_c,
             target_stirrer_rpm, heating_minutes, stirring_minutes)
        VALUES
            ('legacy-batch-kept', '2026-05-01T00:00:00+00:00',
             '2026-05-01T01:00:00+00:00', 72.0, 320.0, 30.0, 45.0);

        CREATE TABLE integration_tasks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source TEXT NOT NULL,
            action TEXT NOT NULL,
            request_json TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        INSERT INTO integration_tasks (source, action, request_json, created_at)
        VALUES ('legacy_source', 'set_targets', '{"target":72}', '2026-05-01T00:00:00+00:00');
        "#,
    )
    .unwrap();
    drop(conn);

    let db = Db::open(&path).unwrap();
    let batches = db.recent_batches(10).unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].name, "legacy-batch-kept");

    let tasks = db.integration_tasks(Some("legacy_source"), 10).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].status, "failed");
    assert_eq!(tasks[0].request, json!({ "target": 72 }));
    assert_eq!(tasks[0].response["status"], "failed");
    assert_eq!(tasks[0].external_task_id, None);
    assert!(db
        .index_names_for_diagnostics()
        .unwrap()
        .contains(&"idx_integration_tasks_unique_active_external_task_id".to_string()));
}

#[test]
fn migration_preserves_duplicate_legacy_tasks_and_keeps_earliest_idempotency_record() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy-duplicate-tasks.sqlite3");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE integration_tasks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            external_task_id TEXT,
            source TEXT NOT NULL,
            action TEXT NOT NULL,
            status TEXT NOT NULL,
            request_json TEXT NOT NULL,
            response_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );
        INSERT INTO integration_tasks
            (external_task_id, source, action, status, request_json, response_json, created_at, updated_at)
        VALUES
            ('duplicate-1', 'ainas', 'set_targets', 'received', '{}', 'null',
             '2026-05-01T00:00:00+00:00', '2026-05-01T00:00:00+00:00'),
            ('duplicate-1', 'ainas', 'set_targets', 'executed', '{}', '{"status":"executed"}',
             '2026-05-01T00:01:00+00:00', '2026-05-01T00:01:00+00:00');
        "#,
    )
    .unwrap();
    drop(conn);

    let db = Db::open(&path).unwrap();
    let tasks = db.integration_tasks(Some("ainas"), 10).unwrap();
    assert_eq!(tasks.len(), 2);
    let canonical = tasks.iter().find(|task| task.id == 1).unwrap();
    let duplicate = tasks.iter().find(|task| task.id == 2).unwrap();
    assert_eq!(canonical.external_task_id.as_deref(), Some("duplicate-1"));
    assert_eq!(canonical.status, "received");
    assert_eq!(duplicate.external_task_id, None);
    assert_eq!(duplicate.status, "executed");

    let replay = db
        .create_integration_task(
            "ainas",
            Some("duplicate-1"),
            "set_targets",
            &json!({ "target_temperature_c": 72.0 }),
        )
        .unwrap();
    assert_eq!(replay.id, 1);
    assert_eq!(db.integration_tasks(Some("ainas"), 10).unwrap().len(), 2);
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
        db.finish_batch(batch.id).unwrap();
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
fn skips_legacy_invalid_finished_batches_but_errors_on_invalid_unfinished_or_by_id_reads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactor.sqlite3");
    let db = Db::open(&path).unwrap();
    let valid = db
        .create_batch("valid restored batch", 70.0, 400.0, 45.0, 60.0)
        .unwrap();
    db.finish_batch(valid.id).unwrap();
    let invalid_finished = db
        .create_batch("invalid finished restored batch", 71.0, 410.0, 46.0, 61.0)
        .unwrap();
    db.finish_batch(invalid_finished.id).unwrap();
    let invalid_unfinished = db
        .create_batch("invalid unfinished restored batch", 72.0, 420.0, 47.0, 62.0)
        .unwrap();

    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE batches SET target_temperature_c = ?1 WHERE id = ?2",
        params![501.0, invalid_finished.id],
    )
    .unwrap();
    conn.execute(
        "UPDATE batches SET target_stirrer_rpm = ?1 WHERE id = ?2",
        params![2000.01, invalid_unfinished.id],
    )
    .unwrap();

    let recent = db.recent_batches(10).unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].id, valid.id);
    let by_id_err = db.batch_by_id(invalid_finished.id).unwrap_err();
    assert!(
        format!("{by_id_err:#}").contains("invalid batch in database"),
        "unexpected invalid batch read error: {by_id_err:#}"
    );
    let unfinished_err = db.unfinished_batches(10).unwrap_err();
    assert!(
        format!("{unfinished_err:#}").contains("invalid batch in database"),
        "unexpected unfinished batch read error: {unfinished_err:#}"
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

#[test]
fn rejects_invalid_control_event_targets_before_sync_insert() {
    let db = Db::open_memory().unwrap();
    let mut command = sample_command("invalid audit target");
    command.target_temperature_c = 501.0;

    let err = db
        .insert_control_event(
            None,
            "invalid_control_event_target",
            Some(&command),
            "must reject impossible audit target",
        )
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid control event target rejected before DB insert")
            && message.contains("target_temperature_c must be between 0 and 500"),
        "unexpected control event target rejection: {message}"
    );
    assert!(db.recent_control_events(10).unwrap().is_empty());
}

#[test]
fn skips_legacy_invalid_control_events_for_history_but_not_audit_chain_verification() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactor.sqlite3");
    let db = Db::open(&path).unwrap();
    db.insert_control_event(
        None,
        "valid_control_event",
        Some(&sample_command("valid audit target")),
        "valid audit target",
    )
    .unwrap();
    db.insert_control_event(
        None,
        "legacy_invalid_control_event",
        Some(&sample_command("will be corrupted")),
        "legacy invalid audit target",
    )
    .unwrap();

    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE control_events SET target_shake_speed_cpm = ?1 WHERE event_type = ?2",
        params![60.01, "legacy_invalid_control_event"],
    )
    .unwrap();

    let recent = db.recent_control_events(10).unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].event_type, "valid_control_event");
    let audit_events = db.audit_events(10, 0, None).unwrap();
    assert_eq!(audit_events.len(), 1);
    assert_eq!(audit_events[0].event_type, "valid_control_event");
    let chain_err = db.audit_chain_status().unwrap_err();
    assert!(
        format!("{chain_err:#}").contains("invalid control event in database"),
        "unexpected audit chain invalid event error: {chain_err:#}"
    );
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
async fn async_file_database_rejects_invalid_control_event_targets_before_sqlx_insert() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();
    let mut command = sample_command("invalid sqlx audit target");
    command.target_stirrer_rpm = 2000.01;

    let err = db
        .insert_control_event_sqlx(
            None,
            "invalid_sqlx_control_event_target",
            Some(&command),
            "must reject impossible audit target",
        )
        .await
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid control event target rejected before DB insert")
            && message.contains("target_stirrer_rpm must be between 0 and 2000"),
        "unexpected SQLx control event target rejection: {message}"
    );
    assert!(db.recent_control_events_sqlx(10).await.unwrap().is_empty());
}

#[tokio::test]
async fn async_file_database_skips_legacy_invalid_control_events_for_history_but_not_audit_chain_verification(
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactor.sqlite3");
    let db = Db::open(&path).unwrap();
    db.insert_control_event_sqlx(
        None,
        "valid_sqlx_control_event",
        Some(&sample_command("valid sqlx audit target")),
        "valid sqlx audit target",
    )
    .await
    .unwrap();
    db.insert_control_event_sqlx(
        None,
        "legacy_invalid_sqlx_control_event",
        Some(&sample_command("will be corrupted")),
        "legacy invalid sqlx audit target",
    )
    .await
    .unwrap();

    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE control_events SET target_temperature_c = ?1 WHERE event_type = ?2",
        params![501.0, "legacy_invalid_sqlx_control_event"],
    )
    .unwrap();

    let recent = db.recent_control_events_sqlx(10).await.unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].event_type, "valid_sqlx_control_event");
    let audit_events = db.audit_events_sqlx(10, 0, None).await.unwrap();
    assert_eq!(audit_events.len(), 1);
    assert_eq!(audit_events[0].event_type, "valid_sqlx_control_event");
    let chain_err = db.audit_chain_status_sqlx().await.unwrap_err();
    assert!(
        format!("{chain_err:#}").contains("invalid control event in database"),
        "unexpected SQLx audit chain invalid event error: {chain_err:#}"
    );
}

#[tokio::test]
async fn async_file_database_audit_writes_use_sqlx_pool_without_breaking_chain() {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Db::open(dir.path().join("reactor.sqlite3")).unwrap());
    let mut tasks = Vec::new();
    for index in 0..20 {
        let db = Arc::clone(&db);
        tasks.push(tokio::spawn(async move {
            db.insert_control_event_sqlx(
                None,
                "sqlx_audit_write_probe",
                None,
                &format!("event {index}"),
            )
            .await
        }));
    }
    for task in tasks {
        task.await.unwrap().unwrap();
    }

    let status = db.audit_chain_status_sqlx().await.unwrap();
    assert_eq!(status.total_hashed_events, 20);
    assert_eq!(status.checked_events, 20);
    assert_eq!(status.chained_events, 20);
    assert_eq!(status.broken_events, 0);
    assert!(status.valid);
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
        db.finish_batch(batch.id).unwrap();
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
async fn async_file_database_skips_legacy_invalid_finished_batches_but_errors_on_invalid_unfinished_or_by_id_reads(
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactor.sqlite3");
    let db = Db::open(&path).unwrap();
    let valid = db
        .create_batch("valid sqlx restored batch", 70.0, 400.0, 45.0, 60.0)
        .unwrap();
    db.finish_batch(valid.id).unwrap();
    let invalid_finished = db
        .create_batch(
            "invalid sqlx finished restored batch",
            71.0,
            410.0,
            46.0,
            61.0,
        )
        .unwrap();
    db.finish_batch(invalid_finished.id).unwrap();
    let invalid_unfinished = db
        .create_batch(
            "invalid sqlx unfinished restored batch",
            72.0,
            420.0,
            47.0,
            62.0,
        )
        .unwrap();

    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE batches SET heating_minutes = ?1 WHERE id = ?2",
        params![1440.01, invalid_finished.id],
    )
    .unwrap();
    conn.execute(
        "UPDATE batches SET stirring_minutes = ?1 WHERE id = ?2",
        params![1440.01, invalid_unfinished.id],
    )
    .unwrap();

    let recent = db.recent_batches_sqlx(10).await.unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].id, valid.id);
    let by_id_err = db.batch_by_id_sqlx(invalid_finished.id).await.unwrap_err();
    assert!(
        format!("{by_id_err:#}").contains("invalid batch in database"),
        "unexpected SQLx invalid batch read error: {by_id_err:#}"
    );
    let unfinished_err = db.unfinished_batches_sqlx(10).await.unwrap_err();
    assert!(
        format!("{unfinished_err:#}").contains("invalid batch in database"),
        "unexpected SQLx unfinished batch read error: {unfinished_err:#}"
    );
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
async fn async_file_database_rejects_invalid_batch_targets_before_sqlx_insert() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();

    let err = db
        .create_batch_for_process_sqlx(None, "invalid sqlx batch target", 70.0, 2000.01, 36.0, 58.0)
        .await
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid batch target rejected before DB insert")
            && message.contains("target_stirrer_rpm must be between 0 and 2000"),
        "unexpected SQLx batch target rejection: {message}"
    );
    assert!(db.recent_batches_sqlx(10).await.unwrap().is_empty());
}

#[tokio::test]
async fn async_file_database_batch_detail_reads_use_sqlx_pool() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();

    let batch = db
        .create_batch_for_process_sqlx(None, "sqlx detail", 88.5, 460.0, 32.0, 64.0)
        .await
        .unwrap();
    db.finish_batch_sqlx(batch.id).await.unwrap();
    db.insert_product_result_sqlx(&ProductResult {
        batch_id: batch.id,
        yield_percent: 76.5,
        product_ratio: 0.84,
        notes: "detail outcome".to_string(),
    })
    .await
    .unwrap();
    for index in 0..4 {
        db.insert_control_event_sqlx(
            Some(batch.id),
            "sqlx_batch_detail_probe",
            None,
            &format!("batch event {index}"),
        )
        .await
        .unwrap();
    }

    let loaded_batch = db.batch_by_id_sqlx(batch.id).await.unwrap().unwrap();
    assert_eq!(loaded_batch.id, batch.id);
    assert_eq!(loaded_batch.name, "sqlx detail");
    assert_eq!(loaded_batch.target_temperature_c, 88.5);

    let outcome = db
        .batch_outcome_by_id_sqlx(batch.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome.batch_id, batch.id);
    assert_eq!(outcome.yield_percent, 76.5);
    assert_eq!(outcome.product_ratio, 0.84);

    let recent_events = db.recent_control_events_sqlx(2).await.unwrap();
    assert_eq!(
        recent_events
            .iter()
            .map(|event| event.reason.as_str())
            .collect::<Vec<_>>(),
        vec!["batch event 2", "batch event 3"]
    );

    let batch_events = db.control_events_for_batch_sqlx(batch.id, 3).await.unwrap();
    assert_eq!(
        batch_events
            .iter()
            .map(|event| event.reason.as_str())
            .collect::<Vec<_>>(),
        vec!["batch event 1", "batch event 2", "batch event 3"]
    );
}

#[tokio::test]
async fn async_file_database_demo_alarm_reads_use_sqlx_pool() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();

    for index in 0..3 {
        db.insert_demo_alarm(
            "threshold",
            "temperature",
            "warning",
            &format!("demo alarm {index}"),
            Some(80.0 + index as f64),
            Some(90.0),
            "inspect reactor",
        )
        .unwrap();
    }

    let alarms = db.recent_demo_alarms_sqlx(2).await.unwrap();
    assert_eq!(
        alarms
            .iter()
            .map(|alarm| alarm.message.as_str())
            .collect::<Vec<_>>(),
        vec!["demo alarm 1", "demo alarm 2"]
    );
    assert!(alarms.iter().all(|alarm| alarm.active));
}

#[tokio::test]
async fn async_file_database_process_configuration_uses_sqlx_pool() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();

    let process = db
        .create_process_sqlx("sqlx process", "created through SQLx")
        .await
        .unwrap();
    assert_eq!(process.step_count, 0);
    assert_eq!(process.status, "draft");

    let first_step = NewProcessStep {
        name: "Heat".to_string(),
        target_temperature_c: 72.0,
        ramp_rate_c_min: 2.5,
        duration_minutes: 12.0,
        target_stirrer_rpm: 320.0,
        target_shake_speed_cpm: 18.0,
        target_pressure_mpa: 0.4,
        cooling_mode: "natural".to_string(),
    };
    let step = db
        .add_process_step_sqlx(process.id, &first_step)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(step.step_index, 1);
    assert_eq!(step.name, "Heat");

    let updated_step = NewProcessStep {
        name: "Hold".to_string(),
        target_temperature_c: 74.0,
        ramp_rate_c_min: 1.5,
        duration_minutes: 20.0,
        target_stirrer_rpm: 360.0,
        target_shake_speed_cpm: 22.0,
        target_pressure_mpa: 0.5,
        cooling_mode: "forced".to_string(),
    };
    let step = db
        .update_process_step_sqlx(process.id, step.id, &updated_step)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(step.name, "Hold");
    assert_eq!(step.target_temperature_c, 74.0);

    let process = db
        .update_process_sqlx(process.id, "sqlx process v2", "updated", "ready")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(process.name, "sqlx process v2");
    assert_eq!(process.status, "ready");
    assert_eq!(process.version, 2);
    assert_eq!(process.step_count, 1);

    let applied = db
        .mark_process_applied_sqlx(process.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(applied.status, "applied");
    assert!(applied.applied_at.is_some());

    let detail = db.process_detail_sqlx(process.id).await.unwrap().unwrap();
    assert_eq!(detail.process.step_count, 1);
    assert_eq!(detail.steps.len(), 1);
    assert_eq!(detail.steps[0].name, "Hold");

    let processes = db.list_processes_sqlx().await.unwrap();
    assert_eq!(processes.len(), 1);
    assert_eq!(processes[0].id, process.id);
    assert_eq!(processes[0].status, "applied");
}

#[tokio::test]
async fn async_file_database_rejects_invalid_process_step_before_sqlx_insert_or_update() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();
    let process = db
        .create_process_sqlx("sqlx process guard", "DB input guard")
        .await
        .unwrap();
    let mut invalid = sample_process_step("Invalid SQLx");
    invalid.target_pressure_mpa = 10.01;

    let err = db
        .add_process_step_with_audit_sqlx(
            process.id,
            &invalid,
            "process_step_added",
            "must not audit invalid step",
        )
        .await
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid process step rejected before DB insert")
            && message.contains("target_pressure_mpa must be between 0 and 10"),
        "unexpected SQLx process step rejection: {message}"
    );
    assert!(db
        .process_detail_sqlx(process.id)
        .await
        .unwrap()
        .unwrap()
        .steps
        .is_empty());

    let step = db
        .add_process_step_sqlx(process.id, &sample_process_step("Valid SQLx"))
        .await
        .unwrap()
        .unwrap();
    invalid.target_pressure_mpa = 0.4;
    invalid.ramp_rate_c_min = 20.01;
    let err = db
        .update_process_step_sqlx(process.id, step.id, &invalid)
        .await
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid process step rejected before DB insert")
            && message.contains("ramp_rate_c_min must be between -20 and 20"),
        "unexpected SQLx process step update rejection: {message}"
    );
    let detail = db.process_detail_sqlx(process.id).await.unwrap().unwrap();
    assert_eq!(detail.steps.len(), 1);
    assert_eq!(detail.steps[0].name, "Valid SQLx");
    assert!(db.recent_control_events_sqlx(10).await.unwrap().is_empty());
}

#[tokio::test]
async fn async_file_database_rejects_legacy_invalid_process_step_when_reading_detail() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactor.sqlite3");
    let db = Db::open(&path).unwrap();
    let process = db
        .create_process_sqlx("legacy sqlx process step guard", "DB input guard")
        .await
        .unwrap();
    let step = db
        .add_process_step_sqlx(process.id, &sample_process_step("Restored SQLx Step"))
        .await
        .unwrap()
        .unwrap();

    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE process_steps SET duration_minutes = ?1 WHERE id = ?2",
        params![0.0, step.id],
    )
    .unwrap();

    let err = db.process_detail_sqlx(process.id).await.unwrap_err();
    assert!(
        format!("{err:#}").contains("invalid process step in database"),
        "unexpected SQLx process detail read error: {err:#}"
    );
}

#[tokio::test]
async fn concurrent_sqlx_process_step_inserts_keep_ordered_indexes() {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Db::open(dir.path().join("reactor.sqlite3")).unwrap());
    let process = db
        .create_process_sqlx("concurrent sqlx process", "step ordering")
        .await
        .unwrap();

    let mut tasks = Vec::new();
    for index in 0..12 {
        let db = Arc::clone(&db);
        tasks.push(tokio::spawn(async move {
            db.add_process_step_sqlx(
                process.id,
                &NewProcessStep {
                    name: format!("Step {index}"),
                    target_temperature_c: 60.0 + index as f64,
                    ramp_rate_c_min: 1.0,
                    duration_minutes: 5.0,
                    target_stirrer_rpm: 200.0,
                    target_shake_speed_cpm: 10.0,
                    target_pressure_mpa: 0.2,
                    cooling_mode: "natural".to_string(),
                },
            )
            .await
        }));
    }
    for task in tasks {
        task.await.unwrap().unwrap().unwrap();
    }

    let detail = db.process_detail_sqlx(process.id).await.unwrap().unwrap();
    let indexes = detail
        .steps
        .iter()
        .map(|step| step.step_index)
        .collect::<Vec<_>>();
    assert_eq!(indexes, (1..=12).collect::<Vec<_>>());
    assert_eq!(detail.process.step_count, 12);
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
        db.finish_batch(batch.id).unwrap();
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
async fn async_file_database_skips_legacy_invalid_product_results_when_reading_sqlx_outcomes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactor.sqlite3");
    let db = Db::open(&path).unwrap();
    let valid = db
        .create_batch("valid sqlx legacy outcome", 70.0, 400.0, 45.0, 60.0)
        .unwrap();
    db.finish_batch(valid.id).unwrap();
    db.insert_product_result(&ProductResult {
        batch_id: valid.id,
        yield_percent: 72.5,
        product_ratio: 0.82,
        notes: "valid sqlx legacy outcome".to_string(),
    })
    .unwrap();
    let invalid = db
        .create_batch("invalid sqlx legacy outcome", 71.0, 410.0, 46.0, 61.0)
        .unwrap();
    db.finish_batch(invalid.id).unwrap();
    db.insert_product_result(&ProductResult {
        batch_id: invalid.id,
        yield_percent: 73.5,
        product_ratio: 0.83,
        notes: "will be corrupted".to_string(),
    })
    .unwrap();

    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "UPDATE product_results SET product_ratio = ?1 WHERE batch_id = ?2",
        params![1.01, invalid.id],
    )
    .unwrap();

    let outcomes = db.batch_outcomes_sqlx().await.unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].batch_id, valid.id);
    let recent = db.recent_batch_outcomes_sqlx(10).await.unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].batch_id, valid.id);
    assert!(db
        .batch_outcome_by_id_sqlx(invalid.id)
        .await
        .unwrap()
        .is_none());
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
async fn async_file_database_rejects_invalid_sensor_sample_before_sqlx_insert() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();
    let mut invalid = sample(2);
    invalid.ph = 14.01;

    let err = db.insert_sample_sqlx(None, &invalid).await.unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid sensor sample rejected before DB insert")
            && message.contains("ph must be between 0 and 14"),
        "unexpected SQLx sample rejection: {message}"
    );
    assert!(db.recent_samples(10).unwrap().is_empty());
}

#[tokio::test]
async fn async_file_database_skips_legacy_invalid_sensor_samples_when_reading_sqlx_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactor.sqlite3");
    let db = Db::open(&path).unwrap();
    db.insert_sample(None, &sample(1)).unwrap();

    let conn = Connection::open(&path).unwrap();
    conn.execute(
        r#"
        INSERT INTO sensor_samples
            (batch_id, temperature_c, pressure_mpa, stirrer_rpm,
             shake_speed_cpm, tilt_state, tilt_angle_deg, flow_rate_l_min,
             product_concentration_percent, ph, captured_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
        params![
            Option::<i64>::None,
            55.0,
            0.2,
            240.0,
            24.0,
            1,
            12.5,
            2.2,
            10.0,
            14.01,
            Utc::now().to_rfc3339()
        ],
    )
    .unwrap();
    conn.execute(
        r#"
        INSERT INTO sensor_samples
            (batch_id, temperature_c, pressure_mpa, stirrer_rpm,
             shake_speed_cpm, tilt_state, tilt_angle_deg, flow_rate_l_min,
             product_concentration_percent, ph, captured_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
        params![
            Option::<i64>::None,
            56.0,
            0.2,
            240.0,
            24.0,
            300,
            12.5,
            2.2,
            10.0,
            6.8,
            Utc::now().to_rfc3339()
        ],
    )
    .unwrap();

    let recent = db.recent_sample_records_sqlx(10).await.unwrap();
    assert_eq!(recent.len(), 1);
    assert!(recent.iter().all(|record| record.sample.ph <= 14.0));
    let ranged = db
        .samples_between_sqlx(
            Utc::now() - chrono::Duration::minutes(1),
            Utc::now() + chrono::Duration::minutes(1),
            10,
            0,
        )
        .await
        .unwrap();
    assert_eq!(ranged.len(), 1);
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
async fn async_file_database_rejects_invalid_recommendation_before_sqlx_insert() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();
    let mut recommendation = sample_recommendation("invalid sqlx recommendation");
    recommendation.target_stirrer_rpm = 2000.01;

    let err = db
        .insert_recommendation_sqlx(&recommendation)
        .await
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid AI recommendation rejected before DB insert")
            && message.contains("target_stirrer_rpm must be between 0 and 2000"),
        "unexpected SQLx recommendation rejection: {message}"
    );
    assert!(db.latest_recommendation_sqlx().await.unwrap().is_none());
}

#[tokio::test]
async fn async_file_database_ignores_legacy_invalid_latest_recommendation_cache() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactor.sqlite3");
    let db = Db::open(&path).unwrap();
    db.insert_recommendation(&sample_recommendation("valid sqlx cached recommendation"))
        .unwrap();

    let conn = Connection::open(&path).unwrap();
    conn.execute(
        r#"
        INSERT INTO ai_recommendations
            (based_on_batch_count, target_temperature_c, target_stirrer_rpm,
             heating_minutes, stirring_minutes, expected_score, rationale, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
        params![
            2_i64,
            501.0,
            420.0,
            35.0,
            55.0,
            86.0,
            "legacy invalid sqlx cached recommendation",
            Utc::now().to_rfc3339()
        ],
    )
    .unwrap();

    assert!(db.latest_recommendation_sqlx().await.unwrap().is_none());
}

#[tokio::test]
async fn async_file_database_product_result_writes_use_sqlx_pool() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();
    let batch = db
        .create_batch("sqlx outcome write", 77.0, 430.0, 41.0, 62.0)
        .unwrap();
    db.finish_batch(batch.id).unwrap();

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

#[tokio::test]
async fn async_file_database_rejects_invalid_product_result_before_sqlx_insert() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();
    let batch = db
        .create_batch("invalid sqlx result", 70.0, 400.0, 45.0, 60.0)
        .unwrap();
    db.finish_batch(batch.id).unwrap();

    let err = db
        .insert_product_result_sqlx(&ProductResult {
            batch_id: batch.id,
            yield_percent: 72.5,
            product_ratio: -0.01,
            notes: "bad ratio".to_string(),
        })
        .await
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid product result rejected before DB insert")
            && message.contains("product_ratio must be between 0 and 1"),
        "unexpected SQLx product result rejection: {message}"
    );
    assert!(db.batch_outcomes_sqlx().await.unwrap().is_empty());
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

#[tokio::test]
async fn async_file_database_integration_task_mark_executing_is_one_way_from_received() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();
    let task = db
        .create_integration_task_sqlx(
            "mqtt",
            Some("mqtt-executing-db"),
            "set_targets",
            &json!({"reason": "start execution"}),
        )
        .await
        .unwrap();

    let executing = db
        .mark_integration_task_executing_sqlx(task.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(executing.status, "executing");
    assert_eq!(executing.response["status"], "executing");

    db.update_integration_task_sqlx(task.id, "executed", &json!({"status": "executed"}))
        .await
        .unwrap();
    let executed = db
        .mark_integration_task_executing_sqlx(task.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(executed.status, "executed");
}

#[tokio::test]
async fn async_file_database_integration_task_create_is_idempotent_by_external_id() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();

    let first = db
        .create_integration_task_sqlx(
            "mqtt",
            Some("mqtt-idempotent-db"),
            "set_targets",
            &json!({"reason": "first delivery"}),
        )
        .await
        .unwrap();
    db.update_integration_task_sqlx(
        first.id,
        "executed",
        &json!({"status": "executed", "targets": {"temperature_c": 72.5}}),
    )
    .await
    .unwrap();

    let replay = db
        .create_integration_task_sqlx(
            "mqtt",
            Some("mqtt-idempotent-db"),
            "set_targets",
            &json!({"reason": "duplicate delivery"}),
        )
        .await
        .unwrap();

    assert_eq!(replay.id, first.id);
    assert_eq!(replay.status, "executed");
    assert_eq!(replay.request["reason"], "first delivery");
    let tasks = db.integration_tasks_sqlx(Some("mqtt"), 10).await.unwrap();
    assert_eq!(tasks.len(), 1);
}

#[test]
fn rejects_invalid_integration_task_before_sync_insert() {
    let db = Db::open_memory().unwrap();

    let err = db
        .create_integration_task(
            "mqtt",
            Some(" invalid\nexternal\u{200B}id "),
            "set_targets",
            &json!({"reason": "bad external id"}),
        )
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid integration task rejected before DB insert")
            && message.contains("external_task_id"),
        "unexpected integration task external id rejection: {message}"
    );

    let err = db
        .create_integration_task(
            "mqtt",
            Some("invalid-action"),
            "open_valve",
            &json!({"reason": "bad action"}),
        )
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid integration task rejected before DB insert")
            && message.contains("action must be one of"),
        "unexpected integration task action rejection: {message}"
    );

    let err = db
        .create_integration_task("MQTT", Some("invalid-source"), "set_targets", &json!({}))
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid integration task rejected before DB insert")
            && message.contains("source must contain only lowercase ASCII"),
        "unexpected integration task source rejection: {message}"
    );

    let err = db
        .create_integration_task(
            "mqtt",
            Some("invalid-request"),
            "set_targets",
            &json!(["bad"]),
        )
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid integration task rejected before DB insert")
            && message.contains("request JSON must be an object"),
        "unexpected integration task request rejection: {message}"
    );

    assert!(db.integration_tasks(None, 10).unwrap().is_empty());
}

#[test]
fn rejects_invalid_integration_task_update_before_sync_insert() {
    let db = Db::open_memory().unwrap();
    let task = db
        .create_integration_task(
            "mqtt",
            Some("mqtt-update-guard"),
            "set_targets",
            &json!({"reason": "valid"}),
        )
        .unwrap();

    let err = db
        .update_integration_task(task.id, "executing", &json!({"status": "executing"}))
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid integration task update rejected before DB insert")
            && message.contains("status update must be one of"),
        "unexpected integration task status update rejection: {message}"
    );

    let err = db
        .update_integration_task(task.id, "executed", &Value::Null)
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid integration task update rejected before DB insert")
            && message.contains("response JSON must be an object"),
        "unexpected integration task response rejection: {message}"
    );

    let stored = db.integration_task(task.id).unwrap().unwrap();
    assert_eq!(stored.status, "received");
    assert!(stored.response.is_null());
}

#[test]
fn skips_legacy_invalid_integration_tasks_in_history_but_not_replay_reads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactor.sqlite3");
    let db = Db::open(&path).unwrap();
    let valid = db
        .create_integration_task(
            "mqtt",
            Some("mqtt-valid-history"),
            "set_targets",
            &json!({"reason": "valid"}),
        )
        .unwrap();

    let now = Utc::now().to_rfc3339();
    let invalid_id = {
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            r#"
            INSERT INTO integration_tasks
                (external_task_id, source, action, status, request_json, response_json, created_at, updated_at)
            VALUES ('mqtt-invalid-history', 'mqtt', 'open_valve', 'received', '{}', 'null', ?1, ?1)
            "#,
            params![now],
        )
        .unwrap();
        conn.last_insert_rowid()
    };

    let history = db.integration_tasks(Some("mqtt"), 10).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, valid.id);

    let by_id_err = db.integration_task(invalid_id).unwrap_err();
    assert!(
        format!("{by_id_err:#}").contains("invalid integration task in database"),
        "invalid integration task by-id read must fail closed: {by_id_err:#}"
    );

    let replay_err = db
        .create_integration_task(
            "mqtt",
            Some("mqtt-invalid-history"),
            "set_targets",
            &json!({"reason": "must not execute as new"}),
        )
        .unwrap_err();
    assert!(
        format!("{replay_err:#}").contains("invalid integration task in database"),
        "invalid integration task external-id replay must fail closed: {replay_err:#}"
    );
}

#[tokio::test]
async fn async_file_database_rejects_invalid_integration_task_before_sqlx_insert() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();

    let err = db
        .create_integration_task_sqlx(
            "mqtt",
            Some("sqlx-invalid-external\tid"),
            "set_targets",
            &json!({"reason": "bad external id"}),
        )
        .await
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid integration task rejected before DB insert")
            && message.contains("external_task_id"),
        "unexpected SQLx integration task external id rejection: {message}"
    );

    let err = db
        .create_integration_task_sqlx(
            "mqtt",
            Some("sqlx-invalid-action"),
            "open_valve",
            &json!({"reason": "bad action"}),
        )
        .await
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid integration task rejected before DB insert")
            && message.contains("action must be one of"),
        "unexpected SQLx integration task action rejection: {message}"
    );

    assert!(db
        .integration_tasks_sqlx(None, 10)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn async_file_database_rejects_invalid_integration_task_update_before_sqlx_insert() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("reactor.sqlite3")).unwrap();
    let task = db
        .create_integration_task_sqlx(
            "ainas",
            Some("ainas-update-guard"),
            "stop_process",
            &json!({"reason": "valid"}),
        )
        .await
        .unwrap();

    let err = db
        .update_integration_task_sqlx(task.id, "received", &Value::Null)
        .await
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid integration task update rejected before DB insert")
            && message.contains("status update must be one of"),
        "unexpected SQLx integration task status update rejection: {message}"
    );

    let err = db
        .update_integration_task_sqlx(task.id, "failed", &Value::Null)
        .await
        .unwrap_err();
    let message = format!("{err:#}");
    assert!(
        message.contains("invalid integration task update rejected before DB insert")
            && message.contains("response JSON must be an object"),
        "unexpected SQLx integration task response rejection: {message}"
    );

    let stored = db.integration_task_sqlx(task.id).await.unwrap().unwrap();
    assert_eq!(stored.status, "received");
    assert!(stored.response.is_null());
}

#[tokio::test]
async fn async_file_database_skips_legacy_invalid_integration_tasks_in_history_but_not_replay_reads(
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactor.sqlite3");
    let db = Db::open(&path).unwrap();
    let valid = db
        .create_integration_task_sqlx(
            "ainas",
            Some("ainas-valid-history"),
            "set_targets",
            &json!({"reason": "valid"}),
        )
        .await
        .unwrap();

    let now = Utc::now().to_rfc3339();
    let invalid_id = {
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            r#"
            INSERT INTO integration_tasks
                (external_task_id, source, action, status, request_json, response_json, created_at, updated_at)
            VALUES ('ainas-invalid-history', 'ainas', 'set_targets', 'paused', '{}', 'null', ?1, ?1)
            "#,
            params![now],
        )
        .unwrap();
        conn.last_insert_rowid()
    };

    let history = db.integration_tasks_sqlx(Some("ainas"), 10).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, valid.id);

    let by_id_err = db.integration_task_sqlx(invalid_id).await.unwrap_err();
    assert!(
        format!("{by_id_err:#}").contains("invalid integration task in database"),
        "invalid SQLx integration task by-id read must fail closed: {by_id_err:#}"
    );

    let replay_err = db
        .create_integration_task_sqlx(
            "ainas",
            Some("ainas-invalid-history"),
            "set_targets",
            &json!({"reason": "must not execute as new"}),
        )
        .await
        .unwrap_err();
    assert!(
        format!("{replay_err:#}").contains("invalid integration task in database"),
        "invalid SQLx integration task external-id replay must fail closed: {replay_err:#}"
    );
}

#[test]
fn integration_task_active_external_id_is_unique_per_source_in_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactor.sqlite3");
    let _db = Db::open(&path).unwrap();
    let conn = Connection::open(&path).unwrap();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        r#"
        INSERT INTO integration_tasks
            (external_task_id, source, action, status, request_json, response_json, created_at, updated_at)
        VALUES (?1, 'mqtt', 'set_targets', 'received', '{}', 'null', ?2, ?2)
        "#,
        params!["mqtt-db-unique", now],
    )
    .unwrap();

    let err = conn
        .execute(
            r#"
            INSERT INTO integration_tasks
                (external_task_id, source, action, status, request_json, response_json, created_at, updated_at)
            VALUES (?1, 'mqtt', 'set_targets', 'received', '{}', 'null', ?2, ?2)
            "#,
            params!["mqtt-db-unique", now],
        )
        .unwrap_err();

    assert!(
        err.to_string().contains("UNIQUE constraint failed"),
        "unexpected duplicate external task id error: {err}"
    );

    conn.execute(
        r#"
        INSERT INTO integration_tasks
            (external_task_id, source, action, status, request_json, response_json, created_at, updated_at)
        VALUES (?1, 'mqtt', 'set_targets', 'failed', '{}', 'null', ?2, ?2)
        "#,
        params!["mqtt-failed-duplicate", now],
    )
    .unwrap();
    conn.execute(
        r#"
        INSERT INTO integration_tasks
            (external_task_id, source, action, status, request_json, response_json, created_at, updated_at)
        VALUES (?1, 'mqtt', 'set_targets', 'rejected', '{}', 'null', ?2, ?2)
        "#,
        params!["mqtt-failed-duplicate", now],
    )
    .unwrap();
}

#[test]
fn migration_allows_legacy_terminal_duplicate_external_task_ids_but_blocks_new_active_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactor.sqlite3");
    let now = Utc::now().to_rfc3339();
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE integration_tasks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                external_task_id TEXT,
                source TEXT NOT NULL,
                action TEXT NOT NULL,
                status TEXT NOT NULL,
                request_json TEXT NOT NULL,
                response_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            "#,
        )
        .unwrap();
        for status in ["failed", "rejected"] {
            conn.execute(
                r#"
                INSERT INTO integration_tasks
                    (external_task_id, source, action, status, request_json, response_json, created_at, updated_at)
                VALUES ('legacy-terminal-duplicate', 'mqtt', 'set_targets', ?1, '{}', 'null', ?2, ?2)
                "#,
                params![status, now],
            )
            .unwrap();
        }
    }

    let _db = Db::open(&path).unwrap();
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        r#"
        INSERT INTO integration_tasks
            (external_task_id, source, action, status, request_json, response_json, created_at, updated_at)
        VALUES ('new-active-duplicate', 'mqtt', 'set_targets', 'executed', '{}', 'null', ?1, ?1)
        "#,
        params![now],
    )
    .unwrap();
    let err = conn
        .execute(
            r#"
            INSERT INTO integration_tasks
                (external_task_id, source, action, status, request_json, response_json, created_at, updated_at)
            VALUES ('new-active-duplicate', 'mqtt', 'set_targets', 'received', '{}', 'null', ?1, ?1)
            "#,
            params![now],
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("UNIQUE constraint failed"),
        "unexpected active duplicate external task id error: {err}"
    );
}

#[tokio::test]
async fn concurrent_sqlx_integration_task_creates_share_external_task_id() {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Db::open(dir.path().join("reactor.sqlite3")).unwrap());
    let mut tasks = Vec::new();
    for index in 0..12 {
        let db = Arc::clone(&db);
        tasks.push(tokio::spawn(async move {
            db.create_integration_task_sqlx(
                "mqtt",
                Some("mqtt-concurrent-idempotent"),
                "set_targets",
                &json!({"reason": format!("delivery {index}")}),
            )
            .await
        }));
    }

    let mut ids = Vec::new();
    for task in tasks {
        ids.push(task.await.unwrap().unwrap().id);
    }
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 1);
    let stored = db.integration_tasks_sqlx(Some("mqtt"), 10).await.unwrap();
    assert_eq!(stored.len(), 1);
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

#[tokio::test]
async fn backup_and_restore_round_trip_preserves_data_and_schema() {
    use std::path::PathBuf;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let db_path: PathBuf = dir.path().join("reactor.sqlite3");
    let backup_path: PathBuf = dir.path().join("reactor.sqlite3.backup");

    // 1. Open a real Db, write a sample + a process step, take a backup.
    let db = Db::open(&db_path).unwrap();
    db.insert_sample(None, &sample(1)).unwrap();
    let process = db
        .create_process("backup-roundtrip", "online VACUUM INTO acceptance")
        .unwrap();
    let pid = process.id;
    let report = db.backup_to(&backup_path).unwrap();
    assert!(report.size_bytes > 0, "backup file must be non-empty");
    assert!(report.sha256.len() == 64, "sha256 must be 64 hex chars");
    assert!(backup_path.is_file(), "backup file must exist");
    drop(db);

    // 2. Wipe the main file and confirm the record is gone.
    std::fs::remove_file(&db_path).unwrap();
    assert!(!db_path.exists(), "main db must be gone before restore");

    // 3. Restore through the same static restore path used by the CLI.
    let stale_wal = dir.path().join("reactor.sqlite3-wal");
    std::fs::write(&stale_wal, b"stale wal").unwrap();
    let restore_report = Db::restore_file(&backup_path, &db_path, true).unwrap();
    assert_eq!(restore_report.integrity_check, "ok");
    assert_eq!(
        restore_report.sha256, report.sha256,
        "restore_file should copy the backup image byte-for-byte before daemon migrations reopen it"
    );
    assert!(restore_report.tables.iter().any(|name| name == "processes"));
    assert!(
        !stale_wal.exists(),
        "restore_file must remove stale WAL sidecar before replacement"
    );
    let preserved_wal = dir.path().join("reactor.sqlite3-wal.pre-restore");
    assert_eq!(
        std::fs::read(&preserved_wal).unwrap(),
        b"stale wal",
        "restore_file must preserve stale WAL evidence before removing it"
    );
    assert!(
        restore_report
            .preserved_sidecars
            .iter()
            .any(|path| path == &preserved_wal.display().to_string()),
        "restore report should expose preserved sidecar evidence"
    );
    assert!(
        restore_report
            .removed_sidecars
            .iter()
            .any(|path| path == &stale_wal.display().to_string()),
        "restore report should expose removed live sidecar path"
    );
    {
        let restored = Db::open(&db_path).unwrap();
        let processes = restored.list_processes().unwrap();
        assert_eq!(processes.len(), 1, "restored process row must survive");
        assert_eq!(processes[0].id, pid);
    }

    // 4. restore_file also needs to refuse non-SQLite files and refuse
    //    overwrite=false when the target exists.
    let err = Db::restore_file(&backup_path, &db_path, false).unwrap_err();
    assert!(
        err.to_string().contains("without overwrite"),
        "restore_file must require overwrite=true for existing targets"
    );
    std::fs::remove_file(&db_path).unwrap();
    let bogus = dir.path().join("bogus.bin");
    std::fs::write(&bogus, b"not a sqlite database").unwrap();
    let err = Db::restore_file(&bogus, &db_path, true).unwrap_err();
    assert!(
        err.to_string().contains("magic header"),
        "restore_file must reject non-SQLite input"
    );
}

#[test]
fn restore_file_does_not_overwrite_existing_pre_restore_evidence() {
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let target_path = dir.path().join("reactor.sqlite3");
    let source_path = dir.path().join("source.sqlite3");
    let backup_path = dir.path().join("source.sqlite3.snapshot");
    let existing_pre_restore = dir.path().join("reactor.sqlite3.pre-restore");
    let existing_wal_pre_restore = dir.path().join("reactor.sqlite3-wal.pre-restore");
    let live_wal = dir.path().join("reactor.sqlite3-wal");

    let target_db = Db::open(&target_path).unwrap();
    target_db
        .create_process("restore-existing-evidence-target", "target evidence")
        .unwrap();
    drop(target_db);
    let target_before = std::fs::read(&target_path).unwrap();

    let source_db = Db::open(&source_path).unwrap();
    source_db
        .create_process("restore-existing-evidence-source", "replacement")
        .unwrap();
    let report = source_db.backup_to(&backup_path).unwrap();
    drop(source_db);

    std::fs::write(&existing_pre_restore, b"old main evidence").unwrap();
    std::fs::write(&existing_wal_pre_restore, b"old wal evidence").unwrap();
    std::fs::write(&live_wal, b"new live wal evidence").unwrap();

    let restore_report = Db::restore_file(&backup_path, &target_path, true).unwrap();

    assert_eq!(
        std::fs::read(&existing_pre_restore).unwrap(),
        b"old main evidence",
        "restore must not overwrite an existing main pre-restore evidence file"
    );
    assert_eq!(
        std::fs::read(&existing_wal_pre_restore).unwrap(),
        b"old wal evidence",
        "restore must not overwrite an existing WAL pre-restore evidence file"
    );
    let new_main_evidence = dir.path().join("reactor.sqlite3.pre-restore.1");
    let new_wal_evidence = dir.path().join("reactor.sqlite3-wal.pre-restore.1");
    assert_eq!(
        std::fs::read(&new_main_evidence).unwrap(),
        target_before,
        "restore should allocate a new main evidence path"
    );
    assert_eq!(
        std::fs::read(&new_wal_evidence).unwrap(),
        b"new live wal evidence",
        "restore should allocate a new sidecar evidence path"
    );
    let expected_main_evidence = new_main_evidence.display().to_string();
    assert_eq!(
        restore_report.preserved_existing.as_deref(),
        Some(expected_main_evidence.as_str())
    );
    assert!(
        restore_report
            .preserved_sidecars
            .iter()
            .any(|path| path == &new_wal_evidence.display().to_string()),
        "restore report should expose the non-overwriting sidecar evidence path"
    );
    assert_eq!(
        restore_report.sha256, report.sha256,
        "restore should still publish the requested backup"
    );
}

#[test]
fn restore_file_cleans_restore_tmp_when_pre_restore_preservation_fails() {
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let target_path = dir.path().join("reactor.sqlite3");
    let source_path = dir.path().join("source.sqlite3");
    let backup_path = dir.path().join("source.sqlite3.snapshot");
    let live_wal = dir.path().join("reactor.sqlite3-wal");
    let evidence_tmp_dir = dir.path().join(format!(
        "reactor.sqlite3.pre-restore.evidence.tmp.{}",
        std::process::id()
    ));

    let target_db = Db::open(&target_path).unwrap();
    target_db
        .create_process("restore-preserve-failure-target", "target evidence")
        .unwrap();
    drop(target_db);
    let target_before = std::fs::read(&target_path).unwrap();
    std::fs::write(&live_wal, b"live wal evidence").unwrap();

    let source_db = Db::open(&source_path).unwrap();
    source_db
        .create_process("restore-preserve-failure-source", "replacement")
        .unwrap();
    source_db.backup_to(&backup_path).unwrap();
    drop(source_db);

    std::fs::create_dir(&evidence_tmp_dir).unwrap();
    let err = Db::restore_file(&backup_path, &target_path, true).unwrap_err();
    assert!(
        err.to_string()
            .contains("failed to remove stale temporary restore file"),
        "restore should fail before publishing if the pre-restore evidence temp path is unsafe: {err}"
    );
    assert_eq!(
        std::fs::read(&target_path).unwrap(),
        target_before,
        "failed evidence preservation must leave the target DB intact"
    );
    assert_eq!(
        std::fs::read(&live_wal).unwrap(),
        b"live wal evidence",
        "failed evidence preservation must not remove the live WAL"
    );
    assert!(
        !has_restore_tmp_file(dir.path()),
        "failed evidence preservation must clean the validated restore tmp file"
    );
    assert!(
        evidence_tmp_dir.is_dir(),
        "restore must not delete an unsafe pre-existing evidence temp directory"
    );
}

#[test]
fn restore_file_validates_temp_copy_before_publishing_over_target() {
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let target_path = dir.path().join("reactor.sqlite3");
    let valid_source_path = dir.path().join("valid-source.sqlite3");
    let corrupt_source_path = dir.path().join("corrupt-source.sqlite3");

    let target_db = Db::open(&target_path).unwrap();
    let target_process = target_db
        .create_process("restore-target", "must survive failed restore")
        .unwrap();
    drop(target_db);
    let target_before = std::fs::read(&target_path).unwrap();
    let wal_path = dir.path().join("reactor.sqlite3-wal");
    let shm_path = dir.path().join("reactor.sqlite3-shm");
    let journal_path = dir.path().join("reactor.sqlite3-journal");
    std::fs::write(&wal_path, b"wal evidence").unwrap();
    std::fs::write(&shm_path, b"shm evidence").unwrap();
    std::fs::write(&journal_path, b"journal evidence").unwrap();

    let source_db = Db::open(&valid_source_path).unwrap();
    source_db.insert_sample(None, &sample(2)).unwrap();
    drop(source_db);
    let mut corrupt = std::fs::read(&valid_source_path).unwrap();
    corrupt.truncate(corrupt.len() / 2);
    std::fs::write(&corrupt_source_path, corrupt).unwrap();

    let err = Db::restore_file(&corrupt_source_path, &target_path, true).unwrap_err();
    assert!(
        err.to_string().contains("integrity_check")
            || err.to_string().contains("database disk image is malformed")
            || err.to_string().contains("file is not a database"),
        "restore_file should reject a corrupt temporary copy before publish: {err}"
    );
    assert_eq!(
        std::fs::read(&target_path).unwrap(),
        target_before,
        "failed restore must leave the existing target database byte-for-byte intact"
    );
    assert!(
        !has_restore_tmp_file(dir.path()),
        "failed restore must clean temporary restore files"
    );
    assert_eq!(
        std::fs::read(&wal_path).unwrap(),
        b"wal evidence",
        "failed restore must not remove target WAL evidence before backup validation succeeds"
    );
    assert_eq!(
        std::fs::read(&shm_path).unwrap(),
        b"shm evidence",
        "failed restore must not remove target SHM evidence before backup validation succeeds"
    );
    assert_eq!(
        std::fs::read(&journal_path).unwrap(),
        b"journal evidence",
        "failed restore must not remove target JOURNAL evidence before backup validation succeeds"
    );
    assert!(
        !dir.path().join("reactor.sqlite3.pre-restore").exists(),
        "failed restore must not create pre-restore evidence until replacement is ready to publish"
    );
    let target_db = Db::open(&target_path).unwrap();
    assert_eq!(
        target_db
            .list_processes()
            .unwrap()
            .iter()
            .map(|process| process.id)
            .collect::<Vec<_>>(),
        vec![target_process.id],
        "target database should remain readable after failed restore"
    );
}

fn has_restore_tmp_file(dir: &Path) -> bool {
    std::fs::read_dir(dir).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".restore.tmp.")
    })
}
