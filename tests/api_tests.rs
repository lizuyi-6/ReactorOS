use std::{
    io::{Cursor, Read},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use reactor_edge_daemon::{
    ai_provider::{AiProvider, AiProviderConfig, StepFunApiType},
    api::{router, AinasTaskRequest, AppState},
    config::{
        load_device_config, ControlConfig, DeviceConfig, DeviceMode, ForbiddenControlZone,
        OptimizerBounds, SafetyConfig, StirrerSafety, TemperatureSafety,
    },
    control::SafeCommand,
    db::{Db, ProductResult},
    demo::seed_demo_context,
    device::{
        ComponentActionCapability, ComponentControlCommand, ComponentControlOutcome,
        DeviceComponentCapability, PipelineDevice, ReactorDevice, SharedDevice,
    },
    memory::{
        AiMemory, ForbiddenZone, MemoryOptimizerBounds, RecommendationMemory, ReferenceBatch,
    },
    modbus_tcp::{handle_modbus_tcp_pdu, handle_modbus_tcp_stream},
    mqtt::{
        execute_mqtt_task_payload, load_integration_config, mqtt_alert_snapshot,
        validate_integration_config, validate_mqtt_tls_config,
    },
    state::{ControlTargets, DeviceStatusSnapshot, RuntimeState, SensorSnapshot, SharedState},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::RwLock,
};
use tokio_rustls::TlsConnector;

const TEST_TLS_CERT: &str = "tests/fixtures/tls/server.crt";
const TEST_TLS_KEY: &str = "tests/fixtures/tls/server.key";
const TEST_CONFIRM_HEADER: (&str, &str) = ("x-xingshu-test-confirm", "local-e2e");

fn read_zip_entry(archive: &mut zip::ZipArchive<Cursor<Vec<u8>>>, name: &str) -> String {
    let mut entry = archive.by_name(name).unwrap();
    let mut text = String::new();
    entry.read_to_string(&mut text).unwrap();
    text
}

fn test_device() -> SharedDevice {
    Arc::new(PipelineDevice)
}

fn device_config() -> Arc<DeviceConfig> {
    Arc::new(load_device_config("config/device.toml").unwrap())
}

fn auth_header(role: &str) -> String {
    match role {
        "operator" | "engineer" | "admin" => {}
        _ => panic!("unknown test role {role}"),
    }
    let expires_at = Utc::now() + Duration::hours(12);
    let payload = format!("{role}:{role}:{}", expires_at.timestamp());
    let mut hasher = Sha256::new();
    hasher.update(b"xingshu-local-rbac-session-secret");
    hasher.update(b":");
    hasher.update(payload.as_bytes());
    format!("Bearer {payload}:{:x}", hasher.finalize())
}

#[derive(Default)]
struct TestComponentDevice;

#[async_trait::async_trait]
impl ReactorDevice for TestComponentDevice {
    async fn read_sample(&self) -> anyhow::Result<SensorSnapshot> {
        Err(anyhow::anyhow!("test device is driven by pipeline samples"))
    }

    async fn write_targets(&self, _command: &SafeCommand) -> anyhow::Result<()> {
        Ok(())
    }

    fn control_capabilities(&self) -> Vec<DeviceComponentCapability> {
        vec![
            DeviceComponentCapability {
                component_id: "shake_stepper".to_string(),
                component_type: "stepper_motor".to_string(),
                label: "Shake Vessel Stepper".to_string(),
                controllable: true,
                actions: vec![
                    ComponentActionCapability {
                        action: "start".to_string(),
                        label: "Start".to_string(),
                        value_type: "none".to_string(),
                        min: None,
                        max: None,
                        unit: None,
                    },
                    ComponentActionCapability {
                        action: "stop".to_string(),
                        label: "Stop".to_string(),
                        value_type: "none".to_string(),
                        min: None,
                        max: None,
                        unit: None,
                    },
                ],
            },
            DeviceComponentCapability {
                component_id: "stirrer_motor".to_string(),
                component_type: "motor".to_string(),
                label: "Stirrer Motor".to_string(),
                controllable: true,
                actions: vec![ComponentActionCapability {
                    action: "set_rpm".to_string(),
                    label: "Set RPM".to_string(),
                    value_type: "number".to_string(),
                    min: Some(0.0),
                    max: Some(2000.0),
                    unit: Some("RPM".to_string()),
                }],
            },
        ]
    }

    async fn write_component(
        &self,
        command: &ComponentControlCommand,
        targets: &ControlTargets,
        _safety: &SafetyConfig,
    ) -> anyhow::Result<Option<ComponentControlOutcome>> {
        Ok(Some(ComponentControlOutcome {
            component_id: command.component_id.clone(),
            action: command.action.clone(),
            command: None,
            targets: Some(SafeCommand {
                target_temperature_c: targets.temperature_c,
                heat_time_s: targets.heat_time_s,
                hold_time_s: targets.hold_time_s,
                cool_time_s: targets.cool_time_s,
                target_stirrer_rpm: targets.stirrer_rpm,
                target_shake_speed_cpm: if command.action == "stop" {
                    0.0
                } else {
                    targets.shake_speed_cpm.max(30.0)
                },
                target_pressure_mpa: targets.target_pressure_mpa,
                reason: "test component control".to_string(),
            }),
            message: "test component command accepted".to_string(),
        }))
    }
}

fn component_test_device() -> SharedDevice {
    Arc::new(TestComponentDevice)
}

#[derive(Default)]
struct RecordingComponentDevice {
    component_writes: Mutex<Vec<ComponentControlCommand>>,
}

#[async_trait::async_trait]
impl ReactorDevice for RecordingComponentDevice {
    async fn read_sample(&self) -> anyhow::Result<SensorSnapshot> {
        Err(anyhow::anyhow!("test device is driven by pipeline samples"))
    }

    async fn write_targets(&self, _command: &SafeCommand) -> anyhow::Result<()> {
        Ok(())
    }

    fn control_capabilities(&self) -> Vec<DeviceComponentCapability> {
        TestComponentDevice.control_capabilities()
    }

    async fn write_component(
        &self,
        command: &ComponentControlCommand,
        targets: &ControlTargets,
        safety: &SafetyConfig,
    ) -> anyhow::Result<Option<ComponentControlOutcome>> {
        self.component_writes.lock().unwrap().push(command.clone());
        TestComponentDevice
            .write_component(command, targets, safety)
            .await
    }
}

fn recording_component_device() -> (SharedDevice, Arc<RecordingComponentDevice>) {
    let device = Arc::new(RecordingComponentDevice::default());
    (device.clone(), device)
}

#[derive(Default)]
struct FailingComponentDevice;

#[async_trait::async_trait]
impl ReactorDevice for FailingComponentDevice {
    async fn read_sample(&self) -> anyhow::Result<SensorSnapshot> {
        Err(anyhow::anyhow!("test device is driven by pipeline samples"))
    }

    async fn write_targets(&self, _command: &SafeCommand) -> anyhow::Result<()> {
        Ok(())
    }

    fn control_capabilities(&self) -> Vec<DeviceComponentCapability> {
        TestComponentDevice.control_capabilities()
    }

    async fn write_component(
        &self,
        _command: &ComponentControlCommand,
        _targets: &ControlTargets,
        _safety: &SafetyConfig,
    ) -> anyhow::Result<Option<ComponentControlOutcome>> {
        Err(anyhow::anyhow!("component bus timeout"))
    }
}

fn failing_component_device() -> SharedDevice {
    Arc::new(FailingComponentDevice)
}

#[derive(Default)]
struct FailingTargetDevice;

#[async_trait::async_trait]
impl ReactorDevice for FailingTargetDevice {
    async fn read_sample(&self) -> anyhow::Result<SensorSnapshot> {
        Err(anyhow::anyhow!("test device is driven by pipeline samples"))
    }

    async fn write_targets(&self, _command: &SafeCommand) -> anyhow::Result<()> {
        Err(anyhow::anyhow!("target bus timeout"))
    }
}

fn failing_target_device() -> SharedDevice {
    Arc::new(FailingTargetDevice)
}

#[derive(Default)]
struct FailingTargetRecordingDevice {
    writes: Mutex<Vec<SafeCommand>>,
}

#[async_trait::async_trait]
impl ReactorDevice for FailingTargetRecordingDevice {
    async fn read_sample(&self) -> anyhow::Result<SensorSnapshot> {
        Err(anyhow::anyhow!("test device is driven by pipeline samples"))
    }

    async fn write_targets(&self, command: &SafeCommand) -> anyhow::Result<()> {
        self.writes.lock().unwrap().push(command.clone());
        Err(anyhow::anyhow!("target bus timeout"))
    }
}

fn failing_target_recording_device() -> (SharedDevice, Arc<FailingTargetRecordingDevice>) {
    let device = Arc::new(FailingTargetRecordingDevice::default());
    (device.clone(), device)
}

#[derive(Default)]
struct RecordingDevice {
    writes: Mutex<Vec<SafeCommand>>,
}

#[async_trait::async_trait]
impl ReactorDevice for RecordingDevice {
    async fn read_sample(&self) -> anyhow::Result<SensorSnapshot> {
        Err(anyhow::anyhow!("test device is driven by pipeline samples"))
    }

    async fn write_targets(&self, command: &SafeCommand) -> anyhow::Result<()> {
        self.writes.lock().unwrap().push(command.clone());
        Ok(())
    }
}

fn recording_device() -> (SharedDevice, Arc<RecordingDevice>) {
    let device = Arc::new(RecordingDevice::default());
    (device.clone(), device)
}

#[derive(Default)]
struct StartThenFailStopDevice {
    writes: Mutex<Vec<SafeCommand>>,
}

#[async_trait::async_trait]
impl ReactorDevice for StartThenFailStopDevice {
    async fn read_sample(&self) -> anyhow::Result<SensorSnapshot> {
        Err(anyhow::anyhow!("test device is driven by pipeline samples"))
    }

    async fn write_targets(&self, command: &SafeCommand) -> anyhow::Result<()> {
        let mut writes = self.writes.lock().unwrap();
        writes.push(command.clone());
        if writes.len() >= 2 {
            return Err(anyhow::anyhow!("rollback stop bus timeout"));
        }
        Ok(())
    }
}

fn start_then_fail_stop_device() -> (SharedDevice, Arc<StartThenFailStopDevice>) {
    let device = Arc::new(StartThenFailStopDevice::default());
    (device.clone(), device)
}

struct RuntimeTripDevice {
    runtime: SharedState,
    writes: Mutex<Vec<SafeCommand>>,
}

#[async_trait::async_trait]
impl ReactorDevice for RuntimeTripDevice {
    async fn read_sample(&self) -> anyhow::Result<SensorSnapshot> {
        Err(anyhow::anyhow!("test device is driven by pipeline samples"))
    }

    async fn write_targets(&self, command: &SafeCommand) -> anyhow::Result<()> {
        self.writes.lock().unwrap().push(command.clone());
        self.runtime.write().await.emergency_stop = true;
        Ok(())
    }
}

fn runtime_trip_device(runtime: SharedState) -> (SharedDevice, Arc<RuntimeTripDevice>) {
    let device = Arc::new(RuntimeTripDevice {
        runtime,
        writes: Mutex::new(Vec::new()),
    });
    (device.clone(), device)
}

struct ChangeActiveBatchOnWriteDevice {
    runtime: SharedState,
    next_active_batch_id: Option<i64>,
    writes: Mutex<Vec<SafeCommand>>,
}

#[async_trait::async_trait]
impl ReactorDevice for ChangeActiveBatchOnWriteDevice {
    async fn read_sample(&self) -> anyhow::Result<SensorSnapshot> {
        Err(anyhow::anyhow!("test device is driven by pipeline samples"))
    }

    async fn write_targets(&self, command: &SafeCommand) -> anyhow::Result<()> {
        self.writes.lock().unwrap().push(command.clone());
        self.runtime.write().await.active_batch_id = self.next_active_batch_id;
        Ok(())
    }
}

fn change_active_batch_on_write_device(
    runtime: SharedState,
    next_active_batch_id: Option<i64>,
) -> (SharedDevice, Arc<ChangeActiveBatchOnWriteDevice>) {
    let device = Arc::new(ChangeActiveBatchOnWriteDevice {
        runtime,
        next_active_batch_id,
        writes: Mutex::new(Vec::new()),
    });
    (device.clone(), device)
}

struct CreateBatchOnFirstWriteDevice {
    db: Db,
    writes: Mutex<Vec<SafeCommand>>,
}

#[async_trait::async_trait]
impl ReactorDevice for CreateBatchOnFirstWriteDevice {
    async fn read_sample(&self) -> anyhow::Result<SensorSnapshot> {
        Err(anyhow::anyhow!("test device is driven by pipeline samples"))
    }

    async fn write_targets(&self, command: &SafeCommand) -> anyhow::Result<()> {
        let should_create_orphan = {
            let mut writes = self.writes.lock().unwrap();
            writes.push(command.clone());
            writes.len() == 1
        };
        if should_create_orphan {
            self.db.create_batch_for_process(
                None,
                "orphan created after first device write",
                61.0,
                310.0,
                10.0,
                10.0,
            )?;
        }
        Ok(())
    }
}

fn create_batch_on_first_write_device(
    db: Db,
) -> (SharedDevice, Arc<CreateBatchOnFirstWriteDevice>) {
    let device = Arc::new(CreateBatchOnFirstWriteDevice {
        db,
        writes: Mutex::new(Vec::new()),
    });
    (device.clone(), device)
}

struct BreakIntegrationTasksOnWriteDevice {
    db: Db,
    writes: Mutex<Vec<SafeCommand>>,
}

#[async_trait::async_trait]
impl ReactorDevice for BreakIntegrationTasksOnWriteDevice {
    async fn read_sample(&self) -> anyhow::Result<SensorSnapshot> {
        Err(anyhow::anyhow!("test device is driven by pipeline samples"))
    }

    async fn write_targets(&self, command: &SafeCommand) -> anyhow::Result<()> {
        self.writes.lock().unwrap().push(command.clone());
        self.db.break_integration_tasks_for_tests()?;
        Ok(())
    }
}

fn break_integration_tasks_on_write_device(
    db: Db,
) -> (SharedDevice, Arc<BreakIntegrationTasksOnWriteDevice>) {
    let device = Arc::new(BreakIntegrationTasksOnWriteDevice {
        db,
        writes: Mutex::new(Vec::new()),
    });
    (device.clone(), device)
}

use tower::ServiceExt;

fn safety() -> SafetyConfig {
    SafetyConfig {
        control: ControlConfig {
            auto_enabled_default: false,
            manual_lock_default: false,
            control_interval_ms: 2000,
            sensor_timeout_ms: 6000,
            require_device_status_for_control: false,
            write_retry_backoff_ms: 5000,
            safety_guard_timeout_ms: 1000,
            ai_stop_product_concentration_percent: 95.0,
            require_command_ack: false,
            command_ack_timeout_ms: 2000,
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
        forbidden_control_zones: vec![ForbiddenControlZone {
            name: "hot-low-stir".to_string(),
            reason: "bench safety envelope".to_string(),
            min_temperature_c: 125.0,
            max_temperature_c: 160.0,
            min_stirrer_rpm: 0.0,
            max_stirrer_rpm: 350.0,
        }],
    }
}

fn memory() -> Arc<AiMemory> {
    Arc::new(AiMemory::default())
}

fn fresh_sample(
    temperature_c: f64,
    pressure_mpa: f64,
    stirrer_rpm: f64,
    shake_speed_cpm: f64,
    product_concentration_percent: f64,
) -> SensorSnapshot {
    SensorSnapshot {
        temperature_c,
        pressure_mpa,
        stirrer_rpm,
        shake_speed_cpm,
        tilt_state: 1,
        tilt_angle_deg: 12.5,
        flow_rate_l_min: 2.2,
        product_concentration_percent,
        ph: 6.04,
        captured_at: Utc::now(),
    }
}

fn healthy_device_status() -> DeviceStatusSnapshot {
    DeviceStatusSnapshot {
        connected: true,
        last_seen_at: Some(Utc::now()),
        last_frame_ok: true,
        relay: Some(0),
        motor: Some(0),
        tilt: Some(1),
        speed_delay_us: Some(10000),
        port: Some("/dev/ttyUSB0".to_string()),
        baudrate: Some(115200),
        last_command_request_id: None,
        last_command_ok: Some(true),
        last_command_error: None,
        updated_at: Utc::now(),
    }
}

async fn install_runtime_sample(runtime: &SharedState, db: &Db, sample: SensorSnapshot) {
    db.insert_sample(None, &sample).unwrap();
    let mut state = runtime.write().await;
    state.latest_sample = Some(sample);
}

fn add_simple_process(db: &Db, name: &str) -> i64 {
    let process = db.create_process(name, "ai control test").unwrap();
    db.add_process_step(
        process.id,
        &reactor_edge_daemon::db::NewProcessStep {
            name: "heat".to_string(),
            target_temperature_c: 90.0,
            ramp_rate_c_min: 2.0,
            duration_minutes: 30.0,
            target_stirrer_rpm: 240.0,
            target_shake_speed_cpm: 24.0,
            target_pressure_mpa: 0.5,
            cooling_mode: "natural".to_string(),
        },
    )
    .unwrap();
    process.id
}

fn add_ai_outcomes(db: &Db) {
    for (name, temp, rpm, heat, stir, yield_percent, ratio) in [
        ("ai-low", 70.0, 220.0, 40.0, 35.0, 52.0, 0.60),
        ("ai-mid", 92.0, 420.0, 70.0, 65.0, 78.0, 0.82),
        ("ai-best", 102.0, 560.0, 90.0, 80.0, 86.0, 0.91),
    ] {
        let batch = db.create_batch(name, temp, rpm, heat, stir).unwrap();
        db.finish_batch(batch.id).unwrap();
        db.insert_product_result(&ProductResult {
            batch_id: batch.id,
            yield_percent,
            product_ratio: ratio,
            notes: "ai master control seed".to_string(),
        })
        .unwrap();
    }
}

async fn modbus_tcp_test_state() -> AppState {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.latest_sample = Some(SensorSnapshot {
            temperature_c: 42.5,
            pressure_mpa: 0.18,
            stirrer_rpm: 260.0,
            shake_speed_cpm: 30.0,
            tilt_state: 1,
            tilt_angle_deg: 12.5,
            flow_rate_l_min: 2.0,
            product_concentration_percent: 10.0,
            ph: 6.8,
            captured_at: Utc::now(),
        });
    }
    AppState {
        db,
        runtime,
        device: test_device(),
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    }
}

#[tokio::test]
async fn demo_context_seeds_ai_and_process_data_without_sensor_samples() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let ai_memory = memory();
    assert!(seed_demo_context(&db, &safety, &ai_memory).unwrap());
    assert!(!seed_demo_context(&db, &safety, &ai_memory).unwrap());
    assert!(db.recent_samples(10).unwrap().is_empty());

    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: db.clone(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory,
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let live_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(live_response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let demo_response = app
        .oneshot(
            Request::builder()
                .uri("/api/demo/context")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(demo_response.status(), StatusCode::OK);
    let body = to_bytes(demo_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 0);
    assert_eq!(body["data"]["demo"], true);
    assert!(body["data"]["sensor_data_policy"]
        .as_str()
        .unwrap()
        .contains("never fabricates"));
    assert!(body["data"]["processes"].as_array().unwrap().len() >= 2);
    assert!(body["data"]["recent_batches"].as_array().unwrap().len() >= 6);
    assert!(body["data"]["recent_outcomes"].as_array().unwrap().len() >= 6);
    assert!(body["data"]["demo_alarms"].as_array().unwrap().len() >= 2);
    assert!(body["data"]["latest_recommendation"]["target_temperature_c"].is_number());
}

#[tokio::test]
async fn health_endpoint_works() {
    let safety = Arc::new(safety());
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: Db::open_memory().unwrap(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
            db: db.clone(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
async fn live_endpoint_surfaces_unfinished_batch_recovery_alarm() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let orphan = db
        .create_batch_for_process_sqlx(None, "live orphan unfinished", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(35.4, 0.0629, 300.0, 30.0, 12.9)).await;
    let app = router(
        AppState {
            db: db.clone(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
    let alarm = body["alarms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|alarm| alarm["type"] == "unfinished_batch_recovery")
        .expect("live response should surface unfinished batch recovery alarm");
    assert_eq!(alarm["level"], "high");
    assert_eq!(alarm["unfinished_batch_ids"][0], orphan.id);
    assert_eq!(alarm["active_batch_id"], Value::Null);
}

#[tokio::test]
async fn devices_status_marks_unfinished_batch_recovery_as_error() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let orphan = db
        .create_batch_for_process_sqlx(None, "status orphan unfinished", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(35.4, 0.0629, 300.0, 30.0, 12.9)).await;
    let app = router(
        AppState {
            db,
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/devices/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let device = &body["data"]["devices"][0];
    assert_eq!(body["data"]["online_count"], 0);
    assert_eq!(device["online"], false);
    assert_eq!(device["status"], "error");
    assert_eq!(device["unfinished_batch_ids"][0], orphan.id);
    assert_eq!(device["unexpected_unfinished_batch_ids"][0], orphan.id);
    assert!(device["last_control_error"]
        .as_str()
        .unwrap()
        .contains("database has unfinished batch records"));
}

#[tokio::test]
async fn live_endpoint_surfaces_missing_persisted_active_batch_recovery_alarm() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(35.4, 0.0629, 300.0, 30.0, 12.9)).await;
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(42);
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
    let alarm = body["alarms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|alarm| alarm["type"] == "unfinished_batch_recovery")
        .expect("live response should flag runtime batch missing from DB");
    assert_eq!(alarm["active_batch_id"], 42);
    assert_eq!(alarm["unfinished_batch_ids"].as_array().unwrap().len(), 0);
    assert_eq!(alarm["runtime_active_batch_missing"], true);
}

#[tokio::test]
async fn live_endpoint_marks_unproven_downstream_status_as_alarm_in_strict_mode() {
    let mut strict_safety = safety();
    strict_safety.control.require_device_status_for_control = true;
    let safety = Arc::new(strict_safety);
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(35.4, 0.0629, 300.0, 30.0, 12.9)).await;
    let app = router(
        AppState {
            db,
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/live?sample_limit=1&include_processes=false&include_batches=false&include_events=false")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["runtime"]["latest_sample"]["temperature_c"], 35.4);
    assert_eq!(body["device_status"]["online_count"], 0);
    assert_eq!(body["device_status"]["devices"][0]["online"], false);
    assert_eq!(body["device_status"]["devices"][0]["status"], "offline");
    assert!(body["alarms"]
        .as_array()
        .unwrap()
        .iter()
        .any(|alarm| alarm["type"] == "device_status_unavailable" && alarm["level"] == "high"));
}

#[tokio::test]
async fn live_endpoint_marks_downstream_command_fault_as_unhealthy_device_status() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(35.4, 0.0629, 300.0, 30.0, 12.9)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(DeviceStatusSnapshot {
            last_command_request_id: Some("cmd-failed".to_string()),
            last_command_ok: Some(false),
            last_command_error: Some("relay did not acknowledge".to_string()),
            ..healthy_device_status()
        });
    }
    let app = router(
        AppState {
            db,
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/live?sample_limit=1&include_processes=false&include_batches=false&include_events=false")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["device_status"]["online_count"], 0);
    assert_eq!(body["device_status"]["devices"][0]["online"], false);
    assert_eq!(body["device_status"]["devices"][0]["status"], "error");
    assert!(body["alarms"]
        .as_array()
        .unwrap()
        .iter()
        .any(|alarm| alarm["type"] == "downstream_command_fault" && alarm["level"] == "high"));
}

#[tokio::test]
async fn live_endpoint_supports_lightweight_limits_for_low_power_clients() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let base = Utc::now() - chrono::Duration::milliseconds(5);
    for index in 0..5 {
        let mut sample = fresh_sample(35.0 + index as f64, 0.06, 300.0 + index as f64, 30.0, 12.0);
        sample.captured_at = base + chrono::Duration::milliseconds(index);
        db.insert_sample(None, &sample).unwrap();
    }
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.latest_sample = db.recent_samples(1).unwrap().pop();
    }
    let app = router(
        AppState {
            db,
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/live?sample_limit=2&include_processes=false&include_batches=false&include_events=false")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["recent_samples"].as_array().unwrap().len(), 2);
    assert_eq!(body["recent_batches"].as_array().unwrap().len(), 0);
    assert_eq!(body["recent_outcomes"].as_array().unwrap().len(), 0);
    assert_eq!(body["recent_events"].as_array().unwrap().len(), 0);
    assert_eq!(body["processes"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn live_endpoint_returns_service_unavailable_until_pipeline_has_sample() {
    let safety = Arc::new(safety());
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: Db::open_memory().unwrap(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
    assert_eq!(body["data"]["devices"][0]["auto_enabled"], false);
    assert_eq!(body["data"]["devices"][0]["manual_lock"], false);

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
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/reactor/reactor_001/realtime")
                .header("authorization", auth_header("operator"))
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
}

#[tokio::test]
async fn live_endpoint_returns_service_unavailable_when_current_device_read_has_failed() {
    let safety = Arc::new(safety());
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.latest_sample = Some(SensorSnapshot {
            temperature_c: 64.25,
            pressure_mpa: 0.5,
            stirrer_rpm: 125.18,
            shake_speed_cpm: 30.0,
            tilt_state: 1,
            tilt_angle_deg: 12.5,
            flow_rate_l_min: 1.2,
            product_concentration_percent: 50.0,
            ph: 6.15,
            captured_at: Utc::now(),
        });
        state.last_sensor_error =
            Some("json bridge last upstream frame failed XOR check".to_string());
    }
    let app = router(
        AppState {
            db: Db::open_memory().unwrap(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("failed XOR check"));
}

#[tokio::test]
async fn devices_status_counts_online_json_bridge_even_when_sample_is_incomplete() {
    let safety = Arc::new(safety());
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.last_sensor_error =
            Some("json bridge state missing required sensor field temperature_c".to_string());
        state.device_status = Some(DeviceStatusSnapshot {
            connected: true,
            last_seen_at: Some(Utc::now()),
            last_frame_ok: true,
            relay: Some(1),
            motor: Some(1),
            tilt: Some(1),
            speed_delay_us: Some(10000),
            port: Some("/dev/ttyUSB0".to_string()),
            baudrate: Some(115200),
            last_command_request_id: Some("reactor-os-1".to_string()),
            last_command_ok: Some(true),
            last_command_error: None,
            updated_at: Utc::now(),
        });
    }
    let app = router(
        AppState {
            db: Db::open_memory().unwrap(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/devices/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["data"]["online_count"], 1);
    assert_eq!(body["data"]["devices"][0]["online"], true);
    assert_eq!(body["data"]["devices"][0]["status"], "error");
    assert_eq!(body["data"]["devices"][0]["auto_enabled"], false);
    assert_eq!(body["data"]["devices"][0]["manual_lock"], false);
    assert_eq!(body["data"]["devices"][0]["relay"], 1);
    assert_eq!(body["data"]["devices"][0]["motor"], 1);
    assert_eq!(body["data"]["devices"][0]["tilt"], 1);
    assert_eq!(body["data"]["devices"][0]["port"], "/dev/ttyUSB0");
    assert!(body["data"]["devices"][0]["sensors"].is_array());
    assert_eq!(
        body["data"]["devices"][0]["sensors"][0]["value"],
        Value::Null
    );
    assert_eq!(body["data"]["devices"][0]["sensors"][0]["status"], "error");
    assert!(body["data"]["devices"][0]["components"].is_array());
    assert!(body["data"]["sensors"].is_array());
    assert!(body["data"]["components"].is_array());
}

#[tokio::test]
async fn devices_status_does_not_report_online_when_required_device_status_is_missing() {
    let mut strict_safety = safety();
    strict_safety.control.require_device_status_for_control = true;
    let safety = Arc::new(strict_safety);
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let app = router(
        AppState {
            db,
            runtime,
            device: component_test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/devices/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let device = &body["data"]["devices"][0];
    assert_eq!(body["data"]["online_count"], 0);
    assert_eq!(device["online"], false);
    assert_eq!(device["status"], "offline");
    assert!(device["components"]
        .as_array()
        .unwrap()
        .iter()
        .all(|component| component["status"] == "unavailable"));
}

#[tokio::test]
async fn devices_status_marks_downstream_command_fault_unhealthy() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(DeviceStatusSnapshot {
            last_command_request_id: Some("cmd-failed".to_string()),
            last_command_ok: Some(false),
            last_command_error: Some("relay did not acknowledge".to_string()),
            ..healthy_device_status()
        });
    }
    let app = router(
        AppState {
            db,
            runtime,
            device: component_test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/devices/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let device = &body["data"]["devices"][0];
    assert_eq!(body["data"]["online_count"], 0);
    assert_eq!(device["online"], false);
    assert_eq!(device["status"], "error");
    assert_eq!(device["last_command_ok"], false);
    assert!(device["components"]
        .as_array()
        .unwrap()
        .iter()
        .all(|component| component["status"] == "error"));
}

#[tokio::test]
async fn device_capabilities_endpoint_lists_components_and_blocks_unknown_component() {
    let safety = Arc::new(safety());
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: Db::open_memory().unwrap(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/devices/capabilities")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 0);
    assert_eq!(body["data"]["devices"][0]["mode"], "pipeline");
    assert!(body["data"]["devices"][0]["components"]
        .as_array()
        .unwrap()
        .is_empty());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/devices/reactor_001/components/shake_stepper/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"action":"stop"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 404);
    assert!(body["message"].as_str().unwrap().contains("component"));
}

#[tokio::test]
async fn device_status_groups_sensors_and_controllable_components_by_device() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(72.34, 0.12, 321.45, 18.5, 33.3)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(DeviceStatusSnapshot {
            connected: true,
            last_seen_at: Some(Utc::now()),
            last_frame_ok: true,
            relay: Some(1),
            motor: Some(1),
            tilt: Some(1),
            speed_delay_us: Some(10000),
            port: Some("/dev/ttyUSB0".to_string()),
            baudrate: Some(115200),
            last_command_request_id: Some("reactor-os-2".to_string()),
            last_command_ok: Some(true),
            last_command_error: None,
            updated_at: Utc::now(),
        });
    }
    let app = router(
        AppState {
            db,
            runtime,
            device: component_test_device(),
            device_mode: DeviceMode::JsonBridge,
            device_config: device_config(),
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
                .uri("/api/devices/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let device = &body["data"]["devices"][0];
    assert_eq!(device["online"], true);
    assert_eq!(device["sensors"][0]["sensor_id"], "temperature_c");
    assert_eq!(device["sensors"][0]["value"], 72.34);
    assert_eq!(device["sensors"][0]["status"], "online");
    assert!(device["sensors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|sensor| sensor["sensor_id"] == "stirrer_rpm"
            && sensor["component_id"] == "stirrer_motor"));
    assert!(device["components"]
        .as_array()
        .unwrap()
        .iter()
        .any(|component| component["component_id"] == "stirrer_motor"
            && component["actions"][0]["action"] == "set_rpm"));
}

#[tokio::test]
async fn unknown_api_routes_return_json_error_code() {
    let safety = Arc::new(safety());
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: Db::open_memory().unwrap(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .header("authorization", auth_header("engineer"))
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
async fn v1_pipeline_sample_rejects_physically_invalid_values_fail_closed() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.device_status = Some(healthy_device_status());
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    for (field, value, expected_message) in [
        (
            "pressure_mpa",
            -0.01,
            "pressure_mpa must be between 0 and 10",
        ),
        (
            "shake_speed_cpm",
            -1.0,
            "shake_speed_cpm must be between 0 and 60",
        ),
        (
            "product_concentration_percent",
            101.0,
            "product_concentration_percent must be between 0 and 100",
        ),
        ("ph", 14.01, "ph must be between 0 and 14"),
    ] {
        let mut payload = json!({
            "temperature_c": 31.11,
            "pressure_mpa": 0.50,
            "stirrer_rpm": 125.18,
            "shake_speed_cpm": 30.00,
            "tilt_state": 1,
            "flow_rate_l_min": 2.42,
            "product_concentration_percent": 11.10,
            "ph": 6.15
        });
        payload[field] = json!(value);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/reactor/reactor_001/samples")
                    .header("authorization", auth_header("engineer"))
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        let message = body["message"].as_str().unwrap();
        assert!(
            message.contains(expected_message),
            "unexpected invalid sample error for {field}: {message}"
        );
        assert_eq!(db.recent_samples(10).unwrap().len(), 1);
        let state = runtime.read().await;
        assert!(!state.auto_enabled);
        assert!(state.latest_sample.is_none());
        assert!(state.device_status.is_none());
        assert!(state
            .last_sensor_error
            .as_deref()
            .unwrap_or_default()
            .contains(expected_message));
        drop(state);
    }

    let events = db.recent_control_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "field_input_fault_auto_disabled");
    assert!(events[0]
        .reason
        .contains("sensor sample rejected: pressure_mpa must be between 0 and 10"));
}

#[tokio::test]
async fn v1_pipeline_sample_rejects_malformed_payloads_fail_closed() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.device_status = Some(healthy_device_status());
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    for payload in [
        json!({
            "temperature_c": "31.11",
            "pressure_mpa": 0.50,
            "stirrer_rpm": 125.18,
            "shake_speed_cpm": 30.00,
            "tilt_state": 1,
            "flow_rate_l_min": 2.42,
            "product_concentration_percent": 11.10,
            "ph": 6.15
        }),
        json!({
            "pressure_mpa": 0.50,
            "stirrer_rpm": 125.18,
            "shake_speed_cpm": 30.00,
            "tilt_state": 1,
            "flow_rate_l_min": 2.42,
            "product_concentration_percent": 11.10,
            "ph": 6.15
        }),
        json!({
            "temperature_c": 31.11,
            "pressure_mpa": 0.50,
            "stirrer_rpm": 125.18,
            "shake_speed_cpm": 30.00,
            "tilt_state": "tilted",
            "flow_rate_l_min": 2.42,
            "product_concentration_percent": 11.10,
            "ph": 6.15
        }),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/reactor/reactor_001/samples")
                    .header("authorization", auth_header("engineer"))
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert!(body["message"]
            .as_str()
            .unwrap()
            .contains("invalid sensor sample JSON"));
        assert_eq!(db.recent_samples(10).unwrap().len(), 1);
        let state = runtime.read().await;
        assert!(!state.auto_enabled);
        assert!(state.latest_sample.is_none());
        assert!(state.device_status.is_none());
        assert!(state
            .last_sensor_error
            .as_deref()
            .unwrap_or_default()
            .contains("invalid sensor sample JSON"));
        drop(state);
    }

    let events = db.recent_control_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "field_input_fault_auto_disabled");
    assert!(events[0]
        .reason
        .contains("sensor sample rejected: invalid sensor sample JSON"));
}

#[tokio::test]
async fn v1_reactor_routes_reject_unknown_device_id_without_mutating_runtime() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
    }
    let original_targets = runtime.read().await.targets.clone();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let sample = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/reactor/reactor_002/samples")
                .header("authorization", auth_header("engineer"))
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
    assert_eq!(sample.status(), StatusCode::NOT_FOUND);
    assert_eq!(db.recent_samples(10).unwrap().len(), 1);
    assert_eq!(
        runtime
            .read()
            .await
            .latest_sample
            .as_ref()
            .unwrap()
            .temperature_c,
        50.0
    );

    let control = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/reactor/reactor_002/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "command_id": "wrong-device",
                        "params": {
                            "heat_time": 300,
                            "hold_time": 600,
                            "cool_time": 180,
                            "stir_speed": 650,
                            "shake_speed": 30,
                            "target_temp": 90.0,
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
    assert_eq!(control.status(), StatusCode::NOT_FOUND);
    assert_eq!(runtime.read().await.targets, original_targets);

    let process = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/reactor/reactor_002/process")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "process_id": "wrong-device-process",
                        "name": "wrong device process",
                        "phases": [{
                            "phase": "heating",
                            "params": {
                                "duration": 300,
                                "target_temp": 92.0,
                                "stir_speed": 660.0,
                                "shake_speed": 32.0,
                                "target_pressure": 0.6
                            }
                        }]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(process.status(), StatusCode::NOT_FOUND);
    assert_eq!(runtime.read().await.targets, original_targets);

    let realtime = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/reactor/reactor_002/realtime")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(realtime.status(), StatusCode::NOT_FOUND);

    let start_time = (Utc::now() - chrono::Duration::minutes(1))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    let end_time = (Utc::now() + chrono::Duration::minutes(1))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    let history_uri =
        format!("/api/v1/reactor/reactor_002/history?start_time={start_time}&end_time={end_time}");
    let history = app
        .oneshot(
            Request::builder()
                .uri(history_uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(history.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn v1_pipeline_sample_requires_engineering_ingest_permission() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );
    let body = Body::from(
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
    );

    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/reactor/reactor_001/samples")
                .header("content-type", "application/json")
                .body(body)
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);
    assert!(runtime.read().await.latest_sample.is_none());
    assert!(db.recent_samples(1).unwrap().is_empty());

    let operator = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/reactor/reactor_001/samples")
                .header("authorization", auth_header("operator"))
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
    assert_eq!(operator.status(), StatusCode::FORBIDDEN);
    assert!(runtime.read().await.latest_sample.is_none());
    assert!(db.recent_samples(1).unwrap().is_empty());
}

#[tokio::test]
async fn v1_pipeline_sample_does_not_become_field_proof_when_persistence_fails() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );
    db.break_samples_for_tests().unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/reactor/reactor_001/samples")
                .header("authorization", auth_header("engineer"))
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

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let state = runtime.read().await;
    assert!(state.latest_sample.is_none());
    assert!(!state.auto_enabled);
    assert!(state
        .last_sensor_error
        .as_deref()
        .unwrap_or_default()
        .contains("sensor sample persistence failed"));
}

#[tokio::test]
async fn malformed_pipeline_sample_returns_json_error_code() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.device_status = Some(healthy_device_status());
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .header("authorization", auth_header("engineer"))
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

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 400);
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("invalid sensor sample JSON"));
    assert_eq!(body["data"]["error"], body["message"]);
    assert_eq!(db.recent_samples(10).unwrap().len(), 1);
    let state = runtime.read().await;
    assert!(!state.auto_enabled);
    assert!(state.latest_sample.is_none());
    assert!(state.device_status.is_none());
    assert!(state
        .last_sensor_error
        .as_deref()
        .unwrap_or_default()
        .contains("sensor sample rejected: invalid sensor sample JSON"));
    drop(state);
    let events = db.recent_control_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "field_input_fault_auto_disabled");
}

#[tokio::test]
async fn test_pipeline_sample_endpoint_is_not_available_without_test_flag() {
    let safety = Arc::new(safety());
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: Db::open_memory().unwrap(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: true,
        },
        PathBuf::from("static"),
    );

    let denied = app
        .clone()
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
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/test/pipeline-sample")
                .header(TEST_CONFIRM_HEADER.0, TEST_CONFIRM_HEADER.1)
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
async fn test_reset_endpoint_requires_explicit_local_confirmation() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    db.insert_sample(None, &fresh_sample(31.11, 0.5, 125.18, 30.0, 11.1))
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: db.clone(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: true,
        },
        PathBuf::from("static"),
    );

    let denied = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/test/reset")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    assert_eq!(db.recent_samples(1).unwrap().len(), 1);

    let accepted = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/test/reset")
                .header(TEST_CONFIRM_HEADER.0, TEST_CONFIRM_HEADER.1)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::NO_CONTENT);
    assert_eq!(db.recent_samples(1).unwrap().len(), 0);
}

#[tokio::test]
async fn test_reset_endpoint_refuses_unsafe_runtime_states() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety: safety.clone(),
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: true,
        },
        PathBuf::from("static"),
    );
    let active_batch = db
        .create_batch_for_process_sqlx(None, "active reset guard", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();

    enum UnsafeResetState {
        ActiveBatch(i64),
        AutoEnabled,
        EmergencyStop,
        ControlFault,
    }

    let cases = [
        UnsafeResetState::ActiveBatch(active_batch.id),
        UnsafeResetState::AutoEnabled,
        UnsafeResetState::EmergencyStop,
        UnsafeResetState::ControlFault,
    ];

    for case in cases {
        db.insert_sample(None, &fresh_sample(31.11, 0.5, 125.18, 30.0, 11.1))
            .unwrap();
        {
            let mut state = runtime.write().await;
            *state = RuntimeState::from_safety(&safety);
            match case {
                UnsafeResetState::ActiveBatch(id) => state.active_batch_id = Some(id),
                UnsafeResetState::AutoEnabled => state.auto_enabled = true,
                UnsafeResetState::EmergencyStop => state.emergency_stop = true,
                UnsafeResetState::ControlFault => {
                    state.last_control_error = Some("write timeout".to_string());
                }
            }
        }

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/test/reset")
                    .header(TEST_CONFIRM_HEADER.0, TEST_CONFIRM_HEADER.1)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(db.recent_samples(1).unwrap().len(), 1);
    }
}

#[tokio::test]
async fn test_reset_endpoint_refuses_unfinished_db_batch_when_runtime_is_idle() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let orphan = db
        .create_batch_for_process_sqlx(None, "reset orphan unfinished", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    db.insert_sample(None, &fresh_sample(31.11, 0.5, 125.18, 30.0, 11.1))
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/test/reset")
                .header(TEST_CONFIRM_HEADER.0, TEST_CONFIRM_HEADER.1)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("database has unfinished batch"));
    assert_eq!(runtime.read().await.active_batch_id, None);
    assert_eq!(db.recent_samples(1).unwrap().len(), 1);
    assert_eq!(
        db.latest_unfinished_batch_sqlx().await.unwrap().unwrap().id,
        orphan.id
    );
}

#[tokio::test]
async fn pipeline_high_sensor_alarm_disables_auto_control_and_audits() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let batch = db
        .create_batch_for_process_sqlx(None, "high alarm active batch", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.active_batch_id = Some(batch.id);
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: Arc::new(AiMemory {
                sensor_limits: reactor_edge_daemon::memory::SensorLimits {
                    temperature_c: Some(reactor_edge_daemon::memory::SensorLimit {
                        label: "reactor temperature".to_string(),
                        unit: "degC".to_string(),
                        normal_min: Some(20.0),
                        normal_max: Some(140.0),
                        hard_min: Some(0.0),
                        hard_max: Some(160.0),
                        suggestion: "stop heating".to_string(),
                    }),
                    ..Default::default()
                },
                ..AiMemory::default()
            }),
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
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 170.0,
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
    let state = runtime.read().await;
    assert!(!state.auto_enabled);
    assert_eq!(state.active_batch_id, Some(batch.id));
    drop(state);
    let events = db.recent_control_events(10).unwrap();
    assert_eq!(events[0].event_type, "high_sensor_alarm_auto_disabled");
    assert!(events[0].reason.contains("temperature_limit"));
}

#[tokio::test]
async fn pipeline_warning_sensor_alarm_does_not_disable_auto_control() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let batch = db
        .create_batch_for_process_sqlx(None, "warning alarm active batch", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.active_batch_id = Some(batch.id);
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: Arc::new(AiMemory {
                sensor_limits: reactor_edge_daemon::memory::SensorLimits {
                    temperature_c: Some(reactor_edge_daemon::memory::SensorLimit {
                        label: "reactor temperature".to_string(),
                        unit: "degC".to_string(),
                        normal_min: Some(20.0),
                        normal_max: Some(140.0),
                        hard_min: Some(0.0),
                        hard_max: Some(160.0),
                        suggestion: "check cooling".to_string(),
                    }),
                    ..Default::default()
                },
                ..AiMemory::default()
            }),
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
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 145.0,
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
    assert!(runtime.read().await.auto_enabled);
    assert!(db.recent_control_events(10).unwrap().is_empty());
}

#[tokio::test]
async fn recovered_pipeline_sample_clears_old_sensor_error_before_alarm_evaluation() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let batch = db
        .create_batch_for_process_sqlx(None, "recovered sample", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.active_batch_id = Some(batch.id);
        state.last_sensor_error = Some("previous pipeline sample stale".to_string());
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 55.0,
                        "pressure_mpa": 0.10,
                        "stirrer_rpm": 240.0,
                        "shake_speed_cpm": 24.0,
                        "tilt_state": 1,
                        "flow_rate_l_min": 2.0,
                        "product_concentration_percent": 12.0,
                        "ph": 6.8
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let state = runtime.read().await;
    assert!(state.auto_enabled);
    assert_eq!(state.active_batch_id, Some(batch.id));
    assert_eq!(state.last_sensor_error, None);
    assert_eq!(state.latest_sample.as_ref().unwrap().temperature_c, 55.0);
    drop(state);
    assert!(db
        .recent_control_events(10)
        .unwrap()
        .iter()
        .all(|event| event.event_type != "high_sensor_alarm_auto_disabled"));
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
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
    assert_eq!(body["ai_provider"]["model"], "local-ga-sa-pid");
}

#[tokio::test]
async fn latest_recommendation_waits_for_real_product_results() {
    let safety = Arc::new(safety());
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: Db::open_memory().unwrap(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
async fn latest_recommendation_get_is_read_only_when_cache_is_empty() {
    let safety = Arc::new(safety());
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: Db::open_memory().unwrap(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .method("GET")
                .uri("/api/recommendations/latest")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body, Value::Null);
}

#[tokio::test]
async fn recommendation_generation_rejects_unfinished_batch_recovery_state_but_get_remains_read_only(
) {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let finished = db
        .create_batch("finished recommendation seed", 72.0, 420.0, 35.0, 55.0)
        .unwrap();
    db.finish_batch(finished.id).unwrap();
    db.insert_product_result(&ProductResult {
        batch_id: finished.id,
        yield_percent: 87.5,
        product_ratio: 0.91,
        notes: "seed".to_string(),
    })
    .unwrap();
    let orphan = db
        .create_batch_for_process_sqlx(None, "recommendation orphan", 65.0, 320.0, 12.0, 12.0)
        .await
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: db.clone(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let post = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/recommendations/latest")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(post.status(), StatusCode::CONFLICT);
    let body = to_bytes(post.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains(&orphan.id.to_string()));
    assert!(db.latest_recommendation().unwrap().is_none());

    let get = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/recommendations/latest")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get.status(), StatusCode::OK);
}

#[tokio::test]
async fn cached_local_recommendation_is_marked_stale_when_stepfun_is_configured() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    db.insert_recommendation(&reactor_edge_daemon::optimizer::Recommendation {
        based_on_batch_count: 3,
        target_temperature_c: 88.5,
        target_stirrer_rpm: 460.0,
        heating_minutes: 72.0,
        stirring_minutes: 55.0,
        expected_score: 0.84,
        rationale: "Local optimizer cached recommendation".to_string(),
    })
    .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let ai_provider = AiProvider::from_config(AiProviderConfig {
        enabled: true,
        api_key: Some("test-stepfun-key".to_string()),
        base_url: "http://127.0.0.1:9/v1".to_string(),
        api_type: StepFunApiType::ChatCompletions,
        model: "step-3.6".to_string(),
        reasoning_effort: "medium".to_string(),
        timeout_seconds: 1,
    })
    .unwrap()
    .map(Arc::new);
    let app = router(
        AppState {
            db,
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/recommendations/latest")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["provider"]["mode"], "stale_local_recommendation");
    assert_eq!(body["provider"]["model"], "step-3.6");
    assert!(body["provider"]["fallback_reason"]
        .as_str()
        .unwrap()
        .contains("regenerated by StepFun"));
}

#[tokio::test]
async fn ai_experiment_plan_drafts_safety_gated_sop_without_control_write() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    add_ai_outcomes(&db);
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.targets.target_pressure_mpa = 0.5;
        state.targets.shake_speed_cpm = 24.0;
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/ai/experiment-plan")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 0);
    let plan = &body["data"];
    assert_eq!(plan["status"], "draft_requires_operator_review");
    assert!(plan["plan_id"]
        .as_str()
        .unwrap()
        .starts_with("xingshu-plan-"));
    assert_eq!(plan["steps"].as_array().unwrap().len(), 3);
    assert!(plan["sop_summary"]
        .as_str()
        .unwrap()
        .contains("three-stage"));
    assert!(plan["safety_notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note.as_str().unwrap().contains("does not start")));
    assert!(plan["model_boundary"]
        .as_array()
        .unwrap()
        .iter()
        .any(|note| note.as_str().unwrap().contains("Local Qwen LoRA")));
    assert!(plan["steps"][0]["target_temperature_c"].as_f64().unwrap() <= 160.0);
    assert!(plan["steps"][1]["target_stirrer_rpm"].as_f64().unwrap() <= 1200.0);
    let events = db.recent_control_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].event_type,
        "ai_experiment_plan_recommendation_generated"
    );
    assert!(events[0].target_temperature_c.is_some());
    assert!(events[0].target_stirrer_rpm.is_some());
    assert!(runtime.read().await.active_batch_id.is_none());
}

#[tokio::test]
async fn operator_target_update_is_audited_after_safety_validation() {
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
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 92.5,
                        "stirrer_rpm": 460.0,
                        "shake_speed_cpm": 38.0
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
    assert_eq!(body["temperature_c"], 92.5);
    assert_eq!(body["stirrer_rpm"], 460.0);
    assert_eq!(body["shake_speed_cpm"], 38.0);

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
    assert_eq!(event["target_temperature_c"], 92.5);
    assert_eq!(event["target_stirrer_rpm"], 460.0);
    assert_eq!(event["target_shake_speed_cpm"], 38.0);
    assert!(event["event_hash"].as_str().unwrap().len() >= 64);
}

#[tokio::test]
async fn operator_target_update_rejects_out_of_range_values_without_clamping() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
    }
    let original_targets = runtime.read().await.targets.clone();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/control/targets")
                .header("authorization", auth_header("operator"))
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

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("temperature_c: target_temp exceeds device maximum temperature 160.0"));
    assert_eq!(runtime.read().await.targets, original_targets);
    assert!(runtime.read().await.auto_enabled);
    assert!(db.recent_control_events(10).unwrap().is_empty());
}

#[tokio::test]
async fn operator_target_update_rejects_explicit_null_shake_speed_instead_of_inheriting() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.targets.shake_speed_cpm = 24.0;
        state.targets.target_pressure_mpa = 0.5;
        state.auto_enabled = true;
    }
    let original_targets = runtime.read().await.targets.clone();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/control/targets")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 92.5,
                        "stirrer_rpm": 460.0,
                        "shake_speed_cpm": null
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
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("shake_speed_cpm must not be null"));
    assert_eq!(runtime.read().await.targets, original_targets);
    assert!(runtime.read().await.auto_enabled);
    assert!(db.recent_control_events(10).unwrap().is_empty());
}

#[tokio::test]
async fn operator_target_update_rejects_invalid_existing_runtime_targets_without_clamping() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.targets.target_pressure_mpa = 12.0;
        state.auto_enabled = true;
    }
    let original_targets = runtime.read().await.targets.clone();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/control/targets")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 92.5,
                        "stirrer_rpm": 460.0,
                        "shake_speed_cpm": 38.0
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
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("target_pressure_mpa must be between 0 and 10"));
    assert_eq!(runtime.read().await.targets, original_targets);
    assert!(runtime.read().await.auto_enabled);
    assert!(db.recent_control_events(10).unwrap().is_empty());
}

#[tokio::test]
async fn operator_target_update_rejects_forbidden_temperature_stirrer_zone() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/control/targets")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 130.0,
                        "stirrer_rpm": 300.0,
                        "shake_speed_cpm": 30.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("forbidden control zone hot-low-stir"));
    assert_eq!(db.recent_control_events(10).unwrap().len(), 0);
    let runtime = runtime.read().await;
    assert_eq!(runtime.targets.temperature_c, 60.0);
    assert_eq!(runtime.targets.stirrer_rpm, 300.0);
    assert!(runtime.auto_enabled);
}

#[tokio::test]
async fn batch_start_rejects_invalid_targets_before_creating_batch_or_writing_device() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
    }
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    for (payload, expected_status, expected_message) in [
        (
            json!({
                "name": null,
                "target_temperature_c": 82.0,
                "target_stirrer_rpm": 460.0,
                "target_shake_speed_cpm": 24.0,
                "heating_minutes": 75.0,
                "stirring_minutes": 65.0
            }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "must not be null",
        ),
        (
            json!({
                "name": "null process id must not become an ad hoc batch",
                "process_id": null,
                "target_temperature_c": 82.0,
                "target_stirrer_rpm": 460.0,
                "target_shake_speed_cpm": 24.0,
                "heating_minutes": 75.0,
                "stirring_minutes": 65.0
            }),
            StatusCode::BAD_REQUEST,
            "process_id must not be null",
        ),
        (
            json!({
                "name": "null target temperature must not inherit runtime target",
                "target_temperature_c": null,
                "target_stirrer_rpm": 460.0,
                "target_shake_speed_cpm": 24.0,
                "heating_minutes": 75.0,
                "stirring_minutes": 65.0
            }),
            StatusCode::BAD_REQUEST,
            "target_temperature_c must not be null",
        ),
        (
            json!({
                "name": "null heat duration must not default",
                "target_temperature_c": 82.0,
                "target_stirrer_rpm": 460.0,
                "target_shake_speed_cpm": 24.0,
                "heating_minutes": null,
                "stirring_minutes": 65.0
            }),
            StatusCode::BAD_REQUEST,
            "heating_minutes must not be null",
        ),
        (
            json!({
                "name": "negative heat duration must not persist",
                "target_temperature_c": 82.0,
                "target_stirrer_rpm": 460.0,
                "target_shake_speed_cpm": 24.0,
                "heating_minutes": -1.0,
                "stirring_minutes": 65.0
            }),
            StatusCode::BAD_REQUEST,
            "heating_minutes must be between 0",
        ),
        (
            json!({
                "name": "forbidden batch target must not persist",
                "target_temperature_c": 130.0,
                "target_stirrer_rpm": 300.0,
                "target_shake_speed_cpm": 24.0,
                "heating_minutes": 75.0,
                "stirring_minutes": 65.0
            }),
            StatusCode::FORBIDDEN,
            "forbidden control zone hot-low-stir",
        ),
        (
            json!({
                "name": "shake speed out of range must not persist",
                "target_temperature_c": 82.0,
                "target_stirrer_rpm": 460.0,
                "target_shake_speed_cpm": 61.0,
                "heating_minutes": 75.0,
                "stirring_minutes": 65.0
            }),
            StatusCode::BAD_REQUEST,
            "target_shake_speed_cpm must be between 0 and 60",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/batches/start")
                    .header("authorization", auth_header("operator"))
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), expected_status);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        let message = body["message"].as_str().unwrap();
        assert!(
            message.contains(expected_message),
            "unexpected batch start validation error: {message}"
        );
    }

    assert!(db.recent_batches_sqlx(10).await.unwrap().is_empty());
    assert!(db.recent_control_events(10).unwrap().is_empty());
    assert!(recorded_device.writes.lock().unwrap().is_empty());
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, None);
    assert!(state.auto_enabled);
    drop(state);

    {
        let mut state = runtime.write().await;
        state.targets.target_pressure_mpa = 12.0;
        state.auto_enabled = true;
    }
    let invalid_runtime_targets = runtime.read().await.targets.clone();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/batches/start")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "invalid inherited pressure must fail closed",
                        "target_temperature_c": 82.0,
                        "target_stirrer_rpm": 460.0,
                        "target_shake_speed_cpm": 24.0,
                        "heating_minutes": 75.0,
                        "stirring_minutes": 65.0
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
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("target_pressure_mpa must be between 0 and 10"));
    {
        let state = runtime.read().await;
        assert_eq!(state.targets, invalid_runtime_targets);
        assert!(state.auto_enabled);
    }
    assert!(db.recent_batches_sqlx(10).await.unwrap().is_empty());
    assert!(db.recent_control_events(10).unwrap().is_empty());
    assert!(recorded_device.writes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn batch_start_rejects_requests_without_explicit_targets_before_runtime_changes() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let process_id = add_simple_process(&db, "batch start missing explicit target");
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
    }
    let original_targets = runtime.read().await.targets.clone();
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    for payload in [
        json!({}),
        json!({
            "name": "name alone must not start inherited targets"
        }),
        json!({
            "name": "process id alone must not start inherited targets",
            "process_id": process_id
        }),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/batches/start")
                    .header("authorization", auth_header("operator"))
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert!(body["message"]
            .as_str()
            .unwrap()
            .contains("batch start must include at least one explicit target or duration field"));
    }

    assert!(db.recent_batches_sqlx(10).await.unwrap().is_empty());
    assert!(db.recent_control_events(10).unwrap().is_empty());
    assert!(recorded_device.writes.lock().unwrap().is_empty());
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, None);
    assert_eq!(state.targets, original_targets);
    assert!(state.auto_enabled);
}

#[tokio::test]
async fn target_update_requires_downstream_status_when_configured() {
    let mut strict_safety = safety();
    strict_safety.control.require_device_status_for_control = true;
    let safety = Arc::new(strict_safety);
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let original_targets = runtime.read().await.targets.clone();
    let app = router(
        AppState {
            db,
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/control/targets")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 80.0,
                        "stirrer_rpm": 320.0,
                        "shake_speed_cpm": 24.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("device status unavailable"));
    assert_eq!(runtime.read().await.targets, original_targets);
}

#[tokio::test]
async fn target_update_failure_forces_auto_disabled_before_returning_error() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.latest_sample = None;
    }
    let app = router(
        AppState {
            db,
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/control/targets")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 72.0,
                        "stirrer_rpm": 360.0,
                        "shake_speed_cpm": 24.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let state = runtime.read().await;
    assert!(!state.auto_enabled);
    assert_eq!(state.targets.temperature_c, 60.0);
    assert_eq!(state.targets.stirrer_rpm, 300.0);
}

#[tokio::test]
async fn v1_control_failure_forces_auto_disabled_before_returning_error() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.latest_sample = None;
    }
    let app = router(
        AppState {
            db,
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "auto_start": false,
                        "params": {
                            "target_temp": 72.0,
                            "stir_speed": 360.0,
                            "shake_speed": 24.0,
                            "heat_time": 300,
                            "hold_time": 300,
                            "cool_time": 0
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let state = runtime.read().await;
    assert!(!state.auto_enabled);
    assert_eq!(state.targets.temperature_c, 60.0);
    assert_eq!(state.targets.stirrer_rpm, 300.0);
}

#[tokio::test]
async fn target_updates_fail_closed_when_field_state_is_not_proven_safe() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app_state = AppState {
        db: db.clone(),
        runtime: runtime.clone(),
        device: test_device(),
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };
    let app = router(app_state.clone(), PathBuf::from("static"));

    let missing_sample = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/targets")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 70.0,
                        "stirrer_rpm": 360.0,
                        "shake_speed_cpm": 30.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_sample.status(), StatusCode::SERVICE_UNAVAILABLE);

    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.emergency_stop = true;
    }
    let emergency_stop = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/modbus/registers/target_temperature_c/write")
                .header("authorization", auth_header("admin"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "value": 75.0,
                        "reason": "should be blocked by emergency stop"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(emergency_stop.status(), StatusCode::CONFLICT);

    {
        let mut state = runtime.write().await;
        state.emergency_stop = false;
        state.manual_lock = true;
    }
    let manual_lock_receipt = execute_mqtt_task_payload(
        &app_state,
        json!({
            "external_task_id": "mqtt-manual-lock-blocked",
            "action": "set_targets",
            "target_temperature_c": 76.0,
            "reason": "should be blocked by manual lock"
        })
        .to_string()
        .as_bytes(),
    )
    .await;
    assert!(!manual_lock_receipt.ok);
    assert_eq!(manual_lock_receipt.status, "rejected");
    assert!(manual_lock_receipt
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("manual lock is active"));

    {
        let mut state = runtime.write().await;
        state.manual_lock = false;
        state.last_control_error = Some("write timeout".to_string());
    }
    let control_fault = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/targets")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 77.0,
                        "stirrer_rpm": 370.0,
                        "shake_speed_cpm": 0.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(control_fault.status(), StatusCode::SERVICE_UNAVAILABLE);

    let targets = runtime.read().await.targets.clone();
    assert_eq!(targets.temperature_c, 60.0);
    assert_eq!(targets.stirrer_rpm, 300.0);
    assert_eq!(db.recent_control_events(10).unwrap().len(), 0);
}

#[tokio::test]
async fn target_intent_updates_do_not_commit_runtime_when_audit_fails() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let original_targets = runtime.read().await.targets.clone();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );
    db.break_control_events_for_tests().unwrap();

    let operator_targets = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/targets")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 72.0,
                        "stirrer_rpm": 360.0,
                        "shake_speed_cpm": 24.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(operator_targets.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(runtime.read().await.targets, original_targets);

    let v1_control = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/reactor/reactor_001/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "command_id": "cmd_no_audit_commit",
                        "params": {
                            "heat_time": 300,
                            "hold_time": 600,
                            "cool_time": 180,
                            "stir_speed": 650,
                            "shake_speed": 30,
                            "target_temp": 90.0,
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
    assert_eq!(v1_control.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(runtime.read().await.targets, original_targets);
    assert_eq!(runtime.read().await.active_batch_id, None);

    let v1_process = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/reactor/reactor_001/process")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "process_id": "p-no-audit-commit",
                        "name": "audit failure process load",
                        "phases": [{
                            "phase": "heating",
                            "params": {
                                "duration": 300,
                                "target_temp": 92.0,
                                "stir_speed": 660.0,
                                "shake_speed": 32.0,
                                "target_pressure": 0.6
                            }
                        }]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(v1_process.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(runtime.read().await.targets, original_targets);

    let modbus_debug_write = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/modbus/registers/target_pressure_mpa/write")
                .header("authorization", auth_header("admin"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "value": 0.9,
                        "reason": "modbus debug audit failure"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        modbus_debug_write.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(runtime.read().await.targets, original_targets);
}

#[tokio::test]
async fn v1_process_rejects_phase_duration_that_would_have_been_clamped() {
    let mut strict_safety = safety();
    strict_safety.optimizer.max_heating_minutes = 20.0;
    let safety = Arc::new(strict_safety);
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let original_targets = runtime.read().await.targets.clone();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/v1/reactor/reactor_001/process")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "process_id": "p-long-heat",
                        "name": "long heat must fail closed",
                        "phases": [{
                            "phase": "heating",
                            "params": {
                                "duration": 1800,
                                "target_temp": 92.0,
                                "stir_speed": 660.0,
                                "shake_speed": 32.0,
                                "target_pressure": 0.6
                            }
                        }]
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
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("heating duration must be between 0 and 1200"));
    assert_eq!(runtime.read().await.targets, original_targets);
    assert!(db.recent_control_events(10).unwrap().is_empty());
}

#[tokio::test]
async fn v1_process_rejects_unknown_phase_instead_of_ignoring_it() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let original_targets = runtime.read().await.targets.clone();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    for (payload, expected_message) in [
        (
            json!({
                "process_id": "p-blank-phase",
                "name": "blank phase must fail closed",
                "phases": [{
                    "phase": " \n\t",
                    "params": {
                        "duration": 60
                    }
                }]
            }),
            "phase must not be blank",
        ),
        (
            json!({
                "process_id": "p-unknown-phase",
                "name": "unknown phase must fail closed",
                "phases": [{
                    "phase": "pressurizing",
                    "params": {
                        "duration": 60,
                        "target_pressure": 0.6
                    }
                }]
            }),
            "unsupported process phase 'pressurizing'",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/reactor/reactor_001/process")
                    .header("authorization", auth_header("engineer"))
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        let message = body["message"].as_str().unwrap();
        assert!(
            message.contains(expected_message),
            "unexpected v1 phase rejection: {message}"
        );
        assert_eq!(runtime.read().await.targets, original_targets);
        assert!(db.recent_control_events(10).unwrap().is_empty());
    }
}

#[tokio::test]
async fn v1_process_rejects_wrong_typed_phase_params_instead_of_defaulting() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let original_targets = runtime.read().await.targets.clone();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    for (payload, expected_message) in [
        (
            json!({
                "process_id": "p-string-duration",
                "name": "string duration must fail closed",
                "phases": [{
                    "phase": "heating",
                    "params": {
                        "duration": "300",
                        "target_temp": 92.0,
                        "stir_speed": 660.0
                    }
                }]
            }),
            "duration must be a number",
        ),
        (
            json!({
                "process_id": "p-array-params",
                "name": "array params must fail closed",
                "phases": [{
                    "phase": "heating",
                    "params": []
                }]
            }),
            "phase params must be an object",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/reactor/reactor_001/process")
                    .header("authorization", auth_header("engineer"))
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        let message = body["message"].as_str().unwrap();
        assert!(
            message.contains(expected_message),
            "unexpected v1 process type rejection: {message}"
        );
    }

    assert_eq!(runtime.read().await.targets, original_targets);
    assert!(db.recent_control_events(10).unwrap().is_empty());
}

#[tokio::test]
async fn v1_process_rejects_phases_without_recognized_control_params() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let original_targets = runtime.read().await.targets.clone();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/v1/reactor/reactor_001/process")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "process_id": "p-no-control-params",
                        "name": "empty phase params must fail closed",
                        "phases": [
                            {
                                "phase": "heating",
                                "params": {}
                            },
                            {
                                "phase": "holding",
                                "params": {
                                    "note": "ignored"
                                }
                            }
                        ]
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
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("process phases must include at least one recognized control parameter"));
    assert_eq!(runtime.read().await.targets, original_targets);
    assert!(db.recent_control_events(10).unwrap().is_empty());
}

#[tokio::test]
async fn target_intent_updates_recheck_interlocks_after_audit_before_runtime_commit() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let original_targets = runtime.read().await.targets.clone();
    {
        let runtime = runtime.clone();
        let targets_before_audit = original_targets.clone();
        db.after_control_event_success_for_tests(Arc::new(move || {
            let mut state = runtime
                .try_write()
                .expect("runtime lock should be available after audit insert");
            assert_eq!(state.targets, targets_before_audit);
            state.emergency_stop = true;
        }));
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/control/targets")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 72.0,
                        "stirrer_rpm": 360.0,
                        "shake_speed_cpm": 24.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(runtime.read().await.targets, original_targets);
    {
        let state = runtime.read().await;
        assert!(state.emergency_stop);
        assert!(!state.auto_enabled);
    }
    assert!(db
        .recent_control_events(10)
        .unwrap()
        .iter()
        .any(|event| event.event_type == "operator_targets_updated"));
}

#[tokio::test]
async fn target_intent_update_rejects_stale_runtime_targets_after_audit_before_commit() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let original_targets = runtime.read().await.targets.clone();
    let later_targets = ControlTargets {
        temperature_c: 66.0,
        heat_time_s: original_targets.heat_time_s,
        hold_time_s: original_targets.hold_time_s,
        cool_time_s: original_targets.cool_time_s,
        stirrer_rpm: 330.0,
        shake_speed_cpm: original_targets.shake_speed_cpm,
        target_pressure_mpa: original_targets.target_pressure_mpa,
    };
    {
        let runtime = runtime.clone();
        let targets_before_audit = original_targets.clone();
        let later_targets = later_targets.clone();
        db.after_control_event_success_for_tests(Arc::new(move || {
            let mut state = runtime
                .try_write()
                .expect("runtime lock should be available after audit insert");
            assert_eq!(state.targets, targets_before_audit);
            state.targets = later_targets.clone();
        }));
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/control/targets")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 72.0,
                        "stirrer_rpm": 360.0,
                        "shake_speed_cpm": 24.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("stale runtime targets"));
    let state = runtime.read().await;
    assert_eq!(state.targets, later_targets);
    assert!(!state.auto_enabled);
    drop(state);
    assert!(db
        .recent_control_events(10)
        .unwrap()
        .iter()
        .any(|event| event.event_type == "operator_targets_updated"));
}

#[tokio::test]
async fn component_control_fails_closed_but_still_allows_stop_actions() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: component_test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let start_without_sample = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/devices/reactor_001/components/shake_stepper/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"action": "start"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        start_without_sample.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(!runtime.read().await.auto_enabled);

    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.last_control_error = Some("last write failed".to_string());
    }
    let set_rpm_with_fault = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/devices/reactor_001/components/stirrer_motor/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "action": "set_rpm",
                        "value": 420.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(set_rpm_with_fault.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(!runtime.read().await.auto_enabled);

    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.last_control_error = None;
        state.emergency_stop = true;
    }
    let stop_during_emergency = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/devices/reactor_001/components/shake_stepper/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "action": "stop",
                        "reason": " stop remains\navailable\t\u{0007}\u{200B}during emergency "
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stop_during_emergency.status(), StatusCode::OK);
    let targets = runtime.read().await.targets.clone();
    assert_eq!(targets.shake_speed_cpm, 0.0);
    assert!(runtime.read().await.auto_enabled);
    let events = db.recent_control_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "component_control");
    assert_eq!(events[0].reason, "stop remains available during emergency");
}

#[tokio::test]
async fn component_control_blocks_dangerous_actions_during_unfinished_batch_recovery_but_allows_stop(
) {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    db.create_batch_for_process_sqlx(None, "component orphan unfinished", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(healthy_device_status());
    }
    let (device, recorded_device) = recording_component_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/devices/reactor_001/components/shake_stepper/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"action": "start"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(start.status(), StatusCode::CONFLICT);
    let body = to_bytes(start.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("unfinished batch recovery"));
    assert!(recorded_device.component_writes.lock().unwrap().is_empty());

    let stop = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/devices/reactor_001/components/shake_stepper/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "action": "stop",
                        "reason": "stop remains available during unfinished batch recovery"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stop.status(), StatusCode::OK);
    assert_eq!(
        recorded_device.component_writes.lock().unwrap()[0].action,
        "stop"
    );
    assert_eq!(runtime.read().await.targets.shake_speed_cpm, 0.0);
    assert_eq!(
        db.recent_control_events(10).unwrap()[0].event_type,
        "component_control"
    );
}

#[tokio::test]
async fn component_control_rejects_invalid_action_values_before_device_write() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(healthy_device_status());
    }
    let original_targets = runtime.read().await.targets.clone();
    let (device, recorded_device) = recording_component_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    for (path, payload, expected_message) in [
        (
            "/api/devices/reactor_001/components/stirrer_motor/control",
            json!({"action": "set_rpm", "value": 5000.0}),
            "component action set_rpm value must be <= 2000",
        ),
        (
            "/api/devices/reactor_001/components/stirrer_motor/control",
            json!({"action": "set_rpm", "value": -1.0}),
            "component action set_rpm value must be >= 0",
        ),
        (
            "/api/devices/reactor_001/components/stirrer_motor/control",
            json!({"action": "set_rpm"}),
            "component action set_rpm requires value",
        ),
        (
            "/api/devices/reactor_001/components/stirrer_motor/control",
            json!({"action": "set_rpm", "value": "fast"}),
            "component action set_rpm value must be a number",
        ),
        (
            "/api/devices/reactor_001/components/shake_stepper/control",
            json!({"action": "stop", "value": 1.0}),
            "component action stop does not accept a value",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("authorization", auth_header("operator"))
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert!(
            body["message"]
                .as_str()
                .unwrap_or_default()
                .contains(expected_message),
            "unexpected component control rejection body: {body}"
        );
    }

    assert_eq!(runtime.read().await.targets, original_targets);
    assert!(recorded_device.component_writes.lock().unwrap().is_empty());
    assert!(db.recent_control_events(10).unwrap().is_empty());
}

#[tokio::test]
async fn component_control_rejects_null_reason_for_risk_increasing_actions_only() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(healthy_device_status());
    }
    let original_targets = runtime.read().await.targets.clone();
    let (device, recorded_device) = recording_component_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let set_rpm_null_reason = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/devices/reactor_001/components/stirrer_motor/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "action": "set_rpm",
                        "value": 420.0,
                        "reason": null
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(set_rpm_null_reason.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(set_rpm_null_reason.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap_or_default()
        .contains("reason must not be null"));
    assert_eq!(runtime.read().await.targets, original_targets);
    assert!(recorded_device.component_writes.lock().unwrap().is_empty());
    assert!(db.recent_control_events(10).unwrap().is_empty());

    let stop_null_reason = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/devices/reactor_001/components/shake_stepper/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "action": "stop",
                        "value": null,
                        "reason": null
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stop_null_reason.status(), StatusCode::OK);
    assert_eq!(
        recorded_device.component_writes.lock().unwrap()[0].action,
        "stop"
    );
    let state = runtime.read().await;
    assert_eq!(state.targets.shake_speed_cpm, 0.0);
    drop(state);
    let events = db.recent_control_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "component_control");
    assert_eq!(
        events[0].reason,
        "operator component control shake_stepper:stop"
    );
}

#[tokio::test]
async fn unhealthy_device_status_blocks_new_targets_but_not_stop_actions() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(DeviceStatusSnapshot {
            connected: false,
            last_frame_ok: false,
            last_seen_at: Some(Utc::now()),
            ..healthy_device_status()
        });
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: component_test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let set_targets = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/targets")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 72.0,
                        "stirrer_rpm": 360.0,
                        "shake_speed_cpm": 24.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(set_targets.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(set_targets.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("device status is not healthy"));

    let stop = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/devices/reactor_001/components/shake_stepper/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"action": "stop"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stop.status(), StatusCode::OK);
    assert_eq!(runtime.read().await.targets.shake_speed_cpm, 0.0);
    let events = db.recent_control_events(10).unwrap();
    assert_eq!(events[0].event_type, "component_control");
}

#[tokio::test]
async fn component_write_failure_latches_control_fault() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
    }
    let app = router(
        AppState {
            db,
            runtime: runtime.clone(),
            device: failing_component_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/devices/reactor_001/components/stirrer_motor/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "action": "set_rpm",
                        "value": 420.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let state = runtime.read().await;
    assert_eq!(
        state.last_control_error.as_deref(),
        Some("component bus timeout")
    );
    assert!(!state.auto_enabled);
}

#[tokio::test]
async fn component_control_audit_failure_latches_fault_without_committing_runtime_targets() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
    }
    let original_targets = runtime.read().await.targets.clone();
    let (device, recorded_device) = recording_component_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );
    db.break_control_events_for_tests().unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/devices/reactor_001/components/stirrer_motor/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "action": "set_rpm",
                        "value": 420.0,
                        "reason": "audit storage failed after component write"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(recorded_device.component_writes.lock().unwrap().len(), 1);
    let state = runtime.read().await;
    assert_eq!(state.targets, original_targets);
    assert!(!state.auto_enabled);
    assert!(state
        .last_control_error
        .as_deref()
        .unwrap_or_default()
        .contains("component control audit failed after device action"));
}

#[tokio::test]
async fn component_control_rechecks_interlocks_after_audit_before_runtime_commit() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
    }
    let original_targets = runtime.read().await.targets.clone();
    {
        let runtime = runtime.clone();
        let targets_before_audit = original_targets.clone();
        db.after_control_event_success_for_tests(Arc::new(move || {
            let mut state = runtime
                .try_write()
                .expect("runtime lock should be available after audit insert");
            assert_eq!(state.targets, targets_before_audit);
            state.emergency_stop = true;
        }));
    }
    let (device, recorded_device) = recording_component_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/devices/reactor_001/components/stirrer_motor/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "action": "set_rpm",
                        "value": 420.0,
                        "reason": "emergency stop appeared after component audit"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(recorded_device.component_writes.lock().unwrap().len(), 1);
    let state = runtime.read().await;
    assert_eq!(state.targets, original_targets);
    assert!(state.emergency_stop);
    assert!(!state.auto_enabled);
    assert!(state
        .last_control_error
        .as_deref()
        .unwrap_or_default()
        .contains("component control final interlock failed after device action"));
    assert!(db
        .recent_control_events(10)
        .unwrap()
        .iter()
        .any(|event| event.event_type == "component_control"));
}

#[tokio::test]
async fn component_control_rejects_stale_runtime_targets_after_audit_before_commit() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
    }
    let original_targets = runtime.read().await.targets.clone();
    let later_targets = ControlTargets {
        temperature_c: original_targets.temperature_c,
        heat_time_s: original_targets.heat_time_s,
        hold_time_s: original_targets.hold_time_s,
        cool_time_s: original_targets.cool_time_s,
        stirrer_rpm: 360.0,
        shake_speed_cpm: original_targets.shake_speed_cpm,
        target_pressure_mpa: original_targets.target_pressure_mpa,
    };
    {
        let runtime = runtime.clone();
        let targets_before_audit = original_targets.clone();
        let later_targets = later_targets.clone();
        db.after_control_event_success_for_tests(Arc::new(move || {
            let mut state = runtime
                .try_write()
                .expect("runtime lock should be available after audit insert");
            assert_eq!(state.targets, targets_before_audit);
            state.targets = later_targets.clone();
        }));
    }
    let (device, recorded_device) = recording_component_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/devices/reactor_001/components/stirrer_motor/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "action": "set_rpm",
                        "value": 420.0,
                        "reason": "runtime targets changed after component audit"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("stale runtime targets"));
    assert_eq!(recorded_device.component_writes.lock().unwrap().len(), 1);
    let state = runtime.read().await;
    assert_eq!(state.targets, later_targets);
    assert!(!state.auto_enabled);
    assert!(state
        .last_control_error
        .as_deref()
        .unwrap_or_default()
        .contains("component control final interlock failed after device action"));
    drop(state);
    assert!(db
        .recent_control_events(10)
        .unwrap()
        .iter()
        .any(|event| event.event_type == "component_control"));
}

#[tokio::test]
async fn process_start_fails_closed_when_control_fault_is_uncleared() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.last_control_error = Some("write timeout".to_string());
    }
    let process_id = add_simple_process(&db, "blocked process start");
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri(format!("/api/processes/{process_id}/start"))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, None);
    assert_eq!(state.auto_enabled, false);
    assert_eq!(db.recent_control_events(10).unwrap().len(), 0);
}

#[tokio::test]
async fn control_fault_reset_refuses_to_clear_a_terminated_control_loop_supervisor() {
    // Regression for the main.rs fail-safe monitor: when the control-loop task
    // dies it latches control_loop_terminated = true. reset_control_fault must
    // REFUSE to clear that fault, because the supervisor is gone and is only
    // re-spawned by a process restart. If reset cleared it, the API would report
    // "no fault" while nothing supervises the device, and ensure_target_update_
    // interlock_clear would then let automatic control resume unsupervised.
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(healthy_device_status());
        // Simulate the fail-safe monitor having latched the supervisor death.
        state.control_loop_terminated = true;
        state.latch_control_fault(
            "control loop task terminated; automatic control disabled until process restart and field re-verification",
        );
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let reset = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/fault/reset")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // Field state is otherwise healthy, but the supervisor is dead, so reset
    // must still be refused — a process restart is the only recovery.
    assert_eq!(reset.status(), StatusCode::CONFLICT);
    let body = to_bytes(reset.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("control loop task has terminated"));
    let state = runtime.read().await;
    assert!(
        state.last_control_error.is_some(),
        "fault must NOT be cleared"
    );
    assert!(state.control_loop_terminated);
    drop(state);
    // No successful reset audit row must have been written.
    assert!(!db
        .recent_control_events(10)
        .unwrap()
        .iter()
        .any(|event| event.event_type == "control_fault_reset"));
}

#[tokio::test]
async fn control_fault_requires_explicit_reset_and_keeps_auto_disabled() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.last_control_error = Some("write timeout".to_string());
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: true,
        },
        PathBuf::from("static"),
    );

    let new_sample = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/test/pipeline-sample")
                .header(TEST_CONFIRM_HEADER.0, TEST_CONFIRM_HEADER.1)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 51.0,
                        "pressure_mpa": 0.12,
                        "stirrer_rpm": 250.0,
                        "shake_speed_cpm": 24.0,
                        "tilt_state": 1,
                        "flow_rate_l_min": 2.0,
                        "product_concentration_percent": 11.0,
                        "ph": 6.8
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(new_sample.status(), StatusCode::OK);
    assert_eq!(
        runtime.read().await.last_control_error.as_deref(),
        Some("write timeout")
    );

    let blocked = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/targets")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 72.0,
                        "stirrer_rpm": 360.0,
                        "shake_speed_cpm": 24.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(blocked.status(), StatusCode::SERVICE_UNAVAILABLE);

    let reset = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/fault/reset")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reset.status(), StatusCode::NO_CONTENT);
    let state = runtime.read().await;
    assert_eq!(state.last_control_error, None);
    assert!(!state.auto_enabled);
    drop(state);
    let events = db.recent_control_events(10).unwrap();
    assert!(events
        .iter()
        .any(|event| event.event_type == "control_fault_auto_disabled"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "control_fault_reset"));
}

#[tokio::test]
async fn pipeline_sample_forces_auto_disabled_when_control_fault_is_latched() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.last_control_error = Some("write timeout".to_string());
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .header(TEST_CONFIRM_HEADER.0, TEST_CONFIRM_HEADER.1)
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 51.0,
                        "pressure_mpa": 0.12,
                        "stirrer_rpm": 250.0,
                        "shake_speed_cpm": 24.0,
                        "tilt_state": 1,
                        "flow_rate_l_min": 2.0,
                        "product_concentration_percent": 11.0,
                        "ph": 6.8
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let state = runtime.read().await;
    assert!(!state.auto_enabled);
    assert_eq!(state.last_control_error.as_deref(), Some("write timeout"));
    drop(state);
    let events = db.recent_control_events(10).unwrap();
    assert!(events
        .iter()
        .any(|event| event.event_type == "control_fault_auto_disabled"));
}

#[tokio::test]
async fn control_fault_reset_does_not_clear_fault_when_audit_fails() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.last_control_error = Some("write timeout".to_string());
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );
    db.break_control_events_for_tests().unwrap();

    let reset = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/fault/reset")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reset.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let state = runtime.read().await;
    assert_eq!(state.last_control_error.as_deref(), Some("write timeout"));
    assert!(!state.auto_enabled);
}

#[tokio::test]
async fn control_fault_reset_rejects_unclosed_active_batch_tail() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let batch = db
        .create_batch_for_process_sqlx(None, "unclosed tail", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    db.finish_batch_sqlx(batch.id).await.unwrap();
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(batch.id);
        state.auto_enabled = true;
        state.last_control_error =
            Some("batch finish audit failed after device action".to_string());
    }
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let reset = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/fault/reset")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reset.status(), StatusCode::CONFLICT);
    let body = to_bytes(reset.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("retry stop/finish to close production state"));
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, Some(batch.id));
    assert!(!state.auto_enabled);
    assert_eq!(
        state.last_control_error.as_deref(),
        Some("batch finish audit failed after device action")
    );
    drop(state);

    let finish_retry = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/batches/{}/finish", batch.id))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(finish_retry.status(), StatusCode::NO_CONTENT);
    assert_eq!(runtime.read().await.active_batch_id, None);
    assert_eq!(
        recorded_device.writes.lock().unwrap().len(),
        1,
        "closing an already-finished active batch should repeat the risk-reducing stop write"
    );

    let reset = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/fault/reset")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reset.status(), StatusCode::NO_CONTENT);
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, None);
    assert!(!state.auto_enabled);
    assert!(state.last_control_error.is_none());
}

#[tokio::test]
async fn control_fault_reset_rechecks_fault_identity_after_audit() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.last_control_error = Some("write timeout".to_string());
    }
    {
        let runtime = runtime.clone();
        db.after_control_event_success_for_tests(Arc::new(move || {
            let mut state = runtime
                .try_write()
                .expect("runtime lock should be available after fault reset audit insert");
            state.last_control_error = Some("new relay confirmation fault".to_string());
            state.auto_enabled = true;
        }));
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let reset = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/fault/reset")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reset.status(), StatusCode::CONFLICT);
    let state = runtime.read().await;
    assert_eq!(
        state.last_control_error.as_deref(),
        Some("new relay confirmation fault")
    );
    assert!(!state.auto_enabled);
    drop(state);
    assert!(db
        .recent_control_events(10)
        .unwrap()
        .iter()
        .any(|event| event.event_type == "control_fault_reset"));
}

#[tokio::test]
async fn control_fault_reset_rechecks_fault_generation_after_audit() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.latch_control_fault("write timeout");
    }
    {
        let runtime = runtime.clone();
        db.after_control_event_success_for_tests(Arc::new(move || {
            let mut state = runtime
                .try_write()
                .expect("runtime lock should be available after fault reset audit insert");
            state.latch_control_fault("write timeout");
            state.auto_enabled = true;
        }));
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let reset = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/fault/reset")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(reset.status(), StatusCode::CONFLICT);
    let body = to_bytes(reset.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("latched control fault changed during reset"));
    let state = runtime.read().await;
    assert_eq!(state.last_control_error.as_deref(), Some("write timeout"));
    assert!(!state.auto_enabled);
    assert!(state.control_fault_generation >= 2);
    drop(state);
    assert!(db
        .recent_control_events(10)
        .unwrap()
        .iter()
        .any(|event| event.event_type == "control_fault_reset"));
}

#[tokio::test]
async fn control_fault_reset_rejects_uncleared_downstream_command_failure() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.last_control_error = Some("write timeout".to_string());
        state.device_status = Some(DeviceStatusSnapshot {
            last_command_request_id: Some("cmd-7".to_string()),
            last_command_ok: Some(false),
            last_command_error: Some("driver rejected relay".to_string()),
            ..healthy_device_status()
        });
    }
    let app = router(
        AppState {
            db,
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let reset = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/fault/reset")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reset.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(reset.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("downstream command fault is still reported"));
    assert_eq!(
        runtime.read().await.last_control_error.as_deref(),
        Some("write timeout")
    );
}

#[tokio::test]
async fn control_fault_reset_rejects_unfinished_db_batch_recovery_state() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let batch = db
        .create_batch_for_process_sqlx(None, "orphan db batch", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    {
        let mut state = runtime.write().await;
        state.last_control_error = Some("tail audit failed".to_string());
        state.device_status = Some(healthy_device_status());
    }
    let app = router(
        AppState {
            db,
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let reset = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/fault/reset")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reset.status(), StatusCode::CONFLICT);
    let body = to_bytes(reset.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains("unfinished batch recovery is resolved")
            && message.contains(&batch.id.to_string()),
        "unexpected reset rejection message: {message}"
    );
    let state = runtime.read().await;
    assert_eq!(
        state.last_control_error.as_deref(),
        Some("tail audit failed")
    );
    assert!(!state.auto_enabled);
}

#[tokio::test]
async fn emergency_reset_requires_fresh_sample_and_does_not_clear_control_fault() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.emergency_stop = true;
        state.auto_enabled = true;
        state.last_control_error = Some("write timeout".to_string());
    }
    let app = router(
        AppState {
            db,
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let without_sample = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/emergency-stop/reset")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(without_sample.status(), StatusCode::SERVICE_UNAVAILABLE);
    {
        let state = runtime.read().await;
        assert!(state.emergency_stop);
        assert!(!state.auto_enabled);
    }

    {
        let mut state = runtime.write().await;
        state.latest_sample = Some(fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0));
    }
    let reset = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/emergency-stop/reset")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reset.status(), StatusCode::NO_CONTENT);
    let state = runtime.read().await;
    assert!(!state.emergency_stop);
    assert!(!state.auto_enabled);
    assert_eq!(state.last_control_error.as_deref(), Some("write timeout"));
}

#[tokio::test]
async fn emergency_reset_does_not_clear_emergency_stop_when_audit_fails() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.emergency_stop = true;
        state.auto_enabled = true;
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );
    db.break_control_events_for_tests().unwrap();

    let reset = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/emergency-stop/reset")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reset.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let state = runtime.read().await;
    assert!(state.emergency_stop);
    assert!(!state.auto_enabled);
}

#[tokio::test]
async fn emergency_reset_rechecks_field_state_after_audit() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.emergency_stop = true;
        state.auto_enabled = true;
        state.device_status = Some(healthy_device_status());
    }
    {
        let runtime = runtime.clone();
        db.after_control_event_success_for_tests(Arc::new(move || {
            let mut state = runtime
                .try_write()
                .expect("runtime lock should be available after emergency reset audit insert");
            state.device_status = Some(DeviceStatusSnapshot {
                last_command_request_id: Some("cmd-estop-reset-race".to_string()),
                last_command_ok: Some(false),
                last_command_error: Some("safety relay re-latched".to_string()),
                ..healthy_device_status()
            });
            state.auto_enabled = true;
        }));
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let reset = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/emergency-stop/reset")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reset.status(), StatusCode::SERVICE_UNAVAILABLE);
    let state = runtime.read().await;
    assert!(state.emergency_stop);
    assert!(!state.auto_enabled);
    drop(state);
    assert!(db
        .recent_control_events(10)
        .unwrap()
        .iter()
        .any(|event| event.event_type == "emergency_stop_reset"));
}

#[tokio::test]
async fn emergency_reset_rejects_stop_generation_change_after_audit() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.engage_emergency_stop();
        state.auto_enabled = true;
        state.device_status = Some(healthy_device_status());
    }
    {
        let runtime = runtime.clone();
        db.after_control_event_success_for_tests(Arc::new(move || {
            let mut state = runtime
                .try_write()
                .expect("runtime lock should be available after emergency reset audit insert");
            state.engage_emergency_stop();
            state.auto_enabled = true;
        }));
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let reset = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/emergency-stop/reset")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(reset.status(), StatusCode::CONFLICT);
    let body = to_bytes(reset.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("emergency stop changed during reset"));
    let state = runtime.read().await;
    assert!(state.emergency_stop);
    assert!(!state.auto_enabled);
    assert!(state.emergency_stop_generation >= 2);
    drop(state);
    assert!(db
        .recent_control_events(10)
        .unwrap()
        .iter()
        .any(|event| event.event_type == "emergency_stop_reset"));
}

#[tokio::test]
async fn emergency_reset_rejects_unhealthy_downstream_status_and_command_faults() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.emergency_stop = true;
        state.device_status = Some(DeviceStatusSnapshot {
            connected: false,
            ..healthy_device_status()
        });
    }
    let app = router(
        AppState {
            db,
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let unhealthy_status = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/emergency-stop/reset")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unhealthy_status.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(unhealthy_status.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("device status is not healthy"));
    assert!(runtime.read().await.emergency_stop);

    {
        let mut state = runtime.write().await;
        state.device_status = Some(DeviceStatusSnapshot {
            last_command_request_id: Some("cmd-estop-reset".to_string()),
            last_command_ok: Some(false),
            last_command_error: Some("relay still latched".to_string()),
            ..healthy_device_status()
        });
    }
    let command_fault = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/emergency-stop/reset")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(command_fault.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(command_fault.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("downstream command fault is still reported"));
    assert!(runtime.read().await.emergency_stop);
}

#[tokio::test]
async fn emergency_reset_rejects_open_active_batch_until_stop_or_finish_closes_it() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let batch = db
        .create_batch_for_process_sqlx(None, "emergency active batch", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(batch.id);
        state.emergency_stop = true;
        state.auto_enabled = true;
        state.device_status = Some(healthy_device_status());
    }
    let app = router(
        AppState {
            db,
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let reset = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/emergency-stop/reset")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reset.status(), StatusCode::CONFLICT);
    let body = to_bytes(reset.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains("active batch")
            && message.contains(&batch.id.to_string())
            && message.contains("retry stop/finish"),
        "unexpected emergency reset rejection message: {message}"
    );
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, Some(batch.id));
    assert!(state.emergency_stop);
    assert!(!state.auto_enabled);
}

#[tokio::test]
async fn emergency_reset_rejects_unfinished_db_batch_recovery_state() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let batch = db
        .create_batch_for_process_sqlx(None, "orphan db batch", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    {
        let mut state = runtime.write().await;
        state.emergency_stop = true;
        state.auto_enabled = true;
        state.device_status = Some(healthy_device_status());
    }
    let app = router(
        AppState {
            db,
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let reset = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/emergency-stop/reset")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reset.status(), StatusCode::CONFLICT);
    let body = to_bytes(reset.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains("unfinished batch recovery is resolved")
            && message.contains(&batch.id.to_string()),
        "unexpected emergency reset rejection message: {message}"
    );
    let state = runtime.read().await;
    assert!(state.emergency_stop);
    assert!(!state.auto_enabled);
}

#[tokio::test]
async fn manual_lock_disables_auto_and_unlock_does_not_resume_it() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
    }
    let app = router(
        AppState {
            db,
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let lock = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/manual-lock")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"locked": true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(lock.status(), StatusCode::NO_CONTENT);
    {
        let state = runtime.read().await;
        assert!(state.manual_lock);
        assert!(!state.auto_enabled);
    }

    let unlock = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/manual-lock")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"locked": false}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unlock.status(), StatusCode::NO_CONTENT);
    let state = runtime.read().await;
    assert!(!state.manual_lock);
    assert!(!state.auto_enabled);
}

#[tokio::test]
async fn manual_lock_unlock_rejects_unfinished_db_batch_recovery_state() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let batch = db
        .create_batch_for_process_sqlx(None, "orphan db batch", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    {
        let mut state = runtime.write().await;
        state.manual_lock = true;
        state.auto_enabled = true;
        state.device_status = Some(healthy_device_status());
    }
    let app = router(
        AppState {
            db,
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let unlock = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/manual-lock")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"locked": false}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unlock.status(), StatusCode::CONFLICT);
    let body = to_bytes(unlock.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains("unfinished batch recovery is resolved")
            && message.contains(&batch.id.to_string()),
        "unexpected manual unlock rejection message: {message}"
    );
    let state = runtime.read().await;
    assert!(state.manual_lock);
    assert!(!state.auto_enabled);
}

#[tokio::test]
async fn manual_lock_unlock_requires_proven_safe_field_state() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.manual_lock = true;
        state.auto_enabled = true;
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let without_sample = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/manual-lock")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"locked": false}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(without_sample.status(), StatusCode::SERVICE_UNAVAILABLE);
    {
        let state = runtime.read().await;
        assert!(state.manual_lock);
        assert!(!state.auto_enabled);
    }

    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(DeviceStatusSnapshot {
            connected: false,
            ..healthy_device_status()
        });
    }
    let unhealthy_status = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/manual-lock")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"locked": false}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unhealthy_status.status(), StatusCode::SERVICE_UNAVAILABLE);
    {
        let state = runtime.read().await;
        assert!(state.manual_lock);
        assert!(!state.auto_enabled);
    }

    {
        let mut state = runtime.write().await;
        state.device_status = Some(DeviceStatusSnapshot {
            last_command_request_id: Some("cmd-unlock".to_string()),
            last_command_ok: Some(false),
            last_command_error: Some("relay confirmation missing".to_string()),
            ..healthy_device_status()
        });
    }
    let command_fault = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/manual-lock")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"locked": false}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(command_fault.status(), StatusCode::SERVICE_UNAVAILABLE);
    {
        let state = runtime.read().await;
        assert!(state.manual_lock);
        assert!(!state.auto_enabled);
    }

    {
        let mut state = runtime.write().await;
        state.device_status = Some(healthy_device_status());
        state.last_control_error = Some("write timeout".to_string());
    }
    let control_fault = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/manual-lock")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"locked": false}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(control_fault.status(), StatusCode::SERVICE_UNAVAILABLE);
    {
        let state = runtime.read().await;
        assert!(state.manual_lock);
        assert!(!state.auto_enabled);
    }

    {
        let mut state = runtime.write().await;
        state.last_control_error = None;
        state.emergency_stop = true;
    }
    let emergency_stop = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/manual-lock")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"locked": false}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(emergency_stop.status(), StatusCode::CONFLICT);
    {
        let state = runtime.read().await;
        assert!(state.manual_lock);
        assert!(!state.auto_enabled);
    }

    {
        let mut state = runtime.write().await;
        state.emergency_stop = false;
        state.auto_enabled = true;
    }
    let unlock = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/manual-lock")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"locked": false}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unlock.status(), StatusCode::NO_CONTENT);
    let state = runtime.read().await;
    assert!(!state.manual_lock);
    assert!(!state.auto_enabled);
}

#[tokio::test]
async fn manual_lock_unlock_rejects_lock_generation_change_after_audit() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.engage_manual_lock();
        state.device_status = Some(healthy_device_status());
    }
    {
        let runtime = runtime.clone();
        db.after_control_event_success_for_tests(Arc::new(move || {
            let mut state = runtime
                .try_write()
                .expect("runtime lock should be available after manual unlock audit insert");
            state.engage_manual_lock();
            state.auto_enabled = true;
        }));
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let unlock = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/manual-lock")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"locked": false}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(unlock.status(), StatusCode::CONFLICT);
    let body = to_bytes(unlock.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("a safety latch fired during the audit window"));
    let state = runtime.read().await;
    assert!(state.manual_lock);
    assert!(!state.auto_enabled);
    assert!(state.manual_lock_generation >= 2);
    drop(state);
    // The refused-unlock path must write a manual_unlock_refused audit row so
    // the chain is self-consistent about the lock still being engaged. A
    // manual_lock_off row may also be present (it is the audit anchor the
    // generation re-check needs); if so, the refused row must be newer.
    let events = db.recent_control_events(10).unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "manual_unlock_refused"),
        "refused unlock must leave a manual_unlock_refused audit row: {events:?}"
    );
    let off_idx = events
        .iter()
        .position(|event| event.event_type == "manual_lock_off");
    let refused_idx = events
        .iter()
        .position(|event| event.event_type == "manual_unlock_refused");
    // recent_control_events returns oldest-first (ORDER BY id ASC), so a larger
    // index means a newer event. The refused row must be newer than the off
    // anchor so a reader sees the refusal as the final word.
    if let (Some(off), Some(refused)) = (off_idx, refused_idx) {
        assert!(
            refused > off,
            "manual_unlock_refused must be newer than the manual_lock_off anchor: {events:?}"
        );
    }
}
#[tokio::test]
async fn risk_increasing_switches_do_not_commit_when_audit_fails() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.manual_lock = true;
        state.auto_enabled = false;
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );
    db.break_control_events_for_tests().unwrap();

    let unlock = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/manual-lock")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"locked": false}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unlock.status(), StatusCode::INTERNAL_SERVER_ERROR);
    {
        let state = runtime.read().await;
        assert!(state.manual_lock);
        assert!(!state.auto_enabled);
    }

    {
        let mut state = runtime.write().await;
        state.manual_lock = false;
        state.auto_enabled = false;
    }
    let enable_auto = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/auto")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"enabled": true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(enable_auto.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let state = runtime.read().await;
    assert!(!state.manual_lock);
    assert!(!state.auto_enabled);
}

#[tokio::test]
async fn risk_reducing_switches_keep_conservative_state_when_audit_fails() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );
    db.break_control_events_for_tests().unwrap();

    let disable_auto = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/auto")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"enabled": false}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(disable_auto.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!runtime.read().await.auto_enabled);

    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.manual_lock = false;
    }
    let lock = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/manual-lock")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"locked": true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(lock.status(), StatusCode::INTERNAL_SERVER_ERROR);
    {
        let state = runtime.read().await;
        assert!(state.manual_lock);
        assert!(!state.auto_enabled);
    }

    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.emergency_stop = false;
    }
    let estop = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/emergency-stop")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(estop.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let state = runtime.read().await;
    assert!(state.emergency_stop);
    assert!(!state.auto_enabled);
    assert!(state
        .last_control_error
        .as_deref()
        .unwrap_or_default()
        .contains("emergency stop audit failed after fail-safe state change"));
}

#[test]
fn runtime_startup_never_auto_enables_before_field_verification() {
    let mut safety = safety();
    safety.control.auto_enabled_default = true;
    safety.control.manual_lock_default = false;

    let runtime = RuntimeState::from_safety(&safety);

    assert!(!runtime.manual_lock);
    assert!(!runtime.auto_enabled);

    safety.control.manual_lock_default = true;
    let runtime = RuntimeState::from_safety(&safety);
    assert!(runtime.manual_lock);
    assert!(!runtime.auto_enabled);
}

#[tokio::test]
async fn new_production_rejects_unfinished_db_batch_even_when_runtime_is_idle() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let orphan = db
        .create_batch_for_process_sqlx(None, "orphan unfinished", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    let process_id = add_simple_process(&db, "blocked by orphan");
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let process_start = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/processes/{process_id}/start"))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(process_start.status(), StatusCode::CONFLICT);
    let body = to_bytes(process_start.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains("unfinished batch recovery must be resolved")
            && message.contains(&orphan.id.to_string()),
        "unexpected new production rejection message: {message}"
    );

    let batch_start = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/batches/start")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "must not start",
                        "target_temperature_c": 82.0,
                        "target_stirrer_rpm": 460.0,
                        "target_shake_speed_cpm": 24.0,
                        "heating_minutes": 75.0,
                        "stirring_minutes": 65.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(batch_start.status(), StatusCode::CONFLICT);

    let v1_auto_start = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/reactor/reactor_001/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "auto_start": true,
                        "params": {
                            "target_temp": 70.0,
                            "stir_speed": 300.0,
                            "heat_time": 900.0,
                            "hold_time": 900.0
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(v1_auto_start.status(), StatusCode::CONFLICT);
    assert_eq!(runtime.read().await.active_batch_id, None);
    assert_eq!(
        db.latest_unfinished_batch_sqlx().await.unwrap().unwrap().id,
        orphan.id
    );
    assert_eq!(db.recent_batches_sqlx(10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn new_production_rejects_missing_persisted_active_batch_before_creating_batch() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let process_id = add_simple_process(&db, "blocked by missing active batch record");
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(4242);
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let process_start = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/processes/{process_id}/start"))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(process_start.status(), StatusCode::CONFLICT);

    let batch_start = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/batches/start")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "must not create",
                        "target_temperature_c": 82.0,
                        "target_stirrer_rpm": 460.0,
                        "target_shake_speed_cpm": 24.0,
                        "heating_minutes": 75.0,
                        "stirring_minutes": 65.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(batch_start.status(), StatusCode::CONFLICT);
    let body = to_bytes(batch_start.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains("unfinished batch recovery must be resolved") && message.contains("4242"),
        "unexpected batch start rejection message: {message}"
    );

    let v1_auto_start = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/reactor/reactor_001/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "auto_start": true,
                        "params": {
                            "target_temp": 70.0,
                            "stir_speed": 300.0,
                            "heat_time": 900.0,
                            "hold_time": 900.0
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(v1_auto_start.status(), StatusCode::CONFLICT);
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, Some(4242));
    assert!(!state.auto_enabled);
    drop(state);
    assert!(db.recent_batches_sqlx(10).await.unwrap().is_empty());
}

#[tokio::test]
async fn new_production_rejects_older_unfinished_batch_after_latest_is_closed() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let older = db
        .create_batch_for_process_sqlx(None, "older orphan unfinished", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    let latest = db
        .create_batch_for_process_sqlx(None, "latest recovered unfinished", 65.0, 320.0, 12.0, 12.0)
        .await
        .unwrap();
    db.finish_batch_sqlx(latest.id).await.unwrap();
    let process_id = add_simple_process(&db, "blocked by older orphan");
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let process_start = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/processes/{process_id}/start"))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(process_start.status(), StatusCode::CONFLICT);
    let body = to_bytes(process_start.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains("unfinished batch recovery must be resolved")
            && message.contains(&older.id.to_string()),
        "unexpected new production rejection message: {message}"
    );
    assert_eq!(runtime.read().await.active_batch_id, None);
    assert_eq!(db.recent_batches_sqlx(10).await.unwrap().len(), 2);
}

#[tokio::test]
async fn finish_batch_disables_auto_control() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let batch = db
        .create_batch_for_process_sqlx(None, "manual finish", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(batch.id);
        state.auto_enabled = true;
        state.targets.temperature_c = 82.0;
        state.targets.stirrer_rpm = 460.0;
        state.targets.shake_speed_cpm = 24.0;
        state.targets.target_pressure_mpa = 0.5;
    }
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );
    {
        let runtime = runtime.clone();
        let batch_id = batch.id;
        db.after_control_event_success_for_tests(Arc::new(move || {
            let state = runtime
                .try_read()
                .expect("runtime lock should be available after batch finish audit insert");
            assert_eq!(state.active_batch_id, Some(batch_id));
            assert!(!state.auto_enabled);
            assert_eq!(state.targets.temperature_c, 20.0);
            assert_eq!(state.targets.stirrer_rpm, 0.0);
            assert_eq!(state.targets.shake_speed_cpm, 0.0);
            assert_eq!(state.targets.target_pressure_mpa, 0.0);
        }));
    }

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/batches/{}/finish", batch.id))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, None);
    assert!(!state.auto_enabled);
    assert_eq!(state.targets.temperature_c, 20.0);
    assert_eq!(state.targets.stirrer_rpm, 0.0);
    assert_eq!(state.targets.shake_speed_cpm, 0.0);
    assert_eq!(state.targets.target_pressure_mpa, 0.0);
    assert!(state.last_control_error.is_none());
    drop(state);
    let writes = recorded_device.writes.lock().unwrap();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].target_temperature_c, 20.0);
    assert_eq!(writes[0].target_stirrer_rpm, 0.0);
    assert_eq!(writes[0].target_shake_speed_cpm, 0.0);
    assert_eq!(writes[0].target_pressure_mpa, 0.0);
}

#[tokio::test]
async fn finish_batch_rejects_active_batch_change_after_stop_before_finish() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let batch = db
        .create_batch_for_process_sqlx(None, "finish race original", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    let replacement_batch = db
        .create_batch_for_process_sqlx(None, "finish race replacement", 65.0, 320.0, 12.0, 12.0)
        .await
        .unwrap();
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(batch.id);
        state.auto_enabled = true;
        state.targets.temperature_c = 82.0;
        state.targets.stirrer_rpm = 460.0;
        state.targets.shake_speed_cpm = 24.0;
        state.targets.target_pressure_mpa = 0.5;
    }
    let (device, recorded_device) =
        change_active_batch_on_write_device(runtime.clone(), Some(replacement_batch.id));
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri(format!("/api/batches/{}/finish", batch.id))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, Some(replacement_batch.id));
    assert!(!state.auto_enabled);
    assert_eq!(state.targets.temperature_c, 20.0);
    assert_eq!(state.targets.stirrer_rpm, 0.0);
    assert_eq!(state.targets.shake_speed_cpm, 0.0);
    assert_eq!(state.targets.target_pressure_mpa, 0.0);
    assert!(state
        .last_control_error
        .as_deref()
        .unwrap_or_default()
        .contains("batch finish active batch changed after stop command"));
    drop(state);
    let writes = recorded_device.writes.lock().unwrap();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].target_temperature_c, 20.0);
    assert_eq!(writes[0].target_stirrer_rpm, 0.0);
    assert_eq!(writes[0].target_shake_speed_cpm, 0.0);
    assert_eq!(writes[0].target_pressure_mpa, 0.0);
    drop(writes);
    let batch = db.batch_by_id_sqlx(batch.id).await.unwrap().unwrap();
    assert!(
        batch.finished_at.is_none(),
        "ambiguous active batch identity must not close the original batch"
    );
    assert_eq!(db.audit_event_count(Some("batch_finished")).unwrap(), 0);
}

#[tokio::test]
async fn finish_batch_keeps_runtime_stopped_when_audit_fails() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let batch = db
        .create_batch_for_process_sqlx(None, "finish audit failure", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(batch.id);
        state.auto_enabled = true;
        state.targets.temperature_c = 82.0;
        state.targets.stirrer_rpm = 460.0;
        state.targets.shake_speed_cpm = 24.0;
        state.targets.target_pressure_mpa = 0.5;
    }
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );
    db.break_control_events_for_tests().unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/batches/{}/finish", batch.id))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, Some(batch.id));
    assert!(!state.auto_enabled);
    assert_eq!(state.targets.temperature_c, 20.0);
    assert_eq!(state.targets.stirrer_rpm, 0.0);
    assert_eq!(state.targets.shake_speed_cpm, 0.0);
    assert_eq!(state.targets.target_pressure_mpa, 0.0);
    assert!(state
        .last_control_error
        .as_deref()
        .unwrap_or_default()
        .contains("batch finish audit failed after device action"));
    drop(state);
    let writes = recorded_device.writes.lock().unwrap();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].target_temperature_c, 20.0);
    assert_eq!(writes[0].target_pressure_mpa, 0.0);
    drop(writes);

    db.repair_control_events_for_tests().unwrap();
    {
        let mut state = runtime.write().await;
        state.last_control_error = None;
    }
    let retry = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/batches/{}/finish", batch.id))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(retry.status(), StatusCode::NO_CONTENT);
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, None);
    assert!(!state.auto_enabled);
    assert!(state.last_control_error.is_none());
    drop(state);
    let writes = recorded_device.writes.lock().unwrap();
    assert_eq!(
        writes.len(),
        2,
        "retrying a finished-but-unclosed active batch should repeat the risk-reducing stop write"
    );
    assert_eq!(writes[1].target_temperature_c, 20.0);
    assert_eq!(writes[1].target_stirrer_rpm, 0.0);
    assert_eq!(writes[1].target_shake_speed_cpm, 0.0);
    assert_eq!(writes[1].target_pressure_mpa, 0.0);
}

#[tokio::test]
async fn finish_active_batch_does_not_mark_finished_when_device_stop_fails() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let batch = db
        .create_batch_for_process_sqlx(None, "finish stop failure", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(batch.id);
        state.auto_enabled = true;
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: failing_target_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri(format!("/api/batches/{}/finish", batch.id))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, Some(batch.id));
    assert!(!state.auto_enabled);
    assert!(state.last_control_error.is_some());
    drop(state);
    let batch = db.batch_by_id_sqlx(batch.id).await.unwrap().unwrap();
    assert!(batch.finished_at.is_none());
}

#[tokio::test]
async fn finish_batch_rejects_non_active_batch_while_another_batch_is_running() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let old_batch = db
        .create_batch_for_process_sqlx(None, "old batch", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    let active_batch = db
        .create_batch_for_process_sqlx(None, "active batch", 70.0, 320.0, 20.0, 20.0)
        .await
        .unwrap();
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(active_batch.id);
        state.auto_enabled = true;
    }
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri(format!("/api/batches/{}/finish", old_batch.id))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, Some(active_batch.id));
    assert!(state.auto_enabled);
    drop(state);
    assert!(recorded_device.writes.lock().unwrap().is_empty());
    assert!(db
        .batch_by_id_sqlx(old_batch.id)
        .await
        .unwrap()
        .unwrap()
        .finished_at
        .is_none());
}

#[tokio::test]
async fn finish_batch_rejects_missing_or_already_finished_batch_without_audit() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let finished = db
        .create_batch_for_process_sqlx(None, "already finished", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    db.finish_batch_sqlx(finished.id).await.unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: db.clone(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/batches/999999/finish")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let duplicate = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/batches/{}/finish", finished.id))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
    assert!(db.recent_control_events(10).unwrap().is_empty());
}

#[tokio::test]
async fn finish_active_batch_writes_stop_when_batch_record_is_missing() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let batch = db
        .create_batch_for_process_sqlx(None, "finish missing active batch", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(batch.id);
        state.auto_enabled = true;
        state.targets.temperature_c = 82.0;
        state.targets.stirrer_rpm = 460.0;
        state.targets.shake_speed_cpm = 24.0;
        state.targets.target_pressure_mpa = 0.5;
    }
    db.clear_runtime_data_for_tests().unwrap();
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri(format!("/api/batches/{}/finish", batch.id))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, None);
    assert!(!state.auto_enabled);
    assert_eq!(state.targets.temperature_c, 20.0);
    assert_eq!(state.targets.stirrer_rpm, 0.0);
    assert_eq!(state.targets.shake_speed_cpm, 0.0);
    assert_eq!(state.targets.target_pressure_mpa, 0.0);
    assert!(state.last_control_error.is_none());
    drop(state);
    let writes = recorded_device.writes.lock().unwrap();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].target_temperature_c, 20.0);
    assert_eq!(writes[0].target_stirrer_rpm, 0.0);
    assert_eq!(writes[0].target_shake_speed_cpm, 0.0);
    assert_eq!(writes[0].target_pressure_mpa, 0.0);
    drop(writes);
    let events = db.recent_control_events(10).unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == "batch_finish_recovery_missing_batch"
            && event.batch_id.is_none()
            && event.reason.contains(&batch.id.to_string())
    }));
}

#[tokio::test]
async fn finish_active_missing_batch_keeps_retry_state_when_recovery_audit_fails() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let batch = db
        .create_batch_for_process_sqlx(
            None,
            "finish missing audit failure",
            60.0,
            300.0,
            10.0,
            10.0,
        )
        .await
        .unwrap();
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(batch.id);
        state.auto_enabled = true;
        state.targets.temperature_c = 82.0;
        state.targets.stirrer_rpm = 460.0;
        state.targets.shake_speed_cpm = 24.0;
        state.targets.target_pressure_mpa = 0.5;
    }
    db.clear_runtime_data_for_tests().unwrap();
    db.break_control_events_for_tests().unwrap();
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri(format!("/api/batches/{}/finish", batch.id))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, Some(batch.id));
    assert!(!state.auto_enabled);
    assert_eq!(state.targets.temperature_c, 20.0);
    assert_eq!(state.targets.stirrer_rpm, 0.0);
    assert_eq!(state.targets.shake_speed_cpm, 0.0);
    assert_eq!(state.targets.target_pressure_mpa, 0.0);
    assert!(state
        .last_control_error
        .as_deref()
        .unwrap_or_default()
        .contains("batch finish missing batch recovery audit failed after device action"));
    drop(state);
    let writes = recorded_device.writes.lock().unwrap();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].target_temperature_c, 20.0);
    drop(writes);

    db.repair_control_events_for_tests().unwrap();
    {
        let mut state = runtime.write().await;
        state.last_control_error = None;
    }
    let retry = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/batches/{}/finish", batch.id))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(retry.status(), StatusCode::NO_CONTENT);
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, None);
    assert!(!state.auto_enabled);
    assert!(state.last_control_error.is_none());
    drop(state);
    let writes = recorded_device.writes.lock().unwrap();
    assert_eq!(
        writes.len(),
        2,
        "retrying a missing-but-runtime-active batch should repeat the risk-reducing stop write"
    );
    assert_eq!(writes[1].target_temperature_c, 20.0);
    assert_eq!(writes[1].target_stirrer_rpm, 0.0);
    assert_eq!(writes[1].target_shake_speed_cpm, 0.0);
    assert_eq!(writes[1].target_pressure_mpa, 0.0);
}

#[tokio::test]
async fn deferred_auto_paths_fail_closed_without_proven_safe_field_state() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let enable_auto_without_sample = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/auto")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"enabled": true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        enable_auto_without_sample.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(!runtime.read().await.auto_enabled);

    let disable_auto_without_sample = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/auto")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"enabled": false}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(disable_auto_without_sample.status(), StatusCode::NO_CONTENT);

    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
    }
    let batch_without_sample = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/batches/start")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "should not arm auto loop",
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
    assert_eq!(
        batch_without_sample.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(!runtime.read().await.auto_enabled);
    assert_eq!(runtime.read().await.active_batch_id, None);

    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.last_control_error = Some("previous write failed".to_string());
    }
    let enable_auto_with_fault = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/auto")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"enabled": true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        enable_auto_with_fault.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(!runtime.read().await.auto_enabled);
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.latest_sample = None;
        state.last_control_error = None;
    }

    let v1_process_without_sample = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/reactor/reactor_001/process")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "process_id": "p-unsafe-load",
                        "name": "unsafe load",
                        "phases": [{
                            "phase": "heating",
                            "params": {
                                "duration": 300,
                                "target_temp": 82.0,
                                "stir_speed": 460.0,
                                "shake_speed": 30.0,
                                "target_pressure": 0.5
                            }
                        }]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        v1_process_without_sample.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );

    let state = runtime.read().await;
    assert!(!state.auto_enabled);
    assert_eq!(state.active_batch_id, None);
    assert_eq!(state.targets.temperature_c, 60.0);
    assert_eq!(state.targets.stirrer_rpm, 300.0);
    assert_eq!(db.recent_control_events(10).unwrap().len(), 1);
}

#[tokio::test]
async fn auto_enable_failure_forces_auto_disabled_before_returning_error() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.latest_sample = None;
    }
    let app = router(
        AppState {
            db,
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/control/auto")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"enabled": true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let state = runtime.read().await;
    assert!(!state.auto_enabled);
}

#[tokio::test]
async fn auto_enable_rejects_missing_persisted_active_batch_recovery_state() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(4242);
        state.auto_enabled = false;
    }
    let app = router(
        AppState {
            db,
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/control/auto")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"enabled": true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains("unfinished batch recovery is resolved") && message.contains("4242"),
        "unexpected auto enable rejection message: {message}"
    );
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, Some(4242));
    assert!(!state.auto_enabled);
}

#[tokio::test]
async fn auto_enable_rechecks_batch_recovery_state_after_audit() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let runtime = runtime.clone();
        db.after_control_event_success_for_tests(Arc::new(move || {
            let mut state = runtime
                .try_write()
                .expect("runtime lock should be available after auto enable audit insert");
            state.active_batch_id = Some(4242);
            state.auto_enabled = false;
        }));
    }
    let app = router(
        AppState {
            db,
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/control/auto")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"enabled": true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains("automatic control enable blocked")
            && message.contains("unfinished batch recovery is resolved"),
        "unexpected auto enable rejection message: {message}"
    );
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, Some(4242));
    assert!(!state.auto_enabled);
}

#[tokio::test]
async fn auto_enable_rejects_safety_generation_change_after_audit() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let runtime = runtime.clone();
        db.after_control_event_success_for_tests(Arc::new(move || {
            let mut state = runtime
                .try_write()
                .expect("runtime lock should be available after auto enable audit insert");
            state.latch_control_fault("transient relay confirmation fault");
            state.clear_control_fault();
        }));
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/control/auto")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"enabled": true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("safety state changed during audit"));
    let state = runtime.read().await;
    assert!(!state.auto_enabled);
    assert!(state.last_control_error.is_none());
    assert!(state.control_fault_generation >= 1);
    drop(state);
    assert!(db
        .recent_control_events(10)
        .unwrap()
        .iter()
        .any(|event| event.event_type == "auto_enabled"));
}

#[tokio::test]
async fn set_targets_rejects_safety_generation_change_after_audit() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let baseline_targets = runtime.read().await.targets.clone();
    {
        let runtime = runtime.clone();
        db.after_control_event_success_for_tests(Arc::new(move || {
            let mut state = runtime
                .try_write()
                .expect("runtime lock should be available after target audit insert");
            // A safety latch fires and is cleared again inside the audit window:
            // the boolean ends up clear, but the generation has advanced.
            state.latch_control_fault("transient relay confirmation fault");
            state.clear_control_fault();
        }));
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/control/targets")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "temperature_c": 92.5,
                        "stirrer_rpm": 460.0,
                        "shake_speed_cpm": 38.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("a safety latch fired during the audit window"));
    // The risk-increasing target update must not commit on top of a field state
    // that transiently latched a fault during the audit window.
    let state = runtime.read().await;
    assert_eq!(state.targets, baseline_targets);
    assert!(!state.auto_enabled);
    assert!(state.last_control_error.is_none());
    assert!(state.control_fault_generation >= 1);
    drop(state);
    // The audit event itself was still recorded (the hook runs on its success).
    assert!(db
        .recent_control_events(10)
        .unwrap()
        .iter()
        .any(|event| event.event_type == "operator_targets_updated"));
}

#[tokio::test]
async fn batch_start_rejects_safety_generation_change_after_audit() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let runtime = runtime.clone();
        db.after_control_event_success_for_tests(Arc::new(move || {
            let mut state = runtime
                .try_write()
                .expect("runtime lock should be available after batch start audit insert");
            state.latch_control_fault("transient relay confirmation fault");
            state.clear_control_fault();
        }));
    }
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety: safety.clone(),
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
                .uri("/api/batches/start")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "transient safety latch during batch start audit",
                        "target_temperature_c": 82.0,
                        "target_stirrer_rpm": 460.0,
                        "target_shake_speed_cpm": 24.0,
                        "heating_minutes": 75.0,
                        "stirring_minutes": 65.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("a safety latch fired during the audit window"));

    // Batch start commits auto control and an active batch after its audit, so a
    // transient latch must abort and roll back rather than re-arm production.
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, None);
    assert!(!state.auto_enabled);
    drop(state);
    // The created batch must not be left dangling/unfinished after the refusal.
    assert!(db.unfinished_batches_sqlx(10).await.unwrap().is_empty());
    let writes = recorded_device.writes.lock().unwrap();
    assert_eq!(
        writes.len(),
        2,
        "expected the device start write followed by a rollback stop write"
    );
    assert_eq!(writes[0].target_temperature_c, 82.0);
    assert_eq!(writes[1].target_shake_speed_cpm, 0.0);
    assert_eq!(writes[1].target_temperature_c, safety.temperature.min_c);
}

#[tokio::test]
async fn upper_computer_supports_audit_config_and_modbus_debug_pages() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    db.insert_sample(
        None,
        &SensorSnapshot {
            temperature_c: 42.5,
            pressure_mpa: 0.18,
            stirrer_rpm: 260.0,
            shake_speed_cpm: 30.0,
            tilt_state: 1,
            tilt_angle_deg: 12.5,
            flow_rate_l_min: 2.0,
            product_concentration_percent: 10.0,
            ph: 6.8,
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
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let config_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/config/summary")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(config_response.status(), StatusCode::OK);
    let body = to_bytes(config_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["data"]["device_mode"], "pipeline");
    assert_eq!(
        body["data"]["device"]["modbus"]["registers"]["temperature_c"]["address"],
        0
    );
    assert_eq!(body["data"]["integrations"]["rest_api"], true);
    assert_eq!(body["data"]["integrations"]["ainas_ready"], true);
    assert_eq!(body["data"]["integrations"]["ainas_task_api"], true);
    assert_eq!(
        body["data"]["data_security"]["storage_encryption"]["algorithm"],
        "AES-256-GCM"
    );
    assert_eq!(
        body["data"]["data_security"]["storage_encryption"]["enabled"],
        false
    );
    assert!(
        body["data"]["data_security"]["storage_encryption"]["encrypted_fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field == "integration_tasks.request_json")
    );
    assert_eq!(body["data"]["permissions"]["mode"], "local_role_policy");
    assert_eq!(
        body["data"]["permissions"]["authentication"],
        "bearer_session_enforced"
    );
    assert!(body["data"]["field_scenario"]["kind"].is_string());
    assert_eq!(body["data"]["production_line"]["kind"], "requires_inquiry");
    assert_eq!(
        body["data"]["production_line"]["requires_operator_inquiry"],
        true
    );
    assert_eq!(
        body["data"]["production_line"]["production_adaptation_blocked"],
        true
    );
    assert_eq!(body["data"]["integrations"]["mqtt"], false);
    assert_eq!(
        body["data"]["integrations"]["mqtt_status"]["task_topic"],
        "xingshu/reactor_001/tasks"
    );
    assert!(body["data"]["local_ai"]["model_family"].is_string());
    assert!(body["data"]["local_ai"]["runtime"].is_string());
    assert!(body["data"]["local_ai"]["mode"].is_string());
    assert_eq!(body["data"]["local_ai"]["ready_for_base_inference"], false);
    assert_eq!(body["data"]["local_ai"]["ready_for_lora_inference"], false);
    assert!(body["data"]["local_ai"]["ready_for_inference"].is_boolean());
    assert_eq!(
        body["data"]["local_ai"]["ready_for_inference"],
        body["data"]["local_ai"]["ready_for_lora_inference"]
    );
    assert!(body["data"]["local_ai"]["ready_for_training"].is_boolean());
    assert_eq!(body["data"]["local_ai"]["ready_for_prd_lora"], false);
    assert!(body["data"]["local_ai"]["inference"]["detail"].is_string());
    assert!(body["data"]["local_ai"]["lora_adapter"]["detail"].is_string());
    assert!(body["data"]["local_ai"]["rk_validation"]["detail"].is_string());
    assert!(body["data"]["local_ai"]["missing"].is_array());

    let modbus_map_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/modbus/registers")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(modbus_map_response.status(), StatusCode::OK);
    let body = to_bytes(modbus_map_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["data"]["tcp"]["enabled"], false);
    assert_eq!(body["data"]["read_registers"].as_array().unwrap().len(), 8);
    assert_eq!(body["data"]["write_registers"].as_array().unwrap().len(), 7);
    assert_eq!(body["data"]["coils"].as_array().unwrap().len(), 4);
    assert_eq!(body["data"]["discrete_inputs"].as_array().unwrap().len(), 5);
    assert!(body["data"]["read_registers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|register| register["name"] == "pressure_mpa" && register["address"] == 2));

    let permissions_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/permissions/roles")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(permissions_response.status(), StatusCode::OK);
    let body = to_bytes(permissions_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["data"]["roles"]
        .as_array()
        .unwrap()
        .iter()
        .any(|role| role["role"] == "operator"));

    let write_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/modbus/registers/target_stirrer_rpm/write")
                .header("authorization", auth_header("admin"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "value": 500.0,
                        "reason": "test modbus debug write"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(write_response.status(), StatusCode::OK);
    let body = to_bytes(write_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["data"]["applied_value"], 500.0);
    assert_eq!(body["data"]["raw"], 500);

    let read_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/modbus/registers/target_stirrer_rpm/read")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read_response.status(), StatusCode::OK);
    let body = to_bytes(read_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["data"]["value"], 500.0);
    assert_eq!(body["data"]["source"], "runtime_targets");

    let pressure_read_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/modbus/registers/pressure_mpa/read")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(pressure_read_response.status(), StatusCode::OK);
    let body = to_bytes(pressure_read_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["data"]["value"], 0.18);
    assert_eq!(body["data"]["raw"], 18);

    let pressure_out_of_range_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/modbus/registers/target_pressure_mpa/write")
                .header("authorization", auth_header("admin"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "value": 12.0,
                        "reason": "pressure out of range must not be clamped"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        pressure_out_of_range_response.status(),
        StatusCode::BAD_REQUEST
    );
    let body = to_bytes(pressure_out_of_range_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("target_pressure_mpa must be between 0 and 10"));

    let pressure_write_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/modbus/registers/target_pressure_mpa/write")
                .header("authorization", auth_header("admin"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "value": 9.0,
                        "reason": " =pressure\nvalid\t\u{0007}smoke test "
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(pressure_write_response.status(), StatusCode::OK);
    let body = to_bytes(pressure_write_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["data"]["applied_value"], 9.0);
    assert_eq!(body["data"]["raw"], 900);

    let missing_reason_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/modbus/registers/target_pressure_mpa/write")
                .header("authorization", auth_header("admin"))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "value": 8.0 }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_reason_response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(missing_reason_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("reason is required"));

    let null_reason_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/modbus/registers/target_pressure_mpa/write")
                .header("authorization", auth_header("admin"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "value": 8.0, "reason": null }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        null_reason_response.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let body = to_bytes(null_reason_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("must not be null"));

    let control_only_reason_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/modbus/registers/target_pressure_mpa/write")
                .header("authorization", auth_header("admin"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "value": 8.0, "reason": "\n\t\u{0007}" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        control_only_reason_response.status(),
        StatusCode::BAD_REQUEST
    );

    let audit_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/audit/logs?page=1&page_size=10")
                .header("authorization", auth_header("engineer"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(audit_response.status(), StatusCode::OK);
    let body = to_bytes(audit_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["data"]["total"], 2);
    assert_eq!(body["data"]["chain"]["valid"], true);
    assert!(body["data"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["event_type"] == "modbus_register_write"
            && event["event_hash"].as_str().unwrap().len() == 64));
    assert!(body["data"]["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| {
            event["event_type"] == "modbus_register_write"
                && event["reason"] == "=pressure valid smoke test"
        }));

    let csv_response = app
        .oneshot(
            Request::builder()
                .uri("/api/audit/export.csv")
                .header("authorization", auth_header("engineer"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(csv_response.status(), StatusCode::OK);
    let body = to_bytes(csv_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let csv = String::from_utf8(body.to_vec()).unwrap();
    assert!(csv.contains("modbus_register_write"));
    assert!(csv.contains("event_hash"));
    assert!(csv.contains("\"'=pressure valid smoke test\""));
    assert!(!csv.contains(",=pressure valid smoke test,"));
}

#[tokio::test]
async fn modbus_status_map_does_not_mark_device_connected_without_required_device_status() {
    let mut strict_safety = safety();
    strict_safety.control.require_device_status_for_control = true;
    let safety = Arc::new(strict_safety);
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(42.5, 0.18, 260.0, 30.0, 10.0)).await;
    let app = router(
        AppState {
            db,
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/modbus/registers")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let device_connected = body["data"]["discrete_inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|input| input["name"] == "device_connected")
        .unwrap();
    assert_eq!(device_connected["value"], false);
    let alarm_active = body["data"]["discrete_inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|input| input["name"] == "alarm_active")
        .unwrap();
    assert_eq!(alarm_active["value"], true);
}

#[tokio::test]
async fn modbus_status_map_does_not_mark_device_connected_from_stale_lab_sample() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let mut sample = fresh_sample(42.5, 0.18, 260.0, 30.0, 10.0);
    sample.captured_at = Utc::now() - Duration::milliseconds(safety.control.sensor_timeout_ms + 1);
    install_runtime_sample(&runtime, &db, sample).await;
    let app = router(
        AppState {
            db,
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/modbus/registers")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let discrete_inputs = body["data"]["discrete_inputs"].as_array().unwrap();
    let device_connected = discrete_inputs
        .iter()
        .find(|input| input["name"] == "device_connected")
        .unwrap();
    assert_eq!(device_connected["value"], false);
    let sensor_fresh = discrete_inputs
        .iter()
        .find(|input| input["name"] == "sensor_fresh")
        .unwrap();
    assert_eq!(sensor_fresh["value"], false);
    let alarm_active = discrete_inputs
        .iter()
        .find(|input| input["name"] == "alarm_active")
        .unwrap();
    assert_eq!(alarm_active["value"], true);
}

#[tokio::test]
async fn modbus_status_map_does_not_mark_device_connected_from_future_lab_sample() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let mut sample = fresh_sample(42.5, 0.18, 260.0, 30.0, 10.0);
    sample.captured_at = Utc::now() + Duration::milliseconds(5000);
    install_runtime_sample(&runtime, &db, sample).await;
    let app = router(
        AppState {
            db,
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/modbus/registers")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let discrete_inputs = body["data"]["discrete_inputs"].as_array().unwrap();
    let device_connected = discrete_inputs
        .iter()
        .find(|input| input["name"] == "device_connected")
        .unwrap();
    assert_eq!(device_connected["value"], false);
    let sensor_fresh = discrete_inputs
        .iter()
        .find(|input| input["name"] == "sensor_fresh")
        .unwrap();
    assert_eq!(sensor_fresh["value"], false);
    let alarm_active = discrete_inputs
        .iter()
        .find(|input| input["name"] == "alarm_active")
        .unwrap();
    assert_eq!(alarm_active["value"], true);
}

#[tokio::test]
async fn modbus_status_map_does_not_mark_device_connected_with_downstream_command_fault() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(42.5, 0.18, 260.0, 30.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(DeviceStatusSnapshot {
            last_command_request_id: Some("cmd-failed".to_string()),
            last_command_ok: Some(false),
            last_command_error: Some("relay did not acknowledge".to_string()),
            ..healthy_device_status()
        });
    }
    let app = router(
        AppState {
            db,
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/modbus/registers")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let device_connected = body["data"]["discrete_inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|input| input["name"] == "device_connected")
        .unwrap();
    assert_eq!(device_connected["value"], false);
    let alarm_active = body["data"]["discrete_inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|input| input["name"] == "alarm_active")
        .unwrap();
    assert_eq!(alarm_active["value"], true);
}

#[tokio::test]
async fn modbus_status_map_surfaces_unfinished_db_batch_recovery_state() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let orphan = db
        .create_batch_for_process_sqlx(None, "modbus orphan unfinished", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(42.5, 0.18, 260.0, 30.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(healthy_device_status());
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/modbus/registers")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let coils = body["data"]["coils"].as_array().unwrap();
    let process_running = coils
        .iter()
        .find(|coil| coil["name"] == "process_running")
        .unwrap();
    assert_eq!(process_running["value"], true);
    let discrete_inputs = body["data"]["discrete_inputs"].as_array().unwrap();
    let device_connected = discrete_inputs
        .iter()
        .find(|input| input["name"] == "device_connected")
        .unwrap();
    assert_eq!(device_connected["value"], false);
    let alarm_active = discrete_inputs
        .iter()
        .find(|input| input["name"] == "alarm_active")
        .unwrap();
    assert_eq!(alarm_active["value"], true);
    let active_batch = discrete_inputs
        .iter()
        .find(|input| input["name"] == "active_batch")
        .unwrap();
    assert_eq!(active_batch["value"], true);
    assert_eq!(
        db.latest_unfinished_batch_sqlx().await.unwrap().unwrap().id,
        orphan.id
    );
}

#[tokio::test]
async fn modbus_register_write_rejects_unfinished_db_batch_recovery_state() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    db.create_batch_for_process_sqlx(
        None,
        "modbus write orphan unfinished",
        60.0,
        300.0,
        10.0,
        10.0,
    )
    .await
    .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(42.5, 0.18, 260.0, 30.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(healthy_device_status());
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );
    let original_targets = runtime.read().await.targets.clone();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/modbus/registers/target_pressure_mpa/write")
                .header("authorization", auth_header("admin"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "value": 8.0,
                        "reason": "must reject recovery state"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("unfinished batch recovery"));
    assert_eq!(runtime.read().await.targets, original_targets);
    assert!(db
        .audit_events(10, 0, Some("modbus_register_write"))
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn modbus_register_write_failure_forces_auto_disabled_before_returning_error() {
    let mut strict_safety = safety();
    strict_safety.control.require_device_status_for_control = true;
    let safety = Arc::new(strict_safety);
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(42.5, 0.18, 260.0, 30.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );
    let original_targets = runtime.read().await.targets.clone();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/modbus/registers/target_pressure_mpa/write")
                .header("authorization", auth_header("admin"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "value": 8.0,
                        "reason": "must fail closed before rejected debug write"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("device status unavailable"));
    let state = runtime.read().await;
    assert!(!state.auto_enabled);
    assert_eq!(state.targets, original_targets);
    drop(state);
    assert!(db
        .audit_events(10, 0, Some("modbus_register_write"))
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn modbus_register_write_rejects_invalid_existing_runtime_targets_without_clamping() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(42.5, 0.18, 260.0, 30.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.targets.target_pressure_mpa = 12.0;
    }
    let original_targets = runtime.read().await.targets.clone();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/modbus/registers/target_temperature_c/write")
                .header("authorization", auth_header("admin"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "value": 72.5,
                        "reason": "must reject inherited invalid target instead of clamping"
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
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("target_pressure_mpa must be between 0 and 10"));
    let state = runtime.read().await;
    assert!(
        state.auto_enabled,
        "invalid inherited Modbus debug target input must not disable automatic control before commit prep"
    );
    assert_eq!(state.targets, original_targets);
    drop(state);
    assert!(db
        .audit_events(10, 0, Some("modbus_register_write"))
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn rbac_login_and_role_permissions_gate_sensitive_upper_computer_actions() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let app = router(
        AppState {
            db,
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let login = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "username": "engineer", "password": "engineer123" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    let body = to_bytes(login.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let token = body["data"]["token"].as_str().unwrap();
    assert_eq!(body["data"]["user"]["role"], "engineer");
    assert!(body["data"]["user"]["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|permission| permission == "modbus_debug"));

    let me = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/auth/me")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(me.status(), StatusCode::OK);

    let unauthenticated_write = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/control/targets")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "temperature_c": 60.0, "stirrer_rpm": 300.0 }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthenticated_write.status(), StatusCode::UNAUTHORIZED);

    let operator_modbus_write = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/modbus/registers/target_temperature_c/write")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "value": 61.0 }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(operator_modbus_write.status(), StatusCode::FORBIDDEN);

    let engineer_modbus_write = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/modbus/registers/target_temperature_c/write")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "value": 61.0,
                        "reason": "engineer rbac write smoke test"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(engineer_modbus_write.status(), StatusCode::FORBIDDEN);

    let admin_modbus_write = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/modbus/registers/target_temperature_c/write")
                .header("authorization", auth_header("admin"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "value": 61.0,
                        "reason": "admin rbac write smoke test"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(admin_modbus_write.status(), StatusCode::OK);

    let admin_modbus_write_without_reason = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/modbus/registers/target_temperature_c/write")
                .header("authorization", auth_header("admin"))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "value": 62.0 }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        admin_modbus_write_without_reason.status(),
        StatusCode::BAD_REQUEST
    );
}

#[test]
fn integration_config_template_defines_disabled_tls_mqtt_bridge() {
    let config = load_integration_config("config/integration.toml").unwrap();

    assert!(!config.mqtt.enabled);
    assert!(config.mqtt.use_tls);
    assert_eq!(config.mqtt.port, 8883);
    assert_eq!(config.mqtt.task_topic, "xingshu/reactor_001/tasks");
    assert_eq!(
        config.mqtt.receipt_topic,
        "xingshu/reactor_001/task_receipts"
    );
    assert_eq!(config.mqtt.alert_topic, "xingshu/reactor_001/alerts");
    assert_eq!(config.mqtt.alert_interval_s, 5);
    assert_eq!(
        config.mqtt.ca_cert.as_deref(),
        Some(std::path::Path::new("output/tls-test/server.crt"))
    );
    assert_eq!(
        config.mqtt.client_cert.as_deref(),
        Some(std::path::Path::new("output/tls-test/server.crt"))
    );
    assert_eq!(
        config.mqtt.client_key.as_deref(),
        Some(std::path::Path::new("output/tls-test/server.key"))
    );
    assert!(!config.modbus_tcp.enabled);
    assert!(config.modbus_tcp.require_tls);
    assert_eq!(config.modbus_tcp.bind, "0.0.0.0:502");
    assert_eq!(
        config.modbus_tcp.tls_cert.as_deref(),
        Some(std::path::Path::new("output/tls-test/server.crt"))
    );
    assert_eq!(
        config.modbus_tcp.tls_key.as_deref(),
        Some(std::path::Path::new("output/tls-test/server.key"))
    );
}

#[test]
fn mqtt_tls_requires_explicit_ca_certificate() {
    let config: reactor_edge_daemon::mqtt::IntegrationConfig = toml::from_str(
        r#"
[mqtt]
enabled = true
host = "broker.example.com"
port = 8883
client_id = "xingshu-test"
use_tls = true
ca_cert = ""
client_cert = ""
client_key = ""
keep_alive_s = 30
queue_capacity = 16
task_topic = "xingshu/test/tasks"
receipt_topic = "xingshu/test/task_receipts"
status_topic = "xingshu/test/status"
alert_topic = "xingshu/test/alerts"
alert_interval_s = 5

[modbus_tcp]
enabled = false
bind = "127.0.0.1:1502"
unit_id = 1
require_tls = true
tls_cert = ""
tls_key = ""
max_pdu_bytes = 253
"#,
    )
    .unwrap();

    let err = validate_mqtt_tls_config(&config.mqtt).unwrap_err();
    assert!(err.to_string().contains("MQTT TLS requires ca_cert"));
}

#[test]
fn integration_config_load_rejects_enabled_unreliable_mqtt_or_modbus_settings() {
    let config: reactor_edge_daemon::mqtt::IntegrationConfig = toml::from_str(
        r#"
[mqtt]
enabled = true
host = "broker.example.com"
port = 8883
client_id = "xingshu-test"
use_tls = false
ca_cert = ""
client_cert = ""
client_key = ""
keep_alive_s = 0
queue_capacity = 16
task_topic = "xingshu/test/tasks"
receipt_topic = "xingshu/test/task_receipts"
status_topic = "xingshu/test/status"
alert_topic = "xingshu/test/alerts"
alert_interval_s = 5

[modbus_tcp]
enabled = false
bind = "127.0.0.1:1502"
unit_id = 1
require_tls = false
tls_cert = ""
tls_key = ""
max_pdu_bytes = 253
"#,
    )
    .unwrap();
    let err = validate_integration_config(&config)
        .unwrap_err()
        .to_string();
    assert!(err.contains("MQTT keep_alive_s"));

    let temp_dir = tempfile::tempdir().unwrap();
    let integration_path = temp_dir.path().join("integration.toml");
    std::fs::write(
        &integration_path,
        r#"
[mqtt]
enabled = false
host = "127.0.0.1"
port = 8883
client_id = "xingshu-test"
use_tls = true
ca_cert = ""
client_cert = ""
client_key = ""
keep_alive_s = 30
queue_capacity = 16
task_topic = "xingshu/test/tasks"
receipt_topic = "xingshu/test/task_receipts"
status_topic = "xingshu/test/status"
alert_topic = "xingshu/test/alerts"
alert_interval_s = 5

[modbus_tcp]
enabled = true
bind = "not-an-address"
unit_id = 1
require_tls = false
tls_cert = ""
tls_key = ""
max_pdu_bytes = 253
"#,
    )
    .unwrap();
    let err = format!(
        "{:#}",
        load_integration_config(&integration_path).unwrap_err()
    );
    assert!(err.contains("invalid integration config"));
    assert!(err.contains("bind address is invalid"));
}

#[tokio::test]
async fn mqtt_alert_snapshot_reports_runtime_alarm_state() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.emergency_stop = true;
        state.last_control_error = Some("device write failed".to_string());
    }
    let app_state = AppState {
        db,
        runtime,
        device: test_device(),
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };

    let snapshot = mqtt_alert_snapshot(&app_state).await.unwrap();

    assert!(snapshot.active);
    assert_eq!(snapshot.active_count, 3);
    assert_eq!(snapshot.high_count, 2);
    assert_eq!(snapshot.warning_count, 1);
    assert!(snapshot.emergency_stop);
    assert!(!snapshot.sensor_fresh);
    assert_eq!(
        snapshot.alarms[0].get("type").and_then(Value::as_str),
        Some("sensor_data_unavailable")
    );
    assert_eq!(
        snapshot.alarms[1].get("type").and_then(Value::as_str),
        Some("emergency_stop")
    );
}

#[tokio::test]
async fn mqtt_alert_snapshot_marks_missing_or_stale_sensor_data_as_high_alarm() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app_state = AppState {
        db: db.clone(),
        runtime: runtime.clone(),
        device: test_device(),
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety: safety.clone(),
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };

    let missing = mqtt_alert_snapshot(&app_state).await.unwrap();
    assert!(missing.active);
    assert_eq!(missing.high_count, 1);
    assert!(!missing.sensor_fresh);
    assert!(missing.alarms.iter().any(|alarm| {
        alarm["type"] == "sensor_data_unavailable"
            && alarm["message"]
                .as_str()
                .unwrap()
                .contains("no persisted pipeline sample")
    }));

    let mut sample = fresh_sample(35.0, 0.06, 300.0, 30.0, 12.0);
    sample.captured_at =
        Utc::now() - chrono::Duration::milliseconds(safety.control.sensor_timeout_ms + 1000);
    install_runtime_sample(&runtime, &db, sample).await;

    let stale = mqtt_alert_snapshot(&app_state).await.unwrap();
    assert!(stale.active);
    assert_eq!(stale.high_count, 1);
    assert!(!stale.sensor_fresh);
    assert!(stale
        .alarms
        .iter()
        .any(|alarm| alarm["type"] == "sensor_data_unavailable"
            && alarm["message"].as_str().unwrap().contains("stale")));

    let mut future_sample = fresh_sample(35.0, 0.06, 300.0, 30.0, 12.0);
    future_sample.captured_at = Utc::now() + chrono::Duration::milliseconds(5000);
    install_runtime_sample(&runtime, &db, future_sample).await;

    let future = mqtt_alert_snapshot(&app_state).await.unwrap();
    assert!(future.active);
    assert_eq!(future.high_count, 1);
    assert!(!future.sensor_fresh);
    assert!(future
        .alarms
        .iter()
        .any(|alarm| alarm["type"] == "sensor_data_unavailable"
            && alarm["message"].as_str().unwrap().contains("future")));
}

#[tokio::test]
async fn mqtt_alert_snapshot_surfaces_unfinished_batch_recovery_alarm() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let orphan = db
        .create_batch_for_process_sqlx(None, "mqtt orphan unfinished", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(healthy_device_status());
    }
    let app_state = AppState {
        db,
        runtime,
        device: test_device(),
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };

    let snapshot = mqtt_alert_snapshot(&app_state).await.unwrap();

    assert!(snapshot.active);
    assert_eq!(snapshot.high_count, 1);
    assert!(snapshot.alarms.iter().any(|alarm| {
        alarm["type"] == "unfinished_batch_recovery"
            && alarm["unfinished_batch_ids"][0] == orphan.id
            && alarm["active_batch_id"] == Value::Null
    }));
}

#[tokio::test]
async fn mqtt_task_payload_executes_targets_and_persists_receipt() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let original_targets = runtime.read().await.targets.clone();
    let app_state = AppState {
        db: db.clone(),
        runtime: runtime.clone(),
        device: test_device(),
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };

    for payload in [
        json!({
            "external_task_id": null,
            "action": "set_targets",
            "target_temperature_c": 72.5,
            "reason": "null external task id must not lose idempotency"
        }),
        json!({
            "external_task_id": "mqtt-null-reason",
            "action": "set_targets",
            "target_temperature_c": 72.5,
            "reason": null
        }),
    ] {
        let receipt = execute_mqtt_task_payload(&app_state, payload.to_string().as_bytes()).await;
        assert!(!receipt.ok);
        assert_eq!(receipt.source, "mqtt");
        assert_eq!(receipt.status, "rejected");
        assert!(receipt
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("must not be null"));
    }
    assert_eq!(runtime.read().await.targets, original_targets);
    assert!(db.integration_tasks(Some("mqtt"), 10).unwrap().is_empty());
    assert!(db.recent_control_events(10).unwrap().is_empty());

    let extra_process_id = execute_mqtt_task_payload(
        &app_state,
        json!({
            "external_task_id": "mqtt-targets-with-process",
            "action": "set_targets",
            "process_id": 42,
            "target_temperature_c": 72.5,
            "reason": "ambiguous target scope must fail closed"
        })
        .to_string()
        .as_bytes(),
    )
    .await;
    assert!(!extra_process_id.ok);
    assert_eq!(extra_process_id.status, "rejected");
    assert!(extra_process_id
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("process_id is not accepted for set_targets"));
    assert_eq!(runtime.read().await.targets, original_targets);
    assert!(db.integration_tasks(Some("mqtt"), 10).unwrap().is_empty());
    assert!(db.recent_control_events(10).unwrap().is_empty());

    let receipt = execute_mqtt_task_payload(
        &app_state,
        json!({
            "external_task_id": "mqtt-live-001",
            "action": "set_targets",
            "target_temperature_c": 72.5,
            "target_pressure_mpa": 0.8,
            "target_shake_speed_cpm": 45.0,
            "reason": "mqtt validation"
        })
        .to_string()
        .as_bytes(),
    )
    .await;

    assert!(receipt.ok);
    assert_eq!(receipt.source, "mqtt");
    assert_eq!(receipt.status, "executed");
    assert_eq!(receipt.external_task_id.as_deref(), Some("mqtt-live-001"));
    let targets = runtime.read().await.targets.clone();
    assert_eq!(targets.temperature_c, 72.5);
    assert_eq!(targets.target_pressure_mpa, 0.8);
    assert_eq!(targets.shake_speed_cpm, 45.0);

    let tasks = db.integration_tasks(Some("mqtt"), 10).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].source, "mqtt");
    assert_eq!(tasks[0].status, "executed");
    assert_eq!(tasks[0].action, "set_targets");

    let rejected = execute_mqtt_task_payload(&app_state, b"{not-json").await;
    assert!(!rejected.ok);
    assert_eq!(rejected.status, "rejected");
    assert!(rejected.error.unwrap().contains("invalid MQTT task JSON"));
}

#[tokio::test]
async fn integration_set_targets_rejects_out_of_range_remote_values_without_side_effects() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
    }
    let original_targets = runtime.read().await.targets.clone();
    let (device, recorded_device) = recording_device();
    let app_state = AppState {
        db: db.clone(),
        runtime: runtime.clone(),
        device,
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };

    for (payload, expected_message) in [
        (
            json!({
                "external_task_id": "mqtt-invalid-heat",
                "action": "set_targets",
                "target_temperature_c": 72.5,
                "heat_time_s": -1.0,
                "reason": "negative remote heat time must fail closed"
            }),
            "heat_time_s must be between 0",
        ),
        (
            json!({
                "external_task_id": "mqtt-invalid-shake",
                "action": "set_targets",
                "target_shake_speed_cpm": 61.0,
                "reason": "remote shake speed out of range must fail closed"
            }),
            "target_shake_speed_cpm must be between 0 and 60",
        ),
        (
            json!({
                "external_task_id": "mqtt-null-pressure",
                "action": "set_targets",
                "target_pressure_mpa": null,
                "reason": "remote null pressure must fail closed"
            }),
            "target_pressure_mpa must not be null",
        ),
    ] {
        let receipt = execute_mqtt_task_payload(&app_state, payload.to_string().as_bytes()).await;

        assert!(!receipt.ok);
        assert_eq!(receipt.source, "mqtt");
        assert_eq!(receipt.status, "rejected");
        assert!(receipt
            .error
            .as_deref()
            .unwrap_or_default()
            .contains(expected_message));
    }

    assert_eq!(runtime.read().await.targets, original_targets);
    assert!(runtime.read().await.auto_enabled);
    assert!(recorded_device.writes.lock().unwrap().is_empty());
    assert!(db.recent_control_events(10).unwrap().is_empty());
    let tasks = db.integration_tasks(Some("mqtt"), 10).unwrap();
    assert_eq!(tasks.len(), 3);
    assert!(tasks.iter().all(|task| task.status == "rejected"));
    for (external_task_id, expected_message) in [
        ("mqtt-invalid-heat", "must be between"),
        ("mqtt-invalid-shake", "must be between"),
        ("mqtt-null-pressure", "must not be null"),
    ] {
        assert!(tasks.iter().any(|task| {
            task.external_task_id.as_deref() == Some(external_task_id)
                && task.response["message"]
                    .as_str()
                    .unwrap_or_default()
                    .contains(expected_message)
        }));
    }
}

#[tokio::test]
async fn integration_set_targets_rejects_invalid_existing_runtime_targets_without_clamping() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.targets.target_pressure_mpa = 12.0;
        state.auto_enabled = true;
    }
    let original_targets = runtime.read().await.targets.clone();
    let (device, recorded_device) = recording_device();
    let app_state = AppState {
        db: db.clone(),
        runtime: runtime.clone(),
        device,
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };

    let receipt = execute_mqtt_task_payload(
        &app_state,
        json!({
            "external_task_id": "mqtt-invalid-existing-target",
            "action": "set_targets",
            "target_temperature_c": 72.5,
            "reason": "must reject inherited invalid target instead of clamping"
        })
        .to_string()
        .as_bytes(),
    )
    .await;

    assert!(!receipt.ok);
    assert_eq!(receipt.status, "rejected");
    assert!(receipt
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("target_pressure_mpa must be between 0 and 10"));
    assert_eq!(runtime.read().await.targets, original_targets);
    assert!(runtime.read().await.auto_enabled);
    assert!(recorded_device.writes.lock().unwrap().is_empty());
    assert!(db.recent_control_events(10).unwrap().is_empty());
    let tasks = db.integration_tasks(Some("mqtt"), 10).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].status, "rejected");
    assert_eq!(
        tasks[0].external_task_id.as_deref(),
        Some("mqtt-invalid-existing-target")
    );
}

#[tokio::test]
async fn integration_set_targets_failure_forces_auto_disabled_before_returning_error() {
    let mut strict_safety = safety();
    strict_safety.control.require_device_status_for_control = true;
    let safety = Arc::new(strict_safety);
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
    }
    let original_targets = runtime.read().await.targets.clone();
    let app_state = AppState {
        db: db.clone(),
        runtime: runtime.clone(),
        device: test_device(),
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };

    let receipt = execute_mqtt_task_payload(
        &app_state,
        json!({
            "external_task_id": "mqtt-set-targets-fail-closed",
            "action": "set_targets",
            "target_temperature_c": 72.5,
            "target_pressure_mpa": 0.8,
            "target_shake_speed_cpm": 45.0,
            "reason": "mqtt must fail closed before rejected target write"
        })
        .to_string()
        .as_bytes(),
    )
    .await;

    assert!(!receipt.ok);
    assert_eq!(receipt.status, "failed");
    assert!(receipt
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("device status unavailable"));
    let state = runtime.read().await;
    assert!(!state.auto_enabled);
    assert_eq!(state.targets, original_targets);
    drop(state);
    assert!(db.recent_control_events(10).unwrap().is_empty());
    let tasks = db.integration_tasks(Some("mqtt"), 10).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].status, "failed");
}

#[tokio::test]
async fn integration_tasks_are_idempotent_by_external_task_id() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let app_state = AppState {
        db: db.clone(),
        runtime: runtime.clone(),
        device: test_device(),
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };

    let payload = json!({
        "external_task_id": " mqtt-idem\u{200B}potent\n\u{0007}\u{202E}001 ",
        "action": "control:set_targets",
        "target_temperature_c": 72.5,
        "target_pressure_mpa": 0.8,
        "target_shake_speed_cpm": 45.0,
        "reason": " mqtt\nfirst\t\u{0007}\u{2066}delivery "
    });
    let first = execute_mqtt_task_payload(&app_state, payload.to_string().as_bytes()).await;
    assert!(first.ok);
    assert_eq!(runtime.read().await.targets.temperature_c, 72.5);
    let first_task_id = first.task_id;

    let same_payload_replay = execute_mqtt_task_payload(
        &app_state,
        json!({
            "external_task_id": "mqtt-idempotent 001",
            "action": "set_targets",
            "target_temperature_c": 72.5,
            "target_pressure_mpa": 0.8,
            "target_shake_speed_cpm": 45.0,
            "reason": "mqtt first delivery"
        })
        .to_string()
        .as_bytes(),
    )
    .await;
    assert!(same_payload_replay.ok);
    assert_eq!(same_payload_replay.task_id, first_task_id);

    let conflicting_replay = execute_mqtt_task_payload(
        &app_state,
        json!({
            "external_task_id": "mqtt-idempotent 001",
            "action": "set_targets",
            "target_temperature_c": 90.0,
            "target_pressure_mpa": 0.2,
            "target_shake_speed_cpm": 10.0,
            "reason": "mqtt duplicate delivery must not execute"
        })
        .to_string()
        .as_bytes(),
    )
    .await;

    assert!(!conflicting_replay.ok);
    assert_eq!(conflicting_replay.status, "rejected");
    assert!(conflicting_replay
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("different request"));
    let targets = runtime.read().await.targets.clone();
    assert_eq!(targets.temperature_c, 72.5);
    assert_eq!(targets.target_pressure_mpa, 0.8);
    assert_eq!(targets.shake_speed_cpm, 45.0);
    let tasks = db.integration_tasks(Some("mqtt"), 10).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].external_task_id.as_deref(),
        Some("mqtt-idempotent 001")
    );
    assert_eq!(tasks[0].request["reason"], "mqtt first delivery");
    let events = db.recent_control_events(10).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "ainas_targets_updated")
            .count(),
        1
    );
}

#[tokio::test]
async fn integration_task_replay_while_executing_does_not_execute_again() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let original_targets = runtime.read().await.targets.clone();
    let payload = serde_json::to_value(AinasTaskRequest {
        external_task_id: Some("mqtt-executing-001".to_string()),
        action: "set_targets".to_string(),
        process_id: None,
        target_temperature_c: Some(Some(72.5)),
        target_stirrer_rpm: None,
        target_shake_speed_cpm: Some(Some(45.0)),
        target_pressure_mpa: Some(Some(0.8)),
        heat_time_s: None,
        hold_time_s: None,
        cool_time_s: None,
        reason: Some("mqtt executing replay".to_string()),
    })
    .unwrap();
    let task = db
        .create_integration_task_sqlx("mqtt", Some("mqtt-executing-001"), "set_targets", &payload)
        .await
        .unwrap();
    db.mark_integration_task_executing_sqlx(task.id)
        .await
        .unwrap();
    let app_state = AppState {
        db: db.clone(),
        runtime: runtime.clone(),
        device: test_device(),
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };

    let receipt = execute_mqtt_task_payload(&app_state, payload.to_string().as_bytes()).await;

    assert!(!receipt.ok);
    assert_eq!(receipt.status, "executing");
    assert_eq!(receipt.task_id, Some(task.id));
    assert_eq!(runtime.read().await.targets, original_targets);
    assert!(db.recent_control_events(10).unwrap().is_empty());
}

#[tokio::test]
async fn integration_task_replay_with_legacy_invalid_task_fails_closed_without_device_write() {
    let safety = Arc::new(safety());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reactor.sqlite3");
    let db = Db::open(&path).unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let original_targets = runtime.read().await.targets.clone();
    let (device, recorded_device) = recording_device();
    let app_state = AppState {
        db: db.clone(),
        runtime: runtime.clone(),
        device,
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };
    let now = Utc::now().to_rfc3339();
    let invalid_task_id = {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute(
            r#"
            INSERT INTO integration_tasks
                (external_task_id, source, action, status, request_json, response_json, created_at, updated_at)
            VALUES ('mqtt-invalid-replay', 'mqtt', 'open_valve', 'received', '{}', 'null', ?1, ?1)
            "#,
            rusqlite::params![now],
        )
        .unwrap();
        conn.last_insert_rowid()
    };

    let receipt = execute_mqtt_task_payload(
        &app_state,
        json!({
            "external_task_id": "mqtt-invalid-replay",
            "action": "set_targets",
            "target_temperature_c": 72.5,
            "target_pressure_mpa": 0.8,
            "target_shake_speed_cpm": 45.0,
            "reason": "legacy invalid replay must fail closed"
        })
        .to_string()
        .as_bytes(),
    )
    .await;

    assert!(!receipt.ok);
    assert_eq!(receipt.status, "failed");
    assert_eq!(runtime.read().await.targets, original_targets);
    assert!(recorded_device.writes.lock().unwrap().is_empty());
    assert!(db.recent_control_events(10).unwrap().is_empty());
    assert!(
        db.integration_task_sqlx(invalid_task_id)
            .await
            .unwrap_err()
            .to_string()
            .contains("invalid integration task in database"),
        "legacy invalid integration task must remain untrusted"
    );
}

#[tokio::test]
async fn integration_set_targets_does_not_commit_runtime_when_audit_fails() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let original_targets = runtime.read().await.targets.clone();
    let app_state = AppState {
        db: db.clone(),
        runtime: runtime.clone(),
        device: test_device(),
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };
    let app = router(app_state.clone(), PathBuf::from("static"));
    db.break_control_events_for_tests().unwrap();

    let mqtt_receipt = execute_mqtt_task_payload(
        &app_state,
        json!({
            "external_task_id": "mqtt-audit-failed",
            "action": "set_targets",
            "target_temperature_c": 72.5,
            "target_pressure_mpa": 0.8,
            "target_shake_speed_cpm": 45.0,
            "reason": "mqtt audit failure"
        })
        .to_string()
        .as_bytes(),
    )
    .await;
    assert!(!mqtt_receipt.ok);
    assert_eq!(mqtt_receipt.status, "failed");
    assert_eq!(runtime.read().await.targets, original_targets);
    let mqtt_tasks = db.integration_tasks(Some("mqtt"), 10).unwrap();
    assert_eq!(mqtt_tasks.len(), 1);
    assert_eq!(mqtt_tasks[0].status, "failed");

    let ainas_response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/integrations/ainas/tasks")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "external_task_id": "ainas-audit-failed",
                        "action": "set_targets",
                        "target_temperature_c": 74.0,
                        "target_stirrer_rpm": 410.0,
                        "target_pressure_mpa": 0.7,
                        "reason": "ainas audit failure"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ainas_response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(runtime.read().await.targets, original_targets);
    let ainas_tasks = db.integration_tasks(Some("ainas"), 10).unwrap();
    assert_eq!(ainas_tasks.len(), 1);
    assert_eq!(ainas_tasks[0].status, "failed");
}

#[tokio::test]
async fn integration_set_targets_rechecks_interlocks_after_audit_before_runtime_commit() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let original_targets = runtime.read().await.targets.clone();
    {
        let runtime = runtime.clone();
        db.after_control_event_success_for_tests(Arc::new(move || {
            let mut state = runtime
                .try_write()
                .expect("runtime lock should be available after audit insert");
            state.emergency_stop = true;
        }));
    }
    let app_state = AppState {
        db: db.clone(),
        runtime: runtime.clone(),
        device: test_device(),
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };

    let mqtt_receipt = execute_mqtt_task_payload(
        &app_state,
        json!({
            "external_task_id": "mqtt-audit-then-estop",
            "action": "set_targets",
            "target_temperature_c": 72.5,
            "target_pressure_mpa": 0.8,
            "target_shake_speed_cpm": 45.0,
            "reason": "mqtt final interlock trip"
        })
        .to_string()
        .as_bytes(),
    )
    .await;

    assert!(!mqtt_receipt.ok);
    assert_eq!(mqtt_receipt.status, "rejected");
    assert!(mqtt_receipt
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("emergency stop is active"));
    assert_eq!(runtime.read().await.targets, original_targets);
    assert!(runtime.read().await.emergency_stop);
    assert!(db
        .recent_control_events(10)
        .unwrap()
        .iter()
        .any(|event| event.event_type == "ainas_targets_updated"));
    let mqtt_tasks = db.integration_tasks(Some("mqtt"), 10).unwrap();
    assert_eq!(mqtt_tasks.len(), 1);
    assert_eq!(mqtt_tasks[0].status, "rejected");
}

#[tokio::test]
async fn modbus_tcp_pdu_reads_and_writes_safety_gated_map() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.latest_sample = Some(SensorSnapshot {
            temperature_c: 42.5,
            pressure_mpa: 0.18,
            stirrer_rpm: 260.0,
            shake_speed_cpm: 30.0,
            tilt_state: 1,
            tilt_angle_deg: 12.5,
            flow_rate_l_min: 2.0,
            product_concentration_percent: 10.0,
            ph: 6.8,
            captured_at: Utc::now(),
        });
    }
    let app_state = AppState {
        db: db.clone(),
        runtime: runtime.clone(),
        device: test_device(),
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };

    let holding = handle_modbus_tcp_pdu(&app_state, &[0x03, 0x00, 0x00, 0x00, 0x03]).await;
    assert_eq!(
        holding,
        vec![0x03, 0x06, 0x01, 0xA9, 0x01, 0x04, 0x00, 0x12]
    );

    let write_pressure = handle_modbus_tcp_pdu(&app_state, &[0x06, 0x00, 0x0D, 0x03, 0x84]).await;
    assert_eq!(write_pressure, vec![0x06, 0x00, 0x0D, 0x03, 0x84]);
    assert_eq!(runtime.read().await.targets.target_pressure_mpa, 9.0);
    assert_eq!(
        db.audit_events(10, 0, Some("modbus_register_write"))
            .unwrap()
            .len(),
        1
    );

    let target_pressure = handle_modbus_tcp_pdu(&app_state, &[0x03, 0x00, 0x0D, 0x00, 0x01]).await;
    assert_eq!(target_pressure, vec![0x03, 0x02, 0x03, 0x84]);

    let coils = handle_modbus_tcp_pdu(&app_state, &[0x01, 0x00, 0x00, 0x00, 0x04]).await;
    assert_eq!(coils, vec![0x01, 0x01, 0x00]);

    let discrete = handle_modbus_tcp_pdu(&app_state, &[0x02, 0x00, 0x00, 0x00, 0x02]).await;
    assert_eq!(discrete, vec![0x02, 0x01, 0x03]);

    let illegal = handle_modbus_tcp_pdu(&app_state, &[0x03, 0x00, 0x63, 0x00, 0x01]).await;
    assert_eq!(illegal, vec![0x83, 0x02]);
}

#[tokio::test]
async fn modbus_tcp_write_rejects_out_of_range_value_without_runtime_or_audit_side_effects() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(42.5, 0.18, 260.0, 30.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
    }
    let original_targets = runtime.read().await.targets.clone();
    let app_state = AppState {
        db: db.clone(),
        runtime: runtime.clone(),
        device: test_device(),
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };

    let write_pressure = handle_modbus_tcp_pdu(&app_state, &[0x06, 0x00, 0x0D, 0x04, 0x4C]).await;

    assert_eq!(write_pressure, vec![0x86, 0x03]);
    let state = runtime.read().await;
    assert!(
        state.auto_enabled,
        "out-of-range Modbus TCP write must not disable automatic control before commit prep"
    );
    assert_eq!(state.targets, original_targets);
    drop(state);
    assert!(db
        .audit_events(10, 0, Some("modbus_register_write"))
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn modbus_tcp_write_rejects_invalid_existing_runtime_targets_without_clamping() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(42.5, 0.18, 260.0, 30.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.targets.target_pressure_mpa = 12.0;
    }
    let original_targets = runtime.read().await.targets.clone();
    let app_state = AppState {
        db: db.clone(),
        runtime: runtime.clone(),
        device: test_device(),
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };

    let write_temperature =
        handle_modbus_tcp_pdu(&app_state, &[0x06, 0x00, 0x0A, 0x02, 0xD5]).await;

    assert_eq!(write_temperature, vec![0x86, 0x03]);
    let state = runtime.read().await;
    assert!(
        state.auto_enabled,
        "invalid inherited target during Modbus TCP write must not disable automatic control before commit prep"
    );
    assert_eq!(state.targets, original_targets);
    drop(state);
    assert!(db
        .audit_events(10, 0, Some("modbus_register_write"))
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn modbus_tcp_discrete_inputs_do_not_mark_device_connected_without_required_status() {
    let mut strict_safety = safety();
    strict_safety.control.require_device_status_for_control = true;
    let safety = Arc::new(strict_safety);
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(42.5, 0.18, 260.0, 30.0, 10.0)).await;
    let app_state = AppState {
        db,
        runtime,
        device: test_device(),
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };

    let discrete = handle_modbus_tcp_pdu(&app_state, &[0x02, 0x00, 0x00, 0x00, 0x02]).await;
    assert_eq!(discrete, vec![0x02, 0x01, 0x02]);

    let alarm_bits = handle_modbus_tcp_pdu(&app_state, &[0x02, 0x00, 0x02, 0x00, 0x01]).await;
    assert_eq!(alarm_bits, vec![0x02, 0x01, 0x01]);
}

#[tokio::test]
async fn modbus_tcp_discrete_inputs_do_not_mark_device_connected_from_stale_lab_sample() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let mut sample = fresh_sample(42.5, 0.18, 260.0, 30.0, 10.0);
    sample.captured_at = Utc::now() - Duration::milliseconds(safety.control.sensor_timeout_ms + 1);
    install_runtime_sample(&runtime, &db, sample).await;
    let app_state = AppState {
        db,
        runtime,
        device: test_device(),
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };

    let discrete = handle_modbus_tcp_pdu(&app_state, &[0x02, 0x00, 0x00, 0x00, 0x03]).await;
    assert_eq!(discrete, vec![0x02, 0x01, 0x04]);
}

#[tokio::test]
async fn modbus_tcp_discrete_inputs_do_not_mark_device_connected_from_future_lab_sample() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let mut sample = fresh_sample(42.5, 0.18, 260.0, 30.0, 10.0);
    sample.captured_at = Utc::now() + Duration::milliseconds(5000);
    install_runtime_sample(&runtime, &db, sample).await;
    let app_state = AppState {
        db,
        runtime,
        device: test_device(),
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };

    let discrete = handle_modbus_tcp_pdu(&app_state, &[0x02, 0x00, 0x00, 0x00, 0x03]).await;
    assert_eq!(discrete, vec![0x02, 0x01, 0x04]);
}

#[tokio::test]
async fn modbus_tcp_discrete_inputs_do_not_mark_device_connected_with_command_fault() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(42.5, 0.18, 260.0, 30.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(DeviceStatusSnapshot {
            last_command_request_id: Some("cmd-failed".to_string()),
            last_command_ok: Some(false),
            last_command_error: Some("relay did not acknowledge".to_string()),
            ..healthy_device_status()
        });
    }
    let app_state = AppState {
        db,
        runtime,
        device: test_device(),
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };

    let discrete = handle_modbus_tcp_pdu(&app_state, &[0x02, 0x00, 0x00, 0x00, 0x03]).await;
    assert_eq!(discrete, vec![0x02, 0x01, 0x06]);
}

#[tokio::test]
async fn modbus_tcp_bits_surface_unfinished_db_batch_recovery_state() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    db.create_batch_for_process_sqlx(None, "tcp orphan unfinished", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(42.5, 0.18, 260.0, 30.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(healthy_device_status());
    }
    let app_state = AppState {
        db,
        runtime,
        device: test_device(),
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };

    let coils = handle_modbus_tcp_pdu(&app_state, &[0x01, 0x00, 0x03, 0x00, 0x01]).await;
    assert_eq!(coils, vec![0x01, 0x01, 0x01]);
    let discrete = handle_modbus_tcp_pdu(&app_state, &[0x02, 0x00, 0x00, 0x00, 0x05]).await;
    assert_eq!(discrete, vec![0x02, 0x01, 0x1E]);
}

#[tokio::test]
async fn modbus_tcp_stream_accepts_real_mbap_read_request() {
    let app_state = modbus_tcp_test_state().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let _ = handle_modbus_tcp_stream(stream, app_state, 253, 1).await;
    });

    let mut client = TcpStream::connect(addr).await.unwrap();
    client
        .write_all(&[
            0x12, 0x34, // transaction id
            0x00, 0x00, // protocol id
            0x00, 0x06, // unit id + five-byte PDU
            0x01, // unit id
            0x03, // read holding registers
            0x00, 0x00, // start address
            0x00, 0x02, // quantity
        ])
        .await
        .unwrap();
    let mut header = [0_u8; 7];
    client.read_exact(&mut header).await.unwrap();
    assert_eq!(&header, &[0x12, 0x34, 0x00, 0x00, 0x00, 0x07, 0x01]);
    let mut pdu = vec![0_u8; 6];
    client.read_exact(&mut pdu).await.unwrap();
    assert_eq!(pdu, vec![0x03, 0x04, 0x01, 0xA9, 0x01, 0x04]);

    drop(client);
    server.await.unwrap();
}

#[tokio::test]
async fn modbus_tcp_stream_rejects_wrong_unit_id_without_writing_runtime() {
    let app_state = modbus_tcp_test_state().await;
    let runtime = app_state.runtime.clone();
    let db = app_state.db.clone();
    let original_targets = runtime.read().await.targets.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let _ = handle_modbus_tcp_stream(stream, app_state, 253, 1).await;
    });

    let mut client = TcpStream::connect(addr).await.unwrap();
    client
        .write_all(&[
            0x12, 0x36, // transaction id
            0x00, 0x00, // protocol id
            0x00, 0x06, // unit id + five-byte PDU
            0x02, // wrong unit id
            0x06, // write single holding register
            0x00, 0x0D, // target_pressure_mpa
            0x03, 0x84, // 9.00 MPa
        ])
        .await
        .unwrap();
    let mut header = [0_u8; 7];
    client.read_exact(&mut header).await.unwrap();
    assert_eq!(&header, &[0x12, 0x36, 0x00, 0x00, 0x00, 0x03, 0x02]);
    let mut pdu = vec![0_u8; 2];
    client.read_exact(&mut pdu).await.unwrap();
    assert_eq!(pdu, vec![0x86, 0x0B]);

    drop(client);
    server.await.unwrap();
    assert_eq!(runtime.read().await.targets, original_targets);
    assert!(db
        .audit_events(10, 0, Some("modbus_register_write"))
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn modbus_tcp_tls_stream_accepts_real_mbap_read_request() {
    reactor_edge_daemon::tls::install_rustls_provider();
    let app_state = modbus_tcp_test_state().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let certs = reactor_edge_daemon::tls::load_cert_chain(TEST_TLS_CERT).unwrap();
    let key = reactor_edge_daemon::tls::load_private_key(TEST_TLS_KEY).unwrap();
    let server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(server_config));
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let stream = acceptor.accept(stream).await.unwrap();
        let _ = handle_modbus_tcp_stream(stream, app_state, 253, 1).await;
    });

    let mut root_store = rustls::RootCertStore::empty();
    let certs = reactor_edge_daemon::tls::load_cert_chain(TEST_TLS_CERT).unwrap();
    let added = root_store.add_parsable_certificates(certs);
    assert!(added.0 > 0);
    let client_config = Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth(),
    );
    let connector = TlsConnector::from(client_config);
    let server_name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
    let stream = TcpStream::connect(addr).await.unwrap();
    let mut client = connector.connect(server_name, stream).await.unwrap();
    client
        .write_all(&[
            0x12, 0x35, // transaction id
            0x00, 0x00, // protocol id
            0x00, 0x06, // unit id + five-byte PDU
            0x01, // unit id
            0x03, // read holding registers
            0x00, 0x00, // start address
            0x00, 0x02, // quantity
        ])
        .await
        .unwrap();
    let mut header = [0_u8; 7];
    client.read_exact(&mut header).await.unwrap();
    assert_eq!(&header, &[0x12, 0x35, 0x00, 0x00, 0x00, 0x07, 0x01]);
    let mut pdu = vec![0_u8; 6];
    client.read_exact(&mut pdu).await.unwrap();
    assert_eq!(pdu, vec![0x03, 0x04, 0x01, 0xA9, 0x01, 0x04]);

    drop(client);
    server.await.unwrap();
}

#[tokio::test]
async fn ainas_task_api_requires_auth_executes_targets_and_persists_task() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let app = router(
        AppState {
            db: db.clone(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );
    let payload = json!({
        "external_task_id": "ainas-001",
        "action": "set_targets",
        "target_temperature_c": 92.0,
        "target_stirrer_rpm": 480.0,
        "target_shake_speed_cpm": 42.0,
        "reason": "AINAS acceptance task"
    });

    let null_external_task_id = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/integrations/ainas/tasks")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "external_task_id": null,
                        "action": "set_targets",
                        "target_temperature_c": 92.0,
                        "reason": "null external id must not execute"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        null_external_task_id.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let body = to_bytes(null_external_task_id.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("must not be null"));
    assert!(db.integration_tasks(Some("ainas"), 10).unwrap().is_empty());
    assert!(db.recent_control_events(10).unwrap().is_empty());

    let unauthorized = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/integrations/ainas/tasks")
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

    // Operator must NOT be able to dispatch AINAS tasks. The integration
    // path is reserved for engineer/admin via Permission::ApplyIntegrationTask.
    let operator_dispatch = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/integrations/ainas/tasks")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        operator_dispatch.status(),
        StatusCode::FORBIDDEN,
        "operator must not dispatch AINAS tasks"
    );

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/integrations/ainas/tasks")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);
    let body = to_bytes(created.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 0);
    assert_eq!(body["data"]["source"], "ainas");
    assert_eq!(body["data"]["external_task_id"], "ainas-001");
    assert_eq!(body["data"]["action"], "set_targets");
    assert_eq!(body["data"]["status"], "executed");
    assert_eq!(body["data"]["response"]["targets"]["temperature_c"], 92.0);
    assert_eq!(body["data"]["response"]["targets"]["stirrer_rpm"], 480.0);
    assert_eq!(body["data"]["response"]["targets"]["shake_speed_cpm"], 42.0);
    let task_id = body["data"]["id"].as_i64().unwrap();

    let list = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/integrations/ainas/tasks?limit=10")
                .header("authorization", auth_header("engineer"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let body = to_bytes(list.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["data"][0]["id"], task_id);

    let detail = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/integrations/ainas/tasks/{task_id}"))
                .header("authorization", auth_header("engineer"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::OK);

    let events = db.recent_control_events(5).unwrap();
    assert!(events
        .iter()
        .any(|event| event.event_type == "ainas_targets_updated"));
}

#[tokio::test]
async fn ainas_task_api_replays_existing_external_task_without_reexecution() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );
    let payload = json!({
        "external_task_id": " ainas-idem\u{200B}potent\n\u{0007}\u{202E}001 ",
        "action": "set_targets",
        "target_temperature_c": 73.0,
        "target_pressure_mpa": 0.7,
        "target_shake_speed_cpm": 42.0,
        "reason": " ainas\nfirst\t\u{0007}\u{2066}delivery "
    });

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/integrations/ainas/tasks")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let body = to_bytes(first.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let task_id = body["data"]["id"].as_i64().unwrap();

    let same_payload_replay_payload = json!({
        "external_task_id": "ainas-idempotent 001",
        "action": "control:set_targets",
        "target_temperature_c": 73.0,
        "target_pressure_mpa": 0.7,
        "target_shake_speed_cpm": 42.0,
        "reason": "ainas first delivery"
    });
    let same_payload_replay = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/integrations/ainas/tasks")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(same_payload_replay_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(same_payload_replay.status(), StatusCode::OK);
    let body = to_bytes(same_payload_replay.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["data"]["id"].as_i64().unwrap(), task_id);

    let conflicting_replay = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/integrations/ainas/tasks")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "external_task_id": "ainas-idempotent 001",
                        "action": "set_targets",
                        "target_temperature_c": 95.0,
                        "target_pressure_mpa": 0.1,
                        "target_shake_speed_cpm": 5.0,
                        "reason": "ainas duplicate delivery must not execute"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(conflicting_replay.status(), StatusCode::CONFLICT);

    let targets = runtime.read().await.targets.clone();
    assert_eq!(targets.temperature_c, 73.0);
    assert_eq!(targets.target_pressure_mpa, 0.7);
    assert_eq!(targets.shake_speed_cpm, 42.0);
    let tasks = db.integration_tasks(Some("ainas"), 10).unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].external_task_id.as_deref(),
        Some("ainas-idempotent 001")
    );
    assert_eq!(tasks[0].request["reason"], "ainas first delivery");
    assert_eq!(
        db.recent_control_events(10)
            .unwrap()
            .iter()
            .filter(|event| event.event_type == "ainas_targets_updated")
            .count(),
        1
    );
}

#[tokio::test]
async fn ainas_task_api_replay_while_executing_does_not_execute_again() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let original_targets = runtime.read().await.targets.clone();
    let payload = serde_json::to_value(AinasTaskRequest {
        external_task_id: Some("ainas-executing-001".to_string()),
        action: "set_targets".to_string(),
        process_id: None,
        target_temperature_c: Some(Some(73.0)),
        target_stirrer_rpm: None,
        target_shake_speed_cpm: Some(Some(42.0)),
        target_pressure_mpa: Some(Some(0.7)),
        heat_time_s: None,
        hold_time_s: None,
        cool_time_s: None,
        reason: Some("ainas executing replay".to_string()),
    })
    .unwrap();
    let task = db
        .create_integration_task_sqlx(
            "ainas",
            Some("ainas-executing-001"),
            "set_targets",
            &payload,
        )
        .await
        .unwrap();
    db.mark_integration_task_executing_sqlx(task.id)
        .await
        .unwrap();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let replay = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/integrations/ainas/tasks")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(replay.status(), StatusCode::OK);
    let body = to_bytes(replay.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["data"]["id"], task.id);
    assert_eq!(body["data"]["status"], "executing");
    assert_eq!(runtime.read().await.targets, original_targets);
    assert!(db.recent_control_events(10).unwrap().is_empty());
}

#[tokio::test]
async fn ainas_task_api_can_start_and_stop_process_through_safety_lifecycle() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let process_id = add_simple_process(&db, "ainas lifecycle");
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 120.0, 12.0, 20.0)).await;
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    // Engineer has ApplyIntegrationTask; operator does not and must be
    // rejected before reaching the lifecycle path. Verify operator is blocked
    // first, then run the full start/stop cycle as engineer.
    let operator_blocked = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/integrations/ainas/tasks")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "external_task_id": "ainas-operator-blocked",
                        "action": "start_process",
                        "process_id": process_id
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        operator_blocked.status(),
        StatusCode::FORBIDDEN,
        "operator must not start AINAS processes"
    );

    let started = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/integrations/ainas/tasks")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "external_task_id": "ainas-start-001",
                        "action": "start_process",
                        "process_id": process_id
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(started.status(), StatusCode::OK);
    let body = to_bytes(started.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["data"]["status"], "executed");
    assert_eq!(body["data"]["response"]["process"]["id"], process_id);
    assert_eq!(body["data"]["response"]["batch"]["process_id"], process_id);
    let batch_id = body["data"]["response"]["batch"]["id"].as_i64().unwrap();
    assert_eq!(runtime.read().await.active_batch_id, Some(batch_id));

    let stopped = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/integrations/ainas/tasks")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "external_task_id": "ainas-stop-001",
                        "action": "stop_process",
                        "process_id": process_id
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stopped.status(), StatusCode::OK);
    let body = to_bytes(stopped.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["data"]["status"], "executed");
    assert_eq!(body["data"]["response"]["stopped_batch_id"], batch_id);
    assert_eq!(runtime.read().await.active_batch_id, None);

    let events = db.recent_control_events(10).unwrap();
    assert!(events
        .iter()
        .any(|event| event.event_type == "ainas_process_started"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "ainas_process_stopped"));
}

#[tokio::test]
async fn integration_process_actions_reject_null_process_id_without_starting_device() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let process_id = add_simple_process(&db, "null process id guard");
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 120.0, 12.0, 20.0)).await;
    let (device, recorded_device) = recording_device();
    let app_state = AppState {
        db: db.clone(),
        runtime: runtime.clone(),
        device,
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };
    let app = router(app_state.clone(), PathBuf::from("static"));

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/integrations/ainas/tasks")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "external_task_id": "ainas-null-start-process-id",
                        "action": "start_process",
                        "process_id": null
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
    assert!(body["message"]
        .as_str()
        .unwrap_or_default()
        .contains("process_id must not be null"));
    assert_eq!(runtime.read().await.active_batch_id, None);
    assert!(recorded_device.writes.lock().unwrap().is_empty());
    assert!(db.recent_batches_sqlx(10).await.unwrap().is_empty());
    assert!(db.integration_tasks(Some("ainas"), 10).unwrap().is_empty());
    assert!(db.recent_control_events(10).unwrap().is_empty());

    let mqtt_receipt = execute_mqtt_task_payload(
        &app_state,
        json!({
            "external_task_id": "mqtt-null-start-process-id",
            "action": "start_process",
            "process_id": null
        })
        .to_string()
        .as_bytes(),
    )
    .await;
    assert!(!mqtt_receipt.ok);
    assert_eq!(mqtt_receipt.status, "rejected");
    assert!(mqtt_receipt
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("process_id must not be null"));
    assert_eq!(runtime.read().await.active_batch_id, None);
    assert!(recorded_device.writes.lock().unwrap().is_empty());
    assert!(db.recent_batches_sqlx(10).await.unwrap().is_empty());
    assert!(db.integration_tasks(Some("mqtt"), 10).unwrap().is_empty());
    assert!(db.recent_control_events(10).unwrap().is_empty());

    let stop_without_process_id = execute_mqtt_task_payload(
        &app_state,
        json!({
            "external_task_id": "mqtt-stop-current-without-process-id",
            "action": "stop_process",
            "reason": "no active process, but stop path stays callable"
        })
        .to_string()
        .as_bytes(),
    )
    .await;
    assert!(!stop_without_process_id.ok);
    assert_eq!(stop_without_process_id.status, "rejected");
    assert!(stop_without_process_id
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("no active process batch to stop"));
    assert!(db
        .integration_tasks(Some("mqtt"), 10)
        .unwrap()
        .iter()
        .any(|task| {
            task.external_task_id.as_deref() == Some("mqtt-stop-current-without-process-id")
                && task.status == "rejected"
        }));
    assert_eq!(runtime.read().await.active_batch_id, None);
    assert!(recorded_device.writes.lock().unwrap().is_empty());
    assert!(db.recent_batches_sqlx(10).await.unwrap().is_empty());

    let valid_receipt = execute_mqtt_task_payload(
        &app_state,
        json!({
            "external_task_id": "mqtt-valid-start-after-null",
            "action": "start_process",
            "process_id": process_id
        })
        .to_string()
        .as_bytes(),
    )
    .await;
    assert!(valid_receipt.ok);
    assert_eq!(valid_receipt.status, "executed");
    assert!(runtime.read().await.active_batch_id.is_some());
    assert_eq!(recorded_device.writes.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn ainas_process_action_latches_fault_when_receipt_update_fails_after_device_write() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let process_id = add_simple_process(&db, "ainas receipt failure");
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 120.0, 12.0, 20.0)).await;
    let (device, recorded_device) = break_integration_tasks_on_write_device(db.clone());
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/integrations/ainas/tasks")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "external_task_id": "ainas-receipt-fails-after-write",
                        "action": "start_process",
                        "process_id": process_id
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(recorded_device.writes.lock().unwrap().len(), 1);
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, Some(1));
    assert!(!state.auto_enabled);
    assert!(state
        .last_control_error
        .as_deref()
        .unwrap_or_default()
        .contains("integration task start_process receipt audit failed after device action"));
}

#[tokio::test]
async fn integration_set_targets_latches_fault_when_receipt_update_fails_after_runtime_commit() {
    let safety = Arc::new(safety());
    let temp_dir = tempfile::tempdir().unwrap();
    let db = Db::open(temp_dir.path().join("reactor.sqlite3")).unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let app_state = AppState {
        db: db.clone(),
        runtime: runtime.clone(),
        device: test_device(),
        device_mode: DeviceMode::Pipeline,
        device_config: device_config(),
        safety,
        ai_memory: memory(),
        ai_provider: None,
        test_reset_enabled: false,
    };
    db.after_control_event_success_for_tests(Arc::new({
        let db = db.clone();
        move || {
            db.break_integration_tasks_for_tests()
                .expect("integration tasks table should be removable after target audit insert");
        }
    }));

    let mqtt_receipt = execute_mqtt_task_payload(
        &app_state,
        json!({
            "external_task_id": "mqtt-receipt-fails-after-target-commit",
            "action": "set_targets",
            "target_temperature_c": 72.5,
            "target_pressure_mpa": 0.8,
            "target_shake_speed_cpm": 45.0,
            "reason": "mqtt receipt failure after target commit"
        })
        .to_string()
        .as_bytes(),
    )
    .await;

    assert!(!mqtt_receipt.ok);
    assert_eq!(mqtt_receipt.status, "failed");
    let state = runtime.read().await;
    assert_eq!(state.targets.temperature_c, 72.5);
    assert_eq!(state.targets.target_pressure_mpa, 0.8);
    assert_eq!(state.targets.shake_speed_cpm, 45.0);
    assert!(state
        .last_control_error
        .as_deref()
        .unwrap_or_default()
        .contains("integration task set_targets receipt failed after target intent commit"));
    assert!(!state.auto_enabled);
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
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .header("authorization", auth_header("operator"))
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
                .header("authorization", auth_header("operator"))
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
    assert!(body["data"]["items"][0]["batch_id"].is_null());
    assert_eq!(body["data"]["items"][0]["data"]["current_temp"], 85.5);
    assert_eq!(body["data"]["items"][0]["data"]["tilt_state"], 1);
    assert!(
        body["data"]["items"][0]["data"]["tilt_angle"]
            .as_f64()
            .unwrap()
            >= 0.0
    );
    assert_eq!(body["data"]["records"], body["data"]["items"]);
}

#[tokio::test]
async fn v1_realtime_marks_unproven_downstream_status_offline_in_strict_mode() {
    let mut strict_safety = safety();
    strict_safety.control.require_device_status_for_control = true;
    let safety = Arc::new(strict_safety);
    let db = Db::open_memory().unwrap();
    let batch = db
        .create_batch_for_process_sqlx(None, "strict realtime active", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(85.5, 0.12, 800.0, 30.0, 45.5)).await;
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(batch.id);
    }
    let app = router(
        AppState {
            db,
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/v1/reactor/reactor_001/realtime")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["status"], "offline");
    assert_eq!(body["device_online"], false);
    assert_eq!(body["device_status"]["status"], "offline");
    assert_eq!(body["data"]["current_temp"], 85.5);
    assert_eq!(body["data"]["phase"], "offline");
    assert!(body["alarms"]
        .as_array()
        .unwrap()
        .iter()
        .any(|alarm| alarm["type"] == "device_status_unavailable" && alarm["level"] == "high"));
}

#[tokio::test]
async fn v1_realtime_surfaces_unfinished_batch_recovery_offline_state() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let orphan = db
        .create_batch_for_process_sqlx(None, "realtime orphan unfinished", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(85.5, 0.12, 800.0, 30.0, 45.5)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(healthy_device_status());
    }
    let app = router(
        AppState {
            db,
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/v1/reactor/reactor_001/realtime")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["status"], "error");
    assert_eq!(body["device_online"], false);
    assert_eq!(body["device_status"]["unfinished_batch_ids"][0], orphan.id);
    assert_eq!(
        body["device_status"]["unexpected_unfinished_batch_ids"][0],
        orphan.id
    );
    assert_eq!(body["data"]["phase"], "offline");
    assert!(body["alarms"].as_array().unwrap().iter().any(|alarm| {
        alarm["type"] == "unfinished_batch_recovery"
            && alarm["unfinished_batch_ids"][0] == orphan.id
    }));
}

#[tokio::test]
async fn v1_realtime_rejects_stale_sample_instead_of_fabricating_timestamp() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let mut sample = fresh_sample(85.5, 0.12, 800.0, 30.0, 45.5);
    sample.captured_at =
        Utc::now() - chrono::Duration::milliseconds(safety.control.sensor_timeout_ms + 1000);
    install_runtime_sample(&runtime, &db, sample).await;
    let app = router(
        AppState {
            db,
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/v1/reactor/reactor_001/realtime")
                .header("authorization", auth_header("operator"))
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
    assert!(body["data"].get("timestamp").is_none());
    assert!(body["data"].get("current_temp").is_none());
}

#[tokio::test]
async fn v1_realtime_rejects_future_timestamp_sample() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let mut sample = fresh_sample(85.5, 0.12, 800.0, 30.0, 45.5);
    sample.captured_at = Utc::now() + chrono::Duration::milliseconds(5000);
    install_runtime_sample(&runtime, &db, sample).await;
    let app = router(
        AppState {
            db,
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/v1/reactor/reactor_001/realtime")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 503);
    assert!(body["message"].as_str().unwrap().contains("timestamp is"));
    assert!(body["message"].as_str().unwrap().contains("future"));
    assert!(body["data"].get("timestamp").is_none());
}

#[tokio::test]
async fn v1_control_rejects_values_outside_interface_document_ranges() {
    let safety = Arc::new(safety());
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: Db::open_memory().unwrap(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .header("authorization", auth_header("operator"))
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
async fn v1_control_rejects_explicit_null_params_instead_of_defaulting() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let original_targets = runtime.read().await.targets.clone();
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    for (payload, expected_message) in [
        (
            json!({
                "command_id": "cmd_null_heat_time",
                "params": {
                    "heat_time": null,
                    "hold_time": 600,
                    "cool_time": 180,
                    "stir_speed": 650,
                    "shake_speed": 30,
                    "target_temp": 90.0,
                    "target_pressure": 0.5
                },
                "priority": "normal",
                "auto_start": false
            }),
            "heat_time must not be null",
        ),
        (
            json!({
                "command_id": "cmd_null_auto_start",
                "params": {
                    "heat_time": 300,
                    "hold_time": 600,
                    "cool_time": 180,
                    "stir_speed": 650,
                    "shake_speed": 30,
                    "target_temp": 90.0,
                    "target_pressure": 0.5
                },
                "priority": "normal",
                "auto_start": null
            }),
            "auto_start must not be null",
        ),
        (
            json!({
                "command_id": null,
                "params": {
                    "heat_time": 300,
                    "hold_time": 600,
                    "cool_time": 180,
                    "stir_speed": 650,
                    "shake_speed": 30,
                    "target_temp": 90.0,
                    "target_pressure": 0.5
                },
                "priority": "normal",
                "auto_start": false
            }),
            "must not be null",
        ),
        (
            json!({
                "command_id": "cmd_null_timestamp",
                "timestamp": null,
                "params": {
                    "heat_time": 300,
                    "hold_time": 600,
                    "cool_time": 180,
                    "stir_speed": 650,
                    "shake_speed": 30,
                    "target_temp": 90.0,
                    "target_pressure": 0.5
                },
                "priority": "normal",
                "auto_start": false
            }),
            "must not be null",
        ),
        (
            json!({
                "command_id": "cmd_null_priority",
                "params": {
                    "heat_time": 300,
                    "hold_time": 600,
                    "cool_time": 180,
                    "stir_speed": 650,
                    "shake_speed": 30,
                    "target_temp": 90.0,
                    "target_pressure": 0.5
                },
                "priority": null,
                "auto_start": false
            }),
            "must not be null",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/reactor/reactor_001/control")
                    .header("authorization", auth_header("operator"))
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(
            [StatusCode::BAD_REQUEST, StatusCode::UNPROCESSABLE_ENTITY]
                .contains(&response.status()),
            "unexpected status for v1 null payload: {}",
            response.status()
        );
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        let message = body["message"].as_str().unwrap();
        assert!(
            message.contains(expected_message),
            "unexpected v1 control null rejection: {message}"
        );
    }

    assert!(db.recent_control_events(10).unwrap().is_empty());
    assert!(recorded_device.writes.lock().unwrap().is_empty());
    let state = runtime.read().await;
    assert_eq!(state.targets, original_targets);
    assert!(!state.auto_enabled);
}

#[tokio::test]
async fn v1_control_rejects_params_without_recognized_control_fields() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let original_targets = runtime.read().await.targets.clone();
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    for payload in [
        json!({
            "command_id": "cmd_empty_params",
            "params": {},
            "priority": "normal",
            "auto_start": false
        }),
        json!({
            "command_id": "cmd_unknown_params",
            "params": {
                "operator_note": "do not turn this into defaults"
            },
            "priority": "normal",
            "auto_start": false
        }),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/reactor/reactor_001/control")
                    .header("authorization", auth_header("operator"))
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert!(body["message"]
            .as_str()
            .unwrap()
            .contains("control params must include at least one recognized control parameter"));
    }

    assert_eq!(runtime.read().await.targets, original_targets);
    assert!(db.recent_control_events(10).unwrap().is_empty());
    assert!(recorded_device.writes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn v1_control_accepts_optimizer_duration_bounds_used_by_ai_recommendations() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let app = router(
        AppState {
            db,
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .header("authorization", auth_header("operator"))
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
async fn v1_control_auto_start_rolls_back_runtime_when_audit_fails() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );
    db.break_control_events_for_tests().unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/reactor/reactor_001/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "command_id": "cmd_audit_fail",
                        "params": {
                            "heat_time": 300,
                            "hold_time": 600,
                            "cool_time": 180,
                            "stir_speed": 650,
                            "shake_speed": 30,
                            "target_temp": 90.0,
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

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["message"], "internal server error");
    assert!(!body.to_string().contains("control_events"));
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, None);
    assert_eq!(state.auto_enabled, false);
    assert_eq!(state.targets.temperature_c, 20.0);
    assert_eq!(state.targets.stirrer_rpm, 0.0);
    assert_eq!(state.targets.shake_speed_cpm, 0.0);
    assert_eq!(state.targets.target_pressure_mpa, 0.0);
    assert!(state
        .last_control_error
        .as_deref()
        .unwrap_or_default()
        .contains("v1 auto_start audit failed after device action"));
    drop(state);
    let writes = recorded_device.writes.lock().unwrap();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0].target_temperature_c, 90.0);
    assert_eq!(writes[0].target_stirrer_rpm, 650.0);
    assert_eq!(writes[0].target_shake_speed_cpm, 30.0);
    assert_eq!(writes[1].target_temperature_c, 20.0);
    assert_eq!(writes[1].target_stirrer_rpm, 0.0);
    assert_eq!(writes[1].target_shake_speed_cpm, 0.0);
    assert_eq!(writes[1].target_pressure_mpa, 0.0);
}

#[tokio::test]
async fn v1_control_auto_start_rejects_active_batch_before_runtime_changes() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let active = db
        .create_batch_for_process_sqlx(None, "active v1 conflict", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(active.id);
        state.auto_enabled = true;
    }
    let original_targets = runtime.read().await.targets.clone();
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "command_id": "cmd_busy_auto_start",
                        "params": {
                            "heat_time": 300,
                            "hold_time": 600,
                            "cool_time": 180,
                            "stir_speed": 650,
                            "shake_speed": 30,
                            "target_temp": 90.0,
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

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("device is busy running an active batch"));
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, Some(active.id));
    assert_eq!(state.targets, original_targets);
    assert!(state.auto_enabled);
    drop(state);
    assert!(recorded_device.writes.lock().unwrap().is_empty());
    assert!(db.recent_control_events(10).unwrap().is_empty());
    assert_eq!(db.recent_batches_sqlx(10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn process_start_rechecks_interlocks_after_device_write_before_committing_runtime() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let (device, tripped_device) = runtime_trip_device(runtime.clone());
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let batch_start = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/batches/start")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "interlock trip batch",
                        "target_temperature_c": 82.0,
                        "target_stirrer_rpm": 460.0,
                        "target_shake_speed_cpm": 24.0,
                        "heating_minutes": 75.0,
                        "stirring_minutes": 65.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(batch_start.status(), StatusCode::CONFLICT);
    {
        let state = runtime.read().await;
        assert!(state.emergency_stop);
        assert_eq!(state.active_batch_id, None);
        assert!(!state.auto_enabled);
        assert!(state
            .last_control_error
            .as_deref()
            .unwrap_or_default()
            .contains("batch start final interlock failed after device action"));
    }
    let writes = tripped_device.writes.lock().unwrap();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0].target_temperature_c, 82.0);
    assert_eq!(writes[1].target_temperature_c, 20.0);
    assert_eq!(writes[1].target_stirrer_rpm, 0.0);
}

#[tokio::test]
async fn batch_start_keeps_runtime_inactive_until_audit_then_final_interlock() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let audit_hook_ran = Arc::new(AtomicBool::new(false));
    {
        let runtime = runtime.clone();
        let audit_hook_ran = audit_hook_ran.clone();
        db.after_control_event_success_for_tests(Arc::new(move || {
            let mut state = runtime
                .try_write()
                .expect("runtime lock should be available after batch audit insert");
            assert_eq!(state.active_batch_id, None);
            assert!(!state.auto_enabled);
            state.emergency_stop = true;
            audit_hook_ran.store(true, Ordering::SeqCst);
        }));
    }
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/batches/start")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "audit then final interlock trip batch",
                        "target_temperature_c": 82.0,
                        "target_stirrer_rpm": 460.0,
                        "target_shake_speed_cpm": 24.0,
                        "heating_minutes": 75.0,
                        "stirring_minutes": 65.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(audit_hook_ran.load(Ordering::SeqCst));
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, None);
    assert!(!state.auto_enabled);
    assert!(state.emergency_stop);
    assert!(state
        .last_control_error
        .as_deref()
        .unwrap_or_default()
        .contains("batch start final interlock failed after device action"));
    drop(state);
    let writes = recorded_device.writes.lock().unwrap();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0].target_temperature_c, 82.0);
    assert_eq!(writes[1].target_temperature_c, 20.0);
    assert_eq!(writes[1].target_stirrer_rpm, 0.0);
}

#[tokio::test]
async fn batch_start_rechecks_unfinished_db_state_after_device_write_before_activation() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let (device, recorded_device) = create_batch_on_first_write_device(db.clone());
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/batches/start")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "device write creates orphan batch",
                        "target_temperature_c": 82.0,
                        "target_stirrer_rpm": 460.0,
                        "target_shake_speed_cpm": 24.0,
                        "heating_minutes": 75.0,
                        "stirring_minutes": 65.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, None);
    assert!(!state.auto_enabled);
    assert_eq!(state.targets.temperature_c, 20.0);
    assert!(state
        .last_control_error
        .as_deref()
        .unwrap_or_default()
        .contains("batch start final interlock failed after device action"));
    drop(state);
    let unfinished = db.unfinished_batches_sqlx(10).await.unwrap();
    assert_eq!(
        unfinished.len(),
        1,
        "activation batch should be rolled back, leaving only the orphan recovery batch"
    );
    assert_eq!(
        unfinished[0].name,
        "orphan created after first device write"
    );
    let writes = recorded_device.writes.lock().unwrap();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0].target_temperature_c, 82.0);
    assert_eq!(writes[1].target_temperature_c, 20.0);
    assert_eq!(writes[1].target_stirrer_rpm, 0.0);
}

#[tokio::test]
async fn v1_auto_start_rechecks_interlocks_after_device_write_before_committing_runtime() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let (device, tripped_device) = runtime_trip_device(runtime.clone());
    let app = router(
        AppState {
            db,
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "command_id": "cmd_interlock_trip",
                        "params": {
                            "heat_time": 300,
                            "hold_time": 600,
                            "cool_time": 180,
                            "stir_speed": 650,
                            "shake_speed": 30,
                            "target_temp": 90.0,
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

    assert_eq!(response.status(), StatusCode::CONFLICT);
    {
        let state = runtime.read().await;
        assert!(state.emergency_stop);
        assert_eq!(state.active_batch_id, None);
        assert!(!state.auto_enabled);
        assert!(state
            .last_control_error
            .as_deref()
            .unwrap_or_default()
            .contains("v1 auto_start final interlock failed after device action"));
    }
    let writes = tripped_device.writes.lock().unwrap();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0].target_temperature_c, 90.0);
    assert_eq!(writes[1].target_temperature_c, 20.0);
    assert_eq!(writes[1].target_pressure_mpa, 0.0);
}

#[tokio::test]
async fn batch_start_rolls_back_device_and_runtime_when_audit_fails() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );
    db.break_control_events_for_tests().unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/batches/start")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "audit failure batch",
                        "target_temperature_c": 82.0,
                        "target_stirrer_rpm": 460.0,
                        "target_shake_speed_cpm": 24.0,
                        "heating_minutes": 75.0,
                        "stirring_minutes": 65.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, None);
    assert_eq!(state.auto_enabled, false);
    assert_eq!(state.targets.temperature_c, 20.0);
    assert_eq!(state.targets.stirrer_rpm, 0.0);
    assert_eq!(state.targets.shake_speed_cpm, 0.0);
    assert_eq!(state.targets.target_pressure_mpa, 0.0);
    drop(state);
    let writes = recorded_device.writes.lock().unwrap();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0].target_temperature_c, 82.0);
    assert_eq!(writes[0].target_stirrer_rpm, 460.0);
    assert_eq!(writes[0].target_shake_speed_cpm, 24.0);
    assert_eq!(writes[1].target_temperature_c, 20.0);
    assert_eq!(writes[1].target_stirrer_rpm, 0.0);
    assert_eq!(writes[1].target_shake_speed_cpm, 0.0);
    assert_eq!(writes[1].target_pressure_mpa, 0.0);
}

#[tokio::test]
async fn start_failures_before_activation_are_audited_without_arming_runtime() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: failing_target_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let batch_start = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/batches/start")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "device write failure batch",
                        "target_temperature_c": 82.0,
                        "target_stirrer_rpm": 460.0,
                        "target_shake_speed_cpm": 24.0,
                        "heating_minutes": 75.0,
                        "stirring_minutes": 65.0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(batch_start.status(), StatusCode::SERVICE_UNAVAILABLE);
    {
        let state = runtime.read().await;
        assert_eq!(state.active_batch_id, None);
        assert!(!state.auto_enabled);
        assert!(state.last_control_error.is_some());
    }
    {
        let mut state = runtime.write().await;
        state.last_control_error = None;
    }

    let v1_auto_start = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/reactor/reactor_001/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "command_id": "cmd_device_write_fail",
                        "params": {
                            "heat_time": 300,
                            "hold_time": 600,
                            "cool_time": 180,
                            "stir_speed": 650,
                            "shake_speed": 30,
                            "target_temp": 90.0,
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
    assert_eq!(v1_auto_start.status(), StatusCode::SERVICE_UNAVAILABLE);
    {
        let state = runtime.read().await;
        assert_eq!(state.active_batch_id, None);
        assert!(!state.auto_enabled);
        assert!(state.last_control_error.is_some());
    }

    let events = db.recent_control_events(10).unwrap();
    let failure_reasons: Vec<_> = events
        .iter()
        .filter(|event| event.event_type == "process_start_failed")
        .map(|event| event.reason.as_str())
        .collect();
    assert!(failure_reasons
        .iter()
        .any(|reason| reason.contains("batch start failed before activation")));
    assert!(failure_reasons
        .iter()
        .any(|reason| reason.contains("v1 control start failed before activation")));
}

#[tokio::test]
async fn v1_auto_start_does_not_send_stop_write_when_device_start_write_fails() {
    // Regression for c1361d1e: when the device start write fails, the field was
    // never commanded on, so the failure path must NOT call the post-activation
    // rollback (which re-sends a stop write and conservatively re-arms
    // active_batch_id). Only the single failed start write should reach the
    // device, and runtime must stay unarmed. The pre-fix code invoked
    // rollback_v1_auto_start_activation on the start-write-failure branch, which
    // is only correct after a successful start.
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let (device, recorded_device) = failing_target_recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "command_id": "cmd_start_write_fail_no_stop",
                        "params": {
                            "heat_time": 300,
                            "hold_time": 600,
                            "cool_time": 180,
                            "stir_speed": 650,
                            "shake_speed": 30,
                            "target_temp": 90.0,
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
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    {
        let state = runtime.read().await;
        assert_eq!(
            state.active_batch_id, None,
            "runtime must not arm an active batch when the start write never reached the field"
        );
        assert!(!state.auto_enabled);
        assert!(state.last_control_error.is_some());
    }
    let writes = recorded_device.writes.lock().unwrap();
    assert_eq!(
        writes.len(),
        1,
        "device start-write failure must not trigger a rollback stop write; the field was never commanded on"
    );
    drop(writes);

    let events = db.recent_control_events(10).unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == "process_start_failed"
            && event
                .reason
                .contains("v1 control start failed before activation")
    }));
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
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .header("authorization", auth_header("operator"))
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
                .header("authorization", auth_header("operator"))
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
                .header("authorization", auth_header("operator"))
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
                .header("authorization", auth_header("engineer"))
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
async fn product_result_requires_finished_non_active_batch() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let unfinished = db
        .create_batch_for_process_sqlx(None, "unfinished result", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    let active = db
        .create_batch_for_process_sqlx(None, "active result", 65.0, 320.0, 12.0, 12.0)
        .await
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(active.id);
        state.auto_enabled = true;
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let unfinished_result = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/product-results")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "batch_id": unfinished.id,
                        "yield_percent": 80.0,
                        "product_ratio": 0.86,
                        "notes": "should not record unfinished"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unfinished_result.status(), StatusCode::CONFLICT);
    assert!(db.batch_outcome_by_id(unfinished.id).unwrap().is_none());

    let active_result = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/product-results")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "batch_id": active.id,
                        "yield_percent": 82.0,
                        "product_ratio": 0.88,
                        "notes": "should not record active"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(active_result.status(), StatusCode::CONFLICT);
    assert!(db.batch_outcome_by_id(active.id).unwrap().is_none());
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, Some(active.id));
    assert!(state.auto_enabled);
}

#[tokio::test]
async fn product_result_rejects_explicit_null_notes_without_persisting_or_recommending() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let finished = db
        .create_batch_for_process_sqlx(None, "null notes product result", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    db.finish_batch_sqlx(finished.id).await.unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: db.clone(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/product-results")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "batch_id": finished.id,
                        "yield_percent": 82.0,
                        "product_ratio": 0.88,
                        "notes": null
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
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("must not be null"));
    assert!(db.batch_outcome_by_id(finished.id).unwrap().is_none());
    assert!(db.latest_recommendation().unwrap().is_none());
    assert!(db.recent_control_events(10).unwrap().is_empty());
}

#[tokio::test]
async fn product_result_rejects_while_any_batch_is_active() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let finished = db
        .create_batch_for_process_sqlx(None, "finished result target", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    db.finish_batch_sqlx(finished.id).await.unwrap();
    let active = db
        .create_batch_for_process_sqlx(None, "active production", 65.0, 320.0, 12.0, 12.0)
        .await
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(active.id);
        state.auto_enabled = true;
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/product-results")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "batch_id": finished.id,
                        "yield_percent": 80.0,
                        "product_ratio": 0.86,
                        "notes": "should wait for active production"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("active; finish and verify active production first"));
    assert!(db.batch_outcome_by_id(finished.id).unwrap().is_none());
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, Some(active.id));
    assert!(state.auto_enabled);
}

#[tokio::test]
async fn product_result_rejects_unfinished_batch_recovery_state() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let finished = db
        .create_batch_for_process_sqlx(None, "finished result target", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    db.finish_batch_sqlx(finished.id).await.unwrap();
    let orphan = db
        .create_batch_for_process_sqlx(None, "orphan unfinished", 65.0, 320.0, 12.0, 12.0)
        .await
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: db.clone(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/product-results")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "batch_id": finished.id,
                        "yield_percent": 80.0,
                        "product_ratio": 0.86,
                        "notes": "should wait for recovery"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains("until unfinished batch recovery is resolved")
            && message.contains(&orphan.id.to_string()),
        "unexpected recovery rejection: {message}"
    );
    assert!(db.batch_outcome_by_id(finished.id).unwrap().is_none());
}

#[tokio::test]
async fn product_result_does_not_commit_when_audit_fails() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let batch = db
        .create_batch_for_process_sqlx(
            None,
            "audit failure product result",
            60.0,
            300.0,
            10.0,
            10.0,
        )
        .await
        .unwrap();
    db.finish_batch_sqlx(batch.id).await.unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: db.clone(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );
    db.break_control_events_for_tests().unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/product-results")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "batch_id": batch.id,
                        "yield_percent": 80.0,
                        "product_ratio": 0.86,
                        "notes": "must not commit without audit"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["message"], "internal server error");
    assert!(!body.to_string().contains("control_events"));
    assert!(db.batch_outcome_by_id(batch.id).unwrap().is_none());
    assert!(db.latest_recommendation().unwrap().is_none());
}

#[tokio::test]
async fn product_result_does_not_commit_recommendation_when_recommendation_audit_fails() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let batch = db
        .create_batch_for_process_sqlx(
            None,
            "recommendation audit failure product result",
            60.0,
            300.0,
            10.0,
            10.0,
        )
        .await
        .unwrap();
    db.finish_batch_sqlx(batch.id).await.unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: db.clone(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );
    db.fail_control_events_after_successes_for_tests(1);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/product-results")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "batch_id": batch.id,
                        "yield_percent": 80.0,
                        "product_ratio": 0.86,
                        "notes": "recommendation audit must not partially commit"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(db.batch_outcome_by_id(batch.id).unwrap().is_some());
    assert!(db.latest_recommendation().unwrap().is_none());
    let events = db.recent_control_events(10).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "product_result_recorded");
}

#[tokio::test]
async fn batch_export_and_report_are_generated_from_backend_data() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(77.8, 0.42, 405.0, 28.0, 51.0)).await;
    let hostile_batch_name = "=report\u{200B}\n<img src=x onerror=alert(1)>\t[x](javascript:alert(1)) ![x](x) | # heading\u{202E}";
    let cleaned_batch_name =
        "=report <img src=x onerror=alert(1)> [x](javascript:alert(1)) ![x](x) | # heading";
    let hostile_audit_reason =
        "operator\u{200B}\t<script>alert(1)</script> [ack](javascript:x)\u{202E}";
    let hostile_result_notes = "result\u{200B}\n<script>alert(1)</script>\toperator note\u{202E}";
    let app = router(
        AppState {
            db: db.clone(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: true,
        },
        PathBuf::from("static"),
    );

    let start = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/batches/start")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": hostile_batch_name,
                        "target_temperature_c": 78.0,
                        "target_stirrer_rpm": 410.0,
                        "heating_minutes": 55.0,
                        "stirring_minutes": 45.0
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

    for sample in [
        json!({
            "temperature_c": 77.8,
            "pressure_mpa": 0.42,
            "stirrer_rpm": 405.0,
            "shake_speed_cpm": 28.0,
            "tilt_state": 1,
            "flow_rate_l_min": 1.2,
            "product_concentration_percent": 51.0,
            "ph": 6.7
        }),
        json!({
            "temperature_c": 78.4,
            "pressure_mpa": 0.44,
            "stirrer_rpm": 412.0,
            "shake_speed_cpm": 29.0,
            "tilt_state": 0,
            "flow_rate_l_min": 1.3,
            "product_concentration_percent": 55.0,
            "ph": 6.8
        }),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/reactor/reactor_001/samples")
                    .header("authorization", auth_header("engineer"))
                    .header("content-type", "application/json")
                    .body(Body::from(sample.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    let finish = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/batches/{batch_id}/finish"))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(finish.status(), StatusCode::NO_CONTENT);

    let result = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/product-results")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "batch_id": batch_id,
                        "yield_percent": 83.2,
                        "product_ratio": 0.91,
                        "notes": hostile_result_notes
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    assert_eq!(
        db.product_result_notes_for_tests(batch_id)
            .unwrap()
            .as_deref(),
        Some("result <script>alert(1)</script> operator note")
    );

    db.insert_control_event(
        Some(batch_id),
        "manual|operator_note",
        None,
        hostile_audit_reason,
    )
    .unwrap();

    let export = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/batches/export.csv")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(export.status(), StatusCode::OK);
    let body = to_bytes(export.into_body(), usize::MAX).await.unwrap();
    let csv = String::from_utf8(body.to_vec()).unwrap();
    assert!(csv.contains(&format!("\"'{cleaned_batch_name}\"")));
    assert!(!csv.contains(&format!(",{cleaned_batch_name},")));
    assert!(!csv.contains('\u{200B}'));
    assert!(!csv.contains('\u{202E}'));
    assert!(csv.contains("83.2"));

    let xlsx = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/batches/export.xlsx")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(xlsx.status(), StatusCode::OK);
    assert_eq!(
        xlsx.headers()["content-type"],
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    );
    let body = to_bytes(xlsx.into_body(), usize::MAX).await.unwrap();
    assert!(body.starts_with(b"PK\x03\x04"));
    let mut archive = zip::ZipArchive::new(Cursor::new(body.to_vec())).unwrap();
    let workbook = read_zip_entry(&mut archive, "xl/workbook.xml");
    assert!(workbook.contains("Batches"));
    assert!(workbook.contains("Results"));
    assert!(workbook.contains("Summary"));
    let batch_sheet = read_zip_entry(&mut archive, "xl/worksheets/sheet1.xml");
    assert!(batch_sheet.contains(
        "&apos;=report &lt;img src=x onerror=alert(1)&gt; [x](javascript:alert(1)) ![x](x) | # heading"
    ));
    assert!(!batch_sheet.contains("<t>=report <img"));
    assert!(!batch_sheet.contains("<img src=x"));
    assert!(!batch_sheet.contains('\u{200B}'));
    assert!(!batch_sheet.contains('\u{202E}'));
    let summary_sheet = read_zip_entry(&mut archive, "xl/worksheets/sheet3.xml");
    assert!(summary_sheet.contains("average_yield_percent"));

    let report = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/batches/{batch_id}/report.md"))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(report.status(), StatusCode::OK);
    let body = to_bytes(report.into_body(), usize::MAX).await.unwrap();
    let report = String::from_utf8(body.to_vec()).unwrap();
    assert!(report.contains("# Experiment Report"));
    assert!(report.contains(
        "=report &lt;img src=x onerror=alert\\(1\\)&gt; \\[x\\]\\(javascript:alert\\(1\\)\\) \\!\\[x\\]\\(x\\) \\| \\# heading"
    ));
    assert!(report.contains("Sensor Statistics"));
    assert!(report.contains("Yield: 83.20%"));
    assert!(report.contains("batch\\_started"));
    assert!(report.contains("product\\_result\\_recorded"));
    assert!(report.contains(
        "[manual\\|operator\\_note] operator &lt;script&gt;alert\\(1\\)&lt;/script&gt; \\[ack\\]\\(javascript:x\\)"
    ));
    assert!(!report.contains("<img"));
    assert!(!report.contains("<script"));
    assert!(!report.contains("[x](javascript"));
    assert!(!report.contains("![x](x)"));
    assert!(!report.contains("[ack](javascript"));
    assert!(!report.contains("| # heading"));
    assert!(!report.contains('\t'));
    assert!(!report.contains('\u{200B}'));
    assert!(!report.contains('\u{202E}'));
}

#[tokio::test]
async fn process_definition_create_rejects_explicit_null_name_without_persisting() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: db.clone(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/processes")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": null,
                        "description": "null process name must not default"
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
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("must not be null"));
    assert!(db.list_processes().unwrap().is_empty());
    assert!(db.recent_control_events(10).unwrap().is_empty());
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
            db: db.clone(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "name": "RX-78 polymerization", "description": "lab process" })
                        .to_string(),
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

    let null_cooling_mode_step = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/processes/{process_id}/steps"))
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "null cooling mode must not default",
                        "target_temperature_c": 90.0,
                        "ramp_rate_c_min": 2.5,
                        "duration_minutes": 45.0,
                        "target_stirrer_rpm": 300.0,
                        "target_shake_speed_cpm": 30.0,
                        "target_pressure_mpa": 0.5,
                        "cooling_mode": null
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        null_cooling_mode_step.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let body = to_bytes(null_cooling_mode_step.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("must not be null"));
    assert_eq!(
        db.process_detail(process_id).unwrap().unwrap().steps.len(),
        0
    );

    let null_step = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/processes/{process_id}/steps"))
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "null pressure must not default",
                        "target_temperature_c": 90.0,
                        "ramp_rate_c_min": 2.5,
                        "duration_minutes": 45.0,
                        "target_stirrer_rpm": 300.0,
                        "target_shake_speed_cpm": 30.0,
                        "target_pressure_mpa": null,
                        "cooling_mode": "natural"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(null_step.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(null_step.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("target_pressure_mpa must not be null"));
    assert_eq!(
        db.process_detail(process_id).unwrap().unwrap().steps.len(),
        0
    );

    for (name, temp, duration, rpm, pressure) in [
        ("heat", 120.0, 45.0, 150.0, 0.5),
        ("hold", 120.0, 120.0, 300.0, 2.4),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/processes/{process_id}/steps"))
                    .header("authorization", auth_header("engineer"))
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
                            "cooling_mode": "natural"
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
                .header("authorization", auth_header("operator"))
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
async fn process_definition_writes_reject_active_or_unfinished_batch_state() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let process_id = add_simple_process(&db, "blocked process edits");
    let active = db
        .create_batch_for_process_sqlx(None, "active blocks process edit", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(active.id);
        state.auto_enabled = true;
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/processes")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "name": "must not create during active production" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::CONFLICT);
    let body = to_bytes(create.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("active; finish and verify active production first"));

    {
        let mut state = runtime.write().await;
        state.active_batch_id = None;
        state.auto_enabled = false;
    }
    db.finish_batch_sqlx(active.id).await.unwrap();
    let orphan = db
        .create_batch_for_process_sqlx(None, "orphan blocks process edit", 65.0, 320.0, 12.0, 12.0)
        .await
        .unwrap();

    let add_step = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/processes/{process_id}/steps"))
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "must not add during recovery",
                        "target_temperature_c": 90.0,
                        "ramp_rate_c_min": 2.0,
                        "duration_minutes": 30.0,
                        "target_stirrer_rpm": 240.0,
                        "target_shake_speed_cpm": 24.0,
                        "target_pressure_mpa": 0.5,
                        "cooling_mode": "natural"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(add_step.status(), StatusCode::CONFLICT);
    let body = to_bytes(add_step.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains(&orphan.id.to_string()));
    assert_eq!(
        db.process_detail(process_id).unwrap().unwrap().steps.len(),
        1
    );
}

#[tokio::test]
async fn process_start_and_stop_manage_active_batch_and_auto_control() {
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
    let process = db
        .create_process("process lifecycle", "start stop")
        .unwrap();
    db.add_process_step(
        process.id,
        &reactor_edge_daemon::db::NewProcessStep {
            name: "heat".to_string(),
            target_temperature_c: 90.0,
            ramp_rate_c_min: 2.0,
            duration_minutes: 30.0,
            target_stirrer_rpm: 240.0,
            target_shake_speed_cpm: 24.0,
            target_pressure_mpa: 0.5,
            cooling_mode: "natural".to_string(),
        },
    )
    .unwrap();
    db.add_process_step(
        process.id,
        &reactor_edge_daemon::db::NewProcessStep {
            name: "hold".to_string(),
            target_temperature_c: 90.0,
            ramp_rate_c_min: 0.0,
            duration_minutes: 45.0,
            target_stirrer_rpm: 420.0,
            target_shake_speed_cpm: 36.0,
            target_pressure_mpa: 0.8,
            cooling_mode: "natural".to_string(),
        },
    )
    .unwrap();
    let app = router(
        AppState {
            db,
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let started = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/processes/{}/start", process.id))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(started.status(), StatusCode::OK);
    let body = to_bytes(started.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 0);
    assert_eq!(body["data"]["status"], "running");
    assert_eq!(body["data"]["batch"]["process_id"], process.id);
    let batch_id = body["data"]["batch"]["id"].as_i64().unwrap();
    assert_eq!(body["data"]["applied_targets"]["temperature_c"], 90.0);
    assert_eq!(body["data"]["applied_targets"]["stirrer_rpm"], 420.0);
    assert_eq!(body["data"]["applied_targets"]["shake_speed_cpm"], 36.0);

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
    assert_eq!(body["runtime"]["active_batch_id"], batch_id);
    assert_eq!(body["runtime"]["auto_enabled"], true);
    assert!(body["recent_events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["event_type"] == "process_started"));

    let conflict = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/processes/{}/start", process.id))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    let body = to_bytes(conflict.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 409);
    assert!(body["message"].as_str().unwrap().contains("busy"));

    let long_stop_reason = format!(
        "  operator requested stop after\n downstream\tpump inspection \u{0007} alarm acknowledged {}  ",
        "x".repeat(260)
    );
    let expected_stop_reason: String = format!(
        "operator requested stop after downstream pump inspection alarm acknowledged {}",
        "x".repeat(260)
    )
    .chars()
    .take(240)
    .collect();
    let stopped = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/processes/current/stop")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "reason": long_stop_reason
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stopped.status(), StatusCode::OK);
    let body = to_bytes(stopped.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 0);
    assert_eq!(body["data"]["stopped_batch_id"], batch_id);
    assert_eq!(body["data"]["active_batch_id"], Value::Null);
    assert_eq!(body["data"]["auto_enabled"], false);
    assert_eq!(body["data"]["stopped_targets"]["shake_speed_cpm"], 0.0);
    assert_eq!(body["data"]["stopped_targets"]["target_pressure_mpa"], 0.0);
    assert_eq!(body["data"]["batch"]["finished_at"].is_string(), true);

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
    assert_eq!(body["runtime"]["active_batch_id"], Value::Null);
    assert_eq!(body["runtime"]["auto_enabled"], false);
    assert_eq!(body["runtime"]["targets"]["temperature_c"], 20.0);
    assert_eq!(body["runtime"]["targets"]["stirrer_rpm"], 0.0);
    assert_eq!(body["runtime"]["targets"]["shake_speed_cpm"], 0.0);
    assert_eq!(body["runtime"]["targets"]["target_pressure_mpa"], 0.0);
    assert!(body["recent_events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["event_type"] == "process_stopped"
            && event["reason"] == expected_stop_reason));
}

#[tokio::test]
async fn process_start_revalidates_persisted_steps_before_creating_batch_or_writing_device() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
    }
    let process = db
        .create_process("corrupted persisted process", "must fail closed")
        .unwrap();
    let step = db
        .add_process_step(
            process.id,
            &reactor_edge_daemon::db::NewProcessStep {
                name: "heat".to_string(),
                target_temperature_c: 90.0,
                ramp_rate_c_min: 2.0,
                duration_minutes: 30.0,
                target_stirrer_rpm: 240.0,
                target_shake_speed_cpm: 24.0,
                target_pressure_mpa: 0.5,
                cooling_mode: "natural".to_string(),
            },
        )
        .unwrap()
        .unwrap();
    db.corrupt_process_step_for_tests(step.id, Some(-15.0), None, None)
        .unwrap();
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri(format!("/api/processes/{}/start", process.id))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains(&format!("process step {}", step.id))
            && message.contains("duration_minutes must be between 1"),
        "unexpected corrupted process step rejection: {message}"
    );
    assert!(db.recent_batches_sqlx(10).await.unwrap().is_empty());
    assert!(db.recent_control_events(10).unwrap().is_empty());
    assert!(recorded_device.writes.lock().unwrap().is_empty());
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, None);
    assert!(state.auto_enabled);
}

#[tokio::test]
async fn process_start_rejects_role_duration_that_would_have_been_clamped() {
    let mut strict_safety = safety();
    strict_safety.optimizer.max_heating_minutes = 20.0;
    strict_safety.optimizer.max_stirring_minutes = 45.0;
    let safety = Arc::new(strict_safety);
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
    }
    let process = db
        .create_process("role duration clamp regression", "must fail closed")
        .unwrap();
    let step = db
        .add_process_step(
            process.id,
            &reactor_edge_daemon::db::NewProcessStep {
                name: "heat".to_string(),
                target_temperature_c: 90.0,
                ramp_rate_c_min: 2.0,
                duration_minutes: 30.0,
                target_stirrer_rpm: 240.0,
                target_shake_speed_cpm: 24.0,
                target_pressure_mpa: 0.5,
                cooling_mode: "natural".to_string(),
            },
        )
        .unwrap()
        .unwrap();
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri(format!("/api/processes/{}/start", process.id))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let message = body["message"].as_str().unwrap();
    assert!(
        message.contains(&format!(
            "process step {} heating_duration_minutes",
            step.id
        )) && message.contains("must be between 1 and 20"),
        "unexpected role duration rejection: {message}"
    );
    assert!(db.recent_batches_sqlx(10).await.unwrap().is_empty());
    assert!(db.recent_control_events(10).unwrap().is_empty());
    assert!(recorded_device.writes.lock().unwrap().is_empty());
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, None);
    assert!(state.auto_enabled);
}

#[tokio::test]
async fn process_start_rejects_missing_or_empty_process_before_runtime_changes() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let empty_process = db
        .create_process("empty process start must fail closed", "no steps")
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
    }
    let original_targets = runtime.read().await.targets.clone();
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    for (process_id, expected_status, expected_message) in [
        (999_999, StatusCode::NOT_FOUND, "process not found"),
        (
            empty_process.id,
            StatusCode::BAD_REQUEST,
            "process must contain at least one step before starting",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/processes/{process_id}/start"))
                    .header("authorization", auth_header("operator"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), expected_status);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        let message = body["message"].as_str().unwrap();
        assert!(
            message.contains(expected_message),
            "unexpected process start rejection: {message}"
        );
    }

    assert!(db.recent_batches_sqlx(10).await.unwrap().is_empty());
    assert!(db.recent_control_events(10).unwrap().is_empty());
    assert!(recorded_device.writes.lock().unwrap().is_empty());
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, None);
    assert_eq!(state.targets, original_targets);
    assert!(state.auto_enabled);
}

#[tokio::test]
async fn process_start_rolls_back_runtime_when_audit_fails_after_activation() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(35.4, 0.0629, 300.0, 30.0, 12.9)).await;
    let process_id = add_simple_process(&db, "audit failure rollback");
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );
    db.break_control_events_for_tests().unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/processes/{process_id}/start"))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["message"], "internal server error");
    assert!(!body.to_string().contains("control_events"));
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, None);
    assert_eq!(state.auto_enabled, false);
    assert_eq!(state.targets.temperature_c, 20.0);
    assert_eq!(state.targets.stirrer_rpm, 0.0);
    assert_eq!(state.targets.shake_speed_cpm, 0.0);
    assert_eq!(state.targets.target_pressure_mpa, 0.0);
    drop(state);
    let writes = recorded_device.writes.lock().unwrap();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0].target_temperature_c, 90.0);
    assert_eq!(writes[0].target_stirrer_rpm, 240.0);
    assert_eq!(writes[0].target_shake_speed_cpm, 24.0);
    assert_eq!(writes[1].target_temperature_c, 20.0);
    assert_eq!(writes[1].target_stirrer_rpm, 0.0);
    assert_eq!(writes[1].target_shake_speed_cpm, 0.0);
    assert_eq!(writes[1].target_pressure_mpa, 0.0);
}

#[tokio::test]
async fn process_start_preserves_active_recovery_state_when_rollback_stop_fails() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(35.4, 0.0629, 300.0, 30.0, 12.9)).await;
    let process_id = add_simple_process(&db, "audit failure rollback stop failure");
    let (device, recorded_device) = start_then_fail_stop_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );
    db.break_control_events_for_tests().unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/processes/{process_id}/start"))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let state = runtime.read().await;
    let active_batch_id = state
        .active_batch_id
        .expect("active batch must remain for field recovery when rollback stop fails");
    assert!(!state.auto_enabled);
    assert_eq!(state.targets.temperature_c, 90.0);
    assert_eq!(state.targets.stirrer_rpm, 240.0);
    assert_eq!(state.targets.shake_speed_cpm, 24.0);
    let last_control_error = state.last_control_error.as_deref().unwrap_or_default();
    assert!(
        last_control_error.contains("process start audit failed after device action"),
        "unexpected fault: {last_control_error}"
    );
    assert!(
        last_control_error.contains("activation rollback stop command also failed"),
        "fault should preserve rollback stop failure context: {last_control_error}"
    );
    assert!(
        last_control_error.contains("field may still be running"),
        "fault should keep operator in field recovery mode: {last_control_error}"
    );
    drop(state);
    let active_batch = db.batch_by_id_sqlx(active_batch_id).await.unwrap().unwrap();
    assert!(
        active_batch.finished_at.is_none(),
        "batch must remain unfinished until stop/finish recovery succeeds"
    );
    let unfinished = db.unfinished_batches_sqlx(10).await.unwrap();
    assert_eq!(unfinished.len(), 1);
    assert_eq!(unfinished[0].id, active_batch_id);
    let writes = recorded_device.writes.lock().unwrap();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0].target_temperature_c, 90.0);
    assert_eq!(writes[0].target_stirrer_rpm, 240.0);
    assert_eq!(writes[1].target_temperature_c, 20.0);
    assert_eq!(writes[1].target_stirrer_rpm, 0.0);
}

#[tokio::test]
async fn process_start_keeps_runtime_inactive_until_audit_and_process_state_commit() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(35.4, 0.0629, 300.0, 30.0, 12.9)).await;
    let process_id = add_simple_process(&db, "audit then final interlock process");
    let audit_hook_ran = Arc::new(AtomicBool::new(false));
    {
        let runtime = runtime.clone();
        let audit_hook_ran = audit_hook_ran.clone();
        db.after_control_event_success_for_tests(Arc::new(move || {
            let mut state = runtime
                .try_write()
                .expect("runtime lock should be available after process audit insert");
            assert_eq!(state.active_batch_id, None);
            assert!(!state.auto_enabled);
            state.emergency_stop = true;
            audit_hook_ran.store(true, Ordering::SeqCst);
        }));
    }
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri(format!("/api/processes/{process_id}/start"))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(audit_hook_ran.load(Ordering::SeqCst));
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, None);
    assert!(!state.auto_enabled);
    assert!(state.emergency_stop);
    assert!(state
        .last_control_error
        .as_deref()
        .unwrap_or_default()
        .contains("process start final interlock failed after device action"));
    drop(state);
    let writes = recorded_device.writes.lock().unwrap();
    assert_eq!(writes.len(), 2);
    assert_eq!(writes[0].target_temperature_c, 90.0);
    assert_eq!(writes[1].target_temperature_c, 20.0);
    assert_eq!(writes[1].target_stirrer_rpm, 0.0);
}

#[tokio::test]
async fn process_stop_keeps_runtime_stopped_when_audit_fails() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(35.4, 0.0629, 300.0, 30.0, 12.9)).await;
    let process_id = add_simple_process(&db, "stop audit failure");
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let started = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/processes/{process_id}/start"))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(started.status(), StatusCode::OK);
    assert!(runtime.read().await.active_batch_id.is_some());
    assert!(runtime.read().await.auto_enabled);
    db.break_control_events_for_tests().unwrap();

    let stopped = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/processes/{process_id}/stop"))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stopped.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let state = runtime.read().await;
    let batch_id = state
        .active_batch_id
        .expect("active batch remains for stop retry");
    assert!(!state.auto_enabled);
    assert_eq!(state.targets.temperature_c, 20.0);
    assert_eq!(state.targets.stirrer_rpm, 0.0);
    assert_eq!(state.targets.shake_speed_cpm, 0.0);
    assert_eq!(state.targets.target_pressure_mpa, 0.0);
    assert!(state
        .last_control_error
        .as_deref()
        .unwrap_or_default()
        .contains("process stop audit failed after device action"));
    drop(state);

    db.repair_control_events_for_tests().unwrap();
    {
        let mut state = runtime.write().await;
        state.last_control_error = None;
    }
    let retry = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/processes/{process_id}/stop"))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(retry.status(), StatusCode::OK);
    let body = to_bytes(retry.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["data"]["stopped_batch_id"], batch_id);
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, None);
    assert!(!state.auto_enabled);
    assert!(state.last_control_error.is_none());
}

#[tokio::test]
async fn process_stop_retries_stop_write_when_finished_batch_is_still_runtime_active() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(35.4, 0.0629, 300.0, 30.0, 12.9)).await;
    let process_id = add_simple_process(&db, "finished but runtime active stop retry");
    let batch = db
        .create_batch_for_process_sqlx(Some(process_id), "finished active", 90.0, 240.0, 30.0, 30.0)
        .await
        .unwrap();
    db.finish_batch_sqlx(batch.id).await.unwrap();
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(batch.id);
        state.auto_enabled = true;
        state.targets.temperature_c = 90.0;
        state.targets.stirrer_rpm = 240.0;
        state.targets.shake_speed_cpm = 24.0;
        state.targets.target_pressure_mpa = 0.5;
    }
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri(format!("/api/processes/{process_id}/stop"))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, None);
    assert!(!state.auto_enabled);
    assert_eq!(state.targets.temperature_c, 20.0);
    assert_eq!(state.targets.stirrer_rpm, 0.0);
    assert_eq!(state.targets.shake_speed_cpm, 0.0);
    assert_eq!(state.targets.target_pressure_mpa, 0.0);
    drop(state);
    let writes = recorded_device.writes.lock().unwrap();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].target_temperature_c, 20.0);
    assert_eq!(writes[0].target_stirrer_rpm, 0.0);
    assert_eq!(writes[0].target_shake_speed_cpm, 0.0);
    assert_eq!(writes[0].target_pressure_mpa, 0.0);
}

#[tokio::test]
async fn process_stop_rejects_active_batch_change_after_stop_before_finish() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(35.4, 0.0629, 300.0, 30.0, 12.9)).await;
    let process_id = add_simple_process(&db, "stop race process");
    let batch = db
        .create_batch_for_process_sqlx(
            Some(process_id),
            "stop race original",
            90.0,
            240.0,
            30.0,
            30.0,
        )
        .await
        .unwrap();
    let replacement_batch = db
        .create_batch_for_process_sqlx(None, "stop race replacement", 65.0, 320.0, 12.0, 12.0)
        .await
        .unwrap();
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(batch.id);
        state.auto_enabled = true;
        state.targets.temperature_c = 90.0;
        state.targets.stirrer_rpm = 240.0;
        state.targets.shake_speed_cpm = 24.0;
        state.targets.target_pressure_mpa = 0.5;
    }
    let (device, recorded_device) =
        change_active_batch_on_write_device(runtime.clone(), Some(replacement_batch.id));
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri(format!("/api/processes/{process_id}/stop"))
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, Some(replacement_batch.id));
    assert!(!state.auto_enabled);
    assert_eq!(state.targets.temperature_c, 20.0);
    assert_eq!(state.targets.stirrer_rpm, 0.0);
    assert_eq!(state.targets.shake_speed_cpm, 0.0);
    assert_eq!(state.targets.target_pressure_mpa, 0.0);
    assert!(state
        .last_control_error
        .as_deref()
        .unwrap_or_default()
        .contains("process stop active batch changed after stop command"));
    drop(state);
    let writes = recorded_device.writes.lock().unwrap();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].target_temperature_c, 20.0);
    assert_eq!(writes[0].target_stirrer_rpm, 0.0);
    assert_eq!(writes[0].target_shake_speed_cpm, 0.0);
    assert_eq!(writes[0].target_pressure_mpa, 0.0);
    drop(writes);
    let batch = db.batch_by_id_sqlx(batch.id).await.unwrap().unwrap();
    assert!(
        batch.finished_at.is_none(),
        "ambiguous active batch identity must not close the original process batch"
    );
    assert_eq!(db.audit_event_count(Some("process_stopped")).unwrap(), 0);
}

#[tokio::test]
async fn current_process_stop_writes_stop_when_runtime_active_batch_record_is_missing() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(35.4, 0.0629, 300.0, 30.0, 12.9)).await;
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(4242);
        state.auto_enabled = true;
        state.targets.temperature_c = 90.0;
        state.targets.stirrer_rpm = 240.0;
        state.targets.shake_speed_cpm = 24.0;
        state.targets.target_pressure_mpa = 0.5;
    }
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/processes/current/stop")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["data"]["stopped_batch_id"], 4242);
    assert_eq!(body["data"]["batch"], Value::Null);
    assert!(body["data"]["recovery"]
        .as_str()
        .unwrap()
        .contains("active runtime batch 4242 record was missing"));
    let state = runtime.read().await;
    assert_eq!(state.active_batch_id, None);
    assert!(!state.auto_enabled);
    assert_eq!(state.targets.temperature_c, 20.0);
    assert_eq!(state.targets.stirrer_rpm, 0.0);
    assert_eq!(state.targets.shake_speed_cpm, 0.0);
    assert_eq!(state.targets.target_pressure_mpa, 0.0);
    drop(state);
    let writes = recorded_device.writes.lock().unwrap();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].target_temperature_c, 20.0);
    assert_eq!(writes[0].target_stirrer_rpm, 0.0);
    assert_eq!(writes[0].target_shake_speed_cpm, 0.0);
    assert_eq!(writes[0].target_pressure_mpa, 0.0);
    drop(writes);
    let events = db.recent_control_events(10).unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == "process_stop_recovery_missing_batch"
            && event.batch_id.is_none()
            && event.reason.contains("4242")
    }));
}

#[tokio::test]
async fn process_stop_by_id_still_rejects_when_active_batch_record_is_missing() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(35.4, 0.0629, 300.0, 30.0, 12.9)).await;
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(4242);
        state.auto_enabled = true;
    }
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db,
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/processes/7/stop")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(runtime.read().await.active_batch_id, Some(4242));
    assert!(runtime.read().await.auto_enabled);
    assert!(recorded_device.writes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn process_stop_without_active_batch_returns_json_error_code() {
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
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/processes/current/stop")
                .header("authorization", auth_header("operator"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["code"], 409);
    assert!(body["message"].as_str().unwrap().contains("no active"));
}

#[tokio::test]
async fn ai_master_control_rejects_missing_sensor_sample_with_json_error_code() {
    let safety = Arc::new(safety());
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    let app = router(
        AppState {
            db: Db::open_memory().unwrap(),
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/ai/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"dry_run": false}).to_string()))
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
}

#[tokio::test]
async fn ai_control_execute_failure_forces_auto_disabled_before_returning_error() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    add_ai_outcomes(&db);
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.latest_sample = None;
    }
    let app = router(
        AppState {
            db,
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let dry_run = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/control")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"dry_run": true}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(dry_run.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(runtime.read().await.auto_enabled);

    let execute = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/control")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"dry_run": false}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(execute.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = to_bytes(execute.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("sensor data unavailable"));
    let state = runtime.read().await;
    assert!(!state.auto_enabled);
    assert_eq!(state.targets.temperature_c, 60.0);
    assert_eq!(state.targets.stirrer_rpm, 300.0);
}

#[tokio::test]
async fn ai_control_rejects_explicit_null_control_flags_before_side_effects() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    {
        let mut state = runtime.write().await;
        state.auto_enabled = true;
        state.latest_sample = None;
    }
    let original_targets = runtime.read().await.targets.clone();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    for (payload, expected_message) in [
        (
            json!({
                "dry_run": null,
                "allow_target_adjustment": true
            }),
            "dry_run must not be null",
        ),
        (
            json!({
                "dry_run": true,
                "preferred_process_id": null
            }),
            "preferred_process_id must not be null",
        ),
        (
            json!({
                "intent": null,
                "dry_run": true
            }),
            "must not be null",
        ),
        (
            json!({
                "mode": null,
                "dry_run": true
            }),
            "must not be null",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/ai/control")
                    .header("authorization", auth_header("engineer"))
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(
            [StatusCode::BAD_REQUEST, StatusCode::UNPROCESSABLE_ENTITY]
                .contains(&response.status()),
            "unexpected status for AI null payload: {}",
            response.status()
        );
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert!(body["message"].as_str().unwrap().contains(expected_message));
    }

    let state = runtime.read().await;
    assert!(state.auto_enabled);
    assert_eq!(state.targets, original_targets);
    drop(state);
    assert!(db.recent_control_events(10).unwrap().is_empty());
}

#[tokio::test]
async fn ai_target_adjustment_does_not_commit_runtime_when_audit_fails() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    add_ai_outcomes(&db);
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(35.4, 0.0629, 300.0, 0.0, 12.9)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(healthy_device_status());
    }
    let original_targets = runtime.read().await.targets.clone();
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );

    let preview = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/control")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "dry_run": true,
                        "allow_process_start": false,
                        "allow_process_stop": false,
                        "allow_component_control": false,
                        "allow_target_adjustment": true
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(preview.status(), StatusCode::OK);
    let preview_body = to_bytes(preview.into_body(), usize::MAX).await.unwrap();
    let preview_body: Value = serde_json::from_slice(&preview_body).unwrap();
    assert_eq!(preview_body["data"]["decision"], "adjust_targets");

    db.break_control_events_for_tests().unwrap();
    let execute = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/control")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "dry_run": false,
                        "allow_process_start": false,
                        "allow_process_stop": false,
                        "allow_component_control": false,
                        "allow_target_adjustment": true
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(execute.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(runtime.read().await.targets, original_targets);
    assert_eq!(recorded_device.writes.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn ai_target_adjustment_rejects_invalid_existing_runtime_targets_without_clamping() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    add_ai_outcomes(&db);
    let recommendation = reactor_edge_daemon::optimizer::recommend(
        &safety.optimizer,
        &db.recent_batch_outcomes(10).unwrap(),
    );
    db.insert_recommendation(&recommendation).unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(35.4, 0.0629, 300.0, 0.0, 12.9)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(healthy_device_status());
        state.targets.target_pressure_mpa = 12.0;
    }
    let original_targets = runtime.read().await.targets.clone();
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/ai/control")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "dry_run": false,
                        "allow_process_start": false,
                        "allow_process_stop": false,
                        "allow_component_control": false,
                        "allow_target_adjustment": true
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
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("target_pressure_mpa must be between 0 and 10"));
    assert_eq!(runtime.read().await.targets, original_targets);
    assert!(recorded_device.writes.lock().unwrap().is_empty());
    assert!(db.recent_control_events(10).unwrap().is_empty());
}

#[tokio::test]
async fn ai_master_control_dry_run_does_not_persist_generated_recommendation() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    add_ai_outcomes(&db);
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(35.4, 0.0629, 300.0, 0.0, 12.9)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(healthy_device_status());
    }
    let original_targets = runtime.read().await.targets.clone();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/ai/control")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "dry_run": true,
                        "allow_process_start": false,
                        "allow_process_stop": false,
                        "allow_component_control": false,
                        "allow_target_adjustment": true
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
    assert_eq!(body["data"]["decision"], "adjust_targets");
    assert_eq!(runtime.read().await.targets, original_targets);
    assert!(db.latest_recommendation().unwrap().is_none());
    assert!(db.recent_control_events(10).unwrap().is_empty());
}

#[tokio::test]
async fn ai_master_control_latches_fault_when_final_audit_fails_after_target_write() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    add_ai_outcomes(&db);
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(35.4, 0.0629, 300.0, 0.0, 12.9)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(healthy_device_status());
        state.auto_enabled = true;
    }
    let original_targets = runtime.read().await.targets.clone();
    let (device, recorded_device) = recording_device();
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device,
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );
    db.fail_control_events_after_successes_for_tests(2);

    let execute = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/control")
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "dry_run": false,
                        "allow_process_start": false,
                        "allow_process_stop": false,
                        "allow_component_control": false,
                        "allow_target_adjustment": true
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(execute.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_ne!(runtime.read().await.targets, original_targets);
    let state = runtime.read().await;
    assert!(!state.auto_enabled);
    assert!(state
        .last_control_error
        .as_deref()
        .unwrap_or_default()
        .contains("AI master decision audit failed after device action"));
    drop(state);
    assert_eq!(recorded_device.writes.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn ai_master_control_dry_run_plans_process_and_shake_without_side_effects() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    add_simple_process(&db, "ai dry process");
    add_ai_outcomes(&db);
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(35.4, 0.0629, 300.0, 0.0, 12.9)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(DeviceStatusSnapshot {
            connected: true,
            last_seen_at: Some(Utc::now()),
            last_frame_ok: true,
            relay: Some(0),
            motor: Some(0),
            tilt: Some(1),
            speed_delay_us: Some(10000),
            port: Some("/dev/ttyUSB0".to_string()),
            baudrate: Some(115200),
            last_command_request_id: None,
            last_command_ok: Some(true),
            last_command_error: None,
            updated_at: Utc::now(),
        });
    }
    let app = router(
        AppState {
            db,
            runtime: runtime.clone(),
            device: component_test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/v1/ai/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "dry_run": true,
                        "allow_process_start": true,
                        "allow_component_control": true
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
    assert_eq!(body["data"]["dry_run"], true);
    assert!(body["data"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action["action_type"] == "process_start" && action["status"] == "planned"));
    assert!(body["data"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(
            |action| action["action_type"] == "component_control" && action["status"] == "planned"
        ));
    assert_eq!(runtime.read().await.active_batch_id, None);
}

#[tokio::test]
async fn ai_master_control_dry_run_blocks_process_start_when_db_has_unfinished_batch() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    add_simple_process(&db, "ai blocked by orphan");
    add_ai_outcomes(&db);
    let orphan = db
        .create_batch_for_process_sqlx(None, "orphan for ai", 60.0, 300.0, 10.0, 10.0)
        .await
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(35.4, 0.0629, 300.0, 0.0, 12.9)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(healthy_device_status());
    }
    let app = router(
        AppState {
            db,
            runtime: runtime.clone(),
            device: component_test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/v1/ai/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "dry_run": true,
                        "allow_process_start": true,
                        "allow_component_control": true
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
    assert_eq!(body["data"]["decision"], "hold");
    assert_eq!(body["data"]["safety"]["batch_recovery_required"], true);
    assert_eq!(body["data"]["safety"]["unfinished_batch_ids"][0], orphan.id);
    assert_eq!(body["data"]["safety"]["unexpected_batch_ids"][0], orphan.id);
    assert_eq!(body["data"]["safety"]["device_online"], false);
    assert_eq!(body["data"]["safety"]["high_alarm_count"], 1);
    let actions = body["data"]["actions"].as_array().unwrap();
    assert!(actions.iter().any(|action| {
        action["action_type"] == "process_start"
            && action["status"] == "blocked"
            && action["message"]
                .as_str()
                .unwrap()
                .contains(&format!("unfinished batch {}", orphan.id))
    }));
    assert!(!actions.iter().any(|action| {
        action["action_type"] == "process_start" && action["status"] == "planned"
    }));
    assert!(!actions
        .iter()
        .any(|action| action["action_type"] == "component_control"));
    assert_eq!(runtime.read().await.active_batch_id, None);
}

#[tokio::test]
async fn ai_master_control_execute_starts_process_and_audits_decision() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let process_id = add_simple_process(&db, "ai execute process");
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(35.4, 0.0629, 300.0, 0.0, 12.9)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(DeviceStatusSnapshot {
            connected: true,
            last_seen_at: Some(Utc::now()),
            last_frame_ok: true,
            relay: Some(0),
            motor: Some(0),
            tilt: Some(1),
            speed_delay_us: Some(10000),
            port: Some("/dev/ttyUSB0".to_string()),
            baudrate: Some(115200),
            last_command_request_id: None,
            last_command_ok: Some(true),
            last_command_error: None,
            updated_at: Utc::now(),
        });
    }
    let app = router(
        AppState {
            db,
            runtime,
            device: component_test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/ai/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "dry_run": false,
                        "intent": "optimize;\npreserve\u{200B} structured audit fields,\t even with punctuation\u{202E}",
                        "preferred_process_id": process_id,
                        "allow_target_adjustment": false
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
    assert_eq!(body["data"]["dry_run"], false);
    assert!(body["data"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action["action_type"] == "process_start" && action["status"] == "executed"));
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
    assert!(body["runtime"]["active_batch_id"].is_i64());
    assert!(body["recent_events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["event_type"] == "ai_process_started"));
    let audit_event = body["recent_events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["event_type"] == "ai_master_decision")
        .expect("ai master decision audit event should be present");
    let audit_reason: Value = serde_json::from_str(audit_event["reason"].as_str().unwrap())
        .expect("ai master-control audit reason should be structured JSON");
    assert!(audit_reason["decision"]
        .as_str()
        .unwrap()
        .contains("start_process"));
    assert!(audit_reason["rationale"]
        .as_str()
        .unwrap()
        .contains("optimize; preserve structured audit fields"));
    assert!(!audit_reason["rationale"]
        .as_str()
        .unwrap()
        .contains(['\n', '\t', '\u{200B}', '\u{202E}']));
    assert!(audit_reason["actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action["action_type"] == "process_start" && action["status"] == "executed"));
}

#[tokio::test]
async fn ai_master_control_latches_fault_when_final_audit_fails_after_action() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let process_id = add_simple_process(&db, "ai final audit failure");
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(35.4, 0.0629, 300.0, 0.0, 12.9)).await;
    {
        let mut state = runtime.write().await;
        state.device_status = Some(healthy_device_status());
    }
    let app = router(
        AppState {
            db: db.clone(),
            runtime: runtime.clone(),
            device: component_test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
            safety,
            ai_memory: memory(),
            ai_provider: None,
            test_reset_enabled: false,
        },
        PathBuf::from("static"),
    );
    db.fail_control_events_after_successes_for_tests(1);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "dry_run": false,
                        "preferred_process_id": process_id,
                        "allow_target_adjustment": false,
                        "allow_component_control": false
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let state = runtime.read().await;
    assert!(state.active_batch_id.is_some());
    assert!(!state.auto_enabled);
    let last_control_error = state.last_control_error.as_deref().unwrap_or_default();
    assert!(
        last_control_error.contains("AI master decision audit failed after device action"),
        "unexpected control fault: {last_control_error}"
    );
}

#[tokio::test]
async fn ai_master_control_stops_active_process_when_product_concentration_is_high() {
    let safety = Arc::new(safety());
    let db = Db::open_memory().unwrap();
    let process_id = add_simple_process(&db, "ai stop process");
    let batch = db
        .create_batch_for_process(Some(process_id), "running", 90.0, 300.0, 30.0, 45.0)
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(90.0, 0.0629, 300.0, 24.0, 96.0)).await;
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(batch.id);
        state.auto_enabled = true;
        state.device_status = Some(DeviceStatusSnapshot {
            connected: true,
            last_seen_at: Some(Utc::now()),
            last_frame_ok: true,
            relay: Some(0),
            motor: Some(1),
            tilt: Some(1),
            speed_delay_us: Some(10000),
            port: Some("/dev/ttyUSB0".to_string()),
            baudrate: Some(115200),
            last_command_request_id: None,
            last_command_ok: Some(true),
            last_command_error: None,
            updated_at: Utc::now(),
        });
    }
    let app = router(
        AppState {
            db,
            runtime,
            device: component_test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/ai/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "dry_run": false,
                        "allow_process_stop": true,
                        "allow_target_adjustment": false
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
    assert!(body["data"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action["action_type"] == "process_stop" && action["status"] == "executed"));

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
    assert_eq!(body["runtime"]["active_batch_id"], Value::Null);
    assert!(body["recent_events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["event_type"] == "ai_process_stopped"));
}

#[tokio::test]
async fn ai_master_control_respects_configured_product_stop_threshold() {
    let safety = Arc::new(SafetyConfig {
        control: ControlConfig {
            ai_stop_product_concentration_percent: 99.0,
            ..safety().control
        },
        ..safety()
    });
    let db = Db::open_memory().unwrap();
    let process_id = add_simple_process(&db, "ai configurable stop process");
    let batch = db
        .create_batch_for_process(Some(process_id), "running", 90.0, 300.0, 30.0, 45.0)
        .unwrap();
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));
    install_runtime_sample(&runtime, &db, fresh_sample(90.0, 0.0629, 300.0, 24.0, 96.0)).await;
    {
        let mut state = runtime.write().await;
        state.active_batch_id = Some(batch.id);
        state.auto_enabled = true;
        state.device_status = Some(DeviceStatusSnapshot {
            connected: true,
            last_seen_at: Some(Utc::now()),
            last_frame_ok: true,
            relay: Some(0),
            motor: Some(1),
            tilt: Some(1),
            speed_delay_us: Some(10000),
            port: Some("/dev/ttyUSB0".to_string()),
            baudrate: Some(115200),
            last_command_request_id: None,
            last_command_ok: Some(true),
            last_command_error: None,
            updated_at: Utc::now(),
        });
    }
    let app = router(
        AppState {
            db,
            runtime,
            device: component_test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .uri("/api/ai/control")
                .header("authorization", auth_header("operator"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "dry_run": false,
                        "allow_process_stop": true,
                        "allow_target_adjustment": false
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
    assert!(!body["data"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action["action_type"] == "process_stop"));

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
    assert_eq!(body["runtime"]["active_batch_id"], batch.id);
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
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .header("authorization", auth_header("engineer"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "name": "offline configurable process" }).to_string(),
                ))
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
    install_runtime_sample(&runtime, &db, fresh_sample(50.0, 0.1, 240.0, 24.0, 10.0)).await;
    let app = router(
        AppState {
            db,
            runtime,
            device: test_device(),
            device_mode: DeviceMode::Pipeline,
            device_config: device_config(),
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
                .method("POST")
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
    assert_eq!(rec["provider"]["model"], "local-ga-sa-pid");
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
                .header("authorization", auth_header("operator"))
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
