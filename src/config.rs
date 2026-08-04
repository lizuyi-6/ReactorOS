use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DeviceConfig {
    pub mode: DeviceMode,
    pub serial: SerialConfig,
    pub modbus: ModbusConfig,
    pub esp32: Esp32Config,
    #[serde(default)]
    pub json_bridge: JsonBridgeConfig,
    #[serde(default)]
    pub simulation: crate::virtual_sensor::SimulationConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceMode {
    Pipeline,
    Modbus,
    Esp32Serial,
    JsonBridge,
    Simulation,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SerialConfig {
    pub port: String,
    pub baudrate: u32,
    pub parity: String,
    pub stopbits: u8,
    pub bytesize: u8,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModbusConfig {
    pub slave_id: u8,
    pub registers: RegistersConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Esp32Config {
    pub frame_prefix: String,
    pub command_prefix: String,
    pub checksum: bool,
    pub max_line_bytes: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JsonBridgeConfig {
    pub state_path: PathBuf,
    pub control_path: PathBuf,
    pub max_state_age_ms: i64,
    pub request_id_prefix: String,
    pub speed_steps_per_cycle: f64,
    pub speed_deadband_cpm: f64,
    pub temperature_deadband_c: f64,
    pub relay_temperature_control: bool,
    #[serde(default)]
    pub adc: JsonBridgeAdcConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JsonBridgeAdcConfig {
    pub sensor: Option<JsonBridgeAdcSensor>,
    pub scale: f64,
    pub offset: f64,
    pub min_valid: f64,
    pub max_valid: f64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum JsonBridgeAdcSensor {
    TemperatureC,
    PressureMpa,
    StirrerRpm,
    ShakeSpeedCpm,
    FlowRateLMin,
    ProductConcentrationPercent,
    Ph,
}

impl Default for JsonBridgeConfig {
    fn default() -> Self {
        Self {
            state_path: PathBuf::from("/project/state.json"),
            control_path: PathBuf::from("/project/control.json"),
            max_state_age_ms: 6_000,
            request_id_prefix: "reactor-os".to_string(),
            speed_steps_per_cycle: 200.0,
            speed_deadband_cpm: 1.0,
            temperature_deadband_c: 1.0,
            relay_temperature_control: false,
            adc: JsonBridgeAdcConfig::default(),
        }
    }
}

impl Default for JsonBridgeAdcConfig {
    fn default() -> Self {
        Self {
            sensor: None,
            scale: 1.0,
            offset: 0.0,
            min_valid: 0.0,
            max_valid: 4095.0,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RegistersConfig {
    pub temperature_c: ReadRegister,
    pub stirrer_rpm: ReadRegister,
    #[serde(default = "default_pressure_mpa_register")]
    pub pressure_mpa: ReadRegister,
    #[serde(default = "default_shake_speed_cpm_register")]
    pub shake_speed_cpm: ReadRegister,
    #[serde(default = "default_tilt_angle_deg_register")]
    pub tilt_angle_deg: ReadRegister,
    #[serde(default = "default_flow_rate_l_min_register")]
    pub flow_rate_l_min: ReadRegister,
    #[serde(default = "default_product_concentration_percent_register")]
    pub product_concentration_percent: ReadRegister,
    #[serde(default = "default_ph_register")]
    pub ph: ReadRegister,
    pub target_temperature_c: WriteRegister,
    pub target_stirrer_rpm: WriteRegister,
    #[serde(default = "default_target_shake_speed_cpm_register")]
    pub target_shake_speed_cpm: WriteRegister,
    #[serde(default = "default_target_pressure_mpa_register")]
    pub target_pressure_mpa: WriteRegister,
    #[serde(default = "default_heat_time_s_register")]
    pub heat_time_s: WriteRegister,
    #[serde(default = "default_hold_time_s_register")]
    pub hold_time_s: WriteRegister,
    #[serde(default = "default_cool_time_s_register")]
    pub cool_time_s: WriteRegister,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReadRegister {
    pub address: u16,
    pub scale: f64,
    pub offset: f64,
    pub min_valid: f64,
    pub max_valid: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WriteRegister {
    pub address: u16,
    pub scale: f64,
    pub offset: f64,
}

fn read_register(
    address: u16,
    scale: f64,
    offset: f64,
    min_valid: f64,
    max_valid: f64,
) -> ReadRegister {
    ReadRegister {
        address,
        scale,
        offset,
        min_valid,
        max_valid,
    }
}

fn write_register(address: u16, scale: f64, offset: f64) -> WriteRegister {
    WriteRegister {
        address,
        scale,
        offset,
    }
}

fn default_pressure_mpa_register() -> ReadRegister {
    read_register(2, 0.01, 0.0, 0.0, 10.0)
}

fn default_shake_speed_cpm_register() -> ReadRegister {
    read_register(3, 1.0, 0.0, 0.0, 60.0)
}

fn default_tilt_angle_deg_register() -> ReadRegister {
    read_register(4, 0.01, -45.0, -45.0, 45.0)
}

fn default_flow_rate_l_min_register() -> ReadRegister {
    read_register(5, 0.01, 0.0, 0.0, 20.0)
}

fn default_product_concentration_percent_register() -> ReadRegister {
    read_register(6, 0.1, 0.0, 0.0, 100.0)
}

fn default_ph_register() -> ReadRegister {
    read_register(7, 0.01, 0.0, 0.0, 14.0)
}

fn default_target_shake_speed_cpm_register() -> WriteRegister {
    write_register(12, 1.0, 0.0)
}

fn default_target_pressure_mpa_register() -> WriteRegister {
    write_register(13, 0.01, 0.0)
}

fn default_heat_time_s_register() -> WriteRegister {
    write_register(14, 1.0, 0.0)
}

fn default_hold_time_s_register() -> WriteRegister {
    write_register(15, 1.0, 0.0)
}

fn default_cool_time_s_register() -> WriteRegister {
    write_register(16, 1.0, 0.0)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SafetyConfig {
    pub control: ControlConfig,
    pub temperature: TemperatureSafety,
    pub stirrer: StirrerSafety,
    pub optimizer: OptimizerBounds,
    #[serde(default)]
    pub forbidden_control_zones: Vec<ForbiddenControlZone>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ControlConfig {
    pub auto_enabled_default: bool,
    pub manual_lock_default: bool,
    pub control_interval_ms: u64,
    pub sensor_timeout_ms: i64,
    #[serde(default = "default_require_device_status_for_control")]
    pub require_device_status_for_control: bool,
    #[serde(default = "default_control_write_retry_backoff_ms")]
    pub write_retry_backoff_ms: u64,
    #[serde(default = "default_safety_guard_timeout_ms")]
    pub safety_guard_timeout_ms: u64,
    #[serde(default = "default_ai_stop_product_concentration_percent")]
    pub ai_stop_product_concentration_percent: f64,
    /// Require a command-level handshake (downstream ACK) before a write is
    /// treated as complete. Default false preserves the legacy
    /// fire-and-forget behaviour; production preflight should require true.
    #[serde(default = "default_require_command_ack")]
    pub require_command_ack: bool,
    #[serde(default = "default_command_ack_timeout_ms")]
    pub command_ack_timeout_ms: u64,
}

fn default_require_device_status_for_control() -> bool {
    true
}

fn default_control_write_retry_backoff_ms() -> u64 {
    5_000
}

fn default_safety_guard_timeout_ms() -> u64 {
    1_000
}

fn default_ai_stop_product_concentration_percent() -> f64 {
    95.0
}

fn default_require_command_ack() -> bool {
    // Legacy-compatible default. Production deployments should set this true
    // via safety.toml; xingshu ops preflight --production fails closed when it
    // is unset. See docs/command_ack_handshake.md.
    false
}

fn default_command_ack_timeout_ms() -> u64 {
    // Same order of magnitude as safety_guard_timeout_ms (1000), slightly
    // wider to tolerate a downstream ACK round-trip over a slow serial link.
    2_000
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TemperatureSafety {
    pub min_c: f64,
    pub max_c: f64,
    pub max_step_c: f64,
    pub default_target_c: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StirrerSafety {
    pub min_rpm: f64,
    pub max_rpm: f64,
    pub max_step_rpm: f64,
    pub default_target_rpm: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OptimizerBounds {
    pub min_temperature_c: f64,
    pub max_temperature_c: f64,
    pub min_stirrer_rpm: f64,
    pub max_stirrer_rpm: f64,
    pub min_heating_minutes: f64,
    pub max_heating_minutes: f64,
    pub min_stirring_minutes: f64,
    pub max_stirring_minutes: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ForbiddenControlZone {
    pub name: String,
    pub reason: String,
    pub min_temperature_c: f64,
    pub max_temperature_c: f64,
    pub min_stirrer_rpm: f64,
    pub max_stirrer_rpm: f64,
}

impl ForbiddenControlZone {
    pub fn contains(&self, temperature_c: f64, stirrer_rpm: f64) -> bool {
        temperature_c >= self.min_temperature_c
            && temperature_c <= self.max_temperature_c
            && stirrer_rpm >= self.min_stirrer_rpm
            && stirrer_rpm <= self.max_stirrer_rpm
    }
}

pub fn load_device_config(path: impl AsRef<Path>) -> Result<DeviceConfig> {
    let path = path.as_ref();
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read device config {}", path.display()))?;
    let config: DeviceConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse device config {}", path.display()))?;
    validate_device_config(&config)
        .with_context(|| format!("invalid device config {}", path.display()))?;
    Ok(config)
}

pub fn load_safety_config(path: impl AsRef<Path>) -> Result<SafetyConfig> {
    let path = path.as_ref();
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read safety config {}", path.display()))?;
    let config: SafetyConfig = toml::from_str(&raw)
        .with_context(|| format!("failed to parse safety config {}", path.display()))?;
    validate_safety_config(&config)
        .with_context(|| format!("invalid safety config {}", path.display()))?;
    Ok(config)
}

pub fn validate_device_config(config: &DeviceConfig) -> Result<()> {
    ensure_non_empty("serial.port", &config.serial.port)?;
    ensure_one_of(
        "serial.parity",
        &config.serial.parity,
        &["N", "n", "E", "e", "O", "o"],
    )?;
    ensure_in_u8("serial.stopbits", config.serial.stopbits, 1, 2)?;
    ensure_in_u8("serial.bytesize", config.serial.bytesize, 5, 8)?;
    ensure_positive_u64("serial.timeout_ms", config.serial.timeout_ms)?;
    ensure_positive_usize("esp32.max_line_bytes", config.esp32.max_line_bytes)?;
    ensure_positive_i64(
        "json_bridge.max_state_age_ms",
        config.json_bridge.max_state_age_ms,
    )?;
    ensure_positive_f64(
        "json_bridge.speed_steps_per_cycle",
        config.json_bridge.speed_steps_per_cycle,
    )?;
    ensure_non_negative_f64(
        "json_bridge.speed_deadband_cpm",
        config.json_bridge.speed_deadband_cpm,
    )?;
    ensure_non_negative_f64(
        "json_bridge.temperature_deadband_c",
        config.json_bridge.temperature_deadband_c,
    )?;
    validate_adc_config(&config.json_bridge.adc)?;
    validate_registers_config(&config.modbus.registers)?;
    Ok(())
}

pub fn validate_safety_config(config: &SafetyConfig) -> Result<()> {
    ensure_positive_u64(
        "control.control_interval_ms",
        config.control.control_interval_ms,
    )?;
    ensure_positive_i64(
        "control.sensor_timeout_ms",
        config.control.sensor_timeout_ms,
    )?;
    ensure_positive_u64(
        "control.write_retry_backoff_ms",
        config.control.write_retry_backoff_ms,
    )?;
    ensure_positive_u64(
        "control.safety_guard_timeout_ms",
        config.control.safety_guard_timeout_ms,
    )?;
    ensure_f64_range(
        "control.ai_stop_product_concentration_percent",
        config.control.ai_stop_product_concentration_percent,
        0.0,
        100.0,
    )?;
    ensure_positive_u64(
        "control.command_ack_timeout_ms",
        config.control.command_ack_timeout_ms,
    )?;
    ensure_ordered_f64(
        "temperature.min_c",
        config.temperature.min_c,
        "temperature.max_c",
        config.temperature.max_c,
    )?;
    ensure_non_negative_f64("temperature.max_step_c", config.temperature.max_step_c)?;
    ensure_f64_range(
        "temperature.default_target_c",
        config.temperature.default_target_c,
        config.temperature.min_c,
        config.temperature.max_c,
    )?;
    ensure_ordered_f64(
        "stirrer.min_rpm",
        config.stirrer.min_rpm,
        "stirrer.max_rpm",
        config.stirrer.max_rpm,
    )?;
    ensure_non_negative_f64("stirrer.max_step_rpm", config.stirrer.max_step_rpm)?;
    ensure_f64_range(
        "stirrer.default_target_rpm",
        config.stirrer.default_target_rpm,
        config.stirrer.min_rpm,
        config.stirrer.max_rpm,
    )?;
    validate_optimizer_bounds(&config.optimizer)?;
    for zone in &config.forbidden_control_zones {
        validate_forbidden_zone(zone)?;
    }
    Ok(())
}

fn validate_registers_config(registers: &RegistersConfig) -> Result<()> {
    validate_read_register("modbus.registers.temperature_c", &registers.temperature_c)?;
    validate_read_register("modbus.registers.stirrer_rpm", &registers.stirrer_rpm)?;
    validate_read_register("modbus.registers.pressure_mpa", &registers.pressure_mpa)?;
    validate_read_register(
        "modbus.registers.shake_speed_cpm",
        &registers.shake_speed_cpm,
    )?;
    validate_read_register("modbus.registers.tilt_angle_deg", &registers.tilt_angle_deg)?;
    validate_read_register(
        "modbus.registers.flow_rate_l_min",
        &registers.flow_rate_l_min,
    )?;
    validate_read_register(
        "modbus.registers.product_concentration_percent",
        &registers.product_concentration_percent,
    )?;
    validate_read_register("modbus.registers.ph", &registers.ph)?;
    validate_write_register(
        "modbus.registers.target_temperature_c",
        &registers.target_temperature_c,
    )?;
    validate_write_register(
        "modbus.registers.target_stirrer_rpm",
        &registers.target_stirrer_rpm,
    )?;
    validate_write_register(
        "modbus.registers.target_shake_speed_cpm",
        &registers.target_shake_speed_cpm,
    )?;
    validate_write_register(
        "modbus.registers.target_pressure_mpa",
        &registers.target_pressure_mpa,
    )?;
    validate_write_register("modbus.registers.heat_time_s", &registers.heat_time_s)?;
    validate_write_register("modbus.registers.hold_time_s", &registers.hold_time_s)?;
    validate_write_register("modbus.registers.cool_time_s", &registers.cool_time_s)?;
    Ok(())
}

fn validate_read_register(name: &str, register: &ReadRegister) -> Result<()> {
    ensure_non_zero_f64(&format!("{name}.scale"), register.scale)?;
    ensure_finite(&format!("{name}.offset"), register.offset)?;
    ensure_ordered_f64(
        &format!("{name}.min_valid"),
        register.min_valid,
        &format!("{name}.max_valid"),
        register.max_valid,
    )
}

fn validate_write_register(name: &str, register: &WriteRegister) -> Result<()> {
    ensure_non_zero_f64(&format!("{name}.scale"), register.scale)?;
    ensure_finite(&format!("{name}.offset"), register.offset)
}

fn validate_adc_config(config: &JsonBridgeAdcConfig) -> Result<()> {
    ensure_non_zero_f64("json_bridge.adc.scale", config.scale)?;
    ensure_finite("json_bridge.adc.offset", config.offset)?;
    ensure_ordered_f64(
        "json_bridge.adc.min_valid",
        config.min_valid,
        "json_bridge.adc.max_valid",
        config.max_valid,
    )
}

fn validate_optimizer_bounds(bounds: &OptimizerBounds) -> Result<()> {
    ensure_ordered_f64(
        "optimizer.min_temperature_c",
        bounds.min_temperature_c,
        "optimizer.max_temperature_c",
        bounds.max_temperature_c,
    )?;
    ensure_ordered_f64(
        "optimizer.min_stirrer_rpm",
        bounds.min_stirrer_rpm,
        "optimizer.max_stirrer_rpm",
        bounds.max_stirrer_rpm,
    )?;
    ensure_ordered_f64(
        "optimizer.min_heating_minutes",
        bounds.min_heating_minutes,
        "optimizer.max_heating_minutes",
        bounds.max_heating_minutes,
    )?;
    ensure_ordered_f64(
        "optimizer.min_stirring_minutes",
        bounds.min_stirring_minutes,
        "optimizer.max_stirring_minutes",
        bounds.max_stirring_minutes,
    )?;
    ensure_non_negative_f64("optimizer.min_heating_minutes", bounds.min_heating_minutes)?;
    ensure_non_negative_f64(
        "optimizer.min_stirring_minutes",
        bounds.min_stirring_minutes,
    )
}

fn validate_forbidden_zone(zone: &ForbiddenControlZone) -> Result<()> {
    ensure_non_empty("forbidden_control_zones.name", &zone.name)?;
    ensure_ordered_f64(
        "forbidden_control_zones.min_temperature_c",
        zone.min_temperature_c,
        "forbidden_control_zones.max_temperature_c",
        zone.max_temperature_c,
    )?;
    ensure_ordered_f64(
        "forbidden_control_zones.min_stirrer_rpm",
        zone.min_stirrer_rpm,
        "forbidden_control_zones.max_stirrer_rpm",
        zone.max_stirrer_rpm,
    )
}

fn ensure_non_empty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        anyhow::bail!("{field} must not be empty");
    }
    Ok(())
}

fn ensure_one_of(field: &str, value: &str, allowed: &[&str]) -> Result<()> {
    if !allowed.contains(&value) {
        anyhow::bail!("{field} has unsupported value {value}");
    }
    Ok(())
}

fn ensure_in_u8(field: &str, value: u8, min: u8, max: u8) -> Result<()> {
    if !(min..=max).contains(&value) {
        anyhow::bail!("{field} must be between {min} and {max}");
    }
    Ok(())
}

fn ensure_positive_u64(field: &str, value: u64) -> Result<()> {
    if value == 0 {
        anyhow::bail!("{field} must be greater than 0");
    }
    Ok(())
}

fn ensure_positive_usize(field: &str, value: usize) -> Result<()> {
    if value == 0 {
        anyhow::bail!("{field} must be greater than 0");
    }
    Ok(())
}

fn ensure_positive_i64(field: &str, value: i64) -> Result<()> {
    if value <= 0 {
        anyhow::bail!("{field} must be greater than 0");
    }
    Ok(())
}

fn ensure_finite(field: &str, value: f64) -> Result<()> {
    if !value.is_finite() {
        anyhow::bail!("{field} must be finite");
    }
    Ok(())
}

fn ensure_non_zero_f64(field: &str, value: f64) -> Result<()> {
    ensure_finite(field, value)?;
    if value == 0.0 {
        anyhow::bail!("{field} must not be zero");
    }
    Ok(())
}

fn ensure_positive_f64(field: &str, value: f64) -> Result<()> {
    ensure_finite(field, value)?;
    if value <= 0.0 {
        anyhow::bail!("{field} must be greater than 0");
    }
    Ok(())
}

fn ensure_non_negative_f64(field: &str, value: f64) -> Result<()> {
    ensure_finite(field, value)?;
    if value < 0.0 {
        anyhow::bail!("{field} must be greater than or equal to 0");
    }
    Ok(())
}

fn ensure_ordered_f64(min_field: &str, min: f64, max_field: &str, max: f64) -> Result<()> {
    ensure_finite(min_field, min)?;
    ensure_finite(max_field, max)?;
    if min > max {
        anyhow::bail!("{min_field} must be less than or equal to {max_field}");
    }
    Ok(())
}

fn ensure_f64_range(field: &str, value: f64, min: f64, max: f64) -> Result<()> {
    ensure_finite(field, value)?;
    if !(min..=max).contains(&value) {
        anyhow::bail!("{field} must be between {min} and {max}");
    }
    Ok(())
}
