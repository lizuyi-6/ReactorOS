use chrono::Utc;
use reactor_edge_daemon::{
    config::{JsonBridgeAdcSensor, JsonBridgeConfig},
    control::SafeCommand,
    device::{
        build_device, build_json_bridge_control, json_bridge_sample_from_state,
        json_bridge_status_from_state, parse_json_bridge_state, write_json_bridge_control,
        ComponentControlCommand,
    },
    state::ControlTargets,
};
use serde_json::json;
use tempfile::tempdir;

fn bridge_config() -> JsonBridgeConfig {
    let mut config = JsonBridgeConfig::default();
    config.max_state_age_ms = 10_000;
    config.adc.sensor = Some(JsonBridgeAdcSensor::ProductConcentrationPercent);
    config.adc.scale = 100.0 / 4095.0;
    config.adc.offset = 0.0;
    config.adc.min_valid = 0.0;
    config.adc.max_valid = 100.0;
    config
}

fn valid_state_json() -> String {
    json!({
        "connected": true,
        "last_seen_ms": Utc::now().timestamp_millis(),
        "last_frame_hex": "AA BB 00 11 00 00 00 00",
        "last_frame_ok": true,
        "adc": 2048,
        "status": 0b0000_0111,
        "relay": 1,
        "motor": 1,
        "tilt": 1,
        "speed_delay_us": 10000,
        "temperature_c": 64.25,
        "pressure_mpa": 0.50,
        "stirrer_rpm": 125.18,
        "shake_speed_cpm": 30.0,
        "flow_rate_l_min": 1.2,
        "ph": 6.15,
        "last_command_request_id": "reactor-os-1",
        "last_command_ok": true,
        "last_command_error": null,
        "port": "/dev/ttyUSB0",
        "baudrate": 115200
    })
    .to_string()
}

fn assert_json_bridge_tmp_files_clean(path: &std::path::Path) {
    for entry in std::fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            assert_json_bridge_tmp_files_clean(&path);
        } else {
            assert_ne!(path.extension().and_then(|ext| ext.to_str()), Some("tmp"));
        }
    }
}

fn json_bridge_config_for_paths(
    state_path: &std::path::Path,
    control_path: &std::path::Path,
) -> reactor_edge_daemon::config::DeviceConfig {
    let raw_config = format!(
        r#"
mode = "json_bridge"
[serial]
port = "/dev/ttyUSB0"
baudrate = 115200
parity = "N"
stopbits = 1
bytesize = 8
timeout_ms = 1000
[json_bridge]
state_path = "{}"
control_path = "{}"
max_state_age_ms = 10000
request_id_prefix = "reactor-os-test"
speed_steps_per_cycle = 200.0
speed_deadband_cpm = 1.0
temperature_deadband_c = 1.0
relay_temperature_control = false
[modbus]
slave_id = 1
[esp32]
frame_prefix = "RX"
command_prefix = "TX"
checksum = true
max_line_bytes = 256
[modbus.registers.temperature_c]
address = 0
scale = 0.1
offset = 0.0
min_valid = 0.0
max_valid = 500.0
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
"#,
        state_path.to_string_lossy().replace('\\', "\\\\"),
        control_path.to_string_lossy().replace('\\', "\\\\")
    );
    toml::from_str(&raw_config).unwrap()
}

fn component_safety() -> reactor_edge_daemon::config::SafetyConfig {
    reactor_edge_daemon::config::SafetyConfig {
        control: reactor_edge_daemon::config::ControlConfig {
            auto_enabled_default: false,
            manual_lock_default: false,
            control_interval_ms: 2000,
            sensor_timeout_ms: 6000,
            require_device_status_for_control: false,
            write_retry_backoff_ms: 5000,
            safety_guard_timeout_ms: 1000,
            ai_stop_product_concentration_percent: 95.0,
        },
        temperature: reactor_edge_daemon::config::TemperatureSafety {
            min_c: 20.0,
            max_c: 160.0,
            max_step_c: 2.0,
            default_target_c: 60.0,
        },
        stirrer: reactor_edge_daemon::config::StirrerSafety {
            min_rpm: 0.0,
            max_rpm: 1200.0,
            max_step_rpm: 50.0,
            default_target_rpm: 300.0,
        },
        optimizer: reactor_edge_daemon::config::OptimizerBounds {
            min_temperature_c: 35.0,
            max_temperature_c: 140.0,
            min_stirrer_rpm: 100.0,
            max_stirrer_rpm: 1000.0,
            min_heating_minutes: 15.0,
            max_heating_minutes: 240.0,
            min_stirring_minutes: 15.0,
            max_stirring_minutes: 240.0,
        },
        forbidden_control_zones: Vec::new(),
    }
}

#[test]
fn parses_json_bridge_state_into_sensor_sample_without_fake_values() {
    let config = bridge_config();
    let state = parse_json_bridge_state(&valid_state_json()).unwrap();

    let sample = json_bridge_sample_from_state(&config, &state).unwrap();

    assert_eq!(sample.temperature_c, 64.25);
    assert_eq!(sample.pressure_mpa, 0.50);
    assert_eq!(sample.stirrer_rpm, 125.18);
    assert_eq!(sample.shake_speed_cpm, 30.0);
    assert_eq!(sample.tilt_state, 1);
    assert!(sample.tilt_angle_deg >= 0.0);
    assert_eq!(sample.flow_rate_l_min, 1.20);
    assert_eq!(sample.product_concentration_percent, 50.01);
    assert_eq!(sample.ph, 6.15);
}

#[test]
fn rejects_json_bridge_state_when_required_sensor_is_missing() {
    let config = bridge_config();
    let state = parse_json_bridge_state(
        &json!({
            "connected": true,
            "last_seen_ms": Utc::now().timestamp_millis(),
            "last_frame_ok": true,
            "adc": 2048,
            "status": 0b0000_0100,
            "pressure_mpa": 0.50,
            "stirrer_rpm": 125.18,
            "shake_speed_cpm": 30.0,
            "flow_rate_l_min": 1.2,
            "ph": 6.15
        })
        .to_string(),
    )
    .unwrap();

    let err = json_bridge_sample_from_state(&config, &state)
        .unwrap_err()
        .to_string();

    assert!(err.contains("missing required sensor field temperature_c"));
    assert!(err.contains("no fake value generated"));
}

#[test]
fn rejects_stale_or_bad_json_bridge_state() {
    let config = bridge_config();
    let stale = parse_json_bridge_state(
        &json!({
            "connected": true,
            "last_seen_ms": Utc::now().timestamp_millis() - 60_000,
            "last_frame_ok": true,
            "status": 0b0000_0100,
            "temperature_c": 64.25,
            "pressure_mpa": 0.50,
            "stirrer_rpm": 125.18,
            "shake_speed_cpm": 30.0,
            "flow_rate_l_min": 1.2,
            "product_concentration_percent": 50.0,
            "ph": 6.15
        })
        .to_string(),
    )
    .unwrap();
    let err = json_bridge_sample_from_state(&config, &stale)
        .unwrap_err()
        .to_string();
    assert!(err.contains("state stale"));

    let future = parse_json_bridge_state(
        &json!({
            "connected": true,
            "last_seen_ms": Utc::now().timestamp_millis() + 60_000,
            "last_frame_ok": true,
            "status": 0b0000_0100,
            "temperature_c": 64.25,
            "pressure_mpa": 0.50,
            "stirrer_rpm": 125.18,
            "shake_speed_cpm": 30.0,
            "flow_rate_l_min": 1.2,
            "product_concentration_percent": 50.0,
            "ph": 6.15
        })
        .to_string(),
    )
    .unwrap();
    let err = json_bridge_sample_from_state(&config, &future)
        .unwrap_err()
        .to_string();
    assert!(err.contains("timestamp is"));
    assert!(err.contains("in the future"));

    let disconnected = parse_json_bridge_state(
        &json!({
            "connected": false,
            "last_seen_ms": Utc::now().timestamp_millis(),
            "last_frame_ok": true
        })
        .to_string(),
    )
    .unwrap();
    let err = json_bridge_sample_from_state(&config, &disconnected)
        .unwrap_err()
        .to_string();
    assert!(err.contains("downstream disconnected"));
}

#[test]
fn exposes_downstream_device_status_bits() {
    let state = parse_json_bridge_state(&valid_state_json()).unwrap();

    let status = json_bridge_status_from_state(&state).unwrap();

    assert!(status.connected);
    assert!(status.last_frame_ok);
    assert_eq!(status.relay, Some(1));
    assert_eq!(status.motor, Some(1));
    assert_eq!(status.tilt, Some(1));
    assert_eq!(status.speed_delay_us, Some(10000));
    assert_eq!(status.port.as_deref(), Some("/dev/ttyUSB0"));
    assert_eq!(status.baudrate, Some(115200));
}

#[test]
fn writes_control_json_atomically_with_unique_request_id_shape() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("control.json");
    let control = build_json_bridge_control("reactor-os", "motor", Some(json!(1)), None);

    write_json_bridge_control(&path, &control).unwrap();

    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(saved["request_id"]
        .as_str()
        .unwrap()
        .starts_with("reactor-os-"));
    assert_eq!(saved["command"], "motor");
    assert_eq!(saved["value"], 1);
    assert_json_bridge_tmp_files_clean(dir.path());
}

#[test]
fn writes_control_json_creates_and_syncs_parent_directory() {
    let dir = tempdir().unwrap();
    let path = dir
        .path()
        .join("bridge")
        .join("nested")
        .join("control.json");
    let control = build_json_bridge_control("reactor-os", "relay", Some(json!(0)), None);

    write_json_bridge_control(&path, &control).unwrap();

    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["command"], "relay");
    assert_eq!(saved["value"], 0);
    assert_json_bridge_tmp_files_clean(dir.path());
}

#[test]
fn repeated_control_json_writes_use_unique_temp_files_without_residue() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("control.json");
    let first = build_json_bridge_control("reactor-os", "motor", Some(json!(1)), None);
    let second = build_json_bridge_control("reactor-os", "motor", Some(json!(0)), None);

    write_json_bridge_control(&path, &first).unwrap();
    write_json_bridge_control(&path, &second).unwrap();

    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(saved["request_id"], second.request_id);
    assert_eq!(saved["command"], "motor");
    assert_eq!(saved["value"], 0);
    assert_json_bridge_tmp_files_clean(dir.path());
}

#[tokio::test]
async fn json_bridge_device_refuses_control_when_state_is_stale() {
    let dir = tempdir().unwrap();
    let state_path = dir.path().join("state.json");
    let control_path = dir.path().join("control.json");
    std::fs::write(
        &state_path,
        json!({
            "connected": true,
            "last_seen_ms": Utc::now().timestamp_millis() - 60_000,
            "last_frame_ok": true,
            "status": 0b0000_0111,
            "temperature_c": 64.25,
            "pressure_mpa": 0.50,
            "stirrer_rpm": 125.18,
            "shake_speed_cpm": 30.0,
            "flow_rate_l_min": 1.2,
            "product_concentration_percent": 50.0,
            "ph": 6.15
        })
        .to_string(),
    )
    .unwrap();
    let raw_config = format!(
        r#"
mode = "json_bridge"
[serial]
port = "/dev/ttyUSB0"
baudrate = 115200
parity = "N"
stopbits = 1
bytesize = 8
timeout_ms = 1000
[modbus]
slave_id = 1
[esp32]
frame_prefix = "RX"
command_prefix = "TX"
checksum = true
max_line_bytes = 256
[json_bridge]
state_path = "{}"
control_path = "{}"
max_state_age_ms = 6000
request_id_prefix = "reactor-os-test"
speed_steps_per_cycle = 200.0
speed_deadband_cpm = 1.0
temperature_deadband_c = 1.0
relay_temperature_control = false
[modbus.registers.temperature_c]
address = 0
scale = 0.1
offset = 0.0
min_valid = 0.0
max_valid = 500.0
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
"#,
        state_path.to_string_lossy().replace('\\', "\\\\"),
        control_path.to_string_lossy().replace('\\', "\\\\")
    );
    let config = toml::from_str(&raw_config).unwrap();
    let device = build_device(&config).unwrap();
    let command = SafeCommand {
        target_temperature_c: 65.0,
        heat_time_s: 300.0,
        hold_time_s: 600.0,
        cool_time_s: 180.0,
        target_stirrer_rpm: 125.18,
        target_shake_speed_cpm: 35.0,
        target_pressure_mpa: 0.5,
        reason: "test".to_string(),
    };

    let err = device
        .write_targets(&command)
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("refuses control"));
    assert!(err.contains("state stale"));
    assert!(!control_path.exists());
}

#[tokio::test]
async fn json_bridge_write_failure_does_not_cache_command_as_delivered() {
    let dir = tempdir().unwrap();
    let state_path = dir.path().join("state.json");
    let control_path = dir.path().join("control-as-directory");
    std::fs::write(
        &state_path,
        json!({
            "connected": true,
            "last_seen_ms": Utc::now().timestamp_millis(),
            "last_frame_ok": true,
            "status": 0b0000_0111,
            "temperature_c": 64.25,
            "pressure_mpa": 0.50,
            "stirrer_rpm": 125.18,
            "flow_rate_l_min": 1.2,
            "product_concentration_percent": 50.0,
            "ph": 6.15
        })
        .to_string(),
    )
    .unwrap();
    std::fs::create_dir_all(&control_path).unwrap();
    let config = json_bridge_config_for_paths(&state_path, &control_path);
    let device = build_device(&config).unwrap();
    let command = SafeCommand {
        target_temperature_c: 65.0,
        heat_time_s: 300.0,
        hold_time_s: 600.0,
        cool_time_s: 180.0,
        target_stirrer_rpm: 125.18,
        target_shake_speed_cpm: 35.0,
        target_pressure_mpa: 0.5,
        reason: "test".to_string(),
    };

    let err = device
        .write_targets(&command)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("failed to atomically replace"));
    std::fs::remove_dir_all(&control_path).unwrap();

    device.write_targets(&command).await.unwrap();

    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&control_path).unwrap()).unwrap();
    assert_eq!(saved["command"], "speed");
    assert_eq!(saved["value"], "up");
}

#[tokio::test]
async fn json_bridge_allows_stop_command_even_when_state_is_stale() {
    let dir = tempdir().unwrap();
    let state_path = dir.path().join("state.json");
    let control_path = dir.path().join("control.json");
    std::fs::write(
        &state_path,
        json!({
            "connected": true,
            "last_seen_ms": Utc::now().timestamp_millis() - 60_000,
            "last_frame_ok": true,
            "status": 0b0000_0111,
            "temperature_c": 64.25,
            "pressure_mpa": 0.50,
            "stirrer_rpm": 125.18,
            "shake_speed_cpm": 30.0,
            "flow_rate_l_min": 1.2,
            "product_concentration_percent": 50.0,
            "ph": 6.15
        })
        .to_string(),
    )
    .unwrap();
    let raw_config = format!(
        r#"
mode = "json_bridge"
[serial]
port = "/dev/ttyUSB0"
baudrate = 115200
parity = "N"
stopbits = 1
bytesize = 8
timeout_ms = 1000
[modbus]
slave_id = 1
[esp32]
frame_prefix = "RX"
command_prefix = "TX"
checksum = true
max_line_bytes = 256
[json_bridge]
state_path = "{}"
control_path = "{}"
max_state_age_ms = 6000
request_id_prefix = "reactor-os-test"
speed_steps_per_cycle = 200.0
speed_deadband_cpm = 1.0
temperature_deadband_c = 1.0
relay_temperature_control = false
[modbus.registers.temperature_c]
address = 0
scale = 0.1
offset = 0.0
min_valid = 0.0
max_valid = 500.0
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
"#,
        state_path.to_string_lossy().replace('\\', "\\\\"),
        control_path.to_string_lossy().replace('\\', "\\\\")
    );
    let config = toml::from_str(&raw_config).unwrap();
    let device = build_device(&config).unwrap();

    let outcome = device
        .write_component(
            &ComponentControlCommand {
                component_id: "shake_stepper".to_string(),
                action: "stop".to_string(),
                value: None,
            },
            &ControlTargets {
                temperature_c: 64.0,
                heat_time_s: 300.0,
                hold_time_s: 600.0,
                cool_time_s: 180.0,
                stirrer_rpm: 125.0,
                shake_speed_cpm: 30.0,
                target_pressure_mpa: 0.5,
            },
            &reactor_edge_daemon::config::SafetyConfig {
                control: reactor_edge_daemon::config::ControlConfig {
                    auto_enabled_default: false,
                    manual_lock_default: false,
                    control_interval_ms: 2000,
                    sensor_timeout_ms: 6000,
                    require_device_status_for_control: false,
                    write_retry_backoff_ms: 5000,
                    safety_guard_timeout_ms: 1000,
                    ai_stop_product_concentration_percent: 95.0,
                },
                temperature: reactor_edge_daemon::config::TemperatureSafety {
                    min_c: 20.0,
                    max_c: 160.0,
                    max_step_c: 2.0,
                    default_target_c: 60.0,
                },
                stirrer: reactor_edge_daemon::config::StirrerSafety {
                    min_rpm: 0.0,
                    max_rpm: 1200.0,
                    max_step_rpm: 50.0,
                    default_target_rpm: 300.0,
                },
                optimizer: reactor_edge_daemon::config::OptimizerBounds {
                    min_temperature_c: 35.0,
                    max_temperature_c: 140.0,
                    min_stirrer_rpm: 100.0,
                    max_stirrer_rpm: 1000.0,
                    min_heating_minutes: 15.0,
                    max_heating_minutes: 240.0,
                    min_stirring_minutes: 15.0,
                    max_stirring_minutes: 240.0,
                },
                forbidden_control_zones: Vec::new(),
            },
        )
        .await
        .unwrap()
        .unwrap();

    assert_eq!(outcome.component_id, "shake_stepper");
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&control_path).unwrap()).unwrap();
    assert_eq!(saved["command"], "motor");
    assert_eq!(saved["value"], 0);
    assert_eq!(saved["name"], "shake_stepper");
}

#[tokio::test]
async fn json_bridge_component_control_discovers_and_writes_single_component_commands() {
    let dir = tempdir().unwrap();
    let state_path = dir.path().join("state.json");
    let control_path = dir.path().join("control.json");
    std::fs::write(&state_path, valid_state_json()).unwrap();
    let raw_config = format!(
        r#"
mode = "json_bridge"
[serial]
port = "/dev/ttyUSB0"
baudrate = 115200
parity = "N"
stopbits = 1
bytesize = 8
timeout_ms = 1000
[modbus]
slave_id = 1
[esp32]
frame_prefix = "RX"
command_prefix = "TX"
checksum = true
max_line_bytes = 256
[json_bridge]
state_path = "{}"
control_path = "{}"
max_state_age_ms = 6000
request_id_prefix = "reactor-os-test"
speed_steps_per_cycle = 200.0
speed_deadband_cpm = 1.0
temperature_deadband_c = 1.0
relay_temperature_control = false
[modbus.registers.temperature_c]
address = 0
scale = 0.1
offset = 0.0
min_valid = 0.0
max_valid = 500.0
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
"#,
        state_path.to_string_lossy().replace('\\', "\\\\"),
        control_path.to_string_lossy().replace('\\', "\\\\")
    );
    let config = toml::from_str(&raw_config).unwrap();
    let device = build_device(&config).unwrap();
    assert!(device
        .control_capabilities()
        .iter()
        .any(|component| component.component_id == "shake_stepper"));
    assert!(device
        .control_capabilities()
        .iter()
        .any(|component| component.component_id == "stirrer_motor"));

    let outcome = device
        .write_component(
            &ComponentControlCommand {
                component_id: "shake_stepper".to_string(),
                action: "stop".to_string(),
                value: None,
            },
            &ControlTargets {
                temperature_c: 64.0,
                heat_time_s: 300.0,
                hold_time_s: 600.0,
                cool_time_s: 180.0,
                stirrer_rpm: 125.0,
                shake_speed_cpm: 30.0,
                target_pressure_mpa: 0.5,
            },
            &reactor_edge_daemon::config::SafetyConfig {
                control: reactor_edge_daemon::config::ControlConfig {
                    auto_enabled_default: false,
                    manual_lock_default: false,
                    control_interval_ms: 2000,
                    sensor_timeout_ms: 6000,
                    require_device_status_for_control: false,
                    write_retry_backoff_ms: 5000,
                    safety_guard_timeout_ms: 1000,
                    ai_stop_product_concentration_percent: 95.0,
                },
                temperature: reactor_edge_daemon::config::TemperatureSafety {
                    min_c: 20.0,
                    max_c: 160.0,
                    max_step_c: 2.0,
                    default_target_c: 60.0,
                },
                stirrer: reactor_edge_daemon::config::StirrerSafety {
                    min_rpm: 0.0,
                    max_rpm: 1200.0,
                    max_step_rpm: 50.0,
                    default_target_rpm: 300.0,
                },
                optimizer: reactor_edge_daemon::config::OptimizerBounds {
                    min_temperature_c: 35.0,
                    max_temperature_c: 140.0,
                    min_stirrer_rpm: 100.0,
                    max_stirrer_rpm: 1000.0,
                    min_heating_minutes: 15.0,
                    max_heating_minutes: 240.0,
                    min_stirring_minutes: 15.0,
                    max_stirring_minutes: 240.0,
                },
                forbidden_control_zones: Vec::new(),
            },
        )
        .await
        .unwrap()
        .unwrap();

    assert_eq!(outcome.component_id, "shake_stepper");
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&control_path).unwrap()).unwrap();
    assert_eq!(saved["command"], "motor");
    assert_eq!(saved["value"], 0);
    assert_eq!(saved["name"], "shake_stepper");
}

#[tokio::test]
async fn json_bridge_component_control_writes_stirrer_motor_rpm() {
    let dir = tempdir().unwrap();
    let state_path = dir.path().join("state.json");
    let control_path = dir.path().join("control.json");
    std::fs::write(&state_path, valid_state_json()).unwrap();
    let raw_config = format!(
        r#"
mode = "json_bridge"
[serial]
port = "/dev/ttyUSB0"
baudrate = 115200
parity = "N"
stopbits = 1
bytesize = 8
timeout_ms = 1000
[json_bridge]
state_path = "{}"
control_path = "{}"
max_state_age_ms = 10000
request_id_prefix = "reactor-os-test"
speed_steps_per_cycle = 200.0
speed_deadband_cpm = 1.0
temperature_deadband_c = 1.0
relay_temperature_control = false
[modbus]
slave_id = 1
[esp32]
frame_prefix = "RX"
command_prefix = "TX"
checksum = true
max_line_bytes = 256
[json_bridge.adc]
sensor = "product_concentration_percent"
scale = 0.0244200244
offset = 0.0
min_valid = 0.0
max_valid = 100.0
[modbus.registers.temperature_c]
address = 0
scale = 0.1
offset = 0.0
min_valid = 0.0
max_valid = 500.0
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
"#,
        state_path.to_string_lossy().replace('\\', "\\\\"),
        control_path.to_string_lossy().replace('\\', "\\\\")
    );
    let config = toml::from_str(&raw_config).unwrap();
    let device = build_device(&config).unwrap();

    let outcome = device
        .write_component(
            &ComponentControlCommand {
                component_id: "stirrer_motor".to_string(),
                action: "set_rpm".to_string(),
                value: Some(json!(480.25)),
            },
            &ControlTargets {
                temperature_c: 64.0,
                heat_time_s: 300.0,
                hold_time_s: 600.0,
                cool_time_s: 180.0,
                stirrer_rpm: 125.0,
                shake_speed_cpm: 30.0,
                target_pressure_mpa: 0.5,
            },
            &reactor_edge_daemon::config::SafetyConfig {
                control: reactor_edge_daemon::config::ControlConfig {
                    auto_enabled_default: false,
                    manual_lock_default: false,
                    control_interval_ms: 2000,
                    sensor_timeout_ms: 6000,
                    require_device_status_for_control: false,
                    write_retry_backoff_ms: 5000,
                    safety_guard_timeout_ms: 1000,
                    ai_stop_product_concentration_percent: 95.0,
                },
                temperature: reactor_edge_daemon::config::TemperatureSafety {
                    min_c: 20.0,
                    max_c: 160.0,
                    max_step_c: 2.0,
                    default_target_c: 60.0,
                },
                stirrer: reactor_edge_daemon::config::StirrerSafety {
                    min_rpm: 0.0,
                    max_rpm: 1200.0,
                    max_step_rpm: 50.0,
                    default_target_rpm: 300.0,
                },
                optimizer: reactor_edge_daemon::config::OptimizerBounds {
                    min_temperature_c: 35.0,
                    max_temperature_c: 140.0,
                    min_stirrer_rpm: 100.0,
                    max_stirrer_rpm: 1000.0,
                    min_heating_minutes: 15.0,
                    max_heating_minutes: 240.0,
                    min_stirring_minutes: 15.0,
                    max_stirring_minutes: 240.0,
                },
                forbidden_control_zones: Vec::new(),
            },
        )
        .await
        .unwrap()
        .unwrap();

    assert_eq!(outcome.component_id, "stirrer_motor");
    let saved: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&control_path).unwrap()).unwrap();
    assert_eq!(saved["command"], "stir_speed");
    assert_eq!(saved["value"], 480.25);
    assert_eq!(saved["name"], "stirrer_motor");
}

#[tokio::test]
async fn json_bridge_component_control_rejects_out_of_range_value_without_writing_control() {
    let dir = tempdir().unwrap();
    let state_path = dir.path().join("state.json");
    let control_path = dir.path().join("control.json");
    std::fs::write(&state_path, valid_state_json()).unwrap();
    let config = json_bridge_config_for_paths(&state_path, &control_path);
    let device = build_device(&config).unwrap();

    let err = device
        .write_component(
            &ComponentControlCommand {
                component_id: "stirrer_motor".to_string(),
                action: "set_rpm".to_string(),
                value: Some(json!(5000.0)),
            },
            &ControlTargets {
                temperature_c: 64.0,
                heat_time_s: 300.0,
                hold_time_s: 600.0,
                cool_time_s: 180.0,
                stirrer_rpm: 125.0,
                shake_speed_cpm: 30.0,
                target_pressure_mpa: 0.5,
            },
            &component_safety(),
        )
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("component control value must be between 0 and 1200"));
    assert!(!control_path.exists());
}
