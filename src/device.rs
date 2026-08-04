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
use tokio::sync::Mutex as AsyncMutex;
use tokio_modbus::{
    client::{rtu, Reader, Writer},
    Slave,
};
use tokio_serial::{
    DataBits as TokioDataBits, Parity as TokioParity, SerialPortBuilderExt,
    StopBits as TokioStopBits,
};

use crate::{
    config::{
        DeviceConfig, DeviceMode, JsonBridgeAdcSensor, JsonBridgeConfig, ReadRegister,
        SafetyConfig, WriteRegister,
    },
    control::SafeCommand,
    number::round2,
    state::{
        fit_tilt_angle_deg, timestamp_age_ms, ControlTargets, DeviceStatusSnapshot, SensorSnapshot,
    },
};

/// Command-level handshake outcome. The upper computer treats a downstream
/// write as complete only when the device echoes the same `request_id` with a
/// positive ACK. This closes the fire-and-forget blind spot of `write_targets`
/// (which returns Ok as soon as bytes are flushed / a file is written) and the
/// stale-ACK mismatch risk of the next-round `last_command_ok` poll. See
/// `docs/command_ack_handshake.md`.
#[derive(Debug, Clone)]
pub struct CommandAck {
    /// `request_id` echoed by the downstream; must equal the id generated for
    /// this command, else the ACK is stale and must be ignored.
    pub request_id: String,
    pub status: AckStatus,
    /// Targets the downstream reports it actually accepted/applied (after its
    /// own clamping). `None` when the protocol does not echo applied values.
    /// Used to detect a silent clamp or an applied-value mismatch (CLAUDE 3.2).
    pub accepted_targets: Option<ControlTargets>,
}

#[derive(Debug, Clone)]
pub enum AckStatus {
    /// Downstream echoed the request_id and confirmed execution.
    Confirmed,
    /// Downstream received the command but refused (out-of-range, busy, fault).
    Rejected(String),
    /// No matching ACK within the handshake timeout — delivery unconfirmed;
    /// the control loop treats this as a fail-closed condition.
    Timeout,
    /// Device mode has no real handshake and fell back to `write_targets`.
    /// Treated as a configuration error when `require_command_ack` is on.
    Unverified,
}

impl CommandAck {
    pub fn unverified(request_id: impl Into<String>) -> Self {
        Self {
            request_id: request_id.into(),
            status: AckStatus::Unverified,
            accepted_targets: None,
        }
    }
}

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
    /// Command-level handshake: write the command and wait for a matching ACK.
    ///
    /// Default implementation falls back to the fire-and-forget `write_targets`
    /// and reports `Unverified`; real device modes override to perform an
    /// explicit ACK exchange. When `require_command_ack` is enabled the control
    /// loop treats `Unverified` as a configuration error and fails closed.
    async fn write_targets_acknowledged(
        &self,
        command: &SafeCommand,
        request_id: &str,
        timeout: Duration,
    ) -> Result<CommandAck> {
        let _ = timeout;
        self.write_targets(command).await?;
        Ok(CommandAck::unverified(request_id))
    }
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
        DeviceMode::Simulation => Ok(Arc::new(crate::virtual_sensor::VirtualSensorDevice::new(
            config.simulation.clone(),
        ))),
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

    async fn write_targets_acknowledged(
        &self,
        _command: &SafeCommand,
        request_id: &str,
        _timeout: Duration,
    ) -> Result<CommandAck> {
        // Pipeline mode never emits commands (an external controller owns
        // actuation), so there is nothing to confirm; report Confirmed with no
        // applied-targets echo.
        Ok(CommandAck {
            request_id: request_id.to_string(),
            status: AckStatus::Confirmed,
            accepted_targets: None,
        })
    }
}

struct ModbusRtuDevice {
    config: DeviceConfig,
    client: Arc<AsyncMutex<tokio_modbus::client::Context>>,
}

struct Esp32SerialDevice {
    config: DeviceConfig,
    port: Arc<StdMutex<Box<dyn SerialPort>>>,
}

struct JsonBridgeDevice {
    config: JsonBridgeConfig,
    last_commanded_shake_speed_cpm: Arc<StdMutex<Option<f64>>>,
    last_stirrer_command: Arc<StdMutex<Option<f64>>>,
}

#[derive(Debug, Default)]
struct JsonBridgePendingCacheUpdate {
    shake_speed_cpm: Option<f64>,
    stirrer_rpm: Option<f64>,
}

struct JsonBridgePendingControl {
    control: JsonBridgeControl,
    cache_update: JsonBridgePendingCacheUpdate,
}

struct JsonBridgePendingComponentOutcome {
    response: ComponentControlOutcome,
    command: Option<JsonBridgeControl>,
    cache_update: JsonBridgePendingCacheUpdate,
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
        let serial = open_tokio_serial_port(&config)?;
        let client = rtu::attach_slave(serial, Slave(config.modbus.slave_id));

        Ok(Self {
            config,
            client: Arc::new(AsyncMutex::new(client)),
        })
    }
}

impl JsonBridgeDevice {
    fn new(config: JsonBridgeConfig) -> Self {
        Self {
            config,
            last_commanded_shake_speed_cpm: Arc::new(StdMutex::new(None)),
            last_stirrer_command: Arc::new(StdMutex::new(None)),
        }
    }
}

#[async_trait::async_trait]
impl ReactorDevice for ModbusRtuDevice {
    async fn read_sample(&self) -> Result<SensorSnapshot> {
        let config = self.config.clone();
        let mut client = self.client.lock().await;
        let temperature_raw =
            read_holding_register(&mut client, config.modbus.registers.temperature_c.address)
                .await?;
        let stirrer_raw =
            read_holding_register(&mut client, config.modbus.registers.stirrer_rpm.address).await?;
        let temperature_c =
            decode_read_register(temperature_raw, &config.modbus.registers.temperature_c)?;
        let stirrer_rpm = decode_read_register(stirrer_raw, &config.modbus.registers.stirrer_rpm)?;
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
    }

    async fn write_targets(&self, command: &SafeCommand) -> Result<()> {
        let config = self.config.clone();
        let mut client = self.client.lock().await;
        write_single_register(
            &mut client,
            config.modbus.registers.target_temperature_c.address,
            encode_write_register(
                command.target_temperature_c,
                &config.modbus.registers.target_temperature_c,
            )?,
        )
        .await?;
        write_single_register(
            &mut client,
            config.modbus.registers.target_stirrer_rpm.address,
            encode_write_register(
                command.target_stirrer_rpm,
                &config.modbus.registers.target_stirrer_rpm,
            )?,
        )
        .await?;
        Ok(())
    }

    async fn write_targets_acknowledged(
        &self,
        command: &SafeCommand,
        request_id: &str,
        _timeout: Duration,
    ) -> Result<CommandAck> {
        // Modbus FC06 is already a request-response: each write_single_register
        // Ok means the slave echoed the write back (transport ACK). The
        // handshake adds a read-back verification — re-reading the target
        // registers and confirming the slave now HOLDS exactly the raw we wrote.
        // A mismatch means the slave rejected/clamped/overwrote the value: fail
        // closed (Rejected). request_id has no Modbus wire representation (the
        // transaction id is owned by tokio-modbus), so it is only carried in the
        // CommandAck for audit correlation. Read-back is instantaneous, hence
        // timeout is unused.
        let config = self.config.clone();
        let mut client = self.client.lock().await;
        let expected_temp_raw = encode_write_register(
            command.target_temperature_c,
            &config.modbus.registers.target_temperature_c,
        )?;
        let expected_stir_raw = encode_write_register(
            command.target_stirrer_rpm,
            &config.modbus.registers.target_stirrer_rpm,
        )?;
        write_single_register(
            &mut client,
            config.modbus.registers.target_temperature_c.address,
            expected_temp_raw,
        )
        .await?;
        write_single_register(
            &mut client,
            config.modbus.registers.target_stirrer_rpm.address,
            expected_stir_raw,
        )
        .await?;
        let held_temp_raw = read_holding_register(
            &mut client,
            config.modbus.registers.target_temperature_c.address,
        )
        .await?;
        let held_stir_raw = read_holding_register(
            &mut client,
            config.modbus.registers.target_stirrer_rpm.address,
        )
        .await?;
        if held_temp_raw != expected_temp_raw || held_stir_raw != expected_stir_raw {
            let held_temp =
                decode_write_register(held_temp_raw, &config.modbus.registers.target_temperature_c);
            let held_stir =
                decode_write_register(held_stir_raw, &config.modbus.registers.target_stirrer_rpm);
            return Ok(CommandAck {
                request_id: request_id.to_string(),
                status: AckStatus::Rejected(format!(
                    "modbus read-back mismatch: target_temperature_c held {held_temp} sent {}, target_stirrer_rpm held {held_stir} sent {}",
                    command.target_temperature_c, command.target_stirrer_rpm
                )),
                accepted_targets: None,
            });
        }
        let accepted = ControlTargets {
            temperature_c: decode_write_register(
                held_temp_raw,
                &config.modbus.registers.target_temperature_c,
            ),
            heat_time_s: command.heat_time_s,
            hold_time_s: command.hold_time_s,
            cool_time_s: command.cool_time_s,
            stirrer_rpm: decode_write_register(
                held_stir_raw,
                &config.modbus.registers.target_stirrer_rpm,
            ),
            shake_speed_cpm: command.target_shake_speed_cpm,
            target_pressure_mpa: command.target_pressure_mpa,
        };
        Ok(CommandAck {
            request_id: request_id.to_string(),
            status: AckStatus::Confirmed,
            accepted_targets: Some(accepted),
        })
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

    async fn write_targets_acknowledged(
        &self,
        command: &SafeCommand,
        request_id: &str,
        timeout: Duration,
    ) -> Result<CommandAck> {
        let config = self.config.clone();
        let port = Arc::clone(&self.port);
        let command = command.clone();
        let request_id = request_id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut port = port
                .lock()
                .map_err(|_| anyhow!("serial port lock poisoned"))?;
            // Send the command carrying our rid so the downstream can echo it.
            let frame = build_esp32_command_with_rid(
                &config.esp32.command_prefix,
                &command,
                &request_id,
                config.esp32.checksum,
            );
            port.write_all(frame.as_bytes())?;
            port.flush()?;
            // Read on an independent cloned reader whose timeout is set short
            // (100 ms) so the sample-path reader is unaffected and we never
            // block forever on a silent line. Loop until an ACK frame echoing
            // our rid arrives, or the handshake window expires. Sample frames
            // and unparsable lines are skipped.
            let mut reader_port = port.try_clone()?;
            reader_port.set_timeout(Duration::from_millis(100))?;
            let mut reader = BufReader::new(reader_port);
            let deadline = std::time::Instant::now() + timeout;
            loop {
                if std::time::Instant::now() >= deadline {
                    return Ok(CommandAck {
                        request_id,
                        status: AckStatus::Timeout,
                        accepted_targets: None,
                    });
                }
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Ok(_) => {}
                }
                if let Ok(ack) =
                    parse_esp32_ack_frame(&line, &config.esp32.frame_prefix, config.esp32.checksum)
                {
                    if ack.request_id == request_id {
                        let status = if ack.ok {
                            AckStatus::Confirmed
                        } else {
                            AckStatus::Rejected(
                                ack.error
                                    .unwrap_or_else(|| "downstream rejected command".to_string()),
                            )
                        };
                        return Ok(CommandAck {
                            request_id,
                            status,
                            accepted_targets: None,
                        });
                    }
                    // ACK for a different (stale) rid: keep waiting for ours.
                }
                // Otherwise a sample frame or malformed line: skip, keep waiting.
            }
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
        let last_stirrer_command = Arc::clone(&self.last_stirrer_command);
        tokio::task::spawn_blocking(move || {
            let current = read_json_bridge_state(&config.state_path)?;
            let control = next_json_bridge_control(
                &config,
                &current,
                &command,
                &last_commanded_shake_speed_cpm,
                &last_stirrer_command,
            )?;
            if let Some(control) = control {
                write_json_bridge_control(&config.control_path, &control.control)?;
                apply_json_bridge_cache_update(
                    &control.cache_update,
                    &last_commanded_shake_speed_cpm,
                    &last_stirrer_command,
                )?;
            }
            Ok(())
        })
        .await?
    }

    async fn write_targets_acknowledged(
        &self,
        command: &SafeCommand,
        request_id: &str,
        timeout: Duration,
    ) -> Result<CommandAck> {
        let config = self.config.clone();
        let command = command.clone();
        let last_commanded_shake_speed_cpm = Arc::clone(&self.last_commanded_shake_speed_cpm);
        let last_stirrer_command = Arc::clone(&self.last_stirrer_command);
        let request_id = request_id.to_string();
        tokio::task::spawn_blocking(move || {
            // Reuse the SafeCommand -> atomic-control conversion, but override
            // the request_id so the downstream echoes back the id the upper
            // computer generated for this handshake (not the bridge's internal
            // sequence). Then poll state.json until the downstream reports it
            // processed this exact request_id.
            let current = read_json_bridge_state(&config.state_path)?;
            let pending = next_json_bridge_control(
                &config,
                &current,
                &command,
                &last_commanded_shake_speed_cpm,
                &last_stirrer_command,
            )?;
            let Some(pending) = pending else {
                // No atomic command to send (all targets within deadband): the
                // downstream has nothing to apply, so the handshake is trivially
                // confirmed.
                return Ok(CommandAck {
                    request_id,
                    status: AckStatus::Confirmed,
                    accepted_targets: None,
                });
            };
            let mut control = pending.control;
            control.request_id = request_id.clone();
            write_json_bridge_control(&config.control_path, &control)?;
            apply_json_bridge_cache_update(
                &pending.cache_update,
                &last_commanded_shake_speed_cpm,
                &last_stirrer_command,
            )?;

            // Poll state.json: Confirmed when the downstream echoes our rid with
            // ok=true, Rejected when ok=false, Timeout when no matching echo
            // arrives within the handshake window.
            let deadline = std::time::Instant::now() + timeout;
            loop {
                if std::time::Instant::now() >= deadline {
                    return Ok(CommandAck {
                        request_id,
                        status: AckStatus::Timeout,
                        accepted_targets: None,
                    });
                }
                if let Ok(state) = read_json_bridge_state(&config.state_path) {
                    if state.last_command_request_id.as_deref() == Some(request_id.as_str()) {
                        match state.last_command_ok {
                            Some(true) => {
                                return Ok(CommandAck {
                                    request_id,
                                    status: AckStatus::Confirmed,
                                    accepted_targets: None,
                                });
                            }
                            Some(false) => {
                                let detail = state.last_command_error.unwrap_or_else(|| {
                                    "downstream rejected command without an error detail"
                                        .to_string()
                                });
                                return Ok(CommandAck {
                                    request_id,
                                    status: AckStatus::Rejected(detail),
                                    accepted_targets: None,
                                });
                            }
                            None => {}
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(50));
            }
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
        let last_stirrer_command = Arc::clone(&self.last_stirrer_command);
        tokio::task::spawn_blocking(move || {
            if json_bridge_direct_control_is_risk_reducing(&request) {
                let outcome =
                    json_bridge_component_control_outcome_without_state(&config, &request)?;
                if let Some(control) = &outcome.command {
                    write_json_bridge_control(&config.control_path, control)?;
                    apply_json_bridge_cache_update(
                        &outcome.cache_update,
                        &last_commanded_shake_speed_cpm,
                        &last_stirrer_command,
                    )?;
                }
                return Ok(outcome.response);
            }
            let state = read_json_bridge_state(&config.state_path)?;
            validate_json_bridge_state(&config, &state)?;
            let outcome = json_bridge_component_control_outcome(
                &config,
                &state,
                &request,
                &targets,
                &safety,
                &last_commanded_shake_speed_cpm,
                &last_stirrer_command,
            )?;
            if let Some(control) = &outcome.command {
                write_json_bridge_control(&config.control_path, control)?;
                apply_json_bridge_cache_update(
                    &outcome.cache_update,
                    &last_commanded_shake_speed_cpm,
                    &last_stirrer_command,
                )?;
            }
            Ok(outcome.response)
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
            next.temperature_c = component_number_in_range(
                command,
                "value",
                safety.temperature.min_c,
                safety.temperature.max_c,
            )?;
        }
        ("stirrer_motor", "set_rpm") => {
            next.stirrer_rpm = component_number_in_range(
                command,
                "value",
                safety.stirrer.min_rpm,
                safety.stirrer.max_rpm,
            )?;
        }
        ("shake_stepper", "set_speed") => {
            next.shake_speed_cpm = component_number_in_range(command, "value", 0.0, 60.0)?;
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
    Ok(round_component_targets(next))
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

fn component_number_in_range(
    command: &ComponentControlCommand,
    field: &str,
    min: f64,
    max: f64,
) -> Result<f64> {
    let number = component_number(command, field)?;
    if !(min..=max).contains(&number) {
        return Err(anyhow!(
            "component control {field} must be between {min} and {max}"
        ));
    }
    Ok(number)
}

fn round_component_targets(targets: ControlTargets) -> ControlTargets {
    ControlTargets {
        temperature_c: round2(targets.temperature_c),
        heat_time_s: round2(targets.heat_time_s),
        hold_time_s: round2(targets.hold_time_s),
        cool_time_s: round2(targets.cool_time_s),
        stirrer_rpm: round2(targets.stirrer_rpm),
        shake_speed_cpm: round2(targets.shake_speed_cpm),
        target_pressure_mpa: round2(targets.target_pressure_mpa),
    }
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

fn json_bridge_component_control_outcome(
    config: &JsonBridgeConfig,
    state: &JsonBridgeState,
    request: &ComponentControlCommand,
    targets: &ControlTargets,
    safety: &SafetyConfig,
    last_commanded_shake_speed_cpm: &StdMutex<Option<f64>>,
    last_stirrer_command: &StdMutex<Option<f64>>,
) -> Result<JsonBridgePendingComponentOutcome> {
    if let Some(control) = json_bridge_direct_component_control(config, request) {
        return Ok(JsonBridgePendingComponentOutcome {
            response: ComponentControlOutcome {
                component_id: request.component_id.clone(),
                action: request.action.clone(),
                command: Some(control.clone()),
                targets: None,
                message: "component command written to json bridge control.json".to_string(),
            },
            command: Some(control),
            cache_update: JsonBridgePendingCacheUpdate::default(),
        });
    }

    match (request.component_id.as_str(), request.action.as_str()) {
        ("stirrer_motor", "set_rpm") => json_bridge_stirrer_component_outcome(
            config,
            state,
            request,
            targets,
            safety,
            last_stirrer_command,
        ),
        ("shake_stepper", "set_speed") | ("temperature_controller", "set_target_temperature") => {
            json_bridge_target_component_outcome(
                config,
                state,
                request,
                targets,
                safety,
                last_commanded_shake_speed_cpm,
                last_stirrer_command,
            )
        }
        _ => Err(anyhow!(
            "unsupported component control {}:{} for json bridge",
            request.component_id,
            request.action
        )),
    }
}

fn json_bridge_direct_component_control(
    config: &JsonBridgeConfig,
    request: &ComponentControlCommand,
) -> Option<JsonBridgeControl> {
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
        _ => return None,
    };
    Some(control)
}

fn json_bridge_direct_control_is_risk_reducing(request: &ComponentControlCommand) -> bool {
    matches!(
        (request.component_id.as_str(), request.action.as_str()),
        ("shake_stepper", "stop") | ("heater_relay", "off")
    )
}

fn json_bridge_component_control_outcome_without_state(
    config: &JsonBridgeConfig,
    request: &ComponentControlCommand,
) -> Result<JsonBridgePendingComponentOutcome> {
    let Some(control) = json_bridge_direct_component_control(config, request) else {
        return Err(anyhow!(
            "component control {}:{} requires valid json bridge state",
            request.component_id,
            request.action
        ));
    };
    Ok(JsonBridgePendingComponentOutcome {
        response: ComponentControlOutcome {
            component_id: request.component_id.clone(),
            action: request.action.clone(),
            command: Some(control.clone()),
            targets: None,
            message: "risk-reducing component command written to json bridge control.json"
                .to_string(),
        },
        command: Some(control),
        cache_update: JsonBridgePendingCacheUpdate::default(),
    })
}

fn apply_json_bridge_cache_update(
    update: &JsonBridgePendingCacheUpdate,
    last_commanded_shake_speed_cpm: &StdMutex<Option<f64>>,
    last_stirrer_command: &StdMutex<Option<f64>>,
) -> Result<()> {
    if let Some(value) = update.shake_speed_cpm {
        *last_commanded_shake_speed_cpm
            .lock()
            .map_err(|_| anyhow!("json bridge speed cache lock poisoned"))? = Some(value);
    }
    if let Some(value) = update.stirrer_rpm {
        *last_stirrer_command
            .lock()
            .map_err(|_| anyhow!("json bridge stirrer cache lock poisoned"))? = Some(value);
    }
    Ok(())
}

fn json_bridge_stirrer_component_outcome(
    config: &JsonBridgeConfig,
    state: &JsonBridgeState,
    request: &ComponentControlCommand,
    targets: &ControlTargets,
    safety: &SafetyConfig,
    last_stirrer_command: &StdMutex<Option<f64>>,
) -> Result<JsonBridgePendingComponentOutcome> {
    let next_targets = targets_for_component(request, targets, safety)?;
    let safe = safe_command_from_targets(&next_targets, "manual component control");
    let rpm = safe.target_stirrer_rpm;
    let current_rpm = state
        .stirrer_rpm
        .or_else(|| last_stirrer_command.lock().ok().and_then(|cached| *cached));
    if current_rpm
        .map(|current| (rpm - current).abs() <= 0.01)
        .unwrap_or(false)
    {
        return Ok(JsonBridgePendingComponentOutcome {
            response: ComponentControlOutcome {
                component_id: request.component_id.clone(),
                action: request.action.clone(),
                command: None,
                targets: Some(safe),
                message: "stirrer target already matches json bridge state".to_string(),
            },
            command: None,
            cache_update: JsonBridgePendingCacheUpdate::default(),
        });
    }
    let control = build_json_bridge_control(
        &config.request_id_prefix,
        "stir_speed",
        Some(serde_json::json!(rpm)),
        Some("stirrer_motor"),
    );
    Ok(JsonBridgePendingComponentOutcome {
        response: ComponentControlOutcome {
            component_id: request.component_id.clone(),
            action: request.action.clone(),
            command: Some(control.clone()),
            targets: Some(safe),
            message: "stirrer RPM written to json bridge control.json".to_string(),
        },
        command: Some(control),
        cache_update: JsonBridgePendingCacheUpdate {
            stirrer_rpm: Some(rpm),
            ..JsonBridgePendingCacheUpdate::default()
        },
    })
}

fn json_bridge_target_component_outcome(
    config: &JsonBridgeConfig,
    state: &JsonBridgeState,
    request: &ComponentControlCommand,
    targets: &ControlTargets,
    safety: &SafetyConfig,
    last_commanded_shake_speed_cpm: &StdMutex<Option<f64>>,
    last_stirrer_command: &StdMutex<Option<f64>>,
) -> Result<JsonBridgePendingComponentOutcome> {
    let next_targets = targets_for_component(request, targets, safety)?;
    let safe = safe_command_from_targets(&next_targets, "manual component control");
    let Some(control) = next_json_bridge_control(
        config,
        state,
        &safe,
        last_commanded_shake_speed_cpm,
        last_stirrer_command,
    )?
    else {
        return Ok(JsonBridgePendingComponentOutcome {
            response: ComponentControlOutcome {
                component_id: request.component_id.clone(),
                action: request.action.clone(),
                command: None,
                targets: Some(safe),
                message: "component target already inside json bridge deadband".to_string(),
            },
            command: None,
            cache_update: JsonBridgePendingCacheUpdate::default(),
        });
    };
    Ok(JsonBridgePendingComponentOutcome {
        response: ComponentControlOutcome {
            component_id: request.component_id.clone(),
            action: request.action.clone(),
            command: Some(control.control.clone()),
            targets: Some(safe),
            message: "component target translated to json bridge command".to_string(),
        },
        command: Some(control.control),
        cache_update: control.cache_update,
    })
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
    pub value: Option<Value>,
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
    value: Option<Value>,
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
    let tmp_path = json_bridge_control_tmp_path(path, control);
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
    sync_json_bridge_parent_dir(parent)?;
    Ok(())
}

fn json_bridge_control_tmp_path(path: &Path, control: &JsonBridgeControl) -> std::path::PathBuf {
    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("control.json");
    let request_id = sanitize_tmp_path_segment(&control.request_id);
    path.with_file_name(format!("{file_name}.{request_id}.{sequence}.tmp"))
}

fn sanitize_tmp_path_segment(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    if sanitized.is_empty() {
        "request".to_string()
    } else {
        sanitized
    }
}

#[cfg(unix)]
fn sync_json_bridge_parent_dir(parent: Option<&Path>) -> Result<()> {
    if let Some(parent) = parent {
        let directory = File::open(parent)
            .with_context(|| format!("failed to open json bridge dir {}", parent.display()))?;
        directory
            .sync_all()
            .with_context(|| format!("failed to sync json bridge dir {}", parent.display()))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn sync_json_bridge_parent_dir(_parent: Option<&Path>) -> Result<()> {
    Ok(())
}

fn open_serial_port(config: &DeviceConfig) -> Result<Box<dyn SerialPort>> {
    serialport::new(&config.serial.port, config.serial.baudrate)
        .timeout(Duration::from_millis(config.serial.timeout_ms))
        .parity(serial_parity(&config.serial.parity)?)
        .stop_bits(serial_stop_bits(config.serial.stopbits)?)
        .data_bits(serial_data_bits(config.serial.bytesize)?)
        .open()
        .map_err(|err| anyhow!("failed to open serial port {}: {err}", config.serial.port))
}

fn open_tokio_serial_port(config: &DeviceConfig) -> Result<tokio_serial::SerialStream> {
    tokio_serial::new(&config.serial.port, config.serial.baudrate)
        .timeout(Duration::from_millis(config.serial.timeout_ms))
        .parity(tokio_serial_parity(&config.serial.parity)?)
        .stop_bits(tokio_serial_stop_bits(config.serial.stopbits)?)
        .data_bits(tokio_serial_data_bits(config.serial.bytesize)?)
        .open_native_async()
        .map_err(|err| {
            anyhow!(
                "failed to open async Modbus RTU serial port {}: {err}",
                config.serial.port
            )
        })
}

fn serial_parity(value: &str) -> Result<Parity> {
    match value {
        "N" | "n" => Ok(Parity::None),
        "E" | "e" => Ok(Parity::Even),
        "O" | "o" => Ok(Parity::Odd),
        other => Err(anyhow!("unsupported serial parity {other}")),
    }
}

fn serial_stop_bits(value: u8) -> Result<StopBits> {
    match value {
        1 => Ok(StopBits::One),
        2 => Ok(StopBits::Two),
        other => Err(anyhow!("unsupported serial stopbits {other}")),
    }
}

fn serial_data_bits(value: u8) -> Result<DataBits> {
    match value {
        5 => Ok(DataBits::Five),
        6 => Ok(DataBits::Six),
        7 => Ok(DataBits::Seven),
        8 => Ok(DataBits::Eight),
        other => Err(anyhow!("unsupported serial bytesize {other}")),
    }
}

fn tokio_serial_parity(value: &str) -> Result<TokioParity> {
    match value {
        "N" | "n" => Ok(TokioParity::None),
        "E" | "e" => Ok(TokioParity::Even),
        "O" | "o" => Ok(TokioParity::Odd),
        other => Err(anyhow!("unsupported serial parity {other}")),
    }
}

fn tokio_serial_stop_bits(value: u8) -> Result<TokioStopBits> {
    match value {
        1 => Ok(TokioStopBits::One),
        2 => Ok(TokioStopBits::Two),
        other => Err(anyhow!("unsupported serial stopbits {other}")),
    }
}

fn tokio_serial_data_bits(value: u8) -> Result<TokioDataBits> {
    match value {
        5 => Ok(TokioDataBits::Five),
        6 => Ok(TokioDataBits::Six),
        7 => Ok(TokioDataBits::Seven),
        8 => Ok(TokioDataBits::Eight),
        other => Err(anyhow!("unsupported serial bytesize {other}")),
    }
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
    let age = timestamp_age_ms(last_seen);
    if age < 0 {
        return Err(anyhow!(
            "json bridge state timestamp is {} ms in the future; check controller clock synchronization",
            -age
        ));
    }
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
    last_stirrer_command: &StdMutex<Option<f64>>,
) -> Result<Option<JsonBridgePendingControl>> {
    if let Err(err) = validate_json_bridge_state(config, state) {
        return Err(anyhow!(
            "json bridge refuses control because state is not valid: {err}"
        ));
    }

    let motor = bit_or_field(state.motor, state.status, 1).unwrap_or(0);
    if command.target_shake_speed_cpm <= 0.01 && motor != 0 {
        return Ok(Some(JsonBridgePendingControl {
            control: build_json_bridge_control(
                &config.request_id_prefix,
                "motor",
                Some(serde_json::json!(0)),
                None,
            ),
            cache_update: JsonBridgePendingCacheUpdate {
                shake_speed_cpm: Some(0.0),
                ..JsonBridgePendingCacheUpdate::default()
            },
        }));
    }
    if command.target_shake_speed_cpm > 0.01 && motor == 0 {
        return Ok(Some(JsonBridgePendingControl {
            control: build_json_bridge_control(
                &config.request_id_prefix,
                "motor",
                Some(serde_json::json!(1)),
                None,
            ),
            cache_update: JsonBridgePendingCacheUpdate {
                shake_speed_cpm: Some(command.target_shake_speed_cpm),
                ..JsonBridgePendingCacheUpdate::default()
            },
        }));
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
        return Ok(Some(JsonBridgePendingControl {
            control: build_json_bridge_control(
                &config.request_id_prefix,
                "speed",
                Some(serde_json::json!(if speed_delta > 0.0 {
                    "up"
                } else {
                    "down"
                })),
                None,
            ),
            cache_update: JsonBridgePendingCacheUpdate {
                shake_speed_cpm: Some(command.target_shake_speed_cpm),
                ..JsonBridgePendingCacheUpdate::default()
            },
        }));
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
            return Ok(Some(JsonBridgePendingControl {
                control: build_json_bridge_control(
                    &config.request_id_prefix,
                    "relay",
                    Some(serde_json::json!(desired_relay)),
                    None,
                ),
                cache_update: JsonBridgePendingCacheUpdate::default(),
            }));
        }
    } else {
    }

    let cached_stirrer = *last_stirrer_command
        .lock()
        .map_err(|_| anyhow!("json bridge stirrer cache lock poisoned"))?;
    let current_stirrer = state.stirrer_rpm.or(cached_stirrer);
    if current_stirrer
        .map(|current| (command.target_stirrer_rpm - current).abs() > 0.01)
        .unwrap_or(true)
    {
        return Ok(Some(JsonBridgePendingControl {
            control: build_json_bridge_control(
                &config.request_id_prefix,
                "stir_speed",
                Some(serde_json::json!(command.target_stirrer_rpm)),
                Some("stirrer_motor"),
            ),
            cache_update: JsonBridgePendingCacheUpdate {
                stirrer_rpm: Some(command.target_stirrer_rpm),
                ..JsonBridgePendingCacheUpdate::default()
            },
        }));
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

/// Command frame carrying a request_id, so the downstream can echo it back in
/// its ACK. Used by the command-level handshake; the legacy fire-and-forget
/// `build_esp32_command` omits the rid.
pub fn build_esp32_command_with_rid(
    prefix: &str,
    command: &SafeCommand,
    request_id: &str,
    checksum_enabled: bool,
) -> String {
    let body = format!(
        "{prefix}|v=1|rid={request_id}|heat_time={:.2}|hold_time={:.2}|cool_time={:.2}|target_temp={:.2}|stir_speed={:.2}|shake_speed={:.2}|target_pressure={:.2}",
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

/// Downstream ACK parsed from an ESP32 ack frame. See docs/command_ack_handshake.md.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Esp32Ack {
    pub request_id: String,
    pub ok: bool,
    pub error: Option<String>,
}

/// Build an ESP32 ACK frame (downstream -> upper computer). Mirrors the command
/// frame grammar: `{prefix}|v=1|type=ack|rid=<id>|ok=<0|1>[|err=<text>]|chk=<hex>`.
pub fn build_esp32_ack_frame(
    prefix: &str,
    request_id: &str,
    ok: bool,
    error: Option<&str>,
    checksum_enabled: bool,
) -> String {
    let ok_field = if ok { "1" } else { "0" };
    let mut body = format!("{prefix}|v=1|type=ack|rid={request_id}|ok={ok_field}");
    if let Some(err) = error {
        body.push_str(&format!("|err={err}"));
    }
    if checksum_enabled {
        format!("{body}|chk={}\n", checksum_hex(body.as_bytes()))
    } else {
        format!("{body}\n")
    }
}

/// Parse an ESP32 ACK frame. Returns Ok only when the line is a valid ack frame
/// (prefix matches, checksum verifies, type=ack). Sample frames and malformed
/// lines return Err so the handshake loop can skip them and keep waiting.
pub fn parse_esp32_ack_frame(
    line: &str,
    expected_prefix: &str,
    checksum_enabled: bool,
) -> Result<Esp32Ack> {
    let line = line.trim();
    let parts: Vec<&str> = line.split('|').collect();
    if parts.first().copied() != Some(expected_prefix) {
        return Err(anyhow!("unexpected esp32 ack frame prefix"));
    }
    let mut fields: HashMap<&str, &str> = HashMap::new();
    let mut checksum_field = None;
    let mut checksum_index = None;
    for (index, part) in parts.iter().enumerate().skip(1) {
        let Some((key, value)) = part.split_once('=') else {
            return Err(anyhow!("invalid esp32 ack frame field {part}"));
        };
        if key == "chk" {
            checksum_field = Some(value);
            checksum_index = Some(index);
        } else {
            fields.insert(key, value);
        }
    }
    if checksum_enabled {
        let chk = checksum_field.ok_or_else(|| anyhow!("esp32 ack frame missing checksum"))?;
        let index =
            checksum_index.ok_or_else(|| anyhow!("esp32 ack frame missing checksum position"))?;
        let body = parts[..index].join("|");
        let expected = checksum_hex(body.as_bytes());
        if !chk.eq_ignore_ascii_case(&expected) {
            return Err(anyhow!(
                "esp32 ack checksum mismatch expected {expected} got {chk}"
            ));
        }
    }
    if required_field(&fields, "v")? != "1" {
        return Err(anyhow!("unsupported esp32 ack frame version"));
    }
    if required_field(&fields, "type")? != "ack" {
        return Err(anyhow!("esp32 frame is not an ack"));
    }
    let request_id = required_field(&fields, "rid")?.to_string();
    let ok = match required_field(&fields, "ok")? {
        "1" | "true" => true,
        "0" | "false" => false,
        other => return Err(anyhow!("invalid esp32 ack ok field {other}")),
    };
    let error = fields.get("err").map(|s| s.to_string());
    Ok(Esp32Ack {
        request_id,
        ok,
        error,
    })
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

/// Inverse of `encode_write_register` for read-back verification: reconstruct
/// the engineering value the slave holds from the raw word read back from a
/// target (write) register. Mirrors `decode_read_register` but operates on a
/// `WriteRegister` (the target registers are write registers, not read ones).
fn decode_write_register(raw: u16, register: &WriteRegister) -> f64 {
    round2(raw as f64 * register.scale + register.offset)
}

async fn read_holding_register(
    client: &mut tokio_modbus::client::Context,
    address: u16,
) -> Result<u16> {
    let words = client
        .read_holding_registers(address, 1)
        .await
        .map_err(|err| anyhow!("modbus RTU read register {address} failed: {err}"))?
        .map_err(|code| anyhow!("modbus RTU read register {address} exception: {code:?}"))?;
    words
        .first()
        .copied()
        .ok_or_else(|| anyhow!("modbus RTU read register {address} returned no data"))
}

async fn write_single_register(
    client: &mut tokio_modbus::client::Context,
    address: u16,
    value: u16,
) -> Result<()> {
    client
        .write_single_register(address, value)
        .await
        .map_err(|err| anyhow!("modbus RTU write register {address} failed: {err}"))?
        .map_err(|code| anyhow!("modbus RTU write register {address} exception: {code:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_ack_unverified_marks_status_without_applied_targets() {
        let ack = CommandAck::unverified("req-unverified");
        assert_eq!(ack.request_id, "req-unverified");
        assert!(matches!(ack.status, AckStatus::Unverified));
        assert!(
            ack.accepted_targets.is_none(),
            "unverified fallback must not fabricate applied targets"
        );
    }

    #[tokio::test]
    async fn default_write_targets_acknowledged_falls_back_to_unverified() {
        // A device that only implements write_targets (no handshake override)
        // must fall back to the default: write Ok -> Unverified, preserving the
        // legacy fire-and-forget path until a real ACK exchange is implemented.
        struct LegacyDevice;
        #[async_trait::async_trait]
        impl ReactorDevice for LegacyDevice {
            async fn read_sample(&self) -> Result<SensorSnapshot> {
                Err(anyhow!("not used in this test"))
            }
            async fn write_targets(&self, _command: &SafeCommand) -> Result<()> {
                Ok(())
            }
        }
        let dev = LegacyDevice;
        let command = SafeCommand {
            target_temperature_c: 50.0,
            heat_time_s: 300.0,
            hold_time_s: 600.0,
            cool_time_s: 180.0,
            target_stirrer_rpm: 300.0,
            target_shake_speed_cpm: 30.0,
            target_pressure_mpa: 0.5,
            reason: "test".to_string(),
        };
        let ack = dev
            .write_targets_acknowledged(&command, "req-legacy", Duration::from_millis(50))
            .await
            .unwrap();
        assert_eq!(ack.request_id, "req-legacy");
        assert!(matches!(ack.status, AckStatus::Unverified));
        assert!(ack.accepted_targets.is_none());
    }

    #[test]
    fn decode_write_register_inverts_encode_formula() {
        // decode_write_register must reconstruct the engineering value from the
        // raw word using the SAME scale/offset as encode (the inverse formula),
        // because the handshake compares the raw read back against the encoded
        // raw and reports the held engineering value via this decode.
        let register = WriteRegister {
            address: 200,
            scale: 2.0,
            offset: 10.0,
        };
        assert_eq!(decode_write_register(5, &register), 20.0); // 5 * 2.0 + 10.0
        assert_eq!(decode_write_register(0, &register), 10.0); // offset only
    }

    #[test]
    fn encode_decode_write_register_round_trips_to_same_raw() {
        // Read-back raw comparison is sound only if encode is idempotent through
        // decode_write_register: encode -> decode -> encode must yield the same
        // raw, so a faithful slave (stores exactly what we wrote) always matches
        // and a clamp/overwrite is always detected.
        let register = WriteRegister {
            address: 100,
            scale: 0.1,
            offset: 0.0,
        };
        for value in [0.0, 50.0, 64.25, 99.9, 150.0] {
            let raw = encode_write_register(value, &register)
                .unwrap_or_else(|err| panic!("encode({value}) failed: {err}"));
            let decoded = decode_write_register(raw, &register);
            let raw_again = encode_write_register(decoded, &register)
                .unwrap_or_else(|err| panic!("re-encode({decoded}) failed: {err}"));
            assert_eq!(
                raw, raw_again,
                "value {value}: encode->decode->encode changed raw ({raw} != {raw_again})"
            );
        }
    }

    #[test]
    fn tokio_modbus_serial_config_maps_standard_rtu_settings() {
        assert_eq!(tokio_serial_parity("N").unwrap(), TokioParity::None);
        assert_eq!(tokio_serial_parity("E").unwrap(), TokioParity::Even);
        assert_eq!(tokio_serial_parity("O").unwrap(), TokioParity::Odd);
        assert_eq!(tokio_serial_stop_bits(1).unwrap(), TokioStopBits::One);
        assert_eq!(tokio_serial_stop_bits(2).unwrap(), TokioStopBits::Two);
        assert_eq!(tokio_serial_data_bits(8).unwrap(), TokioDataBits::Eight);
    }

    #[test]
    fn tokio_modbus_serial_config_rejects_unsupported_rtu_settings() {
        assert!(tokio_serial_parity("M").is_err());
        assert!(tokio_serial_stop_bits(3).is_err());
        assert!(tokio_serial_data_bits(9).is_err());
    }
}
