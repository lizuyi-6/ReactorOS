use reactor_edge_daemon::{
    config::{
        load_device_config, load_safety_config, validate_device_config, validate_safety_config,
        DeviceConfig, DeviceMode, SafetyConfig,
    },
    db::Batch,
    field_scenario::{
        detect_field_scenario, detect_production_line, FieldScenarioContext, FieldScenarioKind,
        ProductionLineKind,
    },
    memory::load_ai_memory,
};

static FIELD_SCENARIO_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn clear_field_scenario_env() {
    std::env::remove_var("XINGSHU_FIELD_SCENARIO");
    std::env::remove_var("XINGSHU_PRODUCTION_LINE");
    std::env::remove_var("XINGSHU_FIELD_SITE_LABEL");
}

#[test]
fn parses_esp32_serial_device_mode() {
    let raw = r#"
mode = "esp32_serial"

[serial]
port = "/dev/ttyUSB0"
baudrate = 115200
parity = "N"
stopbits = 1
bytesize = 8
timeout_ms = 1000

[modbus]
slave_id = 1

[modbus.registers.temperature_c]
address = 0
scale = 0.1
offset = 0.0
min_valid = 0.0
max_valid = 250.0

[modbus.registers.stirrer_rpm]
address = 1
scale = 1.0
offset = 0.0
min_valid = 0.0
max_valid = 2000.0

[modbus.registers.target_temperature_c]
address = 10
scale = 0.1
offset = 0.0

[modbus.registers.target_stirrer_rpm]
address = 11
scale = 1.0
offset = 0.0

[esp32]
frame_prefix = "RX"
command_prefix = "TX"
checksum = true
max_line_bytes = 256

"#;

    let config: DeviceConfig = toml::from_str(raw).unwrap();

    assert_eq!(config.mode, DeviceMode::Esp32Serial);
    assert_eq!(config.serial.baudrate, 115200);
    assert_eq!(config.esp32.frame_prefix, "RX");
}

#[test]
fn default_device_config_uses_external_pipeline_mode() {
    let config = load_device_config("config/device.toml").unwrap();

    assert_eq!(config.mode, DeviceMode::Pipeline);
    assert_eq!(config.modbus.registers.pressure_mpa.address, 2);
    assert_eq!(config.modbus.registers.hold_time_s.address, 15);
}

#[test]
fn hardware_esp32_template_is_valid() {
    let config = load_device_config("config/device.esp32.toml").unwrap();

    assert_eq!(config.mode, DeviceMode::Esp32Serial);
    assert_eq!(config.serial.baudrate, 115200);
    assert_eq!(config.serial.parity, "N");
    assert!(config.esp32.checksum);
}

#[test]
fn hardware_json_bridge_template_is_valid() {
    let config = load_device_config("config/device.json_bridge.toml").unwrap();

    assert_eq!(config.mode, DeviceMode::JsonBridge);
    assert_eq!(
        config.json_bridge.state_path.to_string_lossy(),
        "/project/state.json"
    );
    assert_eq!(
        config.json_bridge.control_path.to_string_lossy(),
        "/project/control.json"
    );
    assert_eq!(config.json_bridge.max_state_age_ms, 6000);
}

#[test]
fn ai_memory_template_is_valid_and_inside_safety_optimizer_bounds() {
    let memory = load_ai_memory("config/ai_memory.toml").unwrap();
    let safety: SafetyConfig =
        toml::from_str(&std::fs::read_to_string("config/safety.toml").unwrap()).unwrap();

    memory
        .validate_against_optimizer_bounds(&safety.optimizer)
        .unwrap();
    let bounds = memory.effective_optimizer_bounds(&safety.optimizer);

    assert_eq!(memory.reference_batches.len(), 3);
    assert_eq!(memory.forbidden_zones.len(), 2);
    assert_eq!(bounds.min_temperature_c, 55.0);
    assert_eq!(bounds.max_temperature_c, 135.0);
    assert_eq!(bounds.min_stirrer_rpm, 250.0);
    assert_eq!(memory.sensor_limits.configured_count(), 8);
}

#[test]
fn field_scenario_defaults_to_offline_demo_for_empty_pipeline() {
    let _env_guard = FIELD_SCENARIO_ENV_LOCK.lock().unwrap();
    clear_field_scenario_env();
    let memory = load_ai_memory("config/ai_memory.toml").unwrap();
    let profile = detect_field_scenario(FieldScenarioContext {
        device_mode: &DeviceMode::Pipeline,
        runtime: None,
        include_runtime_signals: true,
        memory: &memory,
        processes: &[],
        recent_batches: &[],
        recent_outcomes: &[],
    });

    assert_eq!(profile.kind, FieldScenarioKind::OfflineDemo);
    assert!(profile
        .signals
        .iter()
        .any(|signal| signal == "recent_batches=0"));
}

#[test]
fn production_line_flags_petrochemical_materials_conservatively() {
    let _env_guard = FIELD_SCENARIO_ENV_LOCK.lock().unwrap();
    clear_field_scenario_env();
    let mut memory = load_ai_memory("config/ai_memory.toml").unwrap();
    memory.profile.material_family = "petrochemical refinery product".to_string();

    let field_profile = detect_field_scenario(FieldScenarioContext::config_only(
        &DeviceMode::Esp32Serial,
        &memory,
    ));
    let line_profile = detect_production_line(FieldScenarioContext::config_only(
        &DeviceMode::Esp32Serial,
        &memory,
    ));

    assert_eq!(field_profile.kind, FieldScenarioKind::LabResearch);
    assert_eq!(line_profile.kind, ProductionLineKind::PetrochemicalRefining);
    assert!(line_profile.petrochemical_handling_required);
    assert!(line_profile
        .actions
        .iter()
        .any(|action| action.contains("petroleum")));
}

#[test]
fn production_line_flags_biopharmaceutical_materials_independently() {
    let _env_guard = FIELD_SCENARIO_ENV_LOCK.lock().unwrap();
    clear_field_scenario_env();
    let mut memory = load_ai_memory("config/ai_memory.toml").unwrap();
    memory.profile.material_family = "biopharmaceutical fermentation".to_string();

    let field_profile = detect_field_scenario(FieldScenarioContext::config_only(
        &DeviceMode::Esp32Serial,
        &memory,
    ));
    let line_profile = detect_production_line(FieldScenarioContext::config_only(
        &DeviceMode::Esp32Serial,
        &memory,
    ));

    assert_eq!(field_profile.kind, FieldScenarioKind::LabResearch);
    assert_eq!(line_profile.kind, ProductionLineKind::Biopharmaceutical);
    assert!(line_profile.special_handling_required);
    assert!(!line_profile.petrochemical_handling_required);
}

#[test]
fn production_line_env_override_wins_over_auto_detection() {
    let _env_guard = FIELD_SCENARIO_ENV_LOCK.lock().unwrap();
    clear_field_scenario_env();
    std::env::set_var("XINGSHU_PRODUCTION_LINE", "petrochemical_refining");
    std::env::set_var("XINGSHU_FIELD_SITE_LABEL", "refinery line A");
    let memory = load_ai_memory("config/ai_memory.toml").unwrap();

    let field_profile = detect_field_scenario(FieldScenarioContext::config_only(
        &DeviceMode::Pipeline,
        &memory,
    ));
    let line_profile = detect_production_line(FieldScenarioContext::config_only(
        &DeviceMode::Pipeline,
        &memory,
    ));
    clear_field_scenario_env();

    assert_eq!(field_profile.kind, FieldScenarioKind::LabResearch);
    assert_eq!(line_profile.kind, ProductionLineKind::PetrochemicalRefining);
    assert_eq!(line_profile.site_label.as_deref(), Some("refinery line A"));
    assert_eq!(line_profile.confidence, 1.0);
    assert!(line_profile.petrochemical_handling_required);
}

#[test]
fn legacy_petrochemical_scenario_env_maps_to_production_line_only() {
    let _env_guard = FIELD_SCENARIO_ENV_LOCK.lock().unwrap();
    clear_field_scenario_env();
    std::env::set_var("XINGSHU_FIELD_SCENARIO", "petrochemical");
    let memory = load_ai_memory("config/ai_memory.toml").unwrap();

    let field_profile = detect_field_scenario(FieldScenarioContext::config_only(
        &DeviceMode::Pipeline,
        &memory,
    ));
    let line_profile = detect_production_line(FieldScenarioContext::config_only(
        &DeviceMode::Pipeline,
        &memory,
    ));
    clear_field_scenario_env();

    assert_eq!(field_profile.kind, FieldScenarioKind::LabResearch);
    assert_eq!(line_profile.kind, ProductionLineKind::PetrochemicalRefining);
    assert!(line_profile.petrochemical_handling_required);
}

#[test]
fn field_scenario_uses_recent_batches_for_pilot_scale() {
    let _env_guard = FIELD_SCENARIO_ENV_LOCK.lock().unwrap();
    clear_field_scenario_env();
    let memory = load_ai_memory("config/ai_memory.toml").unwrap();
    let now = chrono::Utc::now();
    let batches: Vec<Batch> = (0..5)
        .map(|id| Batch {
            id,
            process_id: None,
            name: format!("pilot batch {id}"),
            started_at: now,
            finished_at: Some(now),
            target_temperature_c: 80.0,
            target_stirrer_rpm: 400.0,
            heating_minutes: 60.0,
            stirring_minutes: 40.0,
        })
        .collect();

    let profile = detect_field_scenario(FieldScenarioContext {
        device_mode: &DeviceMode::Esp32Serial,
        runtime: None,
        include_runtime_signals: false,
        memory: &memory,
        processes: &[],
        recent_batches: &batches,
        recent_outcomes: &[],
    });

    assert_eq!(profile.kind, FieldScenarioKind::PilotScale);
    assert!(profile
        .signals
        .iter()
        .any(|signal| signal == "recent_batches=5"));
}

#[test]
fn safety_template_exposes_production_retry_guard_and_ai_stop_bounds() {
    let safety = load_safety_config("config/safety.toml").unwrap();

    assert_eq!(safety.control.write_retry_backoff_ms, 5000);
    assert_eq!(safety.control.safety_guard_timeout_ms, 1000);
    assert!(safety.control.require_device_status_for_control);
    assert_eq!(safety.control.ai_stop_product_concentration_percent, 95.0);
}

#[test]
fn safety_config_rejects_non_positive_timing_values() {
    let mut safety = load_safety_config("config/safety.toml").unwrap();
    safety.control.sensor_timeout_ms = 0;
    let err = validate_safety_config(&safety).unwrap_err().to_string();
    assert!(err.contains("control.sensor_timeout_ms"));

    let mut safety = load_safety_config("config/safety.toml").unwrap();
    safety.control.control_interval_ms = 0;
    let err = validate_safety_config(&safety).unwrap_err().to_string();
    assert!(err.contains("control.control_interval_ms"));

    let mut safety = load_safety_config("config/safety.toml").unwrap();
    safety.control.ai_stop_product_concentration_percent = 101.0;
    let err = validate_safety_config(&safety).unwrap_err().to_string();
    assert!(err.contains("ai_stop_product_concentration_percent"));
}

#[test]
fn device_config_rejects_unreliable_io_and_register_scaling() {
    let mut config = load_device_config("config/device.toml").unwrap();
    config.serial.timeout_ms = 0;
    let err = validate_device_config(&config).unwrap_err().to_string();
    assert!(err.contains("serial.timeout_ms"));

    let mut config = load_device_config("config/device.toml").unwrap();
    config.serial.parity = "bad".to_string();
    let err = validate_device_config(&config).unwrap_err().to_string();
    assert!(err.contains("serial.parity"));

    let mut config = load_device_config("config/device.toml").unwrap();
    config.json_bridge.max_state_age_ms = -1;
    let err = validate_device_config(&config).unwrap_err().to_string();
    assert!(err.contains("json_bridge.max_state_age_ms"));

    let mut config = load_device_config("config/device.toml").unwrap();
    config.modbus.registers.temperature_c.scale = 0.0;
    let err = validate_device_config(&config).unwrap_err().to_string();
    assert!(err.contains("modbus.registers.temperature_c.scale"));

    let mut config = load_device_config("config/device.toml").unwrap();
    config.modbus.registers.temperature_c.min_valid = 200.0;
    config.modbus.registers.temperature_c.max_valid = 100.0;
    let err = validate_device_config(&config).unwrap_err().to_string();
    assert!(err.contains("modbus.registers.temperature_c.min_valid"));

    let mut config = load_device_config("config/device.toml").unwrap();
    config.modbus.registers.target_temperature_c.scale = 0.0;
    let err = validate_device_config(&config).unwrap_err().to_string();
    assert!(err.contains("modbus.registers.target_temperature_c.scale"));
}
