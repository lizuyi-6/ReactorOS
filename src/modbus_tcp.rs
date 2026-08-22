use std::{collections::BTreeMap, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpListener,
    sync::RwLock,
};
use tokio_rustls::TlsAcceptor;

use crate::{
    api::{alarms_for, apply_modbus_register_write, unfinished_batch_status, AppState},
    config::{RegistersConfig, WriteRegister},
    db::{AuditActor, SYSTEM_AUDIT_ACTOR},
    state::{downstream_command_fault_reason, timestamp_is_fresh, RuntimeState},
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModbusTcpConfig {
    pub enabled: bool,
    pub bind: String,
    pub unit_id: u8,
    pub require_tls: bool,
    pub tls_cert: Option<PathBuf>,
    pub tls_key: Option<PathBuf>,
    pub max_pdu_bytes: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModbusTcpStatus {
    pub enabled: bool,
    pub listening: bool,
    pub bind: String,
    pub unit_id: u8,
    pub require_tls: bool,
    pub tls_status: &'static str,
    pub last_error: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy)]
struct ModbusException {
    function: u8,
    code: u8,
}

impl Default for ModbusTcpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: "0.0.0.0:502".to_string(),
            unit_id: 1,
            require_tls: true,
            tls_cert: None,
            tls_key: None,
            max_pdu_bytes: 253,
        }
    }
}

impl ModbusTcpStatus {
    fn from_config(config: &ModbusTcpConfig) -> Self {
        let tls_configured =
            crate::tls::paired_paths(&config.tls_cert, &config.tls_key, "Modbus TCP TLS")
                .is_ok_and(|paths| paths.is_some());
        Self {
            enabled: config.enabled,
            listening: false,
            bind: config.bind.clone(),
            unit_id: config.unit_id,
            require_tls: config.require_tls,
            tls_status: if config.require_tls {
                if tls_configured {
                    "configured"
                } else {
                    "missing_certificate"
                }
            } else {
                "disabled"
            },
            last_error: None,
            updated_at: Utc::now(),
        }
    }
}

pub fn validate_modbus_tcp_config(config: &ModbusTcpConfig) -> Result<()> {
    if !config.enabled {
        return Ok(());
    }
    config
        .bind
        .parse::<SocketAddr>()
        .with_context(|| format!("Modbus TCP bind address is invalid: {}", config.bind))?;
    if config.unit_id == 0 {
        anyhow::bail!("Modbus TCP unit_id must be between 1 and 247");
    }
    if !(1..=253).contains(&config.max_pdu_bytes) {
        anyhow::bail!("Modbus TCP max_pdu_bytes must be between 1 and 253");
    }
    if config.require_tls {
        let Some((cert_path, key_path)) =
            crate::tls::paired_paths(&config.tls_cert, &config.tls_key, "Modbus TCP TLS")?
        else {
            anyhow::bail!("Modbus TCP TLS is required but tls_cert/tls_key are not configured");
        };
        let _ = crate::tls::load_cert_chain(&cert_path)?;
        let _ = crate::tls::load_private_key(&key_path)?;
    } else {
        let _ = crate::tls::paired_paths(&config.tls_cert, &config.tls_key, "Modbus TCP TLS")?;
    }
    Ok(())
}

type SharedModbusTcpStatus = Arc<RwLock<ModbusTcpStatus>>;

static MODBUS_TCP_STATUS: std::sync::OnceLock<SharedModbusTcpStatus> = std::sync::OnceLock::new();

fn status_handle() -> SharedModbusTcpStatus {
    MODBUS_TCP_STATUS
        .get_or_init(|| {
            Arc::new(RwLock::new(ModbusTcpStatus::from_config(
                &ModbusTcpConfig::default(),
            )))
        })
        .clone()
}

pub async fn modbus_tcp_status_snapshot() -> ModbusTcpStatus {
    status_handle().read().await.clone()
}

pub fn start_modbus_tcp_server(config: ModbusTcpConfig, state: AppState) {
    let status = status_handle();
    tokio::spawn(async move {
        set_status(&status, ModbusTcpStatus::from_config(&config)).await;
        if !config.enabled {
            tracing::info!("Modbus TCP server disabled");
            return;
        }
        if let Err(err) = run_modbus_tcp_server(config, state, status.clone()).await {
            update_status(&status, |snapshot| {
                snapshot.listening = false;
                snapshot.last_error = Some(err.to_string());
            })
            .await;
            tracing::warn!("Modbus TCP server stopped: {err}");
        }
    });
}

async fn run_modbus_tcp_server(
    config: ModbusTcpConfig,
    state: AppState,
    status: SharedModbusTcpStatus,
) -> Result<()> {
    let listener = TcpListener::bind(&config.bind)
        .await
        .with_context(|| format!("failed to bind Modbus TCP {}", config.bind))?;
    let tls_acceptor = if config.require_tls {
        Some(build_tls_acceptor(&config)?)
    } else {
        None
    };
    update_status(&status, |snapshot| {
        snapshot.listening = true;
        snapshot.last_error = None;
        snapshot.tls_status = if config.require_tls {
            "active"
        } else {
            "disabled"
        };
    })
    .await;
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            // Transient accept failures (EMFILE, momentary fd pressure) must
            // not kill the server: log, back off briefly, and keep accepting.
            Err(err) => {
                tracing::warn!("Modbus TCP accept failed (continuing): {err}");
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
        };
        let state = state.clone();
        let max_pdu_bytes = config.max_pdu_bytes.max(1);
        let expected_unit_id = config.unit_id;
        if let Some(acceptor) = tls_acceptor.clone() {
            tokio::spawn(async move {
                let result = async {
                    let stream = acceptor.accept(stream).await?;
                    handle_modbus_tcp_stream(stream, state, max_pdu_bytes, expected_unit_id).await
                }
                .await;
                if let Err(err) = result {
                    tracing::warn!("Modbus TCP TLS client {peer} disconnected: {err}");
                }
            });
        } else {
            tokio::spawn(async move {
                if let Err(err) =
                    handle_modbus_tcp_stream(stream, state, max_pdu_bytes, expected_unit_id).await
                {
                    tracing::warn!("Modbus TCP client {peer} disconnected: {err}");
                }
            });
        }
    }
}

fn build_tls_acceptor(config: &ModbusTcpConfig) -> Result<TlsAcceptor> {
    crate::tls::install_rustls_provider();
    let Some((cert_path, key_path)) =
        crate::tls::paired_paths(&config.tls_cert, &config.tls_key, "Modbus TCP TLS")?
    else {
        anyhow::bail!("Modbus TCP TLS is required but tls_cert/tls_key are not configured");
    };
    let certs = crate::tls::load_cert_chain(&cert_path)?;
    let key = crate::tls::load_private_key(&key_path)?;
    let server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("failed to build Modbus TCP TLS server config")?;
    Ok(TlsAcceptor::from(Arc::new(server_config)))
}

pub async fn handle_modbus_tcp_stream<S>(
    mut stream: S,
    state: AppState,
    max_pdu_bytes: usize,
    expected_unit_id: u8,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    loop {
        let mut header = [0_u8; 7];
        stream.read_exact(&mut header).await?;
        let transaction_id = u16::from_be_bytes([header[0], header[1]]);
        let protocol_id = u16::from_be_bytes([header[2], header[3]]);
        let length = u16::from_be_bytes([header[4], header[5]]) as usize;
        let unit_id = header[6];
        if protocol_id != 0 || length == 0 || length - 1 > max_pdu_bytes {
            let response = exception_response(0, 0x03);
            write_mbap_response(&mut stream, transaction_id, unit_id, &response).await?;
            continue;
        }
        let mut pdu = vec![0_u8; length - 1];
        stream.read_exact(&mut pdu).await?;
        if unit_id != expected_unit_id {
            let function = pdu.first().copied().unwrap_or(0);
            let response = exception_response(function, 0x0B);
            write_mbap_response(&mut stream, transaction_id, unit_id, &response).await?;
            continue;
        }
        let response = handle_modbus_tcp_pdu(&state, &pdu).await;
        write_mbap_response(&mut stream, transaction_id, unit_id, &response).await?;
    }
}

async fn write_mbap_response(
    stream: &mut (impl AsyncWrite + Unpin),
    transaction_id: u16,
    unit_id: u8,
    pdu: &[u8],
) -> Result<()> {
    let mut header = Vec::with_capacity(7 + pdu.len());
    header.extend_from_slice(&transaction_id.to_be_bytes());
    header.extend_from_slice(&0_u16.to_be_bytes());
    header.extend_from_slice(&((pdu.len() + 1) as u16).to_be_bytes());
    header.push(unit_id);
    header.extend_from_slice(pdu);
    stream.write_all(&header).await?;
    Ok(())
}

pub async fn handle_modbus_tcp_pdu(state: &AppState, pdu: &[u8]) -> Vec<u8> {
    if pdu.is_empty() {
        return exception_response(0, 0x03);
    }
    let result = match pdu[0] {
        0x01 => {
            let runtime = state.runtime.read().await.clone();
            match unfinished_batch_status(state, &runtime).await {
                Ok(batch_status) => read_bool_points(
                    pdu,
                    coil_values(&runtime, batch_status.has_unfinished_batch(&runtime)),
                ),
                Err(_) => Err(exception(pdu[0], 0x04)),
            }
        }
        0x02 => {
            let runtime = state.runtime.read().await.clone();
            match unfinished_batch_status(state, &runtime).await {
                Ok(batch_status) => read_bool_points(
                    pdu,
                    discrete_input_values(
                        state,
                        &runtime,
                        batch_status.has_unfinished_batch(&runtime),
                        batch_status.recovery_required(),
                    ),
                ),
                Err(_) => Err(exception(pdu[0], 0x04)),
            }
        }
        0x03 => read_holding_registers(state, pdu).await,
        0x06 => write_single_register(state, pdu).await,
        function => Err(ModbusException {
            function,
            code: 0x01,
        }),
    };
    result.unwrap_or_else(|err| exception_response(err.function, err.code))
}

async fn read_holding_registers(state: &AppState, pdu: &[u8]) -> Result<Vec<u8>, ModbusException> {
    if pdu.len() != 5 {
        return Err(exception(pdu[0], 0x03));
    }
    let start = u16::from_be_bytes([pdu[1], pdu[2]]);
    let quantity = u16::from_be_bytes([pdu[3], pdu[4]]);
    if quantity == 0 || quantity > 125 {
        return Err(exception(pdu[0], 0x03));
    }
    let runtime = state.runtime.read().await;
    let values = holding_register_values(state, &runtime)?;
    let mut response = Vec::with_capacity(2 + quantity as usize * 2);
    response.push(0x03);
    response.push((quantity * 2) as u8);
    for address in start..start + quantity {
        let Some(raw) = values.get(&address) else {
            return Err(exception(pdu[0], 0x02));
        };
        response.extend_from_slice(&raw.to_be_bytes());
    }
    Ok(response)
}

async fn write_single_register(state: &AppState, pdu: &[u8]) -> Result<Vec<u8>, ModbusException> {
    if pdu.len() != 5 {
        return Err(exception(pdu[0], 0x03));
    }
    let address = u16::from_be_bytes([pdu[1], pdu[2]]);
    let raw = u16::from_be_bytes([pdu[3], pdu[4]]);
    let Some((name, register)) =
        write_register_by_address(&state.device_config.modbus.registers, address)
    else {
        return Err(exception(pdu[0], 0x02));
    };
    let value = raw as f64 * register.scale + register.offset;
    apply_modbus_register_write(
        state,
        name,
        value,
        Some(format!("modbus tcp write register {name}")),
        &AuditActor::new("modbus-tcp", SYSTEM_AUDIT_ACTOR),
    )
    .await
    .map_err(|err| {
        if err.status_code().is_server_error() {
            exception(pdu[0], 0x04)
        } else {
            exception(pdu[0], 0x03)
        }
    })?;
    Ok(pdu.to_vec())
}

fn read_bool_points(pdu: &[u8], values: BTreeMap<u16, bool>) -> Result<Vec<u8>, ModbusException> {
    if pdu.len() != 5 {
        return Err(exception(pdu[0], 0x03));
    }
    let start = u16::from_be_bytes([pdu[1], pdu[2]]);
    let quantity = u16::from_be_bytes([pdu[3], pdu[4]]);
    if quantity == 0 || quantity > 2000 {
        return Err(exception(pdu[0], 0x03));
    }
    let byte_count = ((quantity as usize) + 7) / 8;
    let mut bytes = vec![0_u8; byte_count];
    for index in 0..quantity {
        let address = start + index;
        let Some(value) = values.get(&address) else {
            return Err(exception(pdu[0], 0x02));
        };
        if *value {
            bytes[index as usize / 8] |= 1 << (index % 8);
        }
    }
    let mut response = Vec::with_capacity(2 + bytes.len());
    response.push(pdu[0]);
    response.push(byte_count as u8);
    response.extend_from_slice(&bytes);
    Ok(response)
}

fn holding_register_values(
    state: &AppState,
    runtime: &RuntimeState,
) -> Result<BTreeMap<u16, u16>, ModbusException> {
    let registers = &state.device_config.modbus.registers;
    let Some(sample) = runtime.latest_sample.as_ref() else {
        return Err(exception(0x03, 0x04));
    };
    let mut values = BTreeMap::new();
    insert_raw(
        &mut values,
        registers.temperature_c.address,
        sample.temperature_c,
        registers.temperature_c.scale,
        registers.temperature_c.offset,
    )?;
    insert_raw(
        &mut values,
        registers.stirrer_rpm.address,
        sample.stirrer_rpm,
        registers.stirrer_rpm.scale,
        registers.stirrer_rpm.offset,
    )?;
    insert_raw(
        &mut values,
        registers.pressure_mpa.address,
        sample.pressure_mpa,
        registers.pressure_mpa.scale,
        registers.pressure_mpa.offset,
    )?;
    insert_raw(
        &mut values,
        registers.shake_speed_cpm.address,
        sample.shake_speed_cpm,
        registers.shake_speed_cpm.scale,
        registers.shake_speed_cpm.offset,
    )?;
    insert_raw(
        &mut values,
        registers.tilt_angle_deg.address,
        sample.tilt_angle_deg,
        registers.tilt_angle_deg.scale,
        registers.tilt_angle_deg.offset,
    )?;
    insert_raw(
        &mut values,
        registers.flow_rate_l_min.address,
        sample.flow_rate_l_min,
        registers.flow_rate_l_min.scale,
        registers.flow_rate_l_min.offset,
    )?;
    insert_raw(
        &mut values,
        registers.product_concentration_percent.address,
        sample.product_concentration_percent,
        registers.product_concentration_percent.scale,
        registers.product_concentration_percent.offset,
    )?;
    insert_raw(
        &mut values,
        registers.ph.address,
        sample.ph,
        registers.ph.scale,
        registers.ph.offset,
    )?;
    insert_raw(
        &mut values,
        registers.target_temperature_c.address,
        runtime.targets.temperature_c,
        registers.target_temperature_c.scale,
        registers.target_temperature_c.offset,
    )?;
    insert_raw(
        &mut values,
        registers.target_stirrer_rpm.address,
        runtime.targets.stirrer_rpm,
        registers.target_stirrer_rpm.scale,
        registers.target_stirrer_rpm.offset,
    )?;
    insert_raw(
        &mut values,
        registers.target_shake_speed_cpm.address,
        runtime.targets.shake_speed_cpm,
        registers.target_shake_speed_cpm.scale,
        registers.target_shake_speed_cpm.offset,
    )?;
    insert_raw(
        &mut values,
        registers.target_pressure_mpa.address,
        runtime.targets.target_pressure_mpa,
        registers.target_pressure_mpa.scale,
        registers.target_pressure_mpa.offset,
    )?;
    insert_raw(
        &mut values,
        registers.heat_time_s.address,
        runtime.targets.heat_time_s,
        registers.heat_time_s.scale,
        registers.heat_time_s.offset,
    )?;
    insert_raw(
        &mut values,
        registers.hold_time_s.address,
        runtime.targets.hold_time_s,
        registers.hold_time_s.scale,
        registers.hold_time_s.offset,
    )?;
    insert_raw(
        &mut values,
        registers.cool_time_s.address,
        runtime.targets.cool_time_s,
        registers.cool_time_s.scale,
        registers.cool_time_s.offset,
    )?;
    Ok(values)
}

fn insert_raw(
    values: &mut BTreeMap<u16, u16>,
    address: u16,
    value: f64,
    scale: f64,
    offset: f64,
) -> Result<(), ModbusException> {
    values.insert(address, encode_raw(value, scale, offset)?);
    Ok(())
}

fn encode_raw(value: f64, scale: f64, offset: f64) -> Result<u16, ModbusException> {
    if scale == 0.0 {
        return Err(exception(0x03, 0x04));
    }
    let raw = ((value - offset) / scale).round();
    if !(0.0..=u16::MAX as f64).contains(&raw) {
        return Err(exception(0x03, 0x04));
    }
    Ok(raw as u16)
}

fn coil_values(runtime: &RuntimeState, has_unfinished_batch: bool) -> BTreeMap<u16, bool> {
    BTreeMap::from([
        (0, runtime.auto_enabled),
        (1, runtime.manual_lock),
        (2, runtime.emergency_stop),
        (3, has_unfinished_batch),
    ])
}

fn discrete_input_values(
    state: &AppState,
    runtime: &RuntimeState,
    has_unfinished_batch: bool,
    batch_recovery_required: bool,
) -> BTreeMap<u16, bool> {
    let sensor_fresh = runtime
        .latest_sample
        .as_ref()
        .map(|sample| {
            timestamp_is_fresh(sample.captured_at, state.safety.control.sensor_timeout_ms)
        })
        .unwrap_or(false);
    let device_connected = runtime
        .device_status
        .as_ref()
        .map(|device| {
            device.connected
                && device.last_frame_ok
                && downstream_command_fault_reason(device).is_none()
                && device
                    .last_seen_at
                    .as_ref()
                    .map(|last_seen| {
                        timestamp_is_fresh(*last_seen, state.safety.control.sensor_timeout_ms)
                    })
                    .unwrap_or(false)
        })
        .unwrap_or_else(|| !state.safety.control.require_device_status_for_control && sensor_fresh);
    BTreeMap::from([
        (0, device_connected && !batch_recovery_required),
        (1, sensor_fresh),
        (
            2,
            batch_recovery_required
                || !alarms_for(
                    state.safety.as_ref(),
                    runtime,
                    runtime.latest_sample.as_ref(),
                    state.ai_memory.as_ref(),
                )
                .is_empty(),
        ),
        (
            3,
            runtime
                .latest_sample
                .as_ref()
                .map(|sample| sample.tilt_state != 0)
                .unwrap_or(false),
        ),
        (4, has_unfinished_batch),
    ])
}

fn write_register_by_address(
    registers: &RegistersConfig,
    address: u16,
) -> Option<(&'static str, &WriteRegister)> {
    [
        ("target_temperature_c", &registers.target_temperature_c),
        ("target_stirrer_rpm", &registers.target_stirrer_rpm),
        ("target_shake_speed_cpm", &registers.target_shake_speed_cpm),
        ("target_pressure_mpa", &registers.target_pressure_mpa),
        ("heat_time_s", &registers.heat_time_s),
        ("hold_time_s", &registers.hold_time_s),
        ("cool_time_s", &registers.cool_time_s),
    ]
    .into_iter()
    .find(|(_, register)| register.address == address)
}

fn exception(function: u8, code: u8) -> ModbusException {
    ModbusException { function, code }
}

fn exception_response(function: u8, code: u8) -> Vec<u8> {
    vec![function | 0x80, code]
}

async fn set_status(status: &SharedModbusTcpStatus, next: ModbusTcpStatus) {
    *status.write().await = next;
}

async fn update_status(status: &SharedModbusTcpStatus, update: impl FnOnce(&mut ModbusTcpStatus)) {
    let mut snapshot = status.write().await;
    update(&mut snapshot);
    snapshot.updated_at = Utc::now();
}
