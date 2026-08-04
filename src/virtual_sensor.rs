use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{anyhow, Result};
use chrono::Utc;
use rand::{rngs::StdRng, Rng, SeedableRng};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{
    config::SafetyConfig,
    control::SafeCommand,
    device::{
        AckStatus, CommandAck, ComponentControlCommand, ComponentControlOutcome,
        DeviceComponentCapability, ReactorDevice,
    },
    number::round2,
    state::{fit_tilt_angle_deg, DeviceStatusSnapshot, SensorSnapshot},
};

const DEFAULT_BASE_TEMPERATURE_C: f64 = 45.0;
const DEFAULT_BASE_PRESSURE_MPA: f64 = 0.30;
const DEFAULT_BASE_STIRRER_RPM: f64 = 300.0;
const DEFAULT_BASE_SHAKE_SPEED_CPM: f64 = 25.0;
const DEFAULT_BASE_FLOW_RATE: f64 = 2.5;
const DEFAULT_BASE_CONCENTRATION: f64 = 35.0;
const DEFAULT_BASE_PH: f64 = 7.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorSourceType {
    Real,
    Simulation,
}

impl Default for SensorSourceType {
    fn default() -> Self {
        Self::Real
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorQuality {
    Good,
    Uncertain,
    Bad,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScenarioParameters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_value: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_value: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amplitude: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationConfig {
    #[serde(default = "default_scenario")]
    pub scenario: String,
    #[serde(default = "default_seed")]
    pub seed: u64,
    #[serde(default = "default_interval_ms")]
    pub interval_ms: u64,
    #[serde(default = "default_speed")]
    pub speed: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u64>,
    #[serde(default)]
    pub parameters: ScenarioParameters,
    #[serde(default = "default_persist_data")]
    pub persist_data: bool,
}

impl Default for SimulationConfig {
    fn default() -> Self {
        Self {
            scenario: default_scenario(),
            seed: default_seed(),
            interval_ms: default_interval_ms(),
            speed: default_speed(),
            duration_seconds: None,
            parameters: ScenarioParameters::default(),
            persist_data: default_persist_data(),
        }
    }
}

fn default_scenario() -> String {
    "normal".to_string()
}
fn default_seed() -> u64 {
    42
}
fn default_interval_ms() -> u64 {
    1000
}
fn default_speed() -> f64 {
    1.0
}
fn default_persist_data() -> bool {
    false
}

#[derive(Debug, Clone, Serialize)]
pub struct SimulationStatus {
    pub active: bool,
    pub session_id: Option<String>,
    pub scenario: Option<String>,
    pub seed: Option<u64>,
    pub interval_ms: u64,
    pub speed: f64,
    pub tick: u64,
    pub elapsed_seconds: f64,
    pub last_sample: Option<SensorSnapshot>,
    pub source_type: SensorSourceType,
}

pub type SharedSimulationSession = Arc<RwLock<SimulationSession>>;

impl SimulationStatus {
    /// A dormant status returned when the daemon is not running in simulation
    /// mode. Used by `/api/simulation/status` so a read-only query on a
    /// non-simulation deployment returns a parseable body (`active: false`)
    /// instead of an HTTP 4xx error.
    pub fn dormant() -> Self {
        Self {
            active: false,
            session_id: None,
            scenario: None,
            seed: None,
            interval_ms: 0,
            speed: 0.0,
            tick: 0,
            elapsed_seconds: 0.0,
            last_sample: None,
            source_type: SensorSourceType::Real,
        }
    }
}

pub struct SimulationSession {
    pub config: SimulationConfig,
    pub active: bool,
    pub tick: u64,
    pub start_instant: Instant,
    pub rng: StdRng,
    pub session_id: String,
    pub last_sample: Option<SensorSnapshot>,
}

impl SimulationSession {
    pub fn new(config: SimulationConfig) -> Self {
        let session_id = format!("sim-{}", Utc::now().timestamp_millis());
        Self {
            rng: StdRng::seed_from_u64(config.seed),
            config,
            active: true,
            tick: 0,
            start_instant: Instant::now(),
            session_id,
            last_sample: None,
        }
    }

    pub fn status(&self) -> SimulationStatus {
        SimulationStatus {
            active: self.active,
            session_id: Some(self.session_id.clone()),
            scenario: Some(self.config.scenario.clone()),
            seed: Some(self.config.seed),
            interval_ms: self.config.interval_ms,
            speed: self.config.speed,
            tick: self.tick,
            elapsed_seconds: self.start_instant.elapsed().as_secs_f64(),
            last_sample: self.last_sample.clone(),
            source_type: SensorSourceType::Simulation,
        }
    }

    fn elapsed_ms(&self) -> u64 {
        (self.start_instant.elapsed().as_millis() as f64 * self.config.speed) as u64
    }

    pub fn restart(&mut self) {
        self.active = true;
        self.tick = 0;
        self.start_instant = Instant::now();
        self.rng = StdRng::seed_from_u64(self.config.seed);
    }

    pub fn stop(&mut self) {
        self.active = false;
    }

    pub fn switch_scenario(
        &mut self,
        scenario: String,
        seed: Option<u64>,
        parameters: Option<ScenarioParameters>,
    ) {
        self.config.scenario = scenario;
        if let Some(seed) = seed {
            self.config.seed = seed;
        }
        if let Some(params) = parameters {
            self.config.parameters = params;
        }
        self.tick = 0;
        self.start_instant = Instant::now();
        self.rng = StdRng::seed_from_u64(self.config.seed);
    }

    fn generate_sample(&mut self) -> Option<SensorSnapshot> {
        if !self.active {
            return None;
        }
        if let Some(dur) = self.config.duration_seconds {
            // Measure duration in simulated time (scaled by `speed`) so that
            // speed!=1.0 does not distort the configured session lifetime
            // relative to tick/elapsed_ms progression.
            if self.elapsed_ms() / 1000 >= dur {
                self.active = false;
                tracing::info!(
                    "simulation session {} reached duration limit ({}s), stopping",
                    self.session_id,
                    dur
                );
                return None;
            }
        }

        self.tick += 1;
        let interval_ms = self.config.interval_ms.max(1);
        let elapsed_ms = self.elapsed_ms();
        let mut ctx = ScenarioContext {
            tick: self.tick,
            elapsed_ms,
            interval_ms,
            rng: &mut self.rng,
            parameters: &self.config.parameters,
        };
        let output = generate_for_scenario(&self.config.scenario, &mut ctx);
        if let Some(sample) = output {
            self.last_sample = Some(sample.clone());
            Some(sample)
        } else {
            None
        }
    }
}

pub struct VirtualSensorDevice {
    session: SharedSimulationSession,
}

impl VirtualSensorDevice {
    pub fn new(config: SimulationConfig) -> Self {
        let session = Arc::new(RwLock::new(SimulationSession::new(config)));
        Self { session }
    }

    pub fn shared_session(&self) -> SharedSimulationSession {
        Arc::clone(&self.session)
    }
}

#[async_trait::async_trait]
impl ReactorDevice for VirtualSensorDevice {
    async fn read_sample(&self) -> Result<SensorSnapshot> {
        let mut session = self.session.write().await;
        match session.generate_sample() {
            Some(sample) => Ok(sample),
            None => Err(anyhow!("virtual sensor session is not producing data")),
        }
    }

    async fn read_sample_and_status(
        &self,
    ) -> Result<(SensorSnapshot, Option<DeviceStatusSnapshot>)> {
        let sample = self.read_sample().await?;
        let status = DeviceStatusSnapshot {
            connected: true,
            last_seen_at: Some(Utc::now()),
            last_frame_ok: true,
            relay: None,
            motor: None,
            tilt: None,
            speed_delay_us: None,
            port: None,
            baudrate: None,
            last_command_request_id: None,
            last_command_ok: None,
            last_command_error: None,
            updated_at: Utc::now(),
        };
        Ok((sample, Some(status)))
    }

    async fn write_targets(&self, _command: &SafeCommand) -> Result<()> {
        Ok(())
    }

    async fn write_targets_acknowledged(
        &self,
        _command: &SafeCommand,
        request_id: &str,
        _timeout: Duration,
    ) -> Result<CommandAck> {
        Ok(CommandAck {
            request_id: request_id.to_string(),
            status: AckStatus::Confirmed,
            accepted_targets: None,
        })
    }

    async fn read_device_status(&self) -> Result<Option<DeviceStatusSnapshot>> {
        Ok(Some(DeviceStatusSnapshot {
            connected: true,
            last_seen_at: Some(Utc::now()),
            last_frame_ok: true,
            relay: None,
            motor: None,
            tilt: None,
            speed_delay_us: None,
            port: None,
            baudrate: None,
            last_command_request_id: None,
            last_command_ok: None,
            last_command_error: None,
            updated_at: Utc::now(),
        }))
    }

    fn control_capabilities(&self) -> Vec<DeviceComponentCapability> {
        Vec::new()
    }

    async fn write_component(
        &self,
        _command: &ComponentControlCommand,
        _targets: &crate::state::ControlTargets,
        _safety: &SafetyConfig,
    ) -> Result<Option<ComponentControlOutcome>> {
        Err(anyhow!(
            "virtual sensor device does not support component control"
        ))
    }
}

pub struct ScenarioContext<'a> {
    pub tick: u64,
    pub elapsed_ms: u64,
    pub interval_ms: u64,
    pub rng: &'a mut StdRng,
    pub parameters: &'a ScenarioParameters,
}

impl<'a> ScenarioContext<'a> {
    fn elapsed_seconds(&self) -> f64 {
        self.elapsed_ms as f64 / 1000.0
    }

    fn noise(&self) -> f64 {
        self.parameters.noise.unwrap_or(0.5).max(0.0)
    }

    /// Symmetric jitter in `[-amplitude, amplitude]`. Returns `0.0` for
    /// non-positive amplitude so `gen_range` never panics on empty/inverted
    /// ranges when an admin submits `noise = 0` or a negative value via the
    /// `/api/simulation/scenario` endpoint.
    fn jitter(&mut self, amplitude: f64) -> f64 {
        if amplitude <= 0.0 {
            0.0
        } else {
            self.rng.gen_range(-amplitude..amplitude)
        }
    }
}

fn make_sample(
    temperature_c: f64,
    pressure_mpa: f64,
    stirrer_rpm: f64,
    shake_speed_cpm: f64,
    tilt_state: u8,
    flow_rate_l_min: f64,
    product_concentration_percent: f64,
    ph: f64,
) -> SensorSnapshot {
    let captured_at = Utc::now();
    SensorSnapshot {
        temperature_c: round2(temperature_c),
        pressure_mpa: round2(pressure_mpa),
        stirrer_rpm: round2(stirrer_rpm),
        shake_speed_cpm: round2(shake_speed_cpm),
        tilt_state,
        tilt_angle_deg: fit_tilt_angle_deg(tilt_state, shake_speed_cpm, captured_at),
        flow_rate_l_min: round2(flow_rate_l_min),
        product_concentration_percent: round2(product_concentration_percent),
        ph: round2(ph),
        captured_at,
    }
}

pub fn generate_for_scenario(name: &str, ctx: &mut ScenarioContext) -> Option<SensorSnapshot> {
    let noise = ctx.noise();
    match name {
        "normal" => {
            let base = base_sample_raw(ctx);
            Some(add_noise_to_sample(base, noise * 0.3, ctx))
        }
        "slow_rise" => {
            let start = ctx.parameters.start_value.unwrap_or(30.0);
            let target = ctx.parameters.target_value.unwrap_or(85.0);
            let duration_s = ctx.parameters.period_seconds.unwrap_or(300.0);
            let progress = (ctx.elapsed_seconds() / duration_s).clamp(0.0, 1.0);
            let temp = start + (target - start) * progress;
            Some(make_sample(
                temp + ctx.jitter(noise),
                DEFAULT_BASE_PRESSURE_MPA,
                DEFAULT_BASE_STIRRER_RPM,
                DEFAULT_BASE_SHAKE_SPEED_CPM,
                0,
                DEFAULT_BASE_FLOW_RATE,
                DEFAULT_BASE_CONCENTRATION,
                DEFAULT_BASE_PH,
            ))
        }
        "slow_fall" => {
            let start = ctx.parameters.start_value.unwrap_or(85.0);
            let target = ctx.parameters.target_value.unwrap_or(30.0);
            let duration_s = ctx.parameters.period_seconds.unwrap_or(300.0);
            let progress = (ctx.elapsed_seconds() / duration_s).clamp(0.0, 1.0);
            let temp = start + (target - start) * progress;
            Some(make_sample(
                temp + ctx.jitter(noise),
                DEFAULT_BASE_PRESSURE_MPA,
                DEFAULT_BASE_STIRRER_RPM,
                DEFAULT_BASE_SHAKE_SPEED_CPM,
                0,
                DEFAULT_BASE_FLOW_RATE,
                DEFAULT_BASE_CONCENTRATION,
                DEFAULT_BASE_PH,
            ))
        }
        "sudden_spike" => {
            let spike_after_ticks = ctx.parameters.start_value.unwrap_or(10.0) as u64;
            let spike_value = ctx.parameters.target_value.unwrap_or(150.0);
            if ctx.tick <= spike_after_ticks {
                let base = base_sample_raw(ctx);
                Some(add_noise_to_sample(base, noise * 0.3, ctx))
            } else {
                let hold_ticks = ctx.parameters.period_seconds.unwrap_or(5.0) as u64;
                if ctx.tick <= spike_after_ticks + hold_ticks {
                    Some(make_sample(
                        spike_value + ctx.jitter(noise),
                        DEFAULT_BASE_PRESSURE_MPA + 0.2,
                        DEFAULT_BASE_STIRRER_RPM,
                        DEFAULT_BASE_SHAKE_SPEED_CPM,
                        0,
                        DEFAULT_BASE_FLOW_RATE,
                        DEFAULT_BASE_CONCENTRATION,
                        DEFAULT_BASE_PH,
                    ))
                } else {
                    let base = base_sample_raw(ctx);
                    Some(add_noise_to_sample(base, noise * 0.3, ctx))
                }
            }
        }
        "out_of_range" => {
            let oob_value = ctx.parameters.target_value.unwrap_or(600.0);
            Some(make_sample(
                oob_value,
                DEFAULT_BASE_PRESSURE_MPA,
                DEFAULT_BASE_STIRRER_RPM,
                DEFAULT_BASE_SHAKE_SPEED_CPM,
                0,
                DEFAULT_BASE_FLOW_RATE,
                DEFAULT_BASE_CONCENTRATION,
                DEFAULT_BASE_PH,
            ))
        }
        "frozen_value" => {
            let frozen_temp = ctx.parameters.start_value.unwrap_or(55.0);
            Some(make_sample(
                frozen_temp,
                DEFAULT_BASE_PRESSURE_MPA,
                DEFAULT_BASE_STIRRER_RPM,
                DEFAULT_BASE_SHAKE_SPEED_CPM,
                0,
                DEFAULT_BASE_FLOW_RATE,
                DEFAULT_BASE_CONCENTRATION,
                DEFAULT_BASE_PH,
            ))
        }
        "sensor_disconnect" => {
            let disconnect_after = ctx.parameters.start_value.unwrap_or(5.0) as u64;
            if ctx.tick <= disconnect_after {
                let base = base_sample_raw(ctx);
                Some(add_noise_to_sample(base, noise * 0.3, ctx))
            } else {
                None
            }
        }
        "noisy_signal" => {
            let base = base_sample_raw(ctx);
            Some(add_noise_to_sample(base, noise * 5.0, ctx))
        }
        "intermittent_data" => {
            let on_ticks = ctx.parameters.start_value.unwrap_or(3.0) as u64;
            let off_ticks = ctx.parameters.target_value.unwrap_or(2.0) as u64;
            let cycle = on_ticks + off_ticks;
            let phase = ctx.tick % cycle.max(1);
            if phase < on_ticks {
                let base = base_sample_raw(ctx);
                Some(add_noise_to_sample(base, noise * 0.3, ctx))
            } else {
                None
            }
        }
        "recovery" => {
            let fault_ticks = ctx.parameters.start_value.unwrap_or(10.0) as u64;
            if ctx.tick <= fault_ticks {
                let fault_temp = ctx.parameters.target_value.unwrap_or(130.0);
                Some(make_sample(
                    fault_temp + ctx.jitter(noise),
                    DEFAULT_BASE_PRESSURE_MPA + 0.15,
                    DEFAULT_BASE_STIRRER_RPM,
                    DEFAULT_BASE_SHAKE_SPEED_CPM,
                    0,
                    DEFAULT_BASE_FLOW_RATE,
                    DEFAULT_BASE_CONCENTRATION,
                    DEFAULT_BASE_PH,
                ))
            } else {
                let base = base_sample_raw(ctx);
                Some(add_noise_to_sample(base, noise * 0.3, ctx))
            }
        }
        _ => {
            tracing::warn!(
                "unknown simulation scenario '{}', falling back to normal",
                name
            );
            let base = base_sample_raw(ctx);
            Some(add_noise_to_sample(base, noise * 0.3, ctx))
        }
    }
}

fn base_sample_raw(_ctx: &ScenarioContext) -> SensorSnapshot {
    make_sample(
        DEFAULT_BASE_TEMPERATURE_C,
        DEFAULT_BASE_PRESSURE_MPA,
        DEFAULT_BASE_STIRRER_RPM,
        DEFAULT_BASE_SHAKE_SPEED_CPM,
        0,
        DEFAULT_BASE_FLOW_RATE,
        DEFAULT_BASE_CONCENTRATION,
        DEFAULT_BASE_PH,
    )
}

fn add_noise_to_sample(
    mut sample: SensorSnapshot,
    magnitude: f64,
    ctx: &mut ScenarioContext,
) -> SensorSnapshot {
    sample.temperature_c = round2(sample.temperature_c + ctx.jitter(magnitude));
    sample.pressure_mpa = round2(sample.pressure_mpa + ctx.jitter(magnitude * 0.02));
    sample.stirrer_rpm = round2(sample.stirrer_rpm + ctx.jitter(magnitude * 2.0));
    sample.shake_speed_cpm = round2(sample.shake_speed_cpm + ctx.jitter(magnitude * 0.2));
    sample.flow_rate_l_min = round2(sample.flow_rate_l_min + ctx.jitter(magnitude * 0.1));
    sample.product_concentration_percent =
        round2(sample.product_concentration_percent + ctx.jitter(magnitude));
    sample.ph = round2(sample.ph + ctx.jitter(magnitude * 0.05));
    sample.tilt_angle_deg = fit_tilt_angle_deg(
        sample.tilt_state,
        sample.shake_speed_cpm,
        sample.captured_at,
    );
    sample
}

pub fn validate_scenario_name(name: &str) -> Result<()> {
    const VALID: &[&str] = &[
        "normal",
        "slow_rise",
        "slow_fall",
        "sudden_spike",
        "out_of_range",
        "frozen_value",
        "sensor_disconnect",
        "noisy_signal",
        "intermittent_data",
        "recovery",
    ];
    if VALID.contains(&name) {
        Ok(())
    } else {
        Err(anyhow!(
            "unknown scenario '{}'; valid scenarios: {}",
            name,
            VALID.join(", ")
        ))
    }
}

pub fn available_scenarios() -> &'static [&'static str] {
    &[
        "normal",
        "slow_rise",
        "slow_fall",
        "sudden_spike",
        "out_of_range",
        "frozen_value",
        "sensor_disconnect",
        "noisy_signal",
        "intermittent_data",
        "recovery",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_produces_same_sequence() {
        let seed = 20260804;
        let mut seq_a: Vec<f64> = Vec::new();
        let mut seq_b: Vec<f64> = Vec::new();
        for tick in 1..=20 {
            let mut rng_a = StdRng::seed_from_u64(seed);
            for _ in 0..tick {
                rng_a.gen::<f64>();
            }
            // Reset and re-advance
            let mut rng_a2 = StdRng::seed_from_u64(seed);
            for _ in 0..tick {
                rng_a2.gen::<f64>();
            }
            let val_a: f64 = rng_a2.gen_range(0.0..1.0);

            let mut rng_b = StdRng::seed_from_u64(seed);
            for _ in 0..tick {
                rng_b.gen::<f64>();
            }
            let val_b: f64 = rng_b.gen_range(0.0..1.0);
            seq_a.push(val_a);
            seq_b.push(val_b);
        }
        assert_eq!(seq_a, seq_b, "same seed must produce same sequence");
    }

    #[test]
    fn normal_scenario_produces_valid_samples() {
        let seed = 42;
        let mut rng = StdRng::seed_from_u64(seed);
        let params = ScenarioParameters::default();
        for tick in 1..=10 {
            let mut ctx = ScenarioContext {
                tick,
                elapsed_ms: tick * 1000,
                interval_ms: 1000,
                rng: &mut rng,
                parameters: &params,
            };
            let sample = generate_for_scenario("normal", &mut ctx);
            assert!(
                sample.is_some(),
                "normal scenario tick {} should produce a sample",
                tick
            );
            let s = sample.unwrap();
            assert!(s.temperature_c > -40.0 && s.temperature_c < 500.0);
            assert!(s.ph > 0.0 && s.ph < 14.0);
        }
    }

    #[test]
    fn slow_rise_shows_increasing_temperature() {
        let seed = 100;
        let mut rng = StdRng::seed_from_u64(seed);
        let params = ScenarioParameters {
            start_value: Some(30.0),
            target_value: Some(90.0),
            period_seconds: Some(10.0),
            ..Default::default()
        };
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
                tick: 10,
                elapsed_ms: 10000,
                interval_ms: 1000,
                rng: &mut rng,
                parameters: &params,
            };
            generate_for_scenario("slow_rise", &mut ctx).unwrap()
        };
        assert!(
            late.temperature_c > early.temperature_c,
            "slow_rise: late temp {} should exceed early temp {}",
            late.temperature_c,
            early.temperature_c
        );
    }

    #[test]
    fn frozen_value_is_constant() {
        let seed = 7;
        let mut rng = StdRng::seed_from_u64(seed);
        let params = ScenarioParameters {
            start_value: Some(55.0),
            ..Default::default()
        };
        let mut temps = Vec::new();
        for tick in 1..=5 {
            let mut ctx = ScenarioContext {
                tick,
                elapsed_ms: tick * 1000,
                interval_ms: 1000,
                rng: &mut rng,
                parameters: &params,
            };
            let s = generate_for_scenario("frozen_value", &mut ctx).unwrap();
            temps.push(s.temperature_c);
        }
        let first = temps[0];
        assert!(
            temps.iter().all(|t| (*t - first).abs() < 0.01),
            "frozen temps should be constant: {:?}",
            temps
        );
    }

    #[test]
    fn sensor_disconnect_returns_none_after_threshold() {
        let seed = 99;
        let mut rng = StdRng::seed_from_u64(seed);
        let params = ScenarioParameters {
            start_value: Some(3.0),
            ..Default::default()
        };
        for tick in 1..=3 {
            let mut ctx = ScenarioContext {
                tick,
                elapsed_ms: tick * 1000,
                interval_ms: 1000,
                rng: &mut rng,
                parameters: &params,
            };
            let s = generate_for_scenario("sensor_disconnect", &mut ctx);
            assert!(s.is_some(), "tick {} should still produce data", tick);
        }
        let mut ctx = ScenarioContext {
            tick: 5,
            elapsed_ms: 5000,
            interval_ms: 1000,
            rng: &mut rng,
            parameters: &params,
        };
        let s = generate_for_scenario("sensor_disconnect", &mut ctx);
        assert!(
            s.is_none(),
            "after threshold, sensor_disconnect should produce no data"
        );
    }

    #[test]
    fn intermittent_data_alternates() {
        let seed = 50;
        let mut rng = StdRng::seed_from_u64(seed);
        let params = ScenarioParameters {
            start_value: Some(3.0),
            target_value: Some(2.0),
            ..Default::default()
        };
        let mut has_data = Vec::new();
        for tick in 1..=10 {
            let mut ctx = ScenarioContext {
                tick,
                elapsed_ms: tick * 1000,
                interval_ms: 1000,
                rng: &mut rng,
                parameters: &params,
            };
            let s = generate_for_scenario("intermittent_data", &mut ctx);
            has_data.push(s.is_some());
        }
        let has_some = has_data.iter().any(|&x| x);
        let has_none = has_data.iter().any(|&x| !x);
        assert!(
            has_some && has_none,
            "intermittent data should alternate between data and gaps: {:?}",
            has_data
        );
    }

    #[test]
    fn validate_rejects_unknown_scenario() {
        assert!(validate_scenario_name("normal").is_ok());
        assert!(validate_scenario_name("nonexistent").is_err());
    }

    #[test]
    fn available_scenarios_includes_all_required() {
        let scenarios = available_scenarios();
        for required in &[
            "normal",
            "slow_rise",
            "slow_fall",
            "sudden_spike",
            "out_of_range",
            "frozen_value",
            "sensor_disconnect",
            "noisy_signal",
            "intermittent_data",
            "recovery",
        ] {
            assert!(
                scenarios.contains(required),
                "required scenario '{}' missing from available_scenarios()",
                required
            );
        }
    }

    #[tokio::test]
    async fn virtual_device_read_sample_returns_data() {
        let config = SimulationConfig {
            scenario: "normal".to_string(),
            seed: 42,
            interval_ms: 100,
            speed: 1.0,
            duration_seconds: None,
            parameters: ScenarioParameters::default(),
            persist_data: false,
        };
        let device = VirtualSensorDevice::new(config);
        let sample = device.read_sample().await.unwrap();
        assert!(sample.temperature_c > -40.0 && sample.temperature_c < 500.0);
        let sample2 = device.read_sample().await.unwrap();
        assert!(sample2.temperature_c > -40.0);
    }

    #[tokio::test]
    async fn virtual_device_session_status_reports_active() {
        let config = SimulationConfig::default();
        let device = VirtualSensorDevice::new(config);
        let status = device.session.read().await.status();
        assert!(status.active);
        assert_eq!(status.scenario.as_deref(), Some("normal"));
        assert_eq!(status.seed, Some(42));
    }

    #[tokio::test]
    async fn virtual_device_disconnect_scenario_eventually_errors() {
        let config = SimulationConfig {
            scenario: "sensor_disconnect".to_string(),
            seed: 1,
            interval_ms: 100,
            speed: 1.0,
            duration_seconds: None,
            parameters: ScenarioParameters {
                start_value: Some(2.0),
                ..Default::default()
            },
            persist_data: false,
        };
        let device = VirtualSensorDevice::new(config);
        let s1 = device.read_sample().await.unwrap();
        assert!(s1.temperature_c > 0.0);
        device.read_sample().await.unwrap();
        let result = device.read_sample().await;
        assert!(
            result.is_err(),
            "third read should error after disconnect threshold"
        );
    }

    #[tokio::test]
    async fn duration_limit_stops_session() {
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
        tokio::time::sleep(Duration::from_millis(50)).await;
        let result = device.read_sample().await;
        assert!(
            result.is_err(),
            "session with 0s duration should not produce data after expiry"
        );
    }
}
