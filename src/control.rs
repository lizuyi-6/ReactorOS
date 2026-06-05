use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    config::SafetyConfig,
    number::round2,
    state::{ControlTargets, SensorSnapshot},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafetyGuardRequest {
    DecideControl {
        safety: SafetyConfig,
        sample: Option<SensorSnapshot>,
        targets: ControlTargets,
        auto_enabled: bool,
        manual_lock: bool,
        emergency_stop: bool,
    },
    ClampTargets {
        safety: SafetyConfig,
        targets: ControlTargets,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SafetyGuardResponse {
    ControlDecision(ControlDecision),
    ClampedTargets(ControlTargets),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SafeCommand {
    pub target_temperature_c: f64,
    pub heat_time_s: f64,
    pub hold_time_s: f64,
    pub cool_time_s: f64,
    pub target_stirrer_rpm: f64,
    pub target_shake_speed_cpm: f64,
    pub target_pressure_mpa: f64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ControlBlockReason {
    AutoDisabled,
    ManualLock,
    EmergencyStop,
    MissingSensorSample,
    SensorStale,
    ForbiddenControlZone,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ControlDecision {
    Write(SafeCommand),
    Blocked(ControlBlockReason),
}

pub fn decide_control(
    safety: &SafetyConfig,
    sample: Option<&SensorSnapshot>,
    targets: &ControlTargets,
    auto_enabled: bool,
    manual_lock: bool,
    emergency_stop: bool,
) -> ControlDecision {
    if emergency_stop {
        return ControlDecision::Blocked(ControlBlockReason::EmergencyStop);
    }
    if manual_lock {
        return ControlDecision::Blocked(ControlBlockReason::ManualLock);
    }
    if !auto_enabled {
        return ControlDecision::Blocked(ControlBlockReason::AutoDisabled);
    }

    let Some(sample) = sample else {
        return ControlDecision::Blocked(ControlBlockReason::MissingSensorSample);
    };

    let age = Utc::now().signed_duration_since(sample.captured_at);
    if age > Duration::milliseconds(safety.control.sensor_timeout_ms) {
        return ControlDecision::Blocked(ControlBlockReason::SensorStale);
    }

    let temperature = clamp_step(
        sample.temperature_c,
        targets.temperature_c,
        safety.temperature.min_c,
        safety.temperature.max_c,
        safety.temperature.max_step_c,
    );
    let stirrer = clamp_step(
        sample.stirrer_rpm,
        targets.stirrer_rpm,
        safety.stirrer.min_rpm,
        safety.stirrer.max_rpm,
        safety.stirrer.max_step_rpm,
    );
    if is_forbidden_control_zone(safety, temperature, stirrer) {
        return ControlDecision::Blocked(ControlBlockReason::ForbiddenControlZone);
    }

    ControlDecision::Write(SafeCommand {
        target_temperature_c: round2(temperature),
        heat_time_s: round2(targets.heat_time_s),
        hold_time_s: round2(targets.hold_time_s),
        cool_time_s: round2(targets.cool_time_s),
        target_stirrer_rpm: round2(stirrer),
        target_shake_speed_cpm: round2(targets.shake_speed_cpm),
        target_pressure_mpa: round2(targets.target_pressure_mpa),
        reason: "auto control within configured safety limits".to_string(),
    })
}

pub fn clamp_operator_targets(safety: &SafetyConfig, targets: ControlTargets) -> ControlTargets {
    ControlTargets {
        temperature_c: round2(
            targets
                .temperature_c
                .clamp(safety.temperature.min_c, safety.temperature.max_c),
        ),
        heat_time_s: round2(targets.heat_time_s.clamp(0.0, 3600.0)),
        hold_time_s: round2(targets.hold_time_s.clamp(0.0, 7200.0)),
        cool_time_s: round2(targets.cool_time_s.clamp(0.0, 3600.0)),
        stirrer_rpm: round2(
            targets
                .stirrer_rpm
                .clamp(safety.stirrer.min_rpm, safety.stirrer.max_rpm),
        ),
        shake_speed_cpm: round2(targets.shake_speed_cpm.clamp(0.0, 60.0)),
        target_pressure_mpa: round2(targets.target_pressure_mpa.clamp(0.0, 10.0)),
    }
}

pub fn is_forbidden_control_zone(
    safety: &SafetyConfig,
    temperature_c: f64,
    stirrer_rpm: f64,
) -> bool {
    safety
        .forbidden_control_zones
        .iter()
        .any(|zone| zone.contains(temperature_c, stirrer_rpm))
}

pub fn forbidden_control_zone<'a>(
    safety: &'a SafetyConfig,
    temperature_c: f64,
    stirrer_rpm: f64,
) -> Option<&'a crate::config::ForbiddenControlZone> {
    safety
        .forbidden_control_zones
        .iter()
        .find(|zone| zone.contains(temperature_c, stirrer_rpm))
}

pub fn evaluate_safety_request(request: SafetyGuardRequest) -> SafetyGuardResponse {
    match request {
        SafetyGuardRequest::DecideControl {
            safety,
            sample,
            targets,
            auto_enabled,
            manual_lock,
            emergency_stop,
        } => SafetyGuardResponse::ControlDecision(decide_control(
            &safety,
            sample.as_ref(),
            &targets,
            auto_enabled,
            manual_lock,
            emergency_stop,
        )),
        SafetyGuardRequest::ClampTargets { safety, targets } => {
            SafetyGuardResponse::ClampedTargets(clamp_operator_targets(&safety, targets))
        }
    }
}

fn clamp_step(current: f64, desired: f64, min: f64, max: f64, max_step: f64) -> f64 {
    let bounded = desired.clamp(min, max);
    let delta = (bounded - current).clamp(-max_step, max_step);
    (current + delta).clamp(min, max)
}
