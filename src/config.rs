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
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceMode {
    Pipeline,
    Modbus,
    Esp32Serial,
    JsonBridge,
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
    #[serde(default = "default_control_write_retry_backoff_ms")]
    pub write_retry_backoff_ms: u64,
    #[serde(default = "default_safety_guard_timeout_ms")]
    pub safety_guard_timeout_ms: u64,
    #[serde(default = "default_ai_stop_product_concentration_percent")]
    pub ai_stop_product_concentration_percent: f64,
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
    toml::from_str(&raw)
        .with_context(|| format!("failed to parse device config {}", path.display()))
}

pub fn load_safety_config(path: impl AsRef<Path>) -> Result<SafetyConfig> {
    let path = path.as_ref();
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read safety config {}", path.display()))?;
    toml::from_str(&raw)
        .with_context(|| format!("failed to parse safety config {}", path.display()))
}
