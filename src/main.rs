use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::Result;
use clap::Parser;
use reactor_edge_daemon::{
    ai_provider::AiProvider,
    api::{serve, AppState},
    config::{load_device_config, load_safety_config, DeviceMode},
    control::{decide_control, ControlDecision},
    db::Db,
    demo::seed_demo_context,
    device::build_device,
    memory::load_ai_memory,
    state::{RuntimeState, SharedState},
};
use tokio::{sync::RwLock, time::sleep};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, default_value = "config/device.toml")]
    config: PathBuf,
    #[arg(long, default_value = "config/safety.toml")]
    safety: PathBuf,
    #[arg(long, default_value = "config/ai_memory.toml")]
    memory: PathBuf,
    #[arg(long, default_value = "data/reactor.sqlite3")]
    db: PathBuf,
    #[arg(long, default_value = "static")]
    assets: PathBuf,
    #[arg(long, default_value = "127.0.0.1:8000")]
    bind: SocketAddr,
    #[arg(long, default_value_t = false)]
    enable_test_reset: bool,
    #[arg(long, default_value_t = false)]
    seed_demo_context: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
    let device_config = load_device_config(&args.config)?;
    let device_mode = device_config.mode.clone();
    let safety = Arc::new(load_safety_config(&args.safety)?);
    let ai_memory = Arc::new(load_ai_memory(&args.memory)?);
    ai_memory.validate_against_optimizer_bounds(&safety.optimizer)?;
    let ai_provider = AiProvider::from_env()?.map(Arc::new);
    let db = Db::open(&args.db)?;
    if args.seed_demo_context {
        let inserted = seed_demo_context(&db, &safety, &ai_memory)?;
        if inserted {
            tracing::info!("seeded demo context without sensor samples");
        } else {
            tracing::info!("demo context already present; skipping seed");
        }
    }
    let device = build_device(&device_config)?;
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));

    let loop_state = Arc::clone(&runtime);
    let loop_db = db.clone();
    let loop_safety = Arc::clone(&safety);
    let loop_device = Arc::clone(&device);
    let loop_device_mode = device_mode.clone();
    tokio::spawn(async move {
        control_loop(
            loop_device,
            loop_db,
            loop_state,
            loop_safety,
            loop_device_mode,
        )
        .await;
    });

    serve(
        AppState {
            db,
            runtime,
            device,
            device_mode,
            safety,
            ai_memory,
            ai_provider,
            test_reset_enabled: args.enable_test_reset,
        },
        args.assets,
        args.bind,
    )
    .await
}

async fn control_loop(
    device: reactor_edge_daemon::device::SharedDevice,
    db: Db,
    runtime: SharedState,
    safety: Arc<reactor_edge_daemon::config::SafetyConfig>,
    device_mode: DeviceMode,
) {
    let interval = Duration::from_millis(safety.control.control_interval_ms);
    let mut last_written_command: Option<SafeCommandFingerprint> = None;
    loop {
        if matches!(device_mode, DeviceMode::Pipeline) {
            let mut state = runtime.write().await;
            if state.latest_sample.is_none() {
                state.last_sensor_error =
                    Some("waiting for external data pipeline sample".to_string());
            }
        } else {
            match device.read_sample_and_status().await {
                Ok((sample, status)) => {
                    let active_batch_id = {
                        let mut state = runtime.write().await;
                        state.latest_sample = Some(sample.clone());
                        state.last_sensor_error = None;
                        state.last_control_error = None;
                        state.device_status = status;
                        state.active_batch_id
                    };
                    if let Err(err) = db.insert_sample(active_batch_id, &sample) {
                        tracing::warn!("failed to persist sensor sample: {err}");
                    }
                }
                Err(err) => {
                    tracing::warn!("sensor read failed: {err}");
                    let mut state = runtime.write().await;
                    state.latest_sample = None;
                    state.last_sensor_error = Some(err.to_string());
                    state.device_status = None;
                }
            }
        }

        let decision = {
            let state = runtime.read().await;
            decide_control(
                &safety,
                state.latest_sample.as_ref(),
                &state.targets,
                state.auto_enabled,
                state.manual_lock,
                state.emergency_stop,
            )
        };

        match decision {
            ControlDecision::Write(command) => {
                let fingerprint = SafeCommandFingerprint::from(&command);
                if last_written_command.as_ref() == Some(&fingerprint) {
                    sleep(interval).await;
                    continue;
                }
                let active_batch_id = runtime.read().await.active_batch_id;
                match device.write_targets(&command).await {
                    Ok(()) => {
                        last_written_command = Some(fingerprint);
                        if let Err(err) = db.insert_control_event(
                            active_batch_id,
                            "device_write",
                            Some(&command),
                            &command.reason,
                        ) {
                            tracing::warn!("failed to persist control event: {err}");
                        }
                    }
                    Err(err) => {
                        tracing::warn!("device write failed: {err}");
                        last_written_command = None;
                        let mut state = runtime.write().await;
                        state.last_control_error = Some(err.to_string());
                    }
                }
            }
            ControlDecision::Blocked(reason) => {
                last_written_command = None;
                tracing::debug!("control blocked: {reason:?}");
            }
        }

        sleep(interval).await;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SafeCommandFingerprint {
    target_temperature_c: i64,
    heat_time_s: i64,
    hold_time_s: i64,
    cool_time_s: i64,
    target_stirrer_rpm: i64,
    target_shake_speed_cpm: i64,
    target_pressure_mpa: i64,
}

impl From<&reactor_edge_daemon::control::SafeCommand> for SafeCommandFingerprint {
    fn from(command: &reactor_edge_daemon::control::SafeCommand) -> Self {
        Self {
            target_temperature_c: scaled(command.target_temperature_c),
            heat_time_s: scaled(command.heat_time_s),
            hold_time_s: scaled(command.hold_time_s),
            cool_time_s: scaled(command.cool_time_s),
            target_stirrer_rpm: scaled(command.target_stirrer_rpm),
            target_shake_speed_cpm: scaled(command.target_shake_speed_cpm),
            target_pressure_mpa: scaled(command.target_pressure_mpa),
        }
    }
}

fn scaled(value: f64) -> i64 {
    (value * 100.0).round() as i64
}
