use std::{
    collections::HashMap,
    fs::{self, File},
    io::{BufRead, BufReader, Write},
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serialport::{DataBits, Parity, SerialPort, StopBits};

use crate::{
    config::{
        DeviceConfig, DeviceMode, JsonBridgeAdcSensor, JsonBridgeConfig, ReadRegister,
        SafetyConfig, WriteRegister,
    },
    control::{clamp_operator_targets, SafeCommand},
    state::{fit_tilt_angle_deg, ControlTargets, DeviceStatusSnapshot, SensorSnapshot},
};

#[async_trait::async_trait]
pub trait ReactorDevice: Send + Sync {
    async fn read_sample(&self) -> Result<SensorSnapshot>;
    async fn read_sample_and_status(
        &self,
    ) -> Result<(SensorSnapshot, Option<DeviceStatusSnapshot>)> {
        let sample = self.read_sample().await?;
        let status = self.read_device_status().await?;
        Ok((sample, status))
    }
    async fn write_targets(&self, command: &SafeCommand) -> Result<()>;
    async fn read_device_status(&self) -> Result<Option<DeviceStatusSnapshot>> {
        Ok(None)
    }
    fn control_capabilities(&self) -> Vec<DeviceComponentCapability> {
        Vec::new()
    }
    async fn write_component(
        &self,
        command: &ComponentControlCommand,
        targets: &ControlTargets,
        safety: &SafetyConfig,
    ) -> Result<Option<ComponentControlOutcome>> {
        let _ = (command, targets, safety);
        Err(anyhow!(
            "component control is not supported by this device mode"
        ))
    }
}

pub type SharedDevice = Arc<dyn ReactorDevice>;

#[derive(Debug, Clone, Serialize)]
pub struct DeviceComponentCapability {
    pub component_id: String,
    pub component_type: String,
    pub label: String,
    pub controllable: bool,
    pub actions: Vec<ComponentActionCapability>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComponentActionCapability {
    pub action: String,
    pub label: String,
    pub value_type: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub unit: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ComponentControlCommand {
    pub component_id: String,
    pub action: String,
    pub value: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComponentControlOutcome {
    pub component_id: String,
    pub action: String,
    pub command: Option<JsonBridgeControl>,
    pub targets: Option<SafeCommand>,
    pub message: String,
}

pub fn build_device(config: &DeviceConfig) -> Result<SharedDevice> {
    match config.mode {
        DeviceMode::Pipeline => Ok(Arc::new(PipelineDevice)),
        DeviceMode::Modbus => Ok(Arc::new(ModbusRtuDevice::new(config.clone())?)),
        DeviceMode::Esp32Serial => Ok(Arc::new(Esp32SerialDevice::new(config.clone())?)),
        DeviceMode::JsonBridge => Ok(Arc::new(JsonBridgeDevice::new(config.json_bridge.clone()))),
    }
}

#[derive(Debug)]
pub struct PipelineDevice;

#[async_trait::async_trait]
impl ReactorDevice for PipelineDevice {
    async fn read_sample(&self) -> Result<SensorSnapshot> {
        Err(anyhow!("waiting for external data pipeline sample"))
    }

    async fn write_targets(&self, _command: &SafeCommand) -> Result<()> {
        Ok(())
    }
}

struct ModbusRtuDevice {
    config: DeviceConfig,
    port: Arc<StdMutex<Box<dyn SerialPort>>>,
}

struct Esp32SerialDevice {
    config: DeviceConfig,
    port: Arc<StdMutex<Box<dyn SerialPort>>>,
}

struct JsonBridgeDevice {
    config: JsonBridgeConfig,
    last_commanded_shake_speed_cpm: Arc<StdMutex<Option<f64>>>,
    last_temperature_command: Arc<StdMutex<Option<f64>>>,
    last_stirrer_command: Arc<StdMutex<Option<f64>>>,
}

impl Esp32SerialDevice {
    fn new(config: DeviceConfig) -> Result<Self> {
        let port = open_serial_port(&config)?;
        Ok(Self {
            config,
            port: Arc::new(StdMutex::new(port)),
        })
    }
}

impl ModbusRtuDevice {
    fn new(config: DeviceConfig) -> Result<Self> {
        let port = open_serial_port(&config)?;

        Ok(Self {
            config,
            port: Arc::new(StdMutex::new(port)),
        })
    }
}

impl JsonBridgeDevice {
    fn new(config: JsonBridgeConfig) -> Self {
        Self {
            config,
            last_commanded_shake_speed_cpm: Arc::new(StdMutex::new(None)),
            last_temperature_command: Arc::new(StdMutex::new(None)),
            last_stirrer_command: Arc::new(StdMutex::new(None)),
        }
    }
}

#[async_trait::async_trait]
impl ReactorDevice for ModbusRtuDevice {
    async fn read_sample(&self) -> Result<SensorSnapshot> {
        let config = self.config.clone();
        let port = Arc::clone(&self.port);
        tokio::task::spawn_blocking(move || {
            let mut port = port
                .lock()
                .map_err(|_| anyhow!("serial port lock poisoned"))?;
            let temperature_raw = read_holding_register(
                &mut **port,
                config.modbus.slave_id,
                config.modbus.registers.temperature_c.address,
            )?;
            let stirrer_raw = read_holding_register(
                &mut **port,
                config.modbus.slave_id,
                config.modbus.registers.stirrer_rpm.address,
            )?;
            let temperature_c =
                decode_read_register(temperature_raw, &config.modbus.registers.temperature_c)?;
            let stirrer_rpm =
                decode_read_register(stirrer_raw, &config.modbus.registers.stirrer_rpm)?;
            Ok(SensorSnapshot {
                temperature_c,
                pressure_mpa: 0.0,
                stirrer_rpm,
                shake_speed_cpm: 0.0,
                tilt_state: 0,
                tilt_angle_deg: 0.0,
                flow_rate_l_min: 0.0,
                product_concentration_percent: 0.0,
                ph: 7.0,
                captured_at: Utc::now(),
            })
        })
        .await?
    }

    async fn write_targets(&self, command: &SafeCommand) -> Result<()> {
        let config = self.config.clone();
        let port = Arc::clone(&self.port);
        let command = command.clone();
        tokio::task::spawn_blocking(move || {
            let mut port = port
                .lock()
                .map_err(|_| anyhow!("serial port lock poisoned"))?;
            write_single_register(
                &mut **port,
                config.modbus.slave_id,
                config.modbus.registers.target_temperature_c.address,
                encode_write_register(
                    command.target_temperature_c,
                    &config.modbus.registers.target_temperature_c,
                )?,
            )?;
            write_single_register(
                &mut **port,
                config.modbus.slave_id,
                config.modbus.registers.target_stirrer_rpm.address,
                encode_write_register(
                    command.target_stirrer_rpm,
                    &config.modbus.registers.target_stirrer_rpm,
                )?,
            )?;
            Ok(())
        })
        .await?
    }

    fn control_capabilities(&self) -> Vec<DeviceComponentCapability> {
        vec![
            component_capability(
                "temperature_controller",
                "temperature_controller",
                "Temperature Controller",
                vec![numeric_action(
                    "set_target_temperature",
                    "Set Target",
                    0.0,
                    500.0,
                    "C",
                )],
            ),
            component_capability(
                "stirrer_motor",
                "motor",
                "Stirrer Motor",
                vec![numeric_action("set_rpm", "Set RPM", 0.0, 2000.0, "RPM")],
            ),
        ]
    }

    async fn write_component(
        &self,
        command: &ComponentControlCommand,
        targets: &ControlTargets,
        safety: &SafetyConfig,
    ) -> Result<Option<ComponentControlOutcome>> {
        let next_targets = targets_for_component(command, targets, safety)?;
        let safe = safe_command_from_targets(&next_targets, "manual component control");
        self.write_targets(&safe).await?;
        Ok(Some(ComponentControlOutcome {
            component_id: command.component_id.clone(),
            action: command.action.clone(),
            command: None,
            targets: Some(safe),
            message: "component target written through Modbus RTU".to_string(),
        }))
    }
}

#[async_trait::async_trait]
impl ReactorDevice for Esp32SerialDevice {
    async fn read_sample(&self) -> Result<SensorSnapshot> {
        let config = self.config.clone();
        let port = Arc::clone(&self.port);
        tokio::task::spawn_blocking(move || {
            let port = port
                .lock()
                .map_err(|_| anyhow!("serial port lock poisoned"))?;
            let reader_port = port.try_clone()?;
            let mut reader = BufReader::new(reader_port);
            let mut line = String::new();
            let bytes = reader.read_line(&mut line)?;
            if bytes == 0 {
                return Err(anyhow!("esp32 serial returned no data"));
            }
            if bytes > config.esp32.max_line_bytes {
                return Err(anyhow!(
                    "esp32 frame too long: {bytes} bytes exceeds {}",
                    config.esp32.max_line_bytes
                ));
            }
            parse_esp32_frame(&line, &config.esp32.frame_prefix, config.esp32.checksum)
        })
        .await?
    }

    async fn write_targets(&self, command: &SafeCommand) -> Result<()> {
        let config = self.config.clone();
        let port = Arc::clone(&self.port);
        let command = command.clone();
        tokio::task::spawn_blocking(move || {
            let mut port = port
                .lock()
                .map_err(|_| anyhow!("serial port lock poisoned"))?;
            let frame = build_esp32_command(
                &config.esp32.command_prefix,
                &command,
                config.esp32.checksum,
            );
            port.write_all(frame.as_bytes())?;
            port.flush()?;
            Ok(())
        })
        .await?
    }

    fn control_capabilities(&self) -> Vec<DeviceComponentCapability> {
        vec![
            component_capability(
                "temperature_controller",
                "temperature_controller",
                "Temperature Controller",
                vec![numeric_action(
                    "set_target_temperature",
                    "Set Target",
                    0.0,
                    500.0,
                    "C",
                )],
            ),
            component_capability(
                "stirrer_motor",
                "motor",
                "Stirrer Motor",
                vec![numeric_action("set_rpm", "Set RPM", 0.0, 2000.0, "RPM")],
            ),
            component_capability(
                "shake_stepper",
                "stepper_motor",
                "Shake Vessel Stepper",
                vec![
                    enum_action("start", "Start"),
                    enum_action("stop", "Stop"),
                    numeric_action("set_speed", "Set Speed", 0.0, 60.0, "CPM"),
                ],
            ),
        ]
    }

    async fn write_component(
        &self,
        command: &ComponentControlCommand,
        targets: &ControlTargets,
        safety: &SafetyConfig,
    ) -> Result<Option<ComponentControlOutcome>> {
        let next_targets = targets_for_component(command, targets, safety)?;
        let safe = safe_command_from_targets(&next_targets, "manual component control");
        self.write_targets(&safe).await?;
        Ok(Some(ComponentControlOutcome {
            component_id: command.component_id.clone(),
            action: command.action.clone(),
            command: None,
            targets: Some(safe),
            message: "component target written through ESP32 serial".to_string(),
        }))
    }
}

#[async_trait::async_trait]
impl ReactorDevice for JsonBridgeDevice {
    async fn read_sample(&self) -> Result<SensorSnapshot> {
        let config = self.config.clone();
        tokio::task::spawn_blocking(move || {
            let state = read_json_bridge_state(&config.state_path)?;
            json_bridge_sample_from_state(&config, &state)
        })
        .await?
    }

    async fn read_sample_and_status(
        &self,
    ) -> Result<(SensorSnapshot, Option<DeviceStatusSnapshot>)> {
        let config = self.config.clone();
        tokio::task::spawn_blocking(move || {
            let state = read_json_bridge_state(&config.state_path)?;
            let sample = json_bridge_sample_from_state(&config, &state)?;
            let status = json_bridge_status_from_state(&state)?;
            Ok((sample, Some(status)))
        })
        .await?
    }

    async fn write_targets(&self, command: &SafeCommand) -> Result<()> {
        let config = self.config.clone();
        let command = command.clone();
        let last_commanded_shake_speed_cpm = Arc::clone(&self.last_commanded_shake_speed_cpm);
        let last_temperature_command = Arc::clone(&self.last_temperature_command);
        let last_stirrer_command = Arc::clone(&self.last_stirrer_command);
        tokio::task::spawn_blocking(move || {
            let current = read_json_bridge_state(&config.state_path)?;
            let control = next_json_bridge_control(
                &config,
                &current,
                &command,
                &last_commanded_shake_speed_cpm,
                &last_temperature_command,
                &last_stirrer_command,
            )?;
            if let Some(control) = control {
                write_json_bridge_control(&config.control_path, &control)?;
            }
            Ok(())
        })
        .await?
    }

    async fn read_device_status(&self) -> Result<Option<DeviceStatusSnapshot>> {
        let config = self.config.clone();
        tokio::task::spawn_blocking(move || {
            let state = read_json_bridge_state(&config.state_path)?;
            Ok(Some(json_bridge_status_from_state(&state)?))
        })
        .await?
    }

    fn control_capabilities(&self) -> Vec<DeviceComponentCapability> {
        let mut capabilities = vec![
            component_capability(
                "shake_stepper",
                "stepper_motor",
                "Shake Vessel Stepper",
                vec![
                    enum_action("start", "Start"),
                    enum_action("stop", "Stop"),
                    enum_action("speed_up", "Speed Up"),
                    enum_action("speed_down", "Speed Down"),
                    numeric_action("set_speed", "Set Speed", 0.0, 60.0, "CPM"),
                ],
            ),
            component_capability(
                "heater_relay",
                "relay",
                "Heater Relay",
                vec![enum_action("on", "On"), enum_action("off", "Off")],
            ),
            component_capability(
                "stirrer_motor",
                "motor",
                "Stirrer Motor",
                vec![numeric_action("set_rpm", "Set RPM", 0.0, 2000.0, "RPM")],
            ),
        ];
        if self.config.relay_temperature_control {
            capabilities.push(component_capability(
                "temperature_controller",
                "temperature_controller",
                "Temperature Controller",
                vec![numeric_action(
                    "set_target_temperature",
                    "Set Target",
                    0.0,
                    500.0,
                    "C",
                )],
            ));
        }
        capabilities
    }

    async fn write_component(
        &self,
        command: &ComponentControlCommand,
        targets: &ControlTargets,
        safety: &SafetyConfig,
    ) -> Result<Option<ComponentControlOutcome>> {
        let config = self.config.clone();
        let request = command.clone();
        let targets = targets.clone();
        let safety = safety.clone();
        let last_commanded_shake_speed_cpm = Arc::clone(&self.last_commanded_shake_speed_cpm);
        let last_temperature_command = Arc::clone(&self.last_temperature_command);
        let last_stirrer_command = Arc::clone(&self.last_stirrer_command);
        tokio::task::spawn_blocking(move || {
            let state = read_json_bridge_state(&config.state_path)?;
            validate_json_bridge_state(&config, &state)?;
            let control = match (request.component_id.as_str(), request.action.as_str()) {
                ("shake_stepper", "start") => build_json_bridge_control(
                    &config.request_id_prefix,
                    "motor",
                    Some(serde_json::json!(1)),
                    Some("shake_stepper"),
                ),
                ("shake_stepper", "stop") => build_json_bridge_control(
                    &config.request_id_prefix,
                    "motor",
                    Some(serde_json::json!(0)),
                    Some("shake_stepper"),
                ),
                ("shake_stepper", "speed_up") => build_json_bridge_control(
                    &config.request_id_prefix,
                    "speed",
                    Some(serde_json::json!("up")),
                    Some("shake_stepper"),
                ),
                ("shake_stepper", "speed_down") => build_json_bridge_control(
                    &config.request_id_prefix,
                    "speed",
                    Some(serde_json::json!("down")),
                    Some("shake_stepper"),
                ),
                ("heater_relay", "on") => build_json_bridge_control(
                    &config.request_id_prefix,
                    "relay",
                    Some(serde_json::json!(1)),
                    Some("heater_relay"),
                ),
                ("heater_relay", "off") => build_json_bridge_control(
                    &config.request_id_prefix,
                    "relay",
                    Some(serde_json::json!(0)),
                    Some("heater_relay"),
                ),
                ("stirrer_motor", "set_rpm") => {
                    let next_targets = targets_for_component(&request, &targets, &safety)?;
                    let safe = safe_command_from_targets(&next_targets, "manual component control");
                    let rpm = safe.target_stirrer_rpm;
                    let current_rpm = state
                        .stirrer_rpm
                        .or_else(|| last_stirrer_command.lock().ok().and_then(|cached| *cached));
                    if current_rpm
                        .map(|current| (rpm - current).abs() <= 0.01)
                        .unwrap_or(false)
                    {
                        return Ok(ComponentControlOutcome {
                            component_id: request.component_id,
                            action: request.action,
                            command: None,
                            targets: Some(safe),
                            message: "stirrer target already matches json bridge state".to_string(),
                        });
                    }
                    let control = build_json_bridge_control(
                        &config.request_id_prefix,
                        "stir_speed",
                        Some(serde_json::json!(rpm)),
                        Some("stirrer_motor"),
                    );
                    *last_stirrer_command
                        .lock()
                        .map_err(|_| anyhow!("json bridge stirrer cache lock poisoned"))? =
                        Some(rpm);
                    write_json_bridge_control(&config.control_path, &control)?;
                    return Ok(ComponentControlOutcome {
                        component_id: request.component_id,
                        action: request.action,
                        command: Some(control),
                        targets: Some(safe),
                        message: "stirrer RPM written to json bridge control.json".to_string(),
                    });
                }
                ("shake_stepper", "set_speed")
                | ("temperature_controller", "set_target_temperature") => {
                    let next_targets = targets_for_component(&request, &targets, &safety)?;
                    let safe = safe_command_from_targets(&next_targets, "manual component control");
                    let Some(control) = next_json_bridge_control(
                        &config,
                        &state,
                        &safe,
                        &last_commanded_shake_speed_cpm,
                        &last_temperature_command,
                        &last_stirrer_command,
                    )?
                    else {
                        return Ok(ComponentControlOutcome {
                            component_id: request.component_id,
                            action: request.action,
                            command: None,
                            targets: Some(safe),
                            message: "component target already inside json bridge deadband"
                                .to_string(),
                        });
                    };
                    write_json_bridge_control(&config.control_path, &control)?;
                    return Ok(ComponentControlOutcome {
                        component_id: request.component_id,
                        action: request.action,
                        command: Some(control),
                        targets: Some(safe),
                        message: "component target translated to json bridge command".to_string(),
                    });
                }
                _ => {
                    return Err(anyhow!(
                        "unsupported component control {}:{} for json bridge",
                        request.component_id,
                        request.action
                    ));
                }
            };
            write_json_bridge_control(&config.control_path, &control)?;
            Ok(ComponentControlOutcome {
                component_id: request.component_id,
                action: request.action,
                command: Some(control),
                targets: None,
                message: "component command written to json bridge control.json".to_string(),
            })
        })
        .await?
        .map(Some)
    }
}

fn component_capability(
    component_id: &str,
    component_type: &str,
    label: &str,
    actions: Vec<ComponentActionCapability>,
) -> DeviceComponentCapability {
    DeviceComponentCapability {
        component_id: component_id.to_string(),
        component_type: component_type.to_string(),
        label: label.to_string(),
        controllable: !actions.is_empty(),
        actions,
    }
}

fn enum_action(action: &str, label: &str) -> ComponentActionCapability {
    ComponentActionCapability {
        action: action.to_string(),
        label: label.to_string(),
        value_type: "none".to_string(),
        min: None,
        max: None,
        unit: None,
    }
}

fn numeric_action(
    action: &str,
    label: &str,
    min: f64,
    max: f64,
    unit: &str,
) -> ComponentActionCapability {
    ComponentActionCapability {
        action: action.to_string(),
        label: label.to_string(),
        value_type: "number".to_string(),
        min: Some(min),
        max: Some(max),
        unit: Some(unit.to_string()),
    }
}

fn targets_for_component(
    command: &ComponentControlCommand,
    current: &ControlTargets,
    safety: &SafetyConfig,
) -> Result<ControlTargets> {
    let mut next = current.clone();
    match (command.component_id.as_str(), command.action.as_str()) {
        ("temperature_controller", "set_target_temperature") => {
            next.temperature_c = component_number(command, "value")?;
        }
        ("stirrer_motor", "set_rpm") => {
            next.stirrer_rpm = component_number(command, "value")?;
        }
        ("shake_stepper", "set_speed") => {
            next.shake_speed_cpm = component_number(command, "value")?;
        }
        ("shake_stepper", "start") => {
            if next.shake_speed_cpm <= 0.01 {
                next.shake_speed_cpm = 30.0;
            }
        }
        ("shake_stepper", "stop") => {
            next.shake_speed_cpm = 0.0;
        }
        _ => {
            return Err(anyhow!(
                "unsupported component control {}:{}",
                command.component_id,
                command.action
            ));
        }
    }
    Ok(clamp_operator_targets(safety, next))
}

fn component_number(command: &ComponentControlCommand, field: &str) -> Result<f64> {
    let value = command.value.as_ref().ok_or_else(|| {
        anyhow!(
            "component control action {} requires {field}",
            command.action
        )
    })?;
    let number = value
        .as_f64()
        .ok_or_else(|| anyhow!("component control {field} must be a number"))?;
    if !number.is_finite() {
        return Err(anyhow!("component control {field} must be finite"));
    }
    Ok(number)
}

fn safe_command_from_targets(targets: &ControlTargets, reason: &str) -> SafeCommand {
    SafeCommand {
        target_temperature_c: targets.temperature_c,
        heat_time_s: targets.heat_time_s,
        hold_time_s: targets.hold_time_s,
        cool_time_s: targets.cool_time_s,
        target_stirrer_rpm: targets.stirrer_rpm,
        target_shake_speed_cpm: targets.shake_speed_cpm,
        target_pressure_mpa: targets.target_pressure_mpa,
        reason: reason.to_string(),
    }
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

#[derive(Debug, Clone, Deserialize)]
pub struct JsonBridgeState {
    pub connected: bool,
    pub last_seen_ms: i64,
    pub last_frame_hex: Option<String>,
    pub last_frame_ok: bool,
    pub adc: Option<u16>,
    pub status: Option<u8>,
    pub relay: Option<u8>,
    pub motor: Option<u8>,
    pub tilt: Option<u8>,
    pub speed_delay_us: Option<u64>,
    pub last_command: Option<String>,
    pub last_command_request_id: Option<String>,
    pub last_command_sent_ms: Option<i64>,
    pub last_command_ok: Option<bool>,
    pub last_command_error: Option<String>,
    pub port: Option<String>,
    pub baudrate: Option<u32>,
    pub bridge_started_ms: Option<i64>,
    pub temperature_c: Option<f64>,
    pub pressure_mpa: Option<f64>,
    pub stirrer_rpm: Option<f64>,
    pub shake_speed_cpm: Option<f64>,
    pub flow_rate_l_min: Option<f64>,
    pub product_concentration_percent: Option<f64>,
    pub ph: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JsonBridgeControl {
    pub request_id: String,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

pub fn parse_json_bridge_state(raw: &str) -> Result<JsonBridgeState> {
    serde_json::from_str(raw.trim_start_matches('\u{feff}'))
        .context("failed to parse json bridge state.json")
}

pub fn json_bridge_sample_from_state(
    config: &JsonBridgeConfig,
    state: &JsonBridgeState,
) -> Result<SensorSnapshot> {
    validate_json_bridge_state(config, state)?;
    let captured_at = timestamp_ms_to_utc(state.last_seen_ms)
        .ok_or_else(|| anyhow!("json bridge state last_seen_ms is out of range"))?;
    let tilt_state = json_bridge_tilt_state(state)?;
    let shake_speed_cpm = bridge_sensor_value(
        config,
        state,
        JsonBridgeAdcSensor::ShakeSpeedCpm,
        "shake_speed_cpm",
    )?;
    Ok(SensorSnapshot {
        temperature_c: bridge_sensor_value(
            config,
            state,
            JsonBridgeAdcSensor::TemperatureC,
            "temperature_c",
        )?,
        pressure_mpa: bridge_sensor_value(
            config,
            state,
            JsonBridgeAdcSensor::PressureMpa,
            "pressure_mpa",
        )?,
        stirrer_rpm: bridge_sensor_value(
            config,
            state,
            JsonBridgeAdcSensor::StirrerRpm,
            "stirrer_rpm",
        )?,
        shake_speed_cpm,
        tilt_state,
        tilt_angle_deg: fit_tilt_angle_deg(tilt_state, shake_speed_cpm, captured_at),
        flow_rate_l_min: bridge_sensor_value(
            config,
            state,
            JsonBridgeAdcSensor::FlowRateLMin,
            "flow_rate_l_min",
        )?,
        product_concentration_percent: bridge_sensor_value(
            config,
            state,
            JsonBridgeAdcSensor::ProductConcentrationPercent,
            "product_concentration_percent",
        )?,
        ph: bridge_sensor_value(config, state, JsonBridgeAdcSensor::Ph, "ph")?,
        captured_at,
    })
}

pub fn json_bridge_status_from_state(state: &JsonBridgeState) -> Result<DeviceStatusSnapshot> {
    Ok(DeviceStatusSnapshot {
        connected: state.connected,
        last_seen_at: timestamp_ms_to_utc(state.last_seen_ms),
        last_frame_ok: state.last_frame_ok,
        relay: bit_or_field(state.relay, state.status, 0),
        motor: bit_or_field(state.motor, state.status, 1),
        tilt: bit_or_field(state.tilt, state.status, 2),
        speed_delay_us: state.speed_delay_us,
        port: state.port.clone(),
        baudrate: state.baudrate,
        last_command_request_id: state.last_command_request_id.clone(),
        last_command_ok: state.last_command_ok,
        last_command_error: state.last_command_error.clone(),
        updated_at: Utc::now(),
    })
}

pub fn build_json_bridge_control(
    prefix: &str,
    command: &str,
    value: Option<serde_json::Value>,
    name: Option<&str>,
) -> JsonBridgeControl {
    static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);
    let sequence = REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    JsonBridgeControl {
        request_id: format!("{prefix}-{}-{sequence}", Utc::now().timestamp_millis()),
        command: command.to_string(),
        value,
        name: name.map(ToString::to_string),
    }
}

pub fn write_json_bridge_control(path: &Path, control: &JsonBridgeControl) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    if let Some(parent) = parent {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create json bridge dir {}", parent.display()))?;
    }
    let tmp_path = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(control)?;
    {
        let mut file = File::create(&tmp_path)
            .with_context(|| format!("failed to create {}", tmp_path.display()))?;
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }
    fs::rename(&tmp_path, path).with_context(|| {
        format!(
            "failed to atomically replace {} with {}",
            path.display(),
            tmp_path.display()
        )
    })?;
    Ok(())
}

fn open_serial_port(config: &DeviceConfig) -> Result<Box<dyn SerialPort>> {
    let parity = match config.serial.parity.as_str() {
        "N" | "n" => Parity::None,
        "E" | "e" => Parity::Even,
        "O" | "o" => Parity::Odd,
        other => return Err(anyhow!("unsupported serial parity {other}")),
    };
    let stop_bits = match config.serial.stopbits {
        1 => StopBits::One,
        2 => StopBits::Two,
        other => return Err(anyhow!("unsupported serial stopbits {other}")),
    };
    let data_bits = match config.serial.bytesize {
        5 => DataBits::Five,
        6 => DataBits::Six,
        7 => DataBits::Seven,
        8 => DataBits::Eight,
        other => return Err(anyhow!("unsupported serial bytesize {other}")),
    };

    serialport::new(&config.serial.port, config.serial.baudrate)
        .timeout(Duration::from_millis(config.serial.timeout_ms))
        .parity(parity)
        .stop_bits(stop_bits)
        .data_bits(data_bits)
        .open()
        .map_err(|err| anyhow!("failed to open serial port {}: {err}", config.serial.port))
}

fn read_json_bridge_state(path: &Path) -> Result<JsonBridgeState> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read json bridge state file {}", path.display()))?;
    parse_json_bridge_state(&raw)
}

fn validate_json_bridge_state(config: &JsonBridgeConfig, state: &JsonBridgeState) -> Result<()> {
    if !state.connected {
        return Err(anyhow!("json bridge reports downstream disconnected"));
    }
    if !state.last_frame_ok {
        return Err(anyhow!("json bridge last upstream frame failed XOR check"));
    }
    let last_seen = timestamp_ms_to_utc(state.last_seen_ms)
        .ok_or_else(|| anyhow!("json bridge state last_seen_ms is out of range"))?;
    let age = Utc::now()
        .signed_duration_since(last_seen)
        .num_milliseconds();
    if age > config.max_state_age_ms {
        return Err(anyhow!(
            "json bridge state stale; last_seen_ms is {age} ms old, max {} ms",
            config.max_state_age_ms
        ));
    }
    Ok(())
}

fn next_json_bridge_control(
    config: &JsonBridgeConfig,
    state: &JsonBridgeState,
    command: &SafeCommand,
    last_commanded_shake_speed_cpm: &StdMutex<Option<f64>>,
    last_temperature_command: &StdMutex<Option<f64>>,
    last_stirrer_command: &StdMutex<Option<f64>>,
) -> Result<Option<JsonBridgeControl>> {
    if let Err(err) = validate_json_bridge_state(config, state) {
        return Err(anyhow!(
            "json bridge refuses control because state is not valid: {err}"
        ));
    }

    let motor = bit_or_field(state.motor, state.status, 1).unwrap_or(0);
    if command.target_shake_speed_cpm <= 0.01 && motor != 0 {
        *last_commanded_shake_speed_cpm
            .lock()
            .map_err(|_| anyhow!("json bridge speed cache lock poisoned"))? = Some(0.0);
        return Ok(Some(build_json_bridge_control(
            &config.request_id_prefix,
            "motor",
            Some(serde_json::json!(0)),
            None,
        )));
    }
    if command.target_shake_speed_cpm > 0.01 && motor == 0 {
        *last_commanded_shake_speed_cpm
            .lock()
            .map_err(|_| anyhow!("json bridge speed cache lock poisoned"))? =
            Some(command.target_shake_speed_cpm);
        return Ok(Some(build_json_bridge_control(
            &config.request_id_prefix,
            "motor",
            Some(serde_json::json!(1)),
            None,
        )));
    }

    let cached_speed = *last_commanded_shake_speed_cpm
        .lock()
        .map_err(|_| anyhow!("json bridge speed cache lock poisoned"))?;
    let current_speed = state
        .shake_speed_cpm
        .or_else(|| speed_delay_us_to_cpm(state.speed_delay_us, config.speed_steps_per_cycle))
        .or(cached_speed)
        .unwrap_or(0.0);
    let speed_delta = command.target_shake_speed_cpm - current_speed;
    if speed_delta.abs() > config.speed_deadband_cpm {
        *last_commanded_shake_speed_cpm
            .lock()
            .map_err(|_| anyhow!("json bridge speed cache lock poisoned"))? =
            Some(command.target_shake_speed_cpm);
        return Ok(Some(build_json_bridge_control(
            &config.request_id_prefix,
            "speed",
            Some(serde_json::json!(if speed_delta > 0.0 {
                "up"
            } else {
                "down"
            })),
            None,
        )));
    }

    if config.relay_temperature_control {
        let current_temperature = state
            .temperature_c
            .or_else(|| {
                if config.adc.sensor == Some(JsonBridgeAdcSensor::TemperatureC) {
                    state.adc.map(|adc| adc as f64 * config.adc.scale + config.adc.offset)
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                anyhow!(
                    "json bridge relay temperature control requires temperature_c or adc temperature mapping"
                )
            })?;
        let relay = bit_or_field(state.relay, state.status, 0).unwrap_or(0);
        let desired_relay = if command.target_temperature_c
            > current_temperature + config.temperature_deadband_c
        {
            1
        } else if command.target_temperature_c < current_temperature - config.temperature_deadband_c
        {
            0
        } else {
            relay
        };
        if desired_relay != relay {
            *last_temperature_command
                .lock()
                .map_err(|_| anyhow!("json bridge temperature cache lock poisoned"))? =
                Some(command.target_temperature_c);
            return Ok(Some(build_json_bridge_control(
                &config.request_id_prefix,
                "relay",
                Some(serde_json::json!(desired_relay)),
                None,
            )));
        }
    } else {
        *last_temperature_command
            .lock()
            .map_err(|_| anyhow!("json bridge temperature cache lock poisoned"))? =
            Some(command.target_temperature_c);
    }

    let cached_stirrer = *last_stirrer_command
        .lock()
        .map_err(|_| anyhow!("json bridge stirrer cache lock poisoned"))?;
    let current_stirrer = state.stirrer_rpm.or(cached_stirrer);
    if current_stirrer
        .map(|current| (command.target_stirrer_rpm - current).abs() > 0.01)
        .unwrap_or(true)
    {
        *last_stirrer_command
            .lock()
            .map_err(|_| anyhow!("json bridge stirrer cache lock poisoned"))? =
            Some(command.target_stirrer_rpm);
        return Ok(Some(build_json_bridge_control(
            &config.request_id_prefix,
            "stir_speed",
            Some(serde_json::json!(command.target_stirrer_rpm)),
            Some("stirrer_motor"),
        )));
    }

    Ok(None)
}

fn bridge_sensor_value(
    config: &JsonBridgeConfig,
    state: &JsonBridgeState,
    sensor: JsonBridgeAdcSensor,
    field_name: &str,
) -> Result<f64> {
    let direct = match sensor {
        JsonBridgeAdcSensor::TemperatureC => state.temperature_c,
        JsonBridgeAdcSensor::PressureMpa => state.pressure_mpa,
        JsonBridgeAdcSensor::StirrerRpm => state.stirrer_rpm,
        JsonBridgeAdcSensor::ShakeSpeedCpm => state
            .shake_speed_cpm
            .or_else(|| speed_delay_us_to_cpm(state.speed_delay_us, config.speed_steps_per_cycle)),
        JsonBridgeAdcSensor::FlowRateLMin => state.flow_rate_l_min,
        JsonBridgeAdcSensor::ProductConcentrationPercent => state.product_concentration_percent,
        JsonBridgeAdcSensor::Ph => state.ph,
    };
    let value = match direct {
        Some(value) => value,
        None if config.adc.sensor == Some(sensor) => {
            let adc = state
                .adc
                .ok_or_else(|| anyhow!("json bridge state missing adc for {field_name}"))?
                as f64;
            let mapped = adc * config.adc.scale + config.adc.offset;
            if !(config.adc.min_valid..=config.adc.max_valid).contains(&mapped) {
                return Err(anyhow!(
                    "json bridge adc mapped {field_name} value {mapped} outside valid range {}..{}",
                    config.adc.min_valid,
                    config.adc.max_valid
                ));
            }
            mapped
        }
        None => {
            return Err(anyhow!(
            "json bridge state missing required sensor field {field_name}; no fake value generated"
        ))
        }
    };
    if !value.is_finite() {
        return Err(anyhow!("json bridge field {field_name} is non-finite"));
    }
    Ok(round2(value))
}

fn json_bridge_tilt_state(state: &JsonBridgeState) -> Result<u8> {
    let value = bit_or_field(state.tilt, state.status, 2)
        .ok_or_else(|| anyhow!("json bridge state missing required tilt bit"))?;
    if value > 1 {
        return Err(anyhow!(
            "json bridge tilt state must be 0 or 1, got {value}"
        ));
    }
    Ok(value)
}

fn bit_or_field(field: Option<u8>, status: Option<u8>, bit: u8) -> Option<u8> {
    field.or_else(|| status.map(|status| (status >> bit) & 1))
}

fn speed_delay_us_to_cpm(speed_delay_us: Option<u64>, steps_per_cycle: f64) -> Option<f64> {
    let delay = speed_delay_us?;
    if delay == 0 || !steps_per_cycle.is_finite() || steps_per_cycle <= 0.0 {
        return None;
    }
    Some(round2(60_000_000.0 / (delay as f64 * steps_per_cycle)))
}

fn timestamp_ms_to_utc(ms: i64) -> Option<DateTime<Utc>> {
    DateTime::<Utc>::from_timestamp_millis(ms)
}

pub fn parse_esp32_frame(
    line: &str,
    expected_prefix: &str,
    checksum_enabled: bool,
) -> Result<SensorSnapshot> {
    let line = line.trim();
    let parts: Vec<&str> = line.split('|').collect();
    if parts.first().copied() != Some(expected_prefix) {
        return Err(anyhow!("unexpected esp32 frame prefix"));
    }

    let mut fields = HashMap::new();
    let mut checksum_field = None;
    let mut checksum_index = None;
    for (index, part) in parts.iter().enumerate().skip(1) {
        let Some((key, value)) = part.split_once('=') else {
            return Err(anyhow!("invalid esp32 frame field {part}"));
        };
        if key == "chk" {
            checksum_field = Some(value);
            checksum_index = Some(index);
        } else {
            fields.insert(key, value);
        }
    }

    if checksum_enabled {
        let Some(chk) = checksum_field else {
            return Err(anyhow!("esp32 frame missing checksum"));
        };
        let Some(index) = checksum_index else {
            return Err(anyhow!("esp32 frame missing checksum position"));
        };
        let body = parts[..index].join("|");
        let expected = checksum_hex(body.as_bytes());
        if !chk.eq_ignore_ascii_case(&expected) {
            return Err(anyhow!(
                "esp32 checksum mismatch expected {expected} got {chk}"
            ));
        }
    }

    let version = required_field(&fields, "v")?;
    if version != "1" {
        return Err(anyhow!("unsupported esp32 frame version {version}"));
    }

    let shake_speed_cpm = parse_f64_any(&fields, &["shake_speed", "shake"], "shake_speed")?;
    let tilt_state = parse_tilt_state_any(&fields, &["tilt_state", "tilt"], "tilt_state")?;
    let captured_at = Utc::now();
    Ok(SensorSnapshot {
        temperature_c: parse_f64_any(&fields, &["temp", "temperature"], "temp")?,
        pressure_mpa: parse_f64_any(&fields, &["pressure"], "pressure")?,
        stirrer_rpm: parse_f64_any(&fields, &["stir_speed", "rpm"], "stir_speed")?,
        shake_speed_cpm,
        tilt_state,
        tilt_angle_deg: fit_tilt_angle_deg(tilt_state, shake_speed_cpm, captured_at),
        flow_rate_l_min: parse_f64_any(&fields, &["flow_rate", "flow"], "flow_rate")?,
        product_concentration_percent: parse_optional_f64_any(
            &fields,
            &["product_concentration", "conc", "concentration"],
            "product_concentration",
        )?
        .unwrap_or(0.0),
        ph: parse_optional_f64_any(&fields, &["ph"], "ph")?.unwrap_or(7.0),
        captured_at,
    })
}

pub fn build_esp32_command(prefix: &str, command: &SafeCommand, checksum_enabled: bool) -> String {
    let body = format!(
        "{prefix}|v=1|heat_time={:.2}|hold_time={:.2}|cool_time={:.2}|target_temp={:.2}|stir_speed={:.2}|shake_speed={:.2}|target_pressure={:.2}",
        command.heat_time_s,
        command.hold_time_s,
        command.cool_time_s,
        command.target_temperature_c,
        command.target_stirrer_rpm,
        command.target_shake_speed_cpm,
        command.target_pressure_mpa
    );
    if checksum_enabled {
        format!("{body}|chk={}\n", checksum_hex(body.as_bytes()))
    } else {
        format!("{body}\n")
    }
}

pub fn build_esp32_sample_frame(
    prefix: &str,
    sample: &SensorSnapshot,
    checksum_enabled: bool,
) -> String {
    let body = format!(
        "{prefix}|v=1|seq=0|ms={}|temp={:.2}|pressure={:.2}|stir_speed={:.2}|shake_speed={:.2}|tilt_state={}|flow_rate={:.2}|product_concentration={:.2}|ph={:.2}",
        sample.captured_at.timestamp_millis().max(0),
        sample.temperature_c,
        sample.pressure_mpa,
        sample.stirrer_rpm,
        sample.shake_speed_cpm,
        sample.tilt_state,
        sample.flow_rate_l_min,
        sample.product_concentration_percent,
        sample.ph
    );
    if checksum_enabled {
        format!("{body}|chk={}\n", checksum_hex(body.as_bytes()))
    } else {
        format!("{body}\n")
    }
}

pub fn checksum_hex(bytes: &[u8]) -> String {
    let checksum = bytes.iter().fold(0_u8, |acc, byte| acc ^ byte);
    format!("{checksum:02X}")
}

fn required_field<'a>(fields: &'a HashMap<&str, &str>, key: &str) -> Result<&'a str> {
    fields
        .get(key)
        .copied()
        .ok_or_else(|| anyhow!("esp32 frame missing field {key}"))
}

fn parse_f64_any(fields: &HashMap<&str, &str>, keys: &[&str], canonical_key: &str) -> Result<f64> {
    for key in keys {
        if let Some(value) = fields.get(key).copied() {
            return parse_f64_value(key, value);
        }
    }
    Err(anyhow!("esp32 frame missing field {canonical_key}"))
}

fn parse_tilt_state_any(
    fields: &HashMap<&str, &str>,
    keys: &[&str],
    canonical_key: &str,
) -> Result<u8> {
    for key in keys {
        if let Some(value) = fields.get(key).copied() {
            return parse_tilt_state_value(key, value);
        }
    }
    Err(anyhow!("esp32 frame missing field {canonical_key}"))
}

fn parse_optional_f64_any(
    fields: &HashMap<&str, &str>,
    keys: &[&str],
    canonical_key: &str,
) -> Result<Option<f64>> {
    for key in keys {
        if let Some(value) = fields.get(key).copied() {
            return parse_f64_value(key, value).map(Some);
        }
    }
    let _ = canonical_key;
    Ok(None)
}

fn parse_f64_value(key: &str, value: &str) -> Result<f64> {
    let parsed = value
        .parse::<f64>()
        .map_err(|err| anyhow!("invalid esp32 field {key}: {err}"))?;
    if !parsed.is_finite() {
        return Err(anyhow!("invalid esp32 field {key}: non-finite value"));
    }
    Ok(parsed)
}

fn parse_tilt_state_value(key: &str, value: &str) -> Result<u8> {
    match value.trim() {
        "0" => Ok(0),
        "1" => Ok(1),
        other => Err(anyhow!(
            "invalid esp32 field {key}: tilt state must be 0 or 1, got {other}"
        )),
    }
}

fn decode_read_register(raw: u16, register: &ReadRegister) -> Result<f64> {
    let value = raw as f64 * register.scale + register.offset;
    if !(register.min_valid..=register.max_valid).contains(&value) {
        return Err(anyhow!(
            "register {} value {value} outside valid range {}..{}",
            register.address,
            register.min_valid,
            register.max_valid
        ));
    }
    Ok(round2(value))
}

fn encode_write_register(value: f64, register: &WriteRegister) -> Result<u16> {
    if register.scale == 0.0 {
        return Err(anyhow!("register {} has zero scale", register.address));
    }
    let raw = ((value - register.offset) / register.scale).round();
    if !(0.0..=u16::MAX as f64).contains(&raw) {
        return Err(anyhow!(
            "value {value} cannot be encoded for register {}",
            register.address
        ));
    }
    Ok(raw as u16)
}

fn read_holding_register(port: &mut dyn SerialPort, slave_id: u8, address: u16) -> Result<u16> {
    let mut frame = vec![slave_id, 0x03, high(address), low(address), 0x00, 0x01];
    append_crc(&mut frame);
    port.write_all(&frame)?;
    port.flush()?;

    let mut response = [0_u8; 7];
    port.read_exact(&mut response)?;
    validate_crc(&response)?;
    if response[0] != slave_id {
        return Err(anyhow!("unexpected slave id {}", response[0]));
    }
    if response[1] & 0x80 != 0 {
        return Err(anyhow!("modbus exception code {}", response[2]));
    }
    if response[1] != 0x03 || response[2] != 0x02 {
        return Err(anyhow!("invalid read response"));
    }
    Ok(u16::from_be_bytes([response[3], response[4]]))
}

fn write_single_register(
    port: &mut dyn SerialPort,
    slave_id: u8,
    address: u16,
    value: u16,
) -> Result<()> {
    let [value_hi, value_lo] = value.to_be_bytes();
    let mut frame = vec![
        slave_id,
        0x06,
        high(address),
        low(address),
        value_hi,
        value_lo,
    ];
    append_crc(&mut frame);
    port.write_all(&frame)?;
    port.flush()?;

    let mut response = [0_u8; 8];
    port.read_exact(&mut response)?;
    validate_crc(&response)?;
    if response[0] != slave_id {
        return Err(anyhow!("unexpected slave id {}", response[0]));
    }
    if response[1] & 0x80 != 0 {
        return Err(anyhow!("modbus exception code {}", response[2]));
    }
    if response[..6] != frame[..6] {
        return Err(anyhow!("write response does not echo request"));
    }
    Ok(())
}

fn append_crc(frame: &mut Vec<u8>) {
    let crc = crc16_modbus(frame);
    frame.push((crc & 0x00ff) as u8);
    frame.push((crc >> 8) as u8);
}

fn validate_crc(frame: &[u8]) -> Result<()> {
    if frame.len() < 3 {
        return Err(anyhow!("modbus frame too short"));
    }
    let expected = crc16_modbus(&frame[..frame.len() - 2]);
    let actual = u16::from_le_bytes([frame[frame.len() - 2], frame[frame.len() - 1]]);
    if expected != actual {
        return Err(anyhow!("invalid modbus crc"));
    }
    Ok(())
}

fn crc16_modbus(bytes: &[u8]) -> u16 {
    let mut crc = 0xffff_u16;
    for byte in bytes {
        crc ^= *byte as u16;
        for _ in 0..8 {
            if crc & 1 == 1 {
                crc = (crc >> 1) ^ 0xa001;
            } else {
                crc >>= 1;
            }
        }
    }
    crc
}

fn high(value: u16) -> u8 {
    (value >> 8) as u8
}

fn low(value: u16) -> u8 {
    (value & 0xff) as u8
}
