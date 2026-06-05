use reactor_edge_daemon::{
    config::{load_device_config, DeviceConfig, DeviceMode, SafetyConfig},
    memory::load_ai_memory,
};

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
fn safety_template_exposes_production_retry_guard_and_ai_stop_bounds() {
    let safety: SafetyConfig =
        toml::from_str(&std::fs::read_to_string("config/safety.toml").unwrap()).unwrap();

    assert_eq!(safety.control.write_retry_backoff_ms, 5000);
    assert_eq!(safety.control.safety_guard_timeout_ms, 1000);
    assert_eq!(safety.control.ai_stop_product_concentration_percent, 95.0);
}
