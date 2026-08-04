use rand::{rngs::StdRng, SeedableRng};
use reactor_edge_daemon::{
    config::{DeviceConfig, DeviceMode},
    device::ReactorDevice,
    state::validate_sensor_snapshot,
    virtual_sensor::{
        generate_for_scenario, validate_scenario_name, ScenarioContext, ScenarioParameters,
        SensorSourceType, SimulationConfig, VirtualSensorDevice,
    },
};

fn sim_config(scenario: &str, seed: u64) -> SimulationConfig {
    SimulationConfig {
        scenario: scenario.to_string(),
        seed,
        interval_ms: 1000,
        speed: 1.0,
        duration_seconds: None,
        parameters: ScenarioParameters::default(),
        persist_data: false,
    }
}

fn run_scenario_ticks(
    scenario: &str,
    seed: u64,
    ticks: u64,
) -> Vec<Option<reactor_edge_daemon::state::SensorSnapshot>> {
    let mut rng = StdRng::seed_from_u64(seed);
    let params = ScenarioParameters::default();
    let mut results = Vec::new();
    for tick in 1..=ticks {
        let mut ctx = ScenarioContext {
            tick,
            elapsed_ms: tick * 1000,
            interval_ms: 1000,
            rng: &mut rng,
            parameters: &params,
        };
        results.push(generate_for_scenario(scenario, &mut ctx));
    }
    results
}

#[test]
fn simulation_mode_parses_from_toml() {
    let toml_str = std::fs::read_to_string("config/device.simulation.toml").unwrap();
    let config: DeviceConfig = toml::from_str(&toml_str).unwrap();
    assert_eq!(config.mode, DeviceMode::Simulation);
    assert_eq!(config.simulation.scenario, "normal");
    assert_eq!(config.simulation.seed, 20260804);
    assert_eq!(config.simulation.interval_ms, 1000);
    assert!(!config.simulation.persist_data);
}

#[test]
fn pipeline_mode_still_uses_real_source_type() {
    let toml_str = std::fs::read_to_string("config/device.toml").unwrap();
    let config: DeviceConfig = toml::from_str(&toml_str).unwrap();
    assert_eq!(config.mode, DeviceMode::Pipeline);
    assert_eq!(SensorSourceType::default(), SensorSourceType::Real);
}

#[test]
fn virtual_data_conforms_to_sensor_schema() {
    let results = run_scenario_ticks("normal", 42, 20);
    for (i, sample_opt) in results.iter().enumerate() {
        let sample = sample_opt
            .as_ref()
            .expect(&format!("normal tick {} should produce a sample", i + 1));
        validate_sensor_snapshot(sample).expect(&format!(
            "normal tick {} sample must pass sensor validation",
            i + 1
        ));
    }
}

#[test]
fn same_seed_produces_reproducible_sequence() {
    let seq_a: Vec<_> = run_scenario_ticks("normal", 20260804, 30)
        .into_iter()
        .map(|s| s.map(|s| s.temperature_c))
        .collect();
    let seq_b: Vec<_> = run_scenario_ticks("normal", 20260804, 30)
        .into_iter()
        .map(|s| s.map(|s| s.temperature_c))
        .collect();
    assert_eq!(
        seq_a, seq_b,
        "same seed must produce same temperature sequence"
    );
}

#[test]
fn different_seed_produces_different_sequence() {
    let seq_a: Vec<_> = run_scenario_ticks("normal", 1, 20)
        .into_iter()
        .map(|s| s.map(|s| s.temperature_c))
        .collect();
    let seq_b: Vec<_> = run_scenario_ticks("normal", 2, 20)
        .into_iter()
        .map(|s| s.map(|s| s.temperature_c))
        .collect();
    assert_ne!(
        seq_a, seq_b,
        "different seeds should produce different sequences"
    );
}

#[test]
fn slow_rise_shows_increasing_trend() {
    let params = ScenarioParameters {
        start_value: Some(30.0),
        target_value: Some(90.0),
        period_seconds: Some(10.0),
        ..Default::default()
    };
    let mut rng = StdRng::seed_from_u64(100);
    let early = {
        let mut ctx = ScenarioContext {
            tick: 1,
            elapsed_ms: 1000,
            interval_ms: 1000,
            rng: &mut rng,
            parameters: &params,
        };
        generate_for_scenario("slow_rise", &mut ctx).unwrap()
    };
    let late = {
        let mut ctx = ScenarioContext {
            tick: 9,
            elapsed_ms: 9000,
            interval_ms: 1000,
            rng: &mut rng,
            parameters: &params,
        };
        generate_for_scenario("slow_rise", &mut ctx).unwrap()
    };
    assert!(
        late.temperature_c > early.temperature_c,
        "slow_rise: late ({}) should exceed early ({})",
        late.temperature_c,
        early.temperature_c
    );
}

#[test]
fn frozen_value_is_constant_across_ticks() {
    let results = run_scenario_ticks("frozen_value", 7, 10);
    let first_temp = results[0].as_ref().unwrap().temperature_c;
    for (i, s) in results.iter().enumerate() {
        let temp = s.as_ref().unwrap().temperature_c;
        assert!(
            (temp - first_temp).abs() < 0.01,
            "frozen tick {} temp {} should match first {}",
            i + 1,
            temp,
            first_temp
        );
    }
}

#[test]
fn sensor_disconnect_returns_none_after_threshold() {
    let results = run_scenario_ticks("sensor_disconnect", 99, 10);
    for (i, s) in results.iter().enumerate() {
        if i < 5 {
            assert!(s.is_some(), "tick {} should still produce data", i + 1);
        } else {
            assert!(
                s.is_none(),
                "tick {} should produce no data after disconnect",
                i + 1
            );
        }
    }
}

#[test]
fn intermittent_data_alternates_data_and_gaps() {
    let results = run_scenario_ticks("intermittent_data", 50, 10);
    let has_some = results.iter().any(|s| s.is_some());
    let has_none = results.iter().any(|s| s.is_none());
    assert!(
        has_some && has_none,
        "intermittent_data must alternate: {:?}",
        results.iter().map(|s| s.is_some()).collect::<Vec<_>>()
    );
}

#[test]
fn recovery_scenario_transitions_from_fault_to_normal() {
    let params = ScenarioParameters {
        start_value: Some(5.0),
        target_value: Some(130.0),
        ..Default::default()
    };
    let mut rng = StdRng::seed_from_u64(3);
    let fault_temps: Vec<f64> = (1..=5)
        .map(|tick| {
            let mut ctx = ScenarioContext {
                tick,
                elapsed_ms: tick * 1000,
                interval_ms: 1000,
                rng: &mut rng,
                parameters: &params,
            };
            generate_for_scenario("recovery", &mut ctx)
                .unwrap()
                .temperature_c
        })
        .collect();
    let recovered_temp = {
        let mut ctx = ScenarioContext {
            tick: 10,
            elapsed_ms: 10000,
            interval_ms: 1000,
            rng: &mut rng,
            parameters: &params,
        };
        generate_for_scenario("recovery", &mut ctx)
            .unwrap()
            .temperature_c
    };
    let avg_fault = fault_temps.iter().sum::<f64>() / fault_temps.len() as f64;
    assert!(
        avg_fault > 100.0,
        "fault phase average temp {} should be high",
        avg_fault
    );
    assert!(
        recovered_temp < 100.0,
        "recovered temp {} should be back to normal range",
        recovered_temp
    );
}

#[tokio::test]
async fn virtual_device_produces_sequential_samples() {
    let device = VirtualSensorDevice::new(sim_config("normal", 42));
    let s1 = device.read_sample().await.unwrap();
    let s2 = device.read_sample().await.unwrap();
    assert!(s1.temperature_c > -40.0 && s1.temperature_c < 500.0);
    assert!(s2.temperature_c > -40.0 && s2.temperature_c < 500.0);
}

#[tokio::test]
async fn session_start_prevents_double_start() {
    let device = VirtualSensorDevice::new(sim_config("normal", 1));
    let session = device.shared_session();
    {
        let mut s = session.write().await;
        assert!(s.active);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            s.restart();
        }));
        assert!(result.is_ok(), "restart while active should not panic");
    }
    {
        let s = session.read().await;
        assert!(s.active);
    }
}

#[tokio::test]
async fn session_stop_and_restart_cycle() {
    let device = VirtualSensorDevice::new(sim_config("normal", 1));
    let session = device.shared_session();
    {
        let mut s = session.write().await;
        s.stop();
        assert!(!s.active);
    }
    let result = device.read_sample().await;
    assert!(
        result.is_err(),
        "stopped session should not produce samples"
    );
    {
        let mut s = session.write().await;
        s.restart();
        assert!(s.active);
    }
    let sample = device.read_sample().await;
    assert!(
        sample.is_ok(),
        "restarted session should produce samples again"
    );
}

#[tokio::test]
async fn session_scenario_switch_resets_tick() {
    let device = VirtualSensorDevice::new(sim_config("normal", 1));
    let session = device.shared_session();
    for _ in 0..10 {
        device.read_sample().await.unwrap();
    }
    {
        let mut s = session.write().await;
        assert_eq!(s.tick, 10);
        s.switch_scenario("frozen_value".to_string(), Some(99), None);
        assert_eq!(s.tick, 0);
        assert_eq!(s.config.scenario, "frozen_value");
        assert_eq!(s.config.seed, 99);
    }
    let sample = device.read_sample().await.unwrap();
    {
        let s = session.read().await;
        assert_eq!(s.tick, 1);
        assert_eq!(s.config.scenario, "frozen_value");
    }
    let _ = sample;
}

#[tokio::test]
async fn virtual_device_write_targets_succeeds() {
    let device = VirtualSensorDevice::new(sim_config("normal", 1));
    use reactor_edge_daemon::control::SafeCommand;
    let cmd = SafeCommand {
        target_temperature_c: 80.0,
        heat_time_s: 300.0,
        hold_time_s: 600.0,
        cool_time_s: 180.0,
        target_stirrer_rpm: 300.0,
        target_shake_speed_cpm: 30.0,
        target_pressure_mpa: 0.5,
        reason: "test".to_string(),
    };
    device.write_targets(&cmd).await.unwrap();
}

#[tokio::test]
async fn virtual_device_returns_connected_status() {
    let device = VirtualSensorDevice::new(sim_config("normal", 1));
    let status = device.read_device_status().await.unwrap().unwrap();
    assert!(status.connected);
    assert!(status.last_frame_ok);
}

#[test]
fn validate_rejects_unknown_scenario() {
    assert!(validate_scenario_name("normal").is_ok());
    assert!(validate_scenario_name("slow_rise").is_ok());
    assert!(validate_scenario_name("nonexistent").is_err());
    assert!(validate_scenario_name("").is_err());
}

#[test]
fn source_type_defaults_to_real() {
    assert_eq!(SensorSourceType::default(), SensorSourceType::Real);
}

#[test]
fn out_of_range_scenario_may_produce_invalid_values() {
    let params = ScenarioParameters {
        target_value: Some(600.0),
        ..Default::default()
    };
    let mut rng = StdRng::seed_from_u64(1);
    let mut ctx = ScenarioContext {
        tick: 1,
        elapsed_ms: 1000,
        interval_ms: 1000,
        rng: &mut rng,
        parameters: &params,
    };
    let sample = generate_for_scenario("out_of_range", &mut ctx).unwrap();
    assert!(
        sample.temperature_c > 500.0,
        "out_of_range should produce temperature exceeding valid range, got {}",
        sample.temperature_c
    );
}

#[tokio::test]
async fn duration_limit_stops_session_automatically() {
    let config = SimulationConfig {
        scenario: "normal".to_string(),
        seed: 1,
        interval_ms: 100,
        speed: 1.0,
        duration_seconds: Some(0),
        parameters: ScenarioParameters::default(),
        persist_data: false,
    };
    let device = VirtualSensorDevice::new(config);
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    let result = device.read_sample().await;
    assert!(
        result.is_err(),
        "session with 0s duration should stop immediately"
    );
    let session = device.shared_session();
    let s = session.read().await;
    assert!(
        !s.active,
        "session should be inactive after duration expiry"
    );
}
