use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceConfig {
    pub mode: DeviceMode,
    pub serial: SerialConfig,
    pub modbus: ModbusConfig,
    pub esp32: Esp32Config,
    pub simulator: SimulatorConfig,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeviceMode {
    Simulator,
    Modbus,
    Esp32Serial,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SerialConfig {
    pub port: String,
    pub baudrate: u32,
    pub parity: String,
    pub stopbits: u8,
    pub bytesize: u8,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModbusConfig {
    pub slave_id: u8,
    pub registers: RegistersConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Esp32Config {
    pub frame_prefix: String,
    pub command_prefix: String,
    pub checksum: bool,
    pub max_line_bytes: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegistersConfig {
    pub temperature_c: ReadRegister,
    pub stirrer_rpm: ReadRegister,
    pub target_temperature_c: WriteRegister,
    pub target_stirrer_rpm: WriteRegister,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReadRegister {
    pub address: u16,
    pub scale: f64,
    pub offset: f64,
    pub min_valid: f64,
    pub max_valid: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WriteRegister {
    pub address: u16,
    pub scale: f64,
    pub offset: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SimulatorConfig {
    pub initial_temperature_c: f64,
    pub initial_stirrer_rpm: f64,
    pub ambient_temperature_c: f64,
    pub heating_response: f64,
    pub cooling_response: f64,
    pub stirrer_response: f64,
    pub noise: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SafetyConfig {
    pub control: ControlConfig,
    pub temperature: TemperatureSafety,
    pub stirrer: StirrerSafety,
    pub optimizer: OptimizerBounds,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ControlConfig {
    pub auto_enabled_default: bool,
    pub manual_lock_default: bool,
    pub control_interval_ms: u64,
    pub sensor_timeout_ms: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TemperatureSafety {
    pub min_c: f64,
    pub max_c: f64,
    pub max_step_c: f64,
    pub default_target_c: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StirrerSafety {
    pub min_rpm: f64,
    pub max_rpm: f64,
    pub max_step_rpm: f64,
    pub default_target_rpm: f64,
}

#[derive(Debug, Clone, Deserialize)]
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
