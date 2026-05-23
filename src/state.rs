use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::config::SafetyConfig;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub emergency_stop: bool,
    pub active_batch_id: Option<i64>,
    pub last_control_error: Option<String>,
}

pub type SharedState = Arc<RwLock<RuntimeState>>;

impl RuntimeState {
    pub fn from_safety(safety: &SafetyConfig) -> Self {
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
            auto_enabled: safety.control.auto_enabled_default,
            manual_lock: safety.control.manual_lock_default,
            emergency_stop: false,
            active_batch_id: None,
            last_control_error: None,
        }
    }
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

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
