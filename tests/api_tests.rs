use std::{path::PathBuf, sync::Arc};

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use chrono::Utc;
use reactor_edge_daemon::{
    api::{router, AppState},
    config::{ControlConfig, OptimizerBounds, SafetyConfig, StirrerSafety, TemperatureSafety},
    db::{Db, ProductResult},
    memory::{
        AiMemory, ForbiddenZone, MemoryOptimizerBounds, RecommendationMemory, ReferenceBatch,
    },
    state::{RuntimeState, SensorSnapshot, SharedState},
};
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tower::ServiceExt;

fn safety() -> SafetyConfig {
    SafetyConfig {
        control: ControlConfig {
            auto_enabled_default: false,
            manual_lock_default: false,
            control_interval_ms: 2000,
            sensor_timeout_ms: 6000,
        },
        temperature: TemperatureSafety {
            min_c: 20.0,
            max_c: 160.0,
            max_step_c: 2.0,
            default_target_c: 60.0,
        },
        stirrer: StirrerSafety {
            min_rpm: 0.0,
            max_rpm: 1200.0,
            max_step_rpm: 50.0,
            default_target_rpm: 300.0,
        },
        optimizer: OptimizerBounds {
            min_temperature_c: 35.0,
            max_temperature_c: 140.0,
            min_stirrer_rpm: 100.0,
            max_stirrer_rpm: 1000.0,
            min_heating_minutes: 15.0,
            max_heating_minutes: 240.0,
            min_stirring_minutes: 15.0,
            max_stirring_minutes: 240.0,
        },
    }
}

fn memory() -> Arc<AiMemory> {
    Arc::new(AiMemory::default())
}

#[tokio::test]
async fn health_endpoint_works() {
    let safety = Arc::new(safety());
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: Db::open_memory().unwrap(),
            runtime,
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn live_endpoint_exposes_poc_alignment_fields() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    db.insert_sample(
        None,
        &SensorSnapshot {
            temperature_c: 35.4,
            pressure_mpa: 0.0629,
            stirrer_rpm: 300.0,
            shake_speed_cpm: 30.0,
            tilt_state: 1,
            tilt_angle_deg: 12.5,
            flow_rate_l_min: 2.2,
            product_concentration_percent: 12.9,
            ph: 6.04,
            captured_at: Utc::now(),
        },
    )
    .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.latest_sample = db.recent_samples(1).unwrap().pop();
    }
    let app = router(
        AppState {
            db,
            runtime,
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["runtime"]["latest_sample"]["pressure_mpa"], 0.0629);
    assert!(body["recent_samples"].as_array().unwrap().len() == 1);
    assert!(body["recent_samples"][0]["batch_id"].is_null());
    assert_eq!(body["latest_recommendation"], Value::Null);
    assert!(body["recent_batches"].is_array());
    assert!(body["recent_outcomes"].is_array());
    assert!(body["recent_events"].is_array());
}

#[tokio::test]
async fn live_endpoint_returns_service_unavailable_until_pipeline_has_sample() {
    let safety = Arc::new(safety());
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: Db::open_memory().unwrap(),
            runtime,
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 503);
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("sensor data unavailable"));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/devices/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 0);
    assert_eq!(body["data"]["total_count"], 1);
    assert_eq!(body["data"]["online_count"], 0);
    assert_eq!(body["data"]["devices"][0]["device_id"], "reactor_001");
    assert_eq!(body["data"]["devices"][0]["online"], false);
    assert_eq!(body["data"]["devices"][0]["status"], "offline");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/reactor/reactor_001/realtime")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 503);
}

#[tokio::test]
async fn unknown_api_routes_return_json_error_code() {
    let safety = Arc::new(safety());
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: Db::open_memory().unwrap(),
            runtime,
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/does-not-exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 404);
    assert_eq!(body["message"], "api route not found");
}

#[tokio::test]
async fn v1_pipeline_sample_endpoint_is_the_external_data_source() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db,
            runtime,
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let unavailable = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unavailable.status(), StatusCode::SERVICE_UNAVAILABLE);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/reactor/reactor_001/samples")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 31.114,
                        "pressure_mpa": 0.504,
                        "stirrer_rpm": 125.184,
                        "shake_speed_cpm": 30.004,
                        "tilt_state": 1,
                        "flow_rate_l_min": 2.424,
                        "product_concentration_percent": 11.104,
                        "ph": 6.154
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 0);
    assert_eq!(body["data"]["device_id"], "reactor_001");
    assert_eq!(body["data"]["sample"]["temperature_c"], 31.11);
    assert_eq!(body["data"]["sample"]["pressure_mpa"], 0.5);
    assert_eq!(body["data"]["sample"]["stirrer_rpm"], 125.18);
    assert_eq!(body["data"]["sample"]["shake_speed_cpm"], 30.0);
    assert_eq!(body["data"]["sample"]["tilt_state"], 1);
    assert!(body["data"]["sample"]["tilt_angle_deg"].as_f64().unwrap() >= 0.0);
    assert_eq!(
        body["data"]["sample"]["product_concentration_percent"],
        11.1
    );
    assert_eq!(body["data"]["sample"]["ph"], 6.15);

    let live = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(live.status(), StatusCode::OK);
    let body = to_bytes(live.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["runtime"]["latest_sample"]["temperature_c"], 31.11);
    assert_eq!(body["runtime"]["latest_sample"]["pressure_mpa"], 0.5);
    assert_eq!(body["runtime"]["latest_sample"]["stirrer_rpm"], 125.18);
    assert_eq!(body["runtime"]["latest_sample"]["shake_speed_cpm"], 30.0);
    assert_eq!(body["runtime"]["latest_sample"]["tilt_state"], 1);
    assert!(
        body["runtime"]["latest_sample"]["tilt_angle_deg"]
            .as_f64()
            .unwrap()
            >= 0.0
    );
    assert_eq!(
        body["runtime"]["latest_sample"]["product_concentration_percent"],
        11.1
    );
    assert_eq!(body["runtime"]["latest_sample"]["ph"], 6.15);
    assert!(body["recent_events"].as_array().unwrap().is_empty());

    let devices = app
        .oneshot(
            Request::builder()
                .uri("/api/devices/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(devices.status(), StatusCode::OK);
    let body = to_bytes(devices.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["data"]["online_count"], 1);
    assert_eq!(body["data"]["devices"][0]["online"], true);
    assert_eq!(body["data"]["devices"][0]["status"], "idle");
}

#[tokio::test]
async fn malformed_pipeline_sample_returns_json_error_code() {
    let safety = Arc::new(safety());
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: Db::open_memory().unwrap(),
            runtime,
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/reactor/reactor_001/samples")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": "not-a-number",
                        "pressure_mpa": 0.3,
                        "stirrer_rpm": 500.0,
                        "shake_speed_cpm": 34.0,
                        "tilt_state": 1,
                        "flow_rate_l_min": 2.5,
                        "product_concentration_percent": 42.0,
                        "ph": 6.9
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 422);
    assert!(body["message"].as_str().unwrap().contains("temperature_c"));
    assert!(body["data"]["error"].as_str().unwrap().contains("f64"));
}

#[tokio::test]
async fn test_pipeline_sample_endpoint_is_not_available_without_test_flag() {
    let safety = Arc::new(safety());
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: Db::open_memory().unwrap(),
            runtime,
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/test/pipeline-sample")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 31.11,
                        "pressure_mpa": 0.50,
                        "stirrer_rpm": 125.18,
                        "shake_speed_cpm": 30.00,
                        "tilt_state": 1,
                        "flow_rate_l_min": 2.42,
                        "product_concentration_percent": 11.10,
                        "ph": 6.15
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_pipeline_sample_endpoint_wraps_the_v1_pipeline_for_e2e() {
    let safety = Arc::new(safety());
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: Db::open_memory().unwrap(),
            runtime,
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: true,
        },
        PathBuf::from("static"),
    );

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/test/pipeline-sample")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 31.11,
                        "pressure_mpa": 0.50,
                        "stirrer_rpm": 125.18,
                        "shake_speed_cpm": 30.00,
                        "tilt_state": 1,
                        "flow_rate_l_min": 2.42,
                        "product_concentration_percent": 11.10,
                        "ph": 6.15
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn live_endpoint_rejects_stale_pipeline_samples() {
    let safety = Arc::new(SafetyConfig {
        control: ControlConfig {
            sensor_timeout_ms: 1,
            ..safety().control
        },
        ..safety()
    });
    let db = Db::open_memory().unwrap();
    db.insert_sample(
        None,
        &SensorSnapshot {
            temperature_c: 31.11,
            pressure_mpa: 0.5,
            stirrer_rpm: 125.18,
            shake_speed_cpm: 30.0,
            tilt_state: 1,
            tilt_angle_deg: 12.5,
            flow_rate_l_min: 2.42,
            product_concentration_percent: 11.1,
            ph: 6.15,
            captured_at: Utc::now() - chrono::Duration::seconds(1),
        },
    )
    .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.latest_sample = db.recent_samples(1).unwrap().pop();
    }
    let app = router(
        AppState {
            db,
            runtime,
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 503);
    assert!(body["message"].as_str().unwrap().contains("stale"));
}

#[tokio::test]
async fn live_endpoint_exposes_only_cached_pipeline_recommendations() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    db.insert_sample(
        None,
        &SensorSnapshot {
            temperature_c: 35.4,
            pressure_mpa: 0.0629,
            stirrer_rpm: 300.0,
            shake_speed_cpm: 30.0,
            tilt_state: 1,
            tilt_angle_deg: 12.5,
            flow_rate_l_min: 2.2,
            product_concentration_percent: 12.9,
            ph: 6.04,
            captured_at: Utc::now(),
        },
    )
    .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.latest_sample = db.recent_samples(1).unwrap().pop();
    }
    let ai_memory = Arc::new(AiMemory {
        recommendation: RecommendationMemory {
            enabled: true,
            use_reference_batches: false,
            bounds: MemoryOptimizerBounds {
                min_temperature_c: Some(90.0),
                max_temperature_c: Some(100.0),
                min_stirrer_rpm: Some(400.0),
                max_stirrer_rpm: Some(500.0),
                min_heating_minutes: Some(30.0),
                max_heating_minutes: Some(50.0),
                min_stirring_minutes: Some(40.0),
                max_stirring_minutes: Some(60.0),
            },
        },
        ..AiMemory::default()
    });
    let batch = db
        .create_batch("real-outcome-seed", 95.0, 450.0, 40.0, 50.0)
        .unwrap();
    db.finish_batch(batch.id).unwrap();
    db.insert_product_result(&ProductResult {
        batch_id: batch.id,
        yield_percent: 82.0,
        product_ratio: 0.88,
        notes: "real product result for live recommendation".to_string(),
    })
    .unwrap();
    let outcomes = db.batch_outcomes().unwrap();
    let recommendation = reactor_edge_daemon::optimizer::recommend_with_memory(
        &safety.optimizer,
        Some(ai_memory.as_ref()),
        &outcomes,
    );
    db.insert_recommendation(&recommendation).unwrap();

    let app = router(
        AppState {
            db,
            runtime,
            safety,
            ai_memory,
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["latest_recommendation"]["based_on_batch_count"], 1);
    assert!(body["latest_recommendation"]["target_temperature_c"].is_number());
    assert!(body["latest_recommendation"]["target_stirrer_rpm"].is_number());
    assert!(body["latest_recommendation"]["heating_minutes"].is_number());
    assert!(body["latest_recommendation"]["stirring_minutes"].is_number());
    assert_eq!(body["ai_provider"]["mode"], "local_optimizer");
    assert_eq!(body["ai_provider"]["model"], "local-tpe-lite");
}

#[tokio::test]
async fn latest_recommendation_waits_for_real_product_results() {
    let safety = Arc::new(safety());
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: Db::open_memory().unwrap(),
            runtime,
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/recommendations/latest")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 503);
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("product result data unavailable"));
}

#[tokio::test]
async fn operator_target_update_is_audited_with_clamped_targets() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    db.insert_sample(
        None,
        &SensorSnapshot {
            temperature_c: 35.4,
            pressure_mpa: 0.0629,
            stirrer_rpm: 300.0,
            shake_speed_cpm: 30.0,
            tilt_state: 1,
            tilt_angle_deg: 12.5,
            flow_rate_l_min: 2.2,
            product_concentration_percent: 12.9,
            ph: 6.04,
            captured_at: Utc::now(),
        },
    )
    .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.latest_sample = db.recent_samples(1).unwrap().pop();
    }
    let app = router(
        AppState {
            db,
            runtime,
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/targets")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 500.0,
                        "stirrer_rpm": 5000.0,
                        "shake_speed_cpm": 99.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["temperature_c"], 160.0);
    assert_eq!(body["stirrer_rpm"], 1200.0);
    assert_eq!(body["shake_speed_cpm"], 60.0);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let event = &body["recent_events"][0];
    assert_eq!(event["event_type"], "operator_targets_updated");
    assert_eq!(event["target_temperature_c"], 160.0);
    assert_eq!(event["target_stirrer_rpm"], 1200.0);
    assert_eq!(event["target_shake_speed_cpm"], 60.0);
}

#[tokio::test]
async fn v1_control_realtime_and_history_match_interface_document_shape() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    db.insert_sample(
        None,
        &SensorSnapshot {
            temperature_c: 85.5,
            pressure_mpa: 0.12,
            stirrer_rpm: 800.0,
            shake_speed_cpm: 30.0,
            tilt_state: 1,
            tilt_angle_deg: 12.5,
            flow_rate_l_min: 2.5,
            product_concentration_percent: 45.5,
            ph: 7.1,
            captured_at: Utc::now(),
        },
    )
    .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.latest_sample = db.recent_samples(1).unwrap().pop();
    }
    let app = router(
        AppState {
            db,
            runtime,
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/reactor/reactor_001/control")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "command_id": "cmd_20240115_001",
                        "timestamp": "2024-01-15T10:30:00Z",
                        "params": {
                            "heat_time": 300,
                            "hold_time": 600,
                            "cool_time": 180,
                            "stir_speed": 850,
                            "shake_speed": 35,
                            "target_temp": 120.0,
                            "target_pressure": 0.5
                        },
                        "priority": "normal",
                        "auto_start": true
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 0);
    assert_eq!(body["message"], "success");
    assert_eq!(body["data"]["command_id"], "cmd_20240115_001");
    assert_eq!(body["data"]["status"], "accepted");
    assert_eq!(body["data"]["estimated_duration"], 1080);
    assert!(body["data"].get("applied_params").is_none());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/reactor/reactor_001/realtime")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["device_id"], "reactor_001");
    assert_eq!(body["status"], "running");
    assert_eq!(body["data"]["current_temp"], 85.5);
    assert_eq!(body["data"]["current_pressure"], 0.12);
    assert_eq!(body["data"]["stir_speed"], 800.0);
    assert_eq!(body["data"]["shake_speed"], 30.0);
    assert_eq!(body["data"]["tilt_state"], 1);
    assert!(body["data"]["tilt_angle"].as_f64().unwrap() >= 0.0);
    assert_eq!(
        body["data"]["tilt_angle_source"],
        "software_fit_from_binary_sensor"
    );
    assert_eq!(body["data"]["flow_rate"], 2.5);
    assert!(body["alarms"].is_array());

    let start_time = (Utc::now() - chrono::Duration::minutes(1))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    let end_time = (Utc::now() + chrono::Duration::minutes(1))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    let uri = format!(
        "/api/v1/reactor/reactor_001/history?start_time={start_time}&end_time={end_time}&page=1&page_size=10&interval=1s"
    );
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 0);
    assert_eq!(body["data"]["device_id"], "reactor_001");
    assert_eq!(body["data"]["page"], 1);
    assert_eq!(body["data"]["items"][0]["data"]["current_temp"], 85.5);
    assert_eq!(body["data"]["items"][0]["data"]["tilt_state"], 1);
    assert!(
        body["data"]["items"][0]["data"]["tilt_angle"]
            .as_f64()
            .unwrap()
            >= 0.0
    );
}

#[tokio::test]
async fn v1_control_rejects_values_outside_interface_document_ranges() {
    let safety = Arc::new(safety());
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: Db::open_memory().unwrap(),
            runtime,
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/reactor/reactor_001/control")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "command_id": "cmd_bad",
                        "params": {
                            "heat_time": 300,
                            "hold_time": 600,
                            "cool_time": 180,
                            "stir_speed": 850,
                            "shake_speed": 35,
                            "target_temp": 500.1,
                            "target_pressure": 0.5
                        },
                        "priority": "normal",
                        "auto_start": false
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 400);
    assert!(body["message"].as_str().unwrap().contains("target_temp"));
}

#[tokio::test]
async fn v1_control_accepts_optimizer_duration_bounds_used_by_ai_recommendations() {
    let safety = Arc::new(safety());
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: Db::open_memory().unwrap(),
            runtime,
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/reactor/reactor_001/control")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "command_id": "cmd_ai_duration_boundary",
                        "params": {
                            "heat_time": 240 * 60,
                            "hold_time": 240 * 60,
                            "cool_time": 180,
                            "stir_speed": 650,
                            "shake_speed": 30,
                            "target_temp": 120.0,
                            "target_pressure": 0.5
                        },
                        "priority": "normal",
                        "auto_start": false
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn non_ai_api_complex_normal_and_error_chain_is_audited() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    db.insert_sample(
        None,
        &SensorSnapshot {
            temperature_c: 35.4,
            pressure_mpa: 0.0629,
            stirrer_rpm: 300.0,
            shake_speed_cpm: 30.0,
            tilt_state: 1,
            tilt_angle_deg: 12.5,
            flow_rate_l_min: 2.2,
            product_concentration_percent: 12.9,
            ph: 6.04,
            captured_at: Utc::now(),
        },
    )
    .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.latest_sample = db.recent_samples(1).unwrap().pop();
    }
    let app = router(
        AppState {
            db,
            runtime,
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let start = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/batches/start")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "acceptance-normal-chain",
                        "target_temperature_c": 82.0,
                        "target_stirrer_rpm": 460.0,
                        "heating_minutes": 75.0,
                        "stirring_minutes": 65.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(start.status(), StatusCode::OK);
    let body = to_bytes(start.into_body(), usize::MAX).await.unwrap();
    let batch: Value = serde_json::from_slice(&body).unwrap();
    let batch_id = batch["id"].as_i64().unwrap();

    let conflict = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/reactor/reactor_001/control")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "command_id": "cmd_conflict",
                        "params": {
                            "heat_time": 300,
                            "hold_time": 600,
                            "cool_time": 180,
                            "stir_speed": 450,
                            "shake_speed": 30,
                            "target_temp": 85.0,
                            "target_pressure": 0.5
                        },
                        "auto_start": true
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);

    let bad_history = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/reactor/reactor_001/history?start_time=bad&end_time=also-bad")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad_history.status(), StatusCode::BAD_REQUEST);

    let stop_uri = format!("/api/batches/{batch_id}/finish");
    let stop = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(stop_uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stop.status(), StatusCode::NO_CONTENT);

    let result = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/product-results")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "batch_id": batch_id,
                        "yield_percent": 81.5,
                        "product_ratio": 0.86,
                        "notes": "non-ai normal chain"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    let body = to_bytes(result.into_body(), usize::MAX).await.unwrap();
    let recommendation: Value = serde_json::from_slice(&body).unwrap();
    assert_two_decimal_parameters([
        recommendation["target_temperature_c"].as_f64().unwrap(),
        recommendation["target_stirrer_rpm"].as_f64().unwrap(),
        recommendation["heating_minutes"].as_f64().unwrap(),
        recommendation["stirring_minutes"].as_f64().unwrap(),
        recommendation["expected_score"].as_f64().unwrap(),
    ]);

    let live = app
        .oneshot(
            Request::builder()
                .uri("/api/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(live.status(), StatusCode::OK);
    let body = to_bytes(live.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let events = body["recent_events"].as_array().unwrap();
    assert!(events
        .iter()
        .any(|event| event["event_type"] == "batch_started"));
    assert!(events
        .iter()
        .any(|event| event["event_type"] == "batch_finished"));
    assert!(events
        .iter()
        .any(|event| event["event_type"] == "product_result_recorded"));
}

#[tokio::test]
async fn process_configuration_persists_steps_and_applies_to_batch_history() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    db.insert_sample(
        None,
        &SensorSnapshot {
            temperature_c: 35.4,
            pressure_mpa: 0.0629,
            stirrer_rpm: 300.0,
            shake_speed_cpm: 30.0,
            tilt_state: 1,
            tilt_angle_deg: 12.5,
            flow_rate_l_min: 2.2,
            product_concentration_percent: 12.9,
            ph: 6.04,
            captured_at: Utc::now(),
        },
    )
    .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.latest_sample = db.recent_samples(1).unwrap().pop();
    }
    let app = router(
        AppState {
            db,
            runtime,
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/processes")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "name": "RX-78 高分子聚合", "description": "lab process" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);
    let body = to_bytes(created.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 0);
    let process_id = body["data"]["id"].as_i64().unwrap();

    for (name, temp, duration, rpm, pressure) in [
        ("加热", 120.0, 45.0, 150.0, 0.5),
        ("保温", 120.0, 120.0, 300.0, 2.4),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/processes/{process_id}/steps"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "name": name,
                            "target_temperature_c": temp,
                            "ramp_rate_c_min": 2.5,
                            "duration_minutes": duration,
                            "target_stirrer_rpm": rpm,
                            "target_shake_speed_cpm": 30,
                            "target_pressure_mpa": pressure,
                            "cooling_mode": "自然"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    let detail = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/processes/{process_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::OK);
    let body = to_bytes(detail.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["data"]["steps"].as_array().unwrap().len(), 2);

    let applied = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/processes/{process_id}/apply"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(applied.status(), StatusCode::OK);
    let body = to_bytes(applied.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let batch_id = body["data"]["batch"]["id"].as_i64().unwrap();
    assert_eq!(
        body["data"]["batch"]["process_id"].as_i64().unwrap(),
        process_id
    );
    assert_eq!(
        body["data"]["applied_targets"]["temperature_c"]
            .as_f64()
            .unwrap(),
        120.0
    );

    let live = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(live.status(), StatusCode::OK);
    let body = to_bytes(live.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["processes"].as_array().unwrap().len(), 1);
    assert_eq!(
        body["recent_batches"][0]["process_id"].as_i64().unwrap(),
        process_id
    );

    let batch_detail = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/batches/{batch_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(batch_detail.status(), StatusCode::OK);
    let body = to_bytes(batch_detail.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 0);
    assert_eq!(body["data"]["batch"]["id"].as_i64().unwrap(), batch_id);
    assert!(body["data"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["event_type"] == "process_applied"));
}

#[tokio::test]
async fn persisted_process_and_batch_lists_do_not_require_live_sample() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db,
            runtime,
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let unavailable = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unavailable.status(), StatusCode::SERVICE_UNAVAILABLE);

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/processes")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "name": "离线可配置工艺" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);
    let body = to_bytes(created.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let process_id = body["data"]["id"].as_i64().unwrap();

    let processes = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/processes")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(processes.status(), StatusCode::OK);
    let body = to_bytes(processes.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 0);
    assert_eq!(body["data"][0]["id"].as_i64().unwrap(), process_id);

    let batches = app
        .oneshot(
            Request::builder()
                .uri("/api/batches")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(batches.status(), StatusCode::OK);
    let body = to_bytes(batches.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 0);
    assert!(body["data"]["batches"].is_array());
    assert!(body["data"]["outcomes"].is_array());
}

#[tokio::test]
async fn ai_api_decision_and_execution_respect_memory_constraints_under_complex_history() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    for (name, temp, rpm, heat, stir, yield_percent, ratio) in [
        ("low-yield-cool", 55.0, 280.0, 45.0, 35.0, 45.0, 0.52),
        (
            "forbidden-looking-hot-slow",
            130.0,
            320.0,
            170.0,
            95.0,
            96.0,
            0.96,
        ),
        ("validated-mid", 92.0, 560.0, 95.0, 70.0, 84.0, 0.88),
        ("validated-neighbor", 98.0, 610.0, 105.0, 80.0, 86.0, 0.90),
    ] {
        let batch = db.create_batch(name, temp, rpm, heat, stir).unwrap();
        db.finish_batch(batch.id).unwrap();
        db.insert_product_result(&ProductResult {
            batch_id: batch.id,
            yield_percent,
            product_ratio: ratio,
            notes: "ai complex scenario".to_string(),
        })
        .unwrap();
    }

    let ai_memory = Arc::new(AiMemory {
        recommendation: RecommendationMemory {
            enabled: true,
            use_reference_batches: true,
            bounds: MemoryOptimizerBounds {
                min_temperature_c: Some(70.0),
                max_temperature_c: Some(120.0),
                min_stirrer_rpm: Some(400.0),
                max_stirrer_rpm: Some(800.0),
                min_heating_minutes: Some(50.0),
                max_heating_minutes: Some(140.0),
                min_stirring_minutes: Some(40.0),
                max_stirring_minutes: Some(120.0),
            },
        },
        reference_batches: vec![
            reference("seed-a", 90.0, 520.0, 90.0, 65.0, 82.0, 0.86),
            reference("seed-b", 100.0, 600.0, 105.0, 78.0, 87.0, 0.91),
            reference("seed-c", 108.0, 690.0, 110.0, 82.0, 83.0, 0.87),
        ],
        forbidden_zones: vec![ForbiddenZone {
            name: "hot-low-stir-degradation-risk".to_string(),
            reason: "Do not repeat a high-yield but unsafe historical island.".to_string(),
            min_temperature_c: Some(120.0),
            max_temperature_c: Some(160.0),
            min_stirrer_rpm: Some(0.0),
            max_stirrer_rpm: Some(380.0),
            min_heating_minutes: None,
            max_heating_minutes: None,
            min_stirring_minutes: None,
            max_stirring_minutes: None,
        }],
        ..AiMemory::default()
    });
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db,
            runtime,
            safety,
            ai_memory: ai_memory.clone(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let rec_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/recommendations/latest")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rec_response.status(), StatusCode::OK);
    let body = to_bytes(rec_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let rec: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(rec["provider"]["mode"], "local_optimizer");
    assert_eq!(rec["provider"]["model"], "local-tpe-lite");
    let temp = rec["target_temperature_c"].as_f64().unwrap();
    let rpm = rec["target_stirrer_rpm"].as_f64().unwrap();
    let heating = rec["heating_minutes"].as_f64().unwrap();
    let stirring = rec["stirring_minutes"].as_f64().unwrap();

    assert!((70.0..=120.0).contains(&temp));
    assert!((400.0..=800.0).contains(&rpm));
    assert!((50.0..=140.0).contains(&heating));
    assert!((40.0..=120.0).contains(&stirring));
    assert_two_decimal_parameters([temp, rpm, heating, stirring]);
    assert!(!ai_memory.forbidden_zones[0].contains(temp, rpm, heating, stirring));
    assert!(rec["rationale"]
        .as_str()
        .unwrap()
        .contains("file reference outcomes"));

    let control_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/reactor/reactor_001/control")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "command_id": "cmd_apply_ai_complex",
                        "params": {
                            "heat_time": heating * 60.0,
                            "hold_time": stirring * 60.0,
                            "cool_time": 180,
                            "stir_speed": rpm,
                            "shake_speed": 30,
                            "target_temp": temp,
                            "target_pressure": 0.5
                        },
                        "priority": "normal",
                        "auto_start": false
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(control_response.status(), StatusCode::OK);
}

fn reference(
    id: &str,
    target_temperature_c: f64,
    target_stirrer_rpm: f64,
    heating_minutes: f64,
    stirring_minutes: f64,
    yield_percent: f64,
    product_ratio: f64,
) -> ReferenceBatch {
    ReferenceBatch {
        id: id.to_string(),
        target_temperature_c,
        target_stirrer_rpm,
        heating_minutes,
        stirring_minutes,
        yield_percent,
        product_ratio,
        notes: String::new(),
    }
}

fn assert_two_decimal_parameters(values: impl IntoIterator<Item = f64>) {
    for value in values {
        let scaled = value * 100.0;
        assert!(
            (scaled - scaled.round()).abs() < 1e-9,
            "parameter should be rounded to two decimals: {value}"
        );
    }
}
