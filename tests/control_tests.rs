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
    state::{
        device_status_field_fault_reason, downstream_command_fault_reason, ControlTargets,
        DeviceStatusSnapshot, RuntimeState, SensorSnapshot,
    },
};

fn safety() -> SafetyConfig {
    SafetyConfig {
        control: ControlConfig {
            auto_enabled_default: false,
            manual_lock_default: false,
            control_interval_ms: 2000,
            sensor_timeout_ms: 6000,
            require_device_status_for_control: false,
            write_retry_backoff_ms: 5000,
            safety_guard_timeout_ms: 1000,
            ai_stop_product_concentration_percent: 95.0,
            require_command_ack: false,
            command_ack_timeout_ms: 2000,
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

    let decision = decide_control(&safety, Some(&sample), &targets, true, false, false, None);

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
        decide_control(&safety, Some(&sample), &targets, true, false, false, None,),
        ControlDecision::Blocked(ControlBlockReason::SensorStale)
    );
    assert_eq!(
        decide_control(&safety, Some(&sample), &targets, true, false, true, None,),
        ControlDecision::Blocked(ControlBlockReason::EmergencyStop)
    );
    assert_eq!(
        decide_control(
            &safety,
            Some(&sample),
            &targets,
            true,
            false,
            false,
            Some("write timeout"),
        ),
        ControlDecision::Blocked(ControlBlockReason::ControlFault)
    );
}

#[test]
fn control_blocks_future_timestamp_samples() {
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
        captured_at: Utc::now() + Duration::milliseconds(5000),
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
        decide_control(&safety, Some(&sample), &targets, true, false, false, None),
        ControlDecision::Blocked(ControlBlockReason::SensorStale)
    );
}

#[test]
fn control_blocks_when_required_downstream_status_is_missing_or_faulted() {
    let mut safety = safety();
    safety.control.require_device_status_for_control = true;
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
        temperature_c: 60.0,
        heat_time_s: 300.0,
        hold_time_s: 600.0,
        cool_time_s: 180.0,
        stirrer_rpm: 300.0,
        shake_speed_cpm: 30.0,
        target_pressure_mpa: 0.5,
    };
    let healthy = DeviceStatusSnapshot {
        connected: true,
        last_seen_at: Some(Utc::now()),
        last_frame_ok: true,
        relay: Some(0),
        motor: Some(0),
        tilt: Some(1),
        speed_delay_us: Some(10000),
        port: Some("/dev/ttyUSB0".to_string()),
        baudrate: Some(115200),
        last_command_request_id: None,
        last_command_ok: Some(true),
        last_command_error: None,
        updated_at: Utc::now(),
    };

    assert_eq!(
        reactor_edge_daemon::control::decide_control_with_device_status(
            &safety,
            Some(&sample),
            &targets,
            true,
            false,
            false,
            None,
            None,
        ),
        ControlDecision::Blocked(ControlBlockReason::MissingDeviceStatus)
    );

    let command_fault = DeviceStatusSnapshot {
        last_command_request_id: Some("cmd-1".to_string()),
        last_command_ok: Some(false),
        last_command_error: Some("relay rejected".to_string()),
        ..healthy.clone()
    };
    assert_eq!(
        reactor_edge_daemon::control::decide_control_with_device_status(
            &safety,
            Some(&sample),
            &targets,
            true,
            false,
            false,
            None,
            Some(&command_fault),
        ),
        ControlDecision::Blocked(ControlBlockReason::DownstreamCommandFault)
    );
}

#[test]
fn field_input_fault_disables_auto_without_latching_control_fault() {
    let safety = safety();
    let mut runtime = RuntimeState::from_safety(&safety);
    runtime.auto_enabled = true;

    let disabled = runtime.disable_auto_for_field_fault("pipeline sample stale");

    assert!(disabled);
    assert!(!runtime.auto_enabled);
    assert_eq!(
        runtime.last_sensor_error.as_deref(),
        Some("pipeline sample stale")
    );
    assert_eq!(runtime.last_control_error, None);

    let disabled_again = runtime.disable_auto_for_field_fault("pipeline sample stale");
    assert!(!disabled_again);
}

#[test]
fn latched_control_fault_forces_auto_disabled_even_if_state_was_inconsistent() {
    let safety = safety();
    let mut runtime = RuntimeState::from_safety(&safety);
    runtime.auto_enabled = true;
    runtime.last_control_error = Some("write timeout".to_string());

    let disabled = runtime.enforce_control_fault_fail_closed();

    assert!(disabled);
    assert!(!runtime.auto_enabled);
    assert_eq!(runtime.last_control_error.as_deref(), Some("write timeout"));

    let disabled_again = runtime.enforce_control_fault_fail_closed();
    assert!(!disabled_again);
}

#[test]
fn unpersisted_sample_is_not_kept_as_field_proof() {
    let safety = safety();
    let mut runtime = RuntimeState::from_safety(&safety);
    runtime.auto_enabled = true;
    runtime.latest_sample = Some(SensorSnapshot {
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
    });

    let disabled = runtime.reject_unpersisted_sample("sensor sample persistence failed");

    assert!(disabled);
    assert!(runtime.latest_sample.is_none());
    assert!(!runtime.auto_enabled);
    assert_eq!(
        runtime.last_sensor_error.as_deref(),
        Some("sensor sample persistence failed")
    );
    assert_eq!(runtime.last_control_error, None);
}

#[test]
fn unpersisted_sample_keeps_downstream_command_fault_visible() {
    let safety = safety();
    let mut runtime = RuntimeState::from_safety(&safety);
    runtime.auto_enabled = true;
    let status = DeviceStatusSnapshot {
        connected: true,
        last_seen_at: Some(Utc::now()),
        last_frame_ok: true,
        relay: Some(0),
        motor: Some(0),
        tilt: Some(1),
        speed_delay_us: Some(10000),
        port: Some("/dev/ttyUSB0".to_string()),
        baudrate: Some(115200),
        last_command_request_id: Some("cmd-99".to_string()),
        last_command_ok: Some(false),
        last_command_error: Some("relay did not settle".to_string()),
        updated_at: Utc::now(),
    };

    let disabled = runtime
        .reject_unpersisted_sample_with_status(Some(status), "sensor sample persistence failed");
    let command_reason = runtime
        .device_status
        .as_ref()
        .and_then(downstream_command_fault_reason)
        .unwrap();
    let changed = runtime.latch_control_fault(command_reason.clone());

    assert!(disabled);
    assert!(changed);
    assert!(runtime.latest_sample.is_none());
    assert_eq!(
        runtime.last_sensor_error.as_deref(),
        Some("sensor sample persistence failed")
    );
    assert_eq!(
        runtime.last_control_error.as_deref(),
        Some("downstream command cmd-99 failed: relay did not settle")
    );
    assert_eq!(
        decide_control(
            &safety,
            runtime.latest_sample.as_ref(),
            &runtime.targets,
            runtime.auto_enabled,
            runtime.manual_lock,
            runtime.emergency_stop,
            runtime.last_control_error.as_deref(),
        ),
        ControlDecision::Blocked(ControlBlockReason::ControlFault)
    );
}

#[test]
fn audit_failure_after_device_action_latches_control_fault() {
    let safety = safety();
    let mut runtime = RuntimeState::from_safety(&safety);
    runtime.auto_enabled = true;
    runtime.latest_sample = Some(SensorSnapshot {
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
    });

    let changed = runtime
        .latch_audit_failure_after_device_action("automatic control device_write", "disk full");

    assert!(changed);
    assert!(!runtime.auto_enabled);
    assert_eq!(
        runtime.last_control_error.as_deref(),
        Some("automatic control device_write audit failed after device action: disk full")
    );
    assert_eq!(
        decide_control(
            &safety,
            runtime.latest_sample.as_ref(),
            &runtime.targets,
            runtime.auto_enabled,
            runtime.manual_lock,
            runtime.emergency_stop,
            runtime.last_control_error.as_deref(),
        ),
        ControlDecision::Blocked(ControlBlockReason::ControlFault)
    );
}

#[test]
fn downstream_status_faults_separate_field_and_command_failures() {
    let status = DeviceStatusSnapshot {
        connected: true,
        last_seen_at: Some(Utc::now()),
        last_frame_ok: true,
        relay: Some(0),
        motor: Some(0),
        tilt: Some(1),
        speed_delay_us: Some(10000),
        port: Some("/dev/ttyUSB0".to_string()),
        baudrate: Some(115200),
        last_command_request_id: Some("req-42".to_string()),
        last_command_ok: Some(false),
        last_command_error: Some("relay timeout".to_string()),
        updated_at: Utc::now(),
    };

    assert_eq!(device_status_field_fault_reason(&status, 6000), None);
    assert_eq!(
        downstream_command_fault_reason(&status).as_deref(),
        Some("downstream command req-42 failed: relay timeout")
    );

    let disconnected = DeviceStatusSnapshot {
        connected: false,
        ..status
    };
    assert_eq!(
        device_status_field_fault_reason(&disconnected, 6000).as_deref(),
        Some("downstream device status is disconnected")
    );

    let future_status = DeviceStatusSnapshot {
        connected: true,
        last_seen_at: Some(Utc::now() + Duration::milliseconds(5000)),
        ..disconnected
    };
    assert!(device_status_field_fault_reason(&future_status, 6000)
        .unwrap()
        .contains("timestamp is"));
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
        decide_control(&safety, Some(&sample), &targets, true, false, false, None,),
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
        control_fault: None,
        device_status: None,
    });
    assert_eq!(
        response,
        SafetyGuardResponse::ControlDecision(decide_control(
            &safety,
            Some(&sample),
            &targets,
            true,
            false,
            false,
            None
        ))
    );
}
