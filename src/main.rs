use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::Result;
use clap::Parser;
use reactor_edge_daemon::{
    api::{serve, AppState},
    config::{load_device_config, load_safety_config},
    control::{decide_control, ControlDecision},
    db::Db,
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
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
    let device_config = load_device_config(&args.config)?;
    let safety = Arc::new(load_safety_config(&args.safety)?);
    let ai_memory = Arc::new(load_ai_memory(&args.memory)?);
    ai_memory.validate_against_optimizer_bounds(&safety.optimizer)?;
    let db = Db::open(&args.db)?;
    let device = build_device(&device_config)?;
    let runtime: SharedState = Arc::new(RwLock::new(RuntimeState::from_safety(&safety)));

    let loop_state = Arc::clone(&runtime);
    let loop_db = db.clone();
    let loop_safety = Arc::clone(&safety);
    let loop_device = Arc::clone(&device);
    tokio::spawn(async move {
        control_loop(loop_device, loop_db, loop_state, loop_safety).await;
    });

    serve(
        AppState {
            db,
            runtime,
            safety,
            ai_memory,
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
) {
    let interval = Duration::from_millis(safety.control.control_interval_ms);
    loop {
        match device.read_sample().await {
            Ok(sample) => {
                let active_batch_id = {
                    let mut state = runtime.write().await;
                    state.latest_sample = Some(sample.clone());
                    state.last_control_error = None;
                    state.active_batch_id
                };
                if let Err(err) = db.insert_sample(active_batch_id, &sample) {
                    tracing::warn!("failed to persist sensor sample: {err}");
                }
            }
            Err(err) => {
                tracing::warn!("sensor read failed: {err}");
                let mut state = runtime.write().await;
                state.last_control_error = Some(err.to_string());
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
                let active_batch_id = runtime.read().await.active_batch_id;
                match device.write_targets(&command).await {
                    Ok(()) => {
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
                        let mut state = runtime.write().await;
                        state.last_control_error = Some(err.to_string());
                    }
                }
            }
            ControlDecision::Blocked(reason) => {
                tracing::debug!("control blocked: {reason:?}");
            }
        }

        sleep(interval).await;
    }
}
