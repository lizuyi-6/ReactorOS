use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{config::SafetyConfig, number::round2};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensorSnapshot {
    pub temperature_c: f64,
    pub pressure_mpa: f64,
    pub stirrer_rpm: f64,
    pub shake_speed_cpm: f64,
    pub tilt_state: u8,
    pub tilt_angle_deg: f64,
    pub flow_rate_l_min: f64,
    pub product_concentration_percent: f64,
    pub ph: f64,
    pub captured_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy)]
pub struct SensorRange {
    pub field: &'static str,
    pub min: f64,
    pub max: f64,
}

impl SensorRange {
    pub fn validate(&self, value: f64) -> Result<(), String> {
        if !value.is_finite() || !(self.min..=self.max).contains(&value) {
            return Err(format!(
                "{} must be between {} and {}",
                self.field, self.min, self.max
            ));
        }
        Ok(())
    }
}

pub const SENSOR_TEMPERATURE_C_RANGE: SensorRange = SensorRange {
    field: "temperature_c",
    min: -40.0,
    max: 500.0,
};
pub const SENSOR_PRESSURE_MPA_RANGE: SensorRange = SensorRange {
    field: "pressure_mpa",
    min: 0.0,
    max: 10.0,
};
pub const SENSOR_STIRRER_RPM_RANGE: SensorRange = SensorRange {
    field: "stirrer_rpm",
    min: 0.0,
    max: 2000.0,
};
pub const SENSOR_SHAKE_SPEED_CPM_RANGE: SensorRange = SensorRange {
    field: "shake_speed_cpm",
    min: 0.0,
    max: 60.0,
};
pub const SENSOR_TILT_ANGLE_DEG_RANGE: SensorRange = SensorRange {
    field: "tilt_angle_deg",
    min: -30.0,
    max: 30.0,
};
pub const SENSOR_FLOW_RATE_L_MIN_RANGE: SensorRange = SensorRange {
    field: "flow_rate_l_min",
    min: 0.0,
    max: 100.0,
};
pub const SENSOR_PRODUCT_CONCENTRATION_PERCENT_RANGE: SensorRange = SensorRange {
    field: "product_concentration_percent",
    min: 0.0,
    max: 100.0,
};
pub const SENSOR_PH_RANGE: SensorRange = SensorRange {
    field: "ph",
    min: 0.0,
    max: 14.0,
};

pub fn validate_sensor_snapshot(sample: &SensorSnapshot) -> Result<(), String> {
    SENSOR_TEMPERATURE_C_RANGE.validate(sample.temperature_c)?;
    SENSOR_PRESSURE_MPA_RANGE.validate(sample.pressure_mpa)?;
    SENSOR_STIRRER_RPM_RANGE.validate(sample.stirrer_rpm)?;
    SENSOR_SHAKE_SPEED_CPM_RANGE.validate(sample.shake_speed_cpm)?;
    SENSOR_TILT_ANGLE_DEG_RANGE.validate(sample.tilt_angle_deg)?;
    SENSOR_FLOW_RATE_L_MIN_RANGE.validate(sample.flow_rate_l_min)?;
    SENSOR_PRODUCT_CONCENTRATION_PERCENT_RANGE.validate(sample.product_concentration_percent)?;
    SENSOR_PH_RANGE.validate(sample.ph)?;
    validate_sensor_tilt_state(sample.tilt_state)
}

pub fn validate_sensor_tilt_state(value: u8) -> Result<(), String> {
    if value <= 1 {
        Ok(())
    } else {
        Err("tilt_state must be 0 or 1 for the shake vessel binary tilt sensor".to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ControlTargets {
    pub temperature_c: f64,
    pub heat_time_s: f64,
    pub hold_time_s: f64,
    pub cool_time_s: f64,
    pub stirrer_rpm: f64,
    pub shake_speed_cpm: f64,
    pub target_pressure_mpa: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeState {
    pub latest_sample: Option<SensorSnapshot>,
    pub targets: ControlTargets,
    pub auto_enabled: bool,
    pub manual_lock: bool,
    #[serde(default, skip_serializing)]
    pub manual_lock_generation: u64,
    pub emergency_stop: bool,
    #[serde(default, skip_serializing)]
    pub emergency_stop_generation: u64,
    pub active_batch_id: Option<i64>,
    pub last_sensor_error: Option<String>,
    pub last_control_error: Option<String>,
    /// Latched `true` by the main.rs fail-safe monitor when the control-loop
    /// task has exited or panicked. Unlike a normal control fault this can ONLY
    /// be cleared by a process restart (the task is spawned once and never
    /// re-spawned), so `reset_control_fault` must refuse to clear it — otherwise
    /// the API would report "no fault" while no supervisor is running. Serialized
    /// so /api/live can surface a dead supervisor to clients.
    #[serde(default)]
    pub control_loop_terminated: bool,
    #[serde(default, skip_serializing)]
    pub control_fault_generation: u64,
    pub device_status: Option<DeviceStatusSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceStatusSnapshot {
    pub connected: bool,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub last_frame_ok: bool,
    pub relay: Option<u8>,
    pub motor: Option<u8>,
    pub tilt: Option<u8>,
    pub speed_delay_us: Option<u64>,
    pub port: Option<String>,
    pub baudrate: Option<u32>,
    pub last_command_request_id: Option<String>,
    pub last_command_ok: Option<bool>,
    pub last_command_error: Option<String>,
    pub updated_at: DateTime<Utc>,
}

pub type SharedState = Arc<RwLock<RuntimeState>>;

pub fn timestamp_age_ms(timestamp: DateTime<Utc>) -> i64 {
    Utc::now()
        .signed_duration_since(timestamp)
        .num_milliseconds()
}

pub fn timestamp_is_fresh(timestamp: DateTime<Utc>, timeout_ms: i64) -> bool {
    let age_ms = timestamp_age_ms(timestamp);
    age_ms >= 0 && age_ms <= timeout_ms
}

impl RuntimeState {
    pub fn from_safety(safety: &SafetyConfig) -> Self {
        let manual_lock = safety.control.manual_lock_default;
        Self {
            latest_sample: None,
            targets: ControlTargets {
                temperature_c: safety.temperature.default_target_c,
                heat_time_s: 300.0,
                hold_time_s: 600.0,
                cool_time_s: 180.0,
                stirrer_rpm: safety.stirrer.default_target_rpm,
                shake_speed_cpm: 30.0,
                target_pressure_mpa: 0.5,
            },
            auto_enabled: false,
            manual_lock,
            manual_lock_generation: if manual_lock { 1 } else { 0 },
            emergency_stop: false,
            emergency_stop_generation: 0,
            active_batch_id: None,
            last_sensor_error: None,
            last_control_error: None,
            control_loop_terminated: false,
            control_fault_generation: 0,
            device_status: None,
        }
    }

    pub fn disable_auto_for_field_fault(&mut self, reason: impl Into<String>) -> bool {
        self.last_sensor_error = Some(reason.into());
        if self.auto_enabled {
            self.auto_enabled = false;
            true
        } else {
            false
        }
    }

    pub fn reject_unpersisted_sample(&mut self, reason: impl Into<String>) -> bool {
        self.latest_sample = None;
        self.disable_auto_for_field_fault(reason)
    }

    pub fn reject_unpersisted_sample_with_status(
        &mut self,
        status: Option<DeviceStatusSnapshot>,
        reason: impl Into<String>,
    ) -> bool {
        self.device_status = status;
        self.reject_unpersisted_sample(reason)
    }

    pub fn latch_control_fault(&mut self, reason: impl Into<String>) -> bool {
        let reason = reason.into();
        let changed = self.last_control_error.as_deref() != Some(reason.as_str());
        let was_auto_enabled = self.auto_enabled;
        self.control_fault_generation = self.control_fault_generation.saturating_add(1);
        self.last_control_error = Some(reason);
        self.auto_enabled = false;
        was_auto_enabled || changed
    }

    pub fn enforce_control_fault_fail_closed(&mut self) -> bool {
        if self.last_control_error.is_some() && self.auto_enabled {
            self.auto_enabled = false;
            true
        } else {
            false
        }
    }

    pub fn latch_audit_failure_after_device_action(&mut self, action: &str, err: &str) -> bool {
        self.latch_control_fault(format!("{action} audit failed after device action: {err}"))
    }

    pub fn clear_control_fault(&mut self) {
        self.last_control_error = None;
        self.auto_enabled = false;
    }

    pub fn engage_manual_lock(&mut self) {
        self.manual_lock = true;
        self.manual_lock_generation = self.manual_lock_generation.saturating_add(1);
        self.auto_enabled = false;
    }

    pub fn clear_manual_lock(&mut self) {
        self.manual_lock = false;
        self.auto_enabled = false;
    }

    pub fn engage_emergency_stop(&mut self) {
        self.emergency_stop = true;
        self.emergency_stop_generation = self.emergency_stop_generation.saturating_add(1);
        self.auto_enabled = false;
    }

    pub fn clear_emergency_stop(&mut self) {
        self.emergency_stop = false;
        self.auto_enabled = false;
    }
}

pub fn device_status_field_fault_reason(
    status: &DeviceStatusSnapshot,
    sensor_timeout_ms: i64,
) -> Option<String> {
    if !status.connected {
        return Some("downstream device status is disconnected".to_string());
    }
    if !status.last_frame_ok {
        return Some("downstream device last upstream frame failed validation".to_string());
    }
    let Some(last_seen) = status.last_seen_at.as_ref() else {
        return Some("downstream device status has no valid last_seen timestamp".to_string());
    };
    let age_ms = timestamp_age_ms(*last_seen);
    if age_ms < 0 {
        return Some(format!(
            "downstream device status timestamp is {} ms in the future",
            -age_ms
        ));
    }
    if age_ms > sensor_timeout_ms {
        return Some(format!(
            "downstream device status stale; last_seen is {age_ms} ms old, max {sensor_timeout_ms} ms"
        ));
    }
    None
}

pub fn downstream_command_fault_reason(status: &DeviceStatusSnapshot) -> Option<String> {
    if status.last_command_ok != Some(false) {
        return None;
    }
    let request_id = status
        .last_command_request_id
        .as_deref()
        .unwrap_or("unknown request");
    let detail = status
        .last_command_error
        .as_deref()
        .filter(|error| !error.trim().is_empty())
        .unwrap_or("downstream command reported failure");
    Some(format!("downstream command {request_id} failed: {detail}"))
}

pub fn fit_tilt_angle_deg(tilt_state: u8, shake_speed_cpm: f64, captured_at: DateTime<Utc>) -> f64 {
    let speed = if shake_speed_cpm.is_finite() {
        shake_speed_cpm.clamp(0.0, 60.0)
    } else {
        0.0
    };
    if speed <= 0.01 {
        return 0.0;
    }

    let period_ms = 60_000.0 / speed;
    let phase = (captured_at.timestamp_millis() as f64).rem_euclid(period_ms) / period_ms;
    let envelope = (phase * std::f64::consts::TAU).sin().abs();
    let sign = if tilt_state == 0 { -1.0 } else { 1.0 };
    round2(sign * 30.0 * envelope)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_sample() -> SensorSnapshot {
        SensorSnapshot {
            temperature_c: 50.0,
            pressure_mpa: 0.12,
            stirrer_rpm: 240.0,
            shake_speed_cpm: 24.0,
            tilt_state: 1,
            tilt_angle_deg: 12.5,
            flow_rate_l_min: 2.2,
            product_concentration_percent: 10.0,
            ph: 6.8,
            captured_at: Utc::now(),
        }
    }

    #[test]
    fn sensor_snapshot_validation_rejects_physically_invalid_field_state() {
        assert!(validate_sensor_snapshot(&valid_sample()).is_ok());

        let mut negative_pressure = valid_sample();
        negative_pressure.pressure_mpa = -0.01;
        assert_eq!(
            validate_sensor_snapshot(&negative_pressure),
            Err("pressure_mpa must be between 0 and 10".to_string())
        );

        let mut high_ph = valid_sample();
        high_ph.ph = 14.01;
        assert_eq!(
            validate_sensor_snapshot(&high_ph),
            Err("ph must be between 0 and 14".to_string())
        );

        let mut impossible_tilt_state = valid_sample();
        impossible_tilt_state.tilt_state = 2;
        assert_eq!(
            validate_sensor_snapshot(&impossible_tilt_state),
            Err("tilt_state must be 0 or 1 for the shake vessel binary tilt sensor".to_string())
        );

        let mut impossible_tilt_angle = valid_sample();
        impossible_tilt_angle.tilt_angle_deg = 30.01;
        assert_eq!(
            validate_sensor_snapshot(&impossible_tilt_angle),
            Err("tilt_angle_deg must be between -30 and 30".to_string())
        );

        let mut non_finite_temperature = valid_sample();
        non_finite_temperature.temperature_c = f64::NAN;
        assert_eq!(
            validate_sensor_snapshot(&non_finite_temperature),
            Err("temperature_c must be between -40 and 500".to_string())
        );
    }
}
