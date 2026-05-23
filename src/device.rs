use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};

use anyhow::{anyhow, Result};
use chrono::Utc;
use serialport::{DataBits, Parity, SerialPort, StopBits};

use crate::{
    config::{DeviceConfig, DeviceMode, ReadRegister, WriteRegister},
    control::SafeCommand,
    state::{fit_tilt_angle_deg, SensorSnapshot},
};

#[async_trait::async_trait]
pub trait ReactorDevice: Send + Sync {
    async fn read_sample(&self) -> Result<SensorSnapshot>;
    async fn write_targets(&self, command: &SafeCommand) -> Result<()>;
}

pub type SharedDevice = Arc<dyn ReactorDevice>;

pub fn build_device(config: &DeviceConfig) -> Result<SharedDevice> {
    match config.mode {
        DeviceMode::Pipeline => Ok(Arc::new(PipelineDevice)),
        DeviceMode::Modbus => Ok(Arc::new(ModbusRtuDevice::new(config.clone())?)),
        DeviceMode::Esp32Serial => Ok(Arc::new(Esp32SerialDevice::new(config.clone())?)),
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
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
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
