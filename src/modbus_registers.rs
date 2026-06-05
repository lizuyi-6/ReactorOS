use chrono::{Duration, Utc};
use serde_json::{json, Value};

use super::{ensure_targets_allowed, AppError, AppState};
use crate::{
    config::{RegistersConfig, WriteRegister},
    control::{clamp_operator_targets, SafeCommand},
    modbus_tcp::ModbusTcpStatus,
    number::round2,
    state::{ControlTargets, RuntimeState},
};

struct ModbusRegisterValue {
    address: u16,
    access: &'static str,
    value: f64,
    scale: f64,
    offset: f64,
    source: &'static str,
}

pub(super) fn registers_payload(
    state: &AppState,
    runtime: &RuntimeState,
    tcp_status: &ModbusTcpStatus,
) -> Value {
    json!({
        "device_id": "reactor_001",
        "mode": state.device_mode,
        "slave_id": state.device_config.modbus.slave_id,
        "serial": state.device_config.serial,
        "tcp": tcp_status,
        "read_registers": [
            read_register_json("temperature_c", "current temperature", state, runtime),
            read_register_json("stirrer_rpm", "current stirrer speed", state, runtime),
            read_register_json("pressure_mpa", "current pressure", state, runtime),
            read_register_json("shake_speed_cpm", "current shake speed", state, runtime),
            read_register_json("tilt_angle_deg", "current tilt angle", state, runtime),
            read_register_json("flow_rate_l_min", "current flow rate", state, runtime),
            read_register_json("product_concentration_percent", "current product concentration", state, runtime),
            read_register_json("ph", "current pH", state, runtime)
        ],
        "write_registers": [
            write_register_json("target_temperature_c", "target temperature", state, runtime),
            write_register_json("target_stirrer_rpm", "target stirrer speed", state, runtime),
            write_register_json("target_shake_speed_cpm", "target shake speed", state, runtime),
            write_register_json("target_pressure_mpa", "target pressure", state, runtime),
            write_register_json("heat_time_s", "heat time", state, runtime),
            write_register_json("hold_time_s", "hold time", state, runtime),
            write_register_json("cool_time_s", "cool time", state, runtime)
        ],
        "coils": coils_json(runtime),
        "discrete_inputs": discrete_inputs_json(state, runtime)
    })
}

pub(super) fn read_register_payload(
    state: &AppState,
    runtime: &RuntimeState,
    register: &str,
) -> Result<Value, AppError> {
    let value = register_value(state, runtime, register)?;
    let raw = encode_modbus_raw(value.value, value.scale, value.offset)?;
    Ok(json!({
        "device_id": "reactor_001",
        "register": register,
        "address": value.address,
        "access": value.access,
        "value": round2(value.value),
        "raw": raw,
        "scale": value.scale,
        "offset": value.offset,
        "source": value.source
    }))
}

pub(crate) async fn apply_modbus_register_write(
    state: &AppState,
    register: &str,
    value: f64,
    reason: Option<String>,
) -> Result<Value, AppError> {
    if !value.is_finite() {
        return Err(AppError::bad_request("value must be finite"));
    }
    let reason = reason
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::bad_request("modbus write reason is required for an auditable target change")
        })?
        .to_string();
    let current = state.runtime.read().await.targets.clone();
    let requested = match register {
        "target_temperature_c" => ControlTargets {
            temperature_c: value,
            ..current
        },
        "target_stirrer_rpm" => ControlTargets {
            stirrer_rpm: value,
            ..current
        },
        "target_shake_speed_cpm" => ControlTargets {
            shake_speed_cpm: value,
            ..current
        },
        "target_pressure_mpa" => ControlTargets {
            target_pressure_mpa: value,
            ..current
        },
        "heat_time_s" => ControlTargets {
            heat_time_s: value,
            ..current
        },
        "hold_time_s" => ControlTargets {
            hold_time_s: value,
            ..current
        },
        "cool_time_s" => ControlTargets {
            cool_time_s: value,
            ..current
        },
        _ => {
            return Err(AppError::bad_request(
                "register is not writable through the Modbus debug API",
            ))
        }
    };
    let targets = clamp_operator_targets(&state.safety, requested);
    ensure_targets_allowed(&state.safety, &targets)?;
    let Some(register_config) =
        write_register_config(&state.device_config.modbus.registers, register)
    else {
        return Err(AppError::bad_request(
            "register is not writable through the Modbus debug API",
        ));
    };
    let applied_value = write_register_applied_value(&targets, register)?;
    let address = register_config.address;
    let scale = register_config.scale;
    let offset = register_config.offset;
    let raw = encode_modbus_raw(applied_value, scale, offset)?;
    {
        let mut runtime = state.runtime.write().await;
        runtime.targets = targets.clone();
    }
    state
        .db
        .insert_control_event_sqlx(
            None,
            "modbus_register_write",
            Some(&SafeCommand {
                target_temperature_c: targets.temperature_c,
                heat_time_s: targets.heat_time_s,
                hold_time_s: targets.hold_time_s,
                cool_time_s: targets.cool_time_s,
                target_stirrer_rpm: targets.stirrer_rpm,
                target_shake_speed_cpm: targets.shake_speed_cpm,
                target_pressure_mpa: targets.target_pressure_mpa,
                reason: reason.clone(),
            }),
            &reason,
        )
        .await?;
    Ok(json!({
        "register": register,
        "address": address,
        "requested_value": value,
        "applied_value": round2(applied_value),
        "raw": raw,
        "scale": scale,
        "offset": offset,
        "targets": targets
    }))
}

fn bool_point_json(
    name: &str,
    label: &str,
    address: u16,
    access: &'static str,
    value: bool,
    source: &'static str,
) -> Value {
    json!({
        "name": name,
        "label": label,
        "address": address,
        "access": access,
        "value": value,
        "raw": if value { 1 } else { 0 },
        "source": source
    })
}

fn coils_json(runtime: &RuntimeState) -> Vec<Value> {
    vec![
        bool_point_json(
            "auto_enabled",
            "auto control coil",
            0,
            "read_write",
            runtime.auto_enabled,
            "runtime_state",
        ),
        bool_point_json(
            "manual_lock",
            "manual lock coil",
            1,
            "read_write",
            runtime.manual_lock,
            "runtime_state",
        ),
        bool_point_json(
            "emergency_stop",
            "emergency stop coil",
            2,
            "read_write",
            runtime.emergency_stop,
            "runtime_state",
        ),
        bool_point_json(
            "process_running",
            "process running coil",
            3,
            "read",
            runtime.active_batch_id.is_some(),
            "runtime_state",
        ),
    ]
}

fn discrete_inputs_json(state: &AppState, runtime: &RuntimeState) -> Vec<Value> {
    let sample_fresh = runtime
        .latest_sample
        .as_ref()
        .map(|sample| {
            Utc::now().signed_duration_since(sample.captured_at)
                <= Duration::milliseconds(state.safety.control.sensor_timeout_ms)
        })
        .unwrap_or(false);
    let device_connected = runtime
        .device_status
        .as_ref()
        .map(|device| device.connected)
        .unwrap_or_else(|| runtime.latest_sample.is_some());
    let alarm_active = runtime.emergency_stop
        || runtime.last_sensor_error.is_some()
        || runtime.last_control_error.is_some();
    let tilt_state = runtime
        .latest_sample
        .as_ref()
        .map(|sample| sample.tilt_state != 0)
        .unwrap_or(false);

    vec![
        bool_point_json(
            "device_connected",
            "device connected input",
            0,
            "read",
            device_connected,
            "runtime_state",
        ),
        bool_point_json(
            "sensor_fresh",
            "fresh sensor input",
            1,
            "read",
            sample_fresh,
            "runtime_state",
        ),
        bool_point_json(
            "alarm_active",
            "alarm active input",
            2,
            "read",
            alarm_active,
            "runtime_state",
        ),
        bool_point_json(
            "tilt_state",
            "tilt state input",
            3,
            "read",
            tilt_state,
            "latest_sample",
        ),
        bool_point_json(
            "active_batch",
            "active batch input",
            4,
            "read",
            runtime.active_batch_id.is_some(),
            "runtime_state",
        ),
    ]
}

fn read_register_json(name: &str, label: &str, state: &AppState, runtime: &RuntimeState) -> Value {
    match register_value(state, runtime, name) {
        Ok(value) => json!({
            "name": name,
            "label": label,
            "address": value.address,
            "access": value.access,
            "value": round2(value.value),
            "raw": encode_modbus_raw(value.value, value.scale, value.offset).ok(),
            "scale": value.scale,
            "offset": value.offset,
            "source": value.source
        }),
        Err(err) => json!({
            "name": name,
            "label": label,
            "access": "read",
            "status": "unavailable",
            "error": err.message()
        }),
    }
}

fn write_register_json(name: &str, label: &str, state: &AppState, runtime: &RuntimeState) -> Value {
    match register_value(state, runtime, name) {
        Ok(value) => json!({
            "name": name,
            "label": label,
            "address": value.address,
            "access": value.access,
            "value": round2(value.value),
            "raw": encode_modbus_raw(value.value, value.scale, value.offset).ok(),
            "scale": value.scale,
            "offset": value.offset,
            "source": value.source
        }),
        Err(err) => json!({
            "name": name,
            "label": label,
            "access": "write",
            "status": "unavailable",
            "error": err.message()
        }),
    }
}

fn register_value(
    state: &AppState,
    runtime: &RuntimeState,
    register: &str,
) -> Result<ModbusRegisterValue, AppError> {
    let registers = &state.device_config.modbus.registers;
    match register {
        "temperature_c" => {
            let Some(sample) = runtime.latest_sample.as_ref() else {
                return Err(AppError::service_unavailable("sensor data unavailable"));
            };
            Ok(ModbusRegisterValue {
                address: registers.temperature_c.address,
                access: "read",
                value: sample.temperature_c,
                scale: registers.temperature_c.scale,
                offset: registers.temperature_c.offset,
                source: "latest_sample",
            })
        }
        "stirrer_rpm" => {
            let Some(sample) = runtime.latest_sample.as_ref() else {
                return Err(AppError::service_unavailable("sensor data unavailable"));
            };
            Ok(ModbusRegisterValue {
                address: registers.stirrer_rpm.address,
                access: "read",
                value: sample.stirrer_rpm,
                scale: registers.stirrer_rpm.scale,
                offset: registers.stirrer_rpm.offset,
                source: "latest_sample",
            })
        }
        "pressure_mpa" => {
            let Some(sample) = runtime.latest_sample.as_ref() else {
                return Err(AppError::service_unavailable("sensor data unavailable"));
            };
            Ok(ModbusRegisterValue {
                address: registers.pressure_mpa.address,
                access: "read",
                value: sample.pressure_mpa,
                scale: registers.pressure_mpa.scale,
                offset: registers.pressure_mpa.offset,
                source: "latest_sample",
            })
        }
        "shake_speed_cpm" => {
            let Some(sample) = runtime.latest_sample.as_ref() else {
                return Err(AppError::service_unavailable("sensor data unavailable"));
            };
            Ok(ModbusRegisterValue {
                address: registers.shake_speed_cpm.address,
                access: "read",
                value: sample.shake_speed_cpm,
                scale: registers.shake_speed_cpm.scale,
                offset: registers.shake_speed_cpm.offset,
                source: "latest_sample",
            })
        }
        "tilt_angle_deg" => {
            let Some(sample) = runtime.latest_sample.as_ref() else {
                return Err(AppError::service_unavailable("sensor data unavailable"));
            };
            Ok(ModbusRegisterValue {
                address: registers.tilt_angle_deg.address,
                access: "read",
                value: sample.tilt_angle_deg,
                scale: registers.tilt_angle_deg.scale,
                offset: registers.tilt_angle_deg.offset,
                source: "latest_sample",
            })
        }
        "flow_rate_l_min" => {
            let Some(sample) = runtime.latest_sample.as_ref() else {
                return Err(AppError::service_unavailable("sensor data unavailable"));
            };
            Ok(ModbusRegisterValue {
                address: registers.flow_rate_l_min.address,
                access: "read",
                value: sample.flow_rate_l_min,
                scale: registers.flow_rate_l_min.scale,
                offset: registers.flow_rate_l_min.offset,
                source: "latest_sample",
            })
        }
        "product_concentration_percent" => {
            let Some(sample) = runtime.latest_sample.as_ref() else {
                return Err(AppError::service_unavailable("sensor data unavailable"));
            };
            Ok(ModbusRegisterValue {
                address: registers.product_concentration_percent.address,
                access: "read",
                value: sample.product_concentration_percent,
                scale: registers.product_concentration_percent.scale,
                offset: registers.product_concentration_percent.offset,
                source: "latest_sample",
            })
        }
        "ph" => {
            let Some(sample) = runtime.latest_sample.as_ref() else {
                return Err(AppError::service_unavailable("sensor data unavailable"));
            };
            Ok(ModbusRegisterValue {
                address: registers.ph.address,
                access: "read",
                value: sample.ph,
                scale: registers.ph.scale,
                offset: registers.ph.offset,
                source: "latest_sample",
            })
        }
        "target_temperature_c" => Ok(ModbusRegisterValue {
            address: registers.target_temperature_c.address,
            access: "write",
            value: runtime.targets.temperature_c,
            scale: registers.target_temperature_c.scale,
            offset: registers.target_temperature_c.offset,
            source: "runtime_targets",
        }),
        "target_stirrer_rpm" => Ok(ModbusRegisterValue {
            address: registers.target_stirrer_rpm.address,
            access: "write",
            value: runtime.targets.stirrer_rpm,
            scale: registers.target_stirrer_rpm.scale,
            offset: registers.target_stirrer_rpm.offset,
            source: "runtime_targets",
        }),
        "target_shake_speed_cpm" => Ok(ModbusRegisterValue {
            address: registers.target_shake_speed_cpm.address,
            access: "write",
            value: runtime.targets.shake_speed_cpm,
            scale: registers.target_shake_speed_cpm.scale,
            offset: registers.target_shake_speed_cpm.offset,
            source: "runtime_targets",
        }),
        "target_pressure_mpa" => Ok(ModbusRegisterValue {
            address: registers.target_pressure_mpa.address,
            access: "write",
            value: runtime.targets.target_pressure_mpa,
            scale: registers.target_pressure_mpa.scale,
            offset: registers.target_pressure_mpa.offset,
            source: "runtime_targets",
        }),
        "heat_time_s" => Ok(ModbusRegisterValue {
            address: registers.heat_time_s.address,
            access: "write",
            value: runtime.targets.heat_time_s,
            scale: registers.heat_time_s.scale,
            offset: registers.heat_time_s.offset,
            source: "runtime_targets",
        }),
        "hold_time_s" => Ok(ModbusRegisterValue {
            address: registers.hold_time_s.address,
            access: "write",
            value: runtime.targets.hold_time_s,
            scale: registers.hold_time_s.scale,
            offset: registers.hold_time_s.offset,
            source: "runtime_targets",
        }),
        "cool_time_s" => Ok(ModbusRegisterValue {
            address: registers.cool_time_s.address,
            access: "write",
            value: runtime.targets.cool_time_s,
            scale: registers.cool_time_s.scale,
            offset: registers.cool_time_s.offset,
            source: "runtime_targets",
        }),
        _ => Err(AppError::not_found("modbus register not found")),
    }
}

fn write_register_config<'a>(
    registers: &'a RegistersConfig,
    register: &str,
) -> Option<&'a WriteRegister> {
    match register {
        "target_temperature_c" => Some(&registers.target_temperature_c),
        "target_stirrer_rpm" => Some(&registers.target_stirrer_rpm),
        "target_shake_speed_cpm" => Some(&registers.target_shake_speed_cpm),
        "target_pressure_mpa" => Some(&registers.target_pressure_mpa),
        "heat_time_s" => Some(&registers.heat_time_s),
        "hold_time_s" => Some(&registers.hold_time_s),
        "cool_time_s" => Some(&registers.cool_time_s),
        _ => None,
    }
}

fn write_register_applied_value(targets: &ControlTargets, register: &str) -> Result<f64, AppError> {
    match register {
        "target_temperature_c" => Ok(targets.temperature_c),
        "target_stirrer_rpm" => Ok(targets.stirrer_rpm),
        "target_shake_speed_cpm" => Ok(targets.shake_speed_cpm),
        "target_pressure_mpa" => Ok(targets.target_pressure_mpa),
        "heat_time_s" => Ok(targets.heat_time_s),
        "hold_time_s" => Ok(targets.hold_time_s),
        "cool_time_s" => Ok(targets.cool_time_s),
        _ => Err(AppError::bad_request(
            "register is not writable through the Modbus debug API",
        )),
    }
}

fn encode_modbus_raw(value: f64, scale: f64, offset: f64) -> Result<u16, AppError> {
    if scale == 0.0 {
        return Err(AppError::bad_request("register scale must not be zero"));
    }
    let raw = ((value - offset) / scale).round();
    if !(0.0..=u16::MAX as f64).contains(&raw) {
        return Err(AppError::bad_request("value cannot be encoded as u16"));
    }
    Ok(raw as u16)
}
