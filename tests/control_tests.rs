use chrono::{Duration, Utc};
use reactor_edge_daemon::{
    config::{
        ControlConfig, ForbiddenControlZone, OptimizerBounds, SafetyConfig, StirrerSafety,
        TemperatureSafety,
    },
    control::{
        clamp_operator_targets, decide_control, evaluate_safety_request, ControlBlockReason,
        ControlDecision, SafetyGuardRequest, SafetyGuardResponse,
    },
    state::{ControlTargets, SensorSnapshot},
};

fn safety() -> SafetyConfig {
    SafetyConfig {
        control: ControlConfig {
            auto_enabled_default: false,
            manual_lock_default: false,
            control_interval_ms: 2000,
            sensor_timeout_ms: 6000,
        },
        temperature: TemperatureSafety {
            min_c: 20.0,
            max_c: 160.0,
            max_step_c: 2.0,
            default_target_c: 60.0,
        },
        stirrer: StirrerSafety {
            min_rpm: 0.0,
            max_rpm: 1200.0,
            max_step_rpm: 50.0,
            default_target_rpm: 300.0,
        },
        optimizer: OptimizerBounds {
            min_temperature_c: 35.0,
            max_temperature_c: 140.0,
            min_stirrer_rpm: 100.0,
            max_stirrer_rpm: 1000.0,
            min_heating_minutes: 15.0,
            max_heating_minutes: 240.0,
            min_stirring_minutes: 15.0,
            max_stirring_minutes: 240.0,
        },
        forbidden_control_zones: vec![ForbiddenControlZone {
            name: "hot-low-stir".to_string(),
            reason: "bench safety envelope".to_string(),
            min_temperature_c: 125.0,
            max_temperature_c: 160.0,
            min_stirrer_rpm: 0.0,
            max_stirrer_rpm: 350.0,
        }],
    }
}

#[test]
fn control_writes_only_one_safe_step() {
    let safety = safety();
    let sample = SensorSnapshot {
        temperature_c: 50.0,
        pressure_mpa: 0.12,
        stirrer_rpm: 200.0,
        shake_speed_cpm: 30.0,
        tilt_state: 1,
        tilt_angle_deg: 12.5,
        flow_rate_l_min: 2.5,
        product_concentration_percent: 45.0,
        ph: 7.0,
        captured_at: Utc::now(),
    };
    let targets = ControlTargets {
        temperature_c: 120.0,
        heat_time_s: 300.0,
        hold_time_s: 600.0,
        cool_time_s: 180.0,
        stirrer_rpm: 900.0,
        shake_speed_cpm: 35.0,
        target_pressure_mpa: 0.5,
    };

    let decision = decide_control(&safety, Some(&sample), &targets, true, false, false);

    assert_eq!(
        decision,
        ControlDecision::Write(reactor_edge_daemon::control::SafeCommand {
            target_temperature_c: 52.0,
            heat_time_s: 300.0,
            hold_time_s: 600.0,
            cool_time_s: 180.0,
            target_stirrer_rpm: 250.0,
            target_shake_speed_cpm: 35.0,
            target_pressure_mpa: 0.5,
            reason: "auto control within configured safety limits".to_string()
        })
    );
}

#[test]
fn control_blocks_stale_samples_and_estop() {
    let safety = safety();
    let sample = SensorSnapshot {
        temperature_c: 50.0,
        pressure_mpa: 0.12,
        stirrer_rpm: 200.0,
        shake_speed_cpm: 30.0,
        tilt_state: 1,
        tilt_angle_deg: 12.5,
        flow_rate_l_min: 2.5,
        product_concentration_percent: 45.0,
        ph: 7.0,
        captured_at: Utc::now() - Duration::seconds(30),
    };
    let targets = ControlTargets {
        temperature_c: 60.0,
        heat_time_s: 300.0,
        hold_time_s: 600.0,
        cool_time_s: 180.0,
        stirrer_rpm: 300.0,
        shake_speed_cpm: 30.0,
        target_pressure_mpa: 0.5,
    };

    assert_eq!(
        decide_control(&safety, Some(&sample), &targets, true, false, false),
        ControlDecision::Blocked(ControlBlockReason::SensorStale)
    );
    assert_eq!(
        decide_control(&safety, Some(&sample), &targets, true, false, true),
        ControlDecision::Blocked(ControlBlockReason::EmergencyStop)
    );
}

#[test]
fn operator_targets_are_clamped_to_safety_bounds() {
    let targets = clamp_operator_targets(
        &safety(),
        ControlTargets {
            temperature_c: 999.0,
            heat_time_s: 5000.0,
            hold_time_s: 8000.0,
            cool_time_s: 5000.0,
            stirrer_rpm: 9999.0,
            shake_speed_cpm: 99.0,
            target_pressure_mpa: 99.0,
        },
    );

    assert_eq!(targets.temperature_c, 160.0);
    assert_eq!(targets.stirrer_rpm, 1200.0);
    assert_eq!(targets.heat_time_s, 3600.0);
    assert_eq!(targets.hold_time_s, 7200.0);
    assert_eq!(targets.cool_time_s, 3600.0);
    assert_eq!(targets.shake_speed_cpm, 60.0);
    assert_eq!(targets.target_pressure_mpa, 10.0);
}

#[test]
fn control_blocks_forbidden_temperature_stirrer_zone() {
    let safety = safety();
    let sample = SensorSnapshot {
        temperature_c: 124.0,
        pressure_mpa: 0.12,
        stirrer_rpm: 340.0,
        shake_speed_cpm: 30.0,
        tilt_state: 1,
        tilt_angle_deg: 12.5,
        flow_rate_l_min: 2.5,
        product_concentration_percent: 45.0,
        ph: 7.0,
        captured_at: Utc::now(),
    };
    let targets = ControlTargets {
        temperature_c: 150.0,
        heat_time_s: 300.0,
        hold_time_s: 600.0,
        cool_time_s: 180.0,
        stirrer_rpm: 100.0,
        shake_speed_cpm: 30.0,
        target_pressure_mpa: 0.5,
    };

    assert_eq!(
        decide_control(&safety, Some(&sample), &targets, true, false, false),
        ControlDecision::Blocked(ControlBlockReason::ForbiddenControlZone)
    );
}

#[test]
fn safety_guard_request_matches_in_process_clamp_and_decision() {
    let safety = safety();
    let targets = ControlTargets {
        temperature_c: 999.0,
        heat_time_s: 5000.0,
        hold_time_s: 8000.0,
        cool_time_s: 5000.0,
        stirrer_rpm: 9999.0,
        shake_speed_cpm: 99.0,
        target_pressure_mpa: 99.0,
    };
    let response = evaluate_safety_request(SafetyGuardRequest::ClampTargets {
        safety: safety.clone(),
        targets: targets.clone(),
    });
    assert_eq!(
        response,
        SafetyGuardResponse::ClampedTargets(clamp_operator_targets(&safety, targets))
    );

    let sample = SensorSnapshot {
        temperature_c: 50.0,
        pressure_mpa: 0.12,
        stirrer_rpm: 200.0,
        shake_speed_cpm: 30.0,
        tilt_state: 1,
        tilt_angle_deg: 12.5,
        flow_rate_l_min: 2.5,
        product_concentration_percent: 45.0,
        ph: 7.0,
        captured_at: Utc::now(),
    };
    let targets = ControlTargets {
        temperature_c: 120.0,
        heat_time_s: 300.0,
        hold_time_s: 600.0,
        cool_time_s: 180.0,
        stirrer_rpm: 900.0,
        shake_speed_cpm: 35.0,
        target_pressure_mpa: 0.5,
    };
    let response = evaluate_safety_request(SafetyGuardRequest::DecideControl {
        safety: safety.clone(),
        sample: Some(sample.clone()),
        targets: targets.clone(),
        auto_enabled: true,
        manual_lock: false,
        emergency_stop: false,
    });
    assert_eq!(
        response,
        SafetyGuardResponse::ControlDecision(decide_control(
            &safety,
            Some(&sample),
            &targets,
            true,
            false,
            false
        ))
    );
}
