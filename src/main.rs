use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use clap::Parser;
use reactor_edge_daemon::{
    ai_provider::AiProvider,
    api::{serve, AppState, HttpTlsConfig},
    bootstrap::{enforce_network_auth_gate, resolve_assets_dir},
    config::{load_device_config, load_safety_config, DeviceMode},
    control::{ControlBlockReason, ControlDecision},
    db::{AuditActor, Db},
    demo::seed_demo_context,
    device::{build_device, AckStatus},
    memory::load_ai_memory,
    modbus_tcp::start_modbus_tcp_server,
    mqtt::{load_integration_config, start_mqtt_bridge},
    runtime_recovery::recover_runtime_from_db,
    safety_guard::evaluate_with_process,
    state::{
        device_status_field_fault_reason, downstream_command_fault_reason, timestamp_age_ms,
        validate_sensor_snapshot, DeviceStatusSnapshot, RuntimeState, SensorSnapshot, SharedState,
    },
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
    #[arg(long, default_value = "config/integration.toml")]
    integration: PathBuf,
    #[arg(long, default_value = "data/reactor.sqlite3")]
    db: PathBuf,
    #[arg(long, default_value = "auto")]
    assets: PathBuf,
    #[arg(long, default_value = "127.0.0.1:8000")]
    bind: SocketAddr,
    #[arg(long)]
    tls_cert: Option<PathBuf>,
    #[arg(long)]
    tls_key: Option<PathBuf>,
    #[arg(long, default_value_t = false)]
    enable_test_reset: bool,
    #[arg(long)]
    safety_guard: Option<PathBuf>,
    #[arg(long, env = "XINGSHU_SEED_DEMO_CONTEXT", default_value_t = false)]
    seed_demo_context: bool,
}

// Cap the async runtime to 2 worker threads. The daemon runs on edge boards
// (e.g. LubanCat 2 / RK3568, 4x Cortex-A55) as a low-load process: one control
// loop, one HTTP server, occasional blocking serial/Modbus I/O. The default
// one-worker-per-core (4 here) just adds idle thread stacks and scheduler
// overhead; 2 workers keep the HTTP server responsive while the control loop or
// a spawn_blocking serial call runs, without paying for cores we never saturate.
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
    let args = Args {
        assets: resolve_assets_dir(&args.assets),
        ..args
    };
    let tls = match (args.tls_cert.clone(), args.tls_key.clone()) {
        (Some(cert), Some(key)) => Some(HttpTlsConfig { cert, key }),
        (None, None) => None,
        _ => bail!("--tls-cert and --tls-key must be provided together"),
    };
    if args.enable_test_reset && !args.bind.ip().is_loopback() {
        bail!(
            "--enable-test-reset may only be used with a loopback bind address; refusing to expose destructive test endpoints on {}",
            args.bind
        );
    }
    let auth_secret = std::env::var("XINGSHU_AUTH_SECRET").ok();
    enforce_network_auth_gate(args.bind, auth_secret.as_deref())?;
    let device_config = load_device_config(&args.config)?;
    let device_mode = device_config.mode.clone();
    let device_config = Arc::new(device_config);
    let safety = Arc::new(load_safety_config(&args.safety)?);
    let ai_memory = Arc::new(load_ai_memory(&args.memory)?);
    let integration = load_integration_config(&args.integration)?;
    ai_memory.validate_against_optimizer_bounds(&safety.optimizer)?;
    let ai_provider = AiProvider::from_env()?.map(Arc::new);
    let db = Db::open(&args.db)?;
    if !db.encryption_status().enabled {
        tracing::warn!(
            "DB column encryption is disabled; AINAS/MQTT integration task payloads will be stored as plaintext. Set XINGSHU_DB_ENCRYPTION_KEY (32 raw bytes / 64 hex / base64 of 32 bytes) to enable AES-256-GCM."
        );
    }
    if args.seed_demo_context {
        let inserted = seed_demo_context(&db, &safety, &ai_memory)?;
        if inserted {
            tracing::info!("seeded demo context without sensor samples");
        } else {
            tracing::info!("demo context already present; skipping seed");
        }
    }
    // Apply the same schema migration to the SQLx pool so the SQLx-only
    // read/write paths see a consistent schema even on a fresh database
    // where the rusqlite write connection has not yet been touched.
    //
    // Migration failure aborts startup. Tolerating it would mean a running
    // daemon with broken SQLx paths that 500 at runtime, which is worse
    // than refusing to start. Operators who genuinely need a degraded
    // boot can set the `XINGSHU_ALLOW_SQLX_MIGRATION_WARNING=1`
    // environment variable so the failure is logged as a warning instead
    // of bailing out.
    let allow_sqlx_warning = std::env::var("XINGSHU_ALLOW_SQLX_MIGRATION_WARNING")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if let Err(err) = db.migrate_sqlx().await {
        if allow_sqlx_warning {
            tracing::warn!("sqlx schema migration step skipped: {err}");
        } else {
            tracing::error!("sqlx schema migration failed: {err}");
            return Err(err);
        }
    }
    let (device, simulation_session) = match &device_mode {
        DeviceMode::Simulation => {
            tracing::info!(
                "starting in SIMULATION mode — virtual sensor data will flow through the normal pipeline; \
                 persist_data={}",
                device_config.simulation.persist_data
            );
            let sim = reactor_edge_daemon::virtual_sensor::VirtualSensorDevice::new(
                device_config.simulation.clone(),
            );
            let session = sim.shared_session();
            let shared: reactor_edge_daemon::device::SharedDevice = Arc::new(sim);
            (shared, Some(session))
        }
        _ => (build_device(&device_config)?, None),
    };
    let mut recovered_runtime = recover_runtime_from_db(&db, &safety).await?;
    if let Some(batch_id) = recovered_runtime.active_batch_id {
        tracing::warn!(
            "recovered unfinished batch {batch_id} at daemon startup; automatic control remains disabled until operator closes the batch"
        );
    }
    // 堆积告警:除 active 外,还有遗留的未完成批次(daemon 多次异常退出会堆积)。
    // 只加日志,不改控制流 —— operator 看到 extra > 0 应排查/关闭遗留批次。
    if let Ok(unfinished) = db.unfinished_batches_sqlx(100).await {
        let active_present = recovered_runtime.active_batch_id.is_some();
        let extra = unfinished
            .len()
            .saturating_sub(if active_present { 1 } else { 0 });
        if extra > 0 {
            tracing::warn!(
                "found {extra} stale unfinished batch(es) besides the active one; \
                 these were left unfinished by prior daemon exits — operator should close or investigate"
            );
        }
    }
    if matches!(device_mode, DeviceMode::Simulation) {
        recovered_runtime.source_type =
            reactor_edge_daemon::virtual_sensor::SensorSourceType::Simulation;
    }
    let runtime: SharedState = Arc::new(RwLock::new(recovered_runtime));

    let loop_state = Arc::clone(&runtime);
    let loop_db = db.clone();
    let loop_safety = Arc::clone(&safety);
    let loop_device = Arc::clone(&device);
    let loop_device_mode = device_mode.clone();
    let loop_safety_guard = args.safety_guard.clone();
    let loop_persist =
        !matches!(device_mode, DeviceMode::Simulation) || device_config.simulation.persist_data;
    let control_task = tokio::spawn(async move {
        control_loop(
            loop_device,
            loop_db,
            loop_state,
            loop_safety,
            loop_device_mode,
            loop_safety_guard,
            loop_persist,
        )
        .await;
    });
    // Fail-safe monitor: the control loop runs in a spawned task whose JoinHandle
    // we previously dropped, so a panic inside it would be silently swallowed and
    // the device would be left in its last commanded state while the API kept
    // serving — the hardest field failure to diagnose. If the task ever exits or
    // panics, disable automatic control, latch a control fault, and record an
    // audit event so the operator must re-verify field state before re-enabling.
    {
        let monitor_state = Arc::clone(&runtime);
        let monitor_db = db.clone();
        tokio::spawn(async move {
            let outcome = control_task.await;
            match &outcome {
                Err(join_err) => {
                    tracing::error!("control loop task panicked; failing safe: {join_err}");
                }
                Ok(()) => {
                    tracing::error!("control loop task exited unexpectedly; failing safe");
                }
            }
            let mut runtime = monitor_state.write().await;
            runtime.auto_enabled = false;
            // Mark the supervisor as dead: reset_control_fault refuses to clear
            // a fault while this is true, so the only way back to automatic
            // control is a process restart (which re-spawns the loop).
            runtime.control_loop_terminated = true;
            runtime.latch_control_fault(
                "control loop task terminated; automatic control disabled until process restart and field re-verification",
            );
            drop(runtime);
            if let Err(err) = monitor_db
                .insert_control_event_sqlx(
                    None,
                    "control_loop_terminated",
                    None,
                    match &outcome {
                        Err(_) => "control loop task panicked; automatic control disabled",
                        Ok(()) => {
                            "control loop task exited unexpectedly; automatic control disabled"
                        }
                    },
                    &AuditActor::system(),
                )
                .await
            {
                tracing::error!("failed to audit control loop termination: {err}");
            }
        });
    }

    let app_state = AppState {
        db,
        runtime,
        device,
        device_mode,
        device_config,
        safety,
        ai_memory,
        ai_provider,
        test_reset_enabled: args.enable_test_reset,
        simulation_session,
    };
    start_mqtt_bridge(integration.mqtt, app_state.clone());
    start_modbus_tcp_server(integration.modbus_tcp, app_state.clone());

    serve(app_state, args.assets, args.bind, tls).await
}

/// Accept a valid sensor sample into runtime state and audit any device-status
/// faults (field fault disables auto; downstream command fault latches a
/// control fault). Shared by the `!persist_samples` path (simulation mode
/// without persistence) and the DB-insert success path, so the two cannot
/// drift apart.
async fn accept_sample_and_audit_faults(
    db: &Db,
    runtime: &SharedState,
    sample: &SensorSnapshot,
    status: &Option<DeviceStatusSnapshot>,
    safety: &reactor_edge_daemon::config::SafetyConfig,
) {
    let fault = {
        let mut state = runtime.write().await;
        state.latest_sample = Some(sample.clone());
        state.last_sensor_error = None;
        state.device_status = status.clone();
        let active_batch_id = state.active_batch_id;
        match status.as_ref() {
            Some(status) => {
                if let Some(reason) =
                    device_status_field_fault_reason(status, safety.control.sensor_timeout_ms)
                {
                    let auto_was_disabled = state.disable_auto_for_field_fault(reason.clone());
                    Some((active_batch_id, auto_was_disabled, reason, false))
                } else if let Some(reason) = downstream_command_fault_reason(status) {
                    let should_audit = state.latch_control_fault(reason.clone());
                    Some((active_batch_id, should_audit, reason, true))
                } else {
                    None
                }
            }
            None => None,
        }
    };
    if let Some((active_batch_id, should_audit, reason, control_fault)) = fault {
        if control_fault {
            audit_downstream_control_fault(db, active_batch_id, should_audit, &reason).await;
        } else {
            audit_field_input_auto_disable(db, active_batch_id, should_audit, &reason).await;
        }
    }
}

async fn control_loop(
    device: reactor_edge_daemon::device::SharedDevice,
    db: Db,
    runtime: SharedState,
    safety: Arc<reactor_edge_daemon::config::SafetyConfig>,
    device_mode: DeviceMode,
    safety_guard: Option<PathBuf>,
    persist_samples: bool,
) {
    let interval = Duration::from_millis(safety.control.control_interval_ms);
    let mut last_written_command: Option<LastWrittenCommand> = None;
    let mut retry_after: Option<DateTime<Utc>> = None;
    // Monotonic counter for command-level handshake request_ids generated by
    // this control-loop task (single spawned task, so a plain u64 suffices).
    let mut command_seq: u64 = 0;
    loop {
        if matches!(device_mode, DeviceMode::Pipeline) {
            let fault = {
                let mut state = runtime.write().await;
                let reason = match &state.latest_sample {
                    Some(sample) => {
                        let age_ms = timestamp_age_ms(sample.captured_at);
                        if age_ms < 0 {
                            Some(format!(
                                "external data pipeline sample timestamp is {} ms in the future; check controller clock synchronization",
                                -age_ms
                            ))
                        } else if age_ms > safety.control.sensor_timeout_ms {
                            Some(format!(
                                "external data pipeline sample stale; last sample is {age_ms} ms old, max {} ms",
                                safety.control.sensor_timeout_ms
                            ))
                        } else {
                            None
                        }
                    }
                    None => Some("waiting for external data pipeline sample".to_string()),
                };
                reason.map(|reason| {
                    let active_batch_id = state.active_batch_id;
                    let auto_was_disabled = state.disable_auto_for_field_fault(reason.clone());
                    (active_batch_id, auto_was_disabled, reason)
                })
            };
            if let Some((active_batch_id, auto_was_disabled, reason)) = fault {
                audit_field_input_auto_disable(&db, active_batch_id, auto_was_disabled, &reason)
                    .await;
            }
        } else {
            // Bound the device read. Serial and file-bridge reads can block
            // indefinitely (firmware stuck mid-line, hung mount, serial lock
            // held by another stuck call); without a bound the whole
            // supervision loop stalls silently — staleness handling,
            // auto-disable and safety-guard consults stop while the HMI keeps
            // showing the last sample. sensor_timeout_ms is the natural
            // budget: a read that cannot finish within the freshness window
            // is a field fault by the same contract the sample freshness
            // check applies.
            let read_budget =
                Duration::from_millis(u64::try_from(safety.control.sensor_timeout_ms).unwrap_or(1));
            let read_result =
                match tokio::time::timeout(read_budget, device.read_sample_and_status()).await {
                    Ok(result) => result,
                    Err(_) => Err(anyhow::anyhow!(
                        "device read timed out after {} ms",
                        read_budget.as_millis()
                    )),
                };
            match read_result {
                Ok((sample, status)) => {
                    let active_batch_id = {
                        let state = runtime.read().await;
                        state.active_batch_id
                    };
                    if let Err(reason) = validate_sensor_snapshot(&sample) {
                        let reason = format!("sensor sample rejected: {reason}");
                        let (auto_was_disabled, command_fault) = {
                            let mut state = runtime.write().await;
                            let auto_was_disabled = state.reject_unpersisted_sample_with_status(
                                status.clone(),
                                reason.clone(),
                            );
                            let command_fault = status
                                .as_ref()
                                .and_then(downstream_command_fault_reason)
                                .map(|command_reason| {
                                    let should_audit =
                                        state.latch_control_fault(command_reason.clone());
                                    (should_audit, command_reason)
                                });
                            (auto_was_disabled, command_fault)
                        };
                        audit_field_input_auto_disable(
                            &db,
                            active_batch_id,
                            auto_was_disabled,
                            &reason,
                        )
                        .await;
                        if let Some((should_audit, command_reason)) = command_fault {
                            audit_downstream_control_fault(
                                &db,
                                active_batch_id,
                                should_audit,
                                &command_reason,
                            )
                            .await;
                        }
                    } else if !persist_samples {
                        accept_sample_and_audit_faults(
                            &db,
                            &runtime,
                            &sample,
                            &status,
                            safety.as_ref(),
                        )
                        .await;
                    } else if let Err(err) = db.insert_sample_sqlx(active_batch_id, &sample).await {
                        tracing::warn!("failed to persist sensor sample: {err}");
                        let reason = format!("sensor sample persistence failed: {err:#}");
                        let (auto_was_disabled, command_fault) = {
                            let mut state = runtime.write().await;
                            let auto_was_disabled = state.reject_unpersisted_sample_with_status(
                                status.clone(),
                                reason.clone(),
                            );
                            let command_fault = status
                                .as_ref()
                                .and_then(downstream_command_fault_reason)
                                .map(|command_reason| {
                                    let should_audit =
                                        state.latch_control_fault(command_reason.clone());
                                    (should_audit, command_reason)
                                });
                            (auto_was_disabled, command_fault)
                        };
                        audit_field_input_auto_disable(
                            &db,
                            active_batch_id,
                            auto_was_disabled,
                            &reason,
                        )
                        .await;
                        if let Some((should_audit, command_reason)) = command_fault {
                            audit_downstream_control_fault(
                                &db,
                                active_batch_id,
                                should_audit,
                                &command_reason,
                            )
                            .await;
                        }
                    } else {
                        accept_sample_and_audit_faults(
                            &db,
                            &runtime,
                            &sample,
                            &status,
                            safety.as_ref(),
                        )
                        .await;
                    }
                }
                Err(err) => {
                    tracing::warn!("sensor read failed: {err}");
                    let reason = err.to_string();
                    // Bound the follow-up status read too: it goes through the
                    // same device/serial lock that may just have timed out.
                    let status = match tokio::time::timeout(
                        read_budget,
                        device.read_device_status(),
                    )
                    .await
                    {
                        Ok(Ok(status)) => status,
                        Ok(Err(status_err)) => {
                            tracing::debug!(
                                "device status read after sensor failure failed: {status_err}"
                            );
                            None
                        }
                        Err(_) => {
                            tracing::debug!(
                                "device status read after sensor failure timed out after {} ms",
                                read_budget.as_millis()
                            );
                            None
                        }
                    };
                    let (active_batch_id, auto_was_disabled, command_fault) = {
                        let mut state = runtime.write().await;
                        state.latest_sample = None;
                        state.device_status = status.clone();
                        let active_batch_id = state.active_batch_id;
                        let auto_was_disabled = state.disable_auto_for_field_fault(reason.clone());
                        let command_fault = status
                            .as_ref()
                            .and_then(downstream_command_fault_reason)
                            .map(|command_reason| {
                                let should_audit =
                                    state.latch_control_fault(command_reason.clone());
                                (should_audit, command_reason)
                            });
                        (active_batch_id, auto_was_disabled, command_fault)
                    };
                    audit_field_input_auto_disable(
                        &db,
                        active_batch_id,
                        auto_was_disabled,
                        &reason,
                    )
                    .await;
                    if let Some((should_audit, command_reason)) = command_fault {
                        audit_downstream_control_fault(
                            &db,
                            active_batch_id,
                            should_audit,
                            &command_reason,
                        )
                        .await;
                    }
                }
            }
        }

        let (active_batch_id, auto_disabled_by_control_fault) = {
            let mut state = runtime.write().await;
            let active_batch_id = state.active_batch_id;
            let auto_disabled = state.enforce_control_fault_fail_closed();
            (active_batch_id, auto_disabled)
        };
        if auto_disabled_by_control_fault {
            audit_control_fault_auto_disable(&db, active_batch_id).await;
        }

        let decision = {
            // Clone a snapshot and drop the read lock BEFORE the optional guard
            // subprocess call: evaluate_with_process blocks for up to
            // safety_guard_timeout_ms, and holding the runtime read lock across
            // it would queue every runtime writer (API handlers, WS) behind the
            // guard wait. spawn_blocking also keeps the std::process wait off
            // the 2 tokio workers (the decision state snapshot semantics are
            // unchanged: the lock was never held past the decision anyway).
            let state = runtime.read().await.clone();
            let safety_for_guard = safety.clone();
            let guard_path = safety_guard.clone();
            let joined = tokio::task::spawn_blocking(move || {
                decide_control_with_optional_guard(&safety_for_guard, &state, guard_path.as_deref())
            })
            .await;
            match joined {
                Ok(decision) => decision,
                Err(err) => {
                    tracing::error!(
                        "safety guard decision task failed: {err}; blocking automatic control this cycle"
                    );
                    ControlDecision::Blocked(ControlBlockReason::ControlFault)
                }
            }
        };

        match decision {
            ControlDecision::Write(command) => {
                if retry_after.is_some_and(|deadline| Utc::now() < deadline) {
                    sleep(interval).await;
                    continue;
                }
                let fingerprint = SafeCommandFingerprint::from(&command);
                if last_written_command
                    .as_ref()
                    .is_some_and(|last| last.matches_recent(&fingerprint, safety.as_ref()))
                {
                    sleep(interval).await;
                    continue;
                }
                // Final interlock before the device write, split in two phases
                // so the safety-guard subprocess wait (up to
                // safety_guard_timeout_ms) never runs while holding the
                // runtime read lock — that would queue every runtime writer
                // (including e-stop engage) behind the guard:
                //   phase 1: batch consistency + guard re-decision on a
                //            snapshot, inside spawn_blocking;
                //   phase 2: short read lock, in-process re-verification that
                //            the LIVE state still produces exactly this
                //            command (catches latch/target changes that
                //            happened during the subprocess wait).
                let active_batch_id = {
                    let state_snapshot = runtime.read().await.clone();
                    let unfinished = match db.unfinished_batches_sqlx(100).await {
                        Ok(unfinished) => unfinished,
                        Err(err) => {
                            tracing::warn!(
                                "automatic control write skipped because unfinished batch state could not be read: {err}"
                            );
                            last_written_command = None;
                            retry_after = None;
                            let active_batch_id = state_snapshot.active_batch_id;
                            let mut state = runtime.write().await;
                            state.latch_control_fault(format!(
                                "automatic control blocked until unfinished batch state can be verified: {err}"
                            ));
                            drop(state);
                            audit_control_fault_auto_disable(&db, active_batch_id).await;
                            sleep(interval).await;
                            continue;
                        }
                    };
                    if let Err(reason) = automatic_batch_state_is_consistent(
                        state_snapshot.active_batch_id,
                        &unfinished,
                    ) {
                        tracing::warn!(
                            "automatic control write skipped because persisted batch state is inconsistent: {reason}"
                        );
                        last_written_command = None;
                        retry_after = None;
                        let active_batch_id = state_snapshot.active_batch_id;
                        let mut state = runtime.write().await;
                        let should_audit = state.latch_control_fault(reason.clone());
                        drop(state);
                        audit_automatic_batch_recovery_block(
                            &db,
                            active_batch_id,
                            should_audit,
                            &reason,
                        )
                        .await;
                        sleep(interval).await;
                        continue;
                    }
                    let safety_for_recheck = safety.clone();
                    let guard_path = safety_guard.clone();
                    let command_for_recheck = command.clone();
                    let snapshot_for_recheck = state_snapshot.clone();
                    let guard_recheck = match tokio::task::spawn_blocking(move || {
                        ensure_automatic_control_write_still_current(
                            &safety_for_recheck,
                            &snapshot_for_recheck,
                            guard_path.as_deref(),
                            &command_for_recheck,
                        )
                    })
                    .await
                    {
                        Ok(result) => result,
                        Err(join_err) => {
                            tracing::error!(
                                "safety guard recheck task failed: {join_err}; skipping this write"
                            );
                            Err(ControlDecision::Blocked(ControlBlockReason::ControlFault))
                        }
                    };
                    // Phase 2, single short read lock: the live state must
                    // still yield exactly this command AND the same active
                    // batch the DB consistency check above was run against —
                    // a batch stop/start during the subprocess wait would
                    // otherwise attribute this write to a batch never verified
                    // against the persisted unfinished set.
                    let (live_decision, live_active_batch_id) = {
                        let state = runtime.read().await;
                        (
                            decide_control_with_optional_guard(safety.as_ref(), &state, None),
                            state.active_batch_id,
                        )
                    };
                    let command_still_current = matches!(
                        &live_decision,
                        ControlDecision::Write(current) if *current == command
                    );
                    let batch_still_current =
                        live_active_batch_id == state_snapshot.active_batch_id;
                    if guard_recheck.is_ok() && command_still_current && !batch_still_current {
                        // Same command, but the active batch changed under us:
                        // not a field fault, just a stale consistency check.
                        // Skip this cycle; the next one re-verifies the new
                        // batch against the persisted unfinished set.
                        tracing::warn!(
                            "automatic control write skipped: active batch changed during final interlock ({:?} -> {:?})",
                            state_snapshot.active_batch_id,
                            live_active_batch_id
                        );
                        last_written_command = None;
                        retry_after = None;
                        sleep(interval).await;
                        continue;
                    }
                    let failed_decision = match guard_recheck {
                        Ok(_) if command_still_current => None,
                        Ok(_) => Some(live_decision),
                        Err(decision) => Some(decision),
                    };
                    match failed_decision {
                        None => live_active_batch_id,
                        Some(decision) => {
                            tracing::warn!(
                                "automatic control write skipped after final interlock recheck: {decision:?}"
                            );
                            last_written_command = None;
                            retry_after = None;
                            if let Some(reason) = automatic_final_interlock_fault_reason(&decision)
                            {
                                let mut state = runtime.write().await;
                                let active_batch_id = state.active_batch_id;
                                let should_audit = state.latch_control_fault(reason.clone());
                                drop(state);
                                audit_automatic_final_interlock_block(
                                    &db,
                                    active_batch_id,
                                    should_audit,
                                    &reason,
                                )
                                .await;
                            }
                            sleep(interval).await;
                            continue;
                        }
                    }
                };
                command_seq = command_seq.wrapping_add(1);
                let request_id = format!("auto-{}-{}", Utc::now().timestamp_millis(), command_seq);
                let ack_timeout = Duration::from_millis(safety.control.command_ack_timeout_ms);
                match device
                    .write_targets_acknowledged(&command, &request_id, ack_timeout)
                    .await
                {
                    Ok(ack) => {
                        let outcome = classify_ack_outcome(
                            &ack.status,
                            safety.control.require_command_ack,
                            safety.control.command_ack_timeout_ms,
                        );
                        if outcome.confirmed {
                            // Confirmed (or legacy unverified-allowed): persist the
                            // device_write audit with the ack status + rid, then mark
                            // the command as last written. An audit write failure still
                            // latches a fault because the device has already acted
                            // (CLAUDE 3.6 fail-after-effective-op).
                            let audit_reason = format!(
                                "{}|ack={}|rid={}",
                                command.reason,
                                ack_status_token(&ack.status),
                                ack.request_id
                            );
                            if let Err(err) = db
                                .insert_control_event_sqlx(
                                    active_batch_id,
                                    "device_write",
                                    Some(&command),
                                    &audit_reason,
                                    &AuditActor::system(),
                                )
                                .await
                            {
                                tracing::warn!("failed to persist control event: {err}");
                                last_written_command = None;
                                retry_after = None;
                                let mut state = runtime.write().await;
                                state.latch_audit_failure_after_device_action(
                                    "automatic control device_write",
                                    &err.to_string(),
                                );
                            } else {
                                last_written_command = Some(LastWrittenCommand {
                                    fingerprint,
                                    written_at: Utc::now(),
                                });
                                retry_after = None;
                            }
                        } else if let Some((fault_reason, event_type)) = outcome.fault {
                            // Non-confirmed ACK: rejected / timeout / unverified-but-
                            // required. Fail closed — latch a control fault, disable
                            // auto, and audit the specific outcome.
                            tracing::warn!("command handshake failed: {fault_reason}");
                            last_written_command = None;
                            if outcome.retry {
                                retry_after = Some(
                                    Utc::now()
                                        + chrono::Duration::from_std(Duration::from_millis(
                                            safety.control.write_retry_backoff_ms,
                                        ))
                                        .unwrap_or_else(|_| chrono::Duration::seconds(5)),
                                );
                            } else {
                                retry_after = None;
                            }
                            let should_audit = {
                                let mut state = runtime.write().await;
                                state.latch_control_fault(fault_reason.clone())
                            };
                            audit_device_write_handshake_fault(
                                &db,
                                active_batch_id,
                                should_audit,
                                event_type,
                                &command,
                                &format!("{}|rid={}", fault_reason, ack.request_id),
                            )
                            .await;
                        }
                    }
                    Err(err) => {
                        // write_targets_acknowledged failed before any ACK could
                        // arrive (serial/modbus IO error). Same fail-closed path as
                        // the legacy device_write_failed branch.
                        tracing::warn!("device write failed: {err}");
                        last_written_command = None;
                        retry_after = Some(
                            Utc::now()
                                + chrono::Duration::from_std(Duration::from_millis(
                                    safety.control.write_retry_backoff_ms,
                                ))
                                .unwrap_or_else(|_| chrono::Duration::seconds(5)),
                        );
                        let mut state = runtime.write().await;
                        state.latch_control_fault(err.to_string());
                        drop(state);
                        let reason =
                            format!("device write failed; automatic control disabled: {err}");
                        if let Err(audit_err) = db
                            .insert_control_event_sqlx(
                                active_batch_id,
                                "device_write_failed",
                                Some(&command),
                                &reason,
                                &AuditActor::system(),
                            )
                            .await
                        {
                            tracing::warn!(
                                "failed to persist device_write_failed event: {audit_err}"
                            );
                        }
                    }
                }
            }
            ControlDecision::Blocked(reason) => {
                last_written_command = None;
                retry_after = None;
                tracing::debug!("control blocked: {reason:?}");
            }
        }

        sleep(interval).await;
    }
}

async fn audit_field_input_auto_disable(
    db: &Db,
    active_batch_id: Option<i64>,
    auto_was_disabled: bool,
    reason: &str,
) {
    if !auto_was_disabled {
        return;
    }
    let audit_reason = format!("field input fault disabled automatic control: {reason}");
    if let Err(err) = db
        .insert_control_event_sqlx(
            active_batch_id,
            "field_input_fault_auto_disabled",
            None,
            &audit_reason,
            &AuditActor::system(),
        )
        .await
    {
        tracing::warn!("failed to persist field_input_fault_auto_disabled event: {err}");
    }
}

async fn audit_downstream_control_fault(
    db: &Db,
    active_batch_id: Option<i64>,
    should_audit: bool,
    reason: &str,
) {
    if !should_audit {
        return;
    }
    let audit_reason =
        format!("downstream command fault latched; automatic control disabled: {reason}");
    if let Err(err) = db
        .insert_control_event_sqlx(
            active_batch_id,
            "downstream_command_fault",
            None,
            &audit_reason,
            &AuditActor::system(),
        )
        .await
    {
        tracing::warn!("failed to persist downstream_command_fault event: {err}");
    }
}

/// Stable token for the CommandAck status embedded in audit reasons.
fn ack_status_token(status: &AckStatus) -> &'static str {
    match status {
        AckStatus::Confirmed => "confirmed",
        AckStatus::Rejected(_) => "rejected",
        AckStatus::Timeout => "timeout",
        AckStatus::Unverified => "unverified",
    }
}

/// Decision derived from a CommandAck status. Extracted from the control loop
/// so the handshake branching is unit-testable without running the full loop.
struct AckOutcome {
    /// Treat the write as delivered: Confirmed, or legacy Unverified when the
    /// operator has NOT required a handshake.
    confirmed: bool,
    /// Schedule a retry-after backoff. Only Timeout retries — a hard reject or
    /// a missing handshake implementation stays latched until manual reset.
    retry: bool,
    /// None when confirmed; otherwise (fault reason, audit event_type).
    fault: Option<(String, &'static str)>,
}

fn classify_ack_outcome(
    status: &AckStatus,
    require_command_ack: bool,
    ack_timeout_ms: u64,
) -> AckOutcome {
    let legacy_unverified = matches!(status, &AckStatus::Unverified) && !require_command_ack;
    if matches!(status, &AckStatus::Confirmed) || legacy_unverified {
        return AckOutcome {
            confirmed: true,
            retry: false,
            fault: None,
        };
    }
    let (reason, event_type): (String, &'static str) = match status {
        AckStatus::Rejected(detail) => (
            format!("downstream rejected command: {detail}"),
            "device_write_rejected",
        ),
        AckStatus::Timeout => (
            format!(
                "command ack timeout after {} ms; delivery unconfirmed",
                ack_timeout_ms
            ),
            "device_write_unconfirmed",
        ),
        AckStatus::Unverified => (
            "require_command_ack is enabled but the device mode has not implemented a handshake ACK"
                .to_string(),
            "device_write_unconfirmed",
        ),
        AckStatus::Confirmed => unreachable!("confirmed is handled in the branch above"),
    };
    AckOutcome {
        confirmed: false,
        retry: matches!(status, &AckStatus::Timeout),
        fault: Some((reason, event_type)),
    }
}

/// Persist a command-handshake fault event (rejected / unconfirmed). Mirrors
/// the other audit_* helpers: only writes when the latch actually advanced so
/// repeated identical faults do not spam the audit log.
async fn audit_device_write_handshake_fault(
    db: &Db,
    active_batch_id: Option<i64>,
    should_audit: bool,
    event_type: &str,
    command: &reactor_edge_daemon::control::SafeCommand,
    reason: &str,
) {
    if !should_audit {
        return;
    }
    if let Err(err) = db
        .insert_control_event_sqlx(
            active_batch_id,
            event_type,
            Some(command),
            reason,
            &AuditActor::system(),
        )
        .await
    {
        tracing::warn!("failed to persist {event_type} event: {err}");
    }
}

async fn audit_control_fault_auto_disable(db: &Db, active_batch_id: Option<i64>) {
    if let Err(err) = db
        .insert_control_event_sqlx(
            active_batch_id,
            "control_fault_auto_disabled",
            None,
            "control fault was already latched; automatic control forced disabled",
            &AuditActor::system(),
        )
        .await
    {
        tracing::warn!("failed to persist control_fault_auto_disabled event: {err}");
    }
}

async fn audit_automatic_batch_recovery_block(
    db: &Db,
    active_batch_id: Option<i64>,
    should_audit: bool,
    reason: &str,
) {
    if !should_audit {
        return;
    }
    let audit_reason = format!("automatic control blocked by unfinished batch recovery: {reason}");
    if let Err(err) = db
        .insert_control_event_sqlx(
            active_batch_id,
            "unfinished_batch_recovery_auto_blocked",
            None,
            &audit_reason,
            &AuditActor::system(),
        )
        .await
    {
        tracing::warn!("failed to persist unfinished_batch_recovery_auto_blocked event: {err}");
    }
}

async fn audit_automatic_final_interlock_block(
    db: &Db,
    active_batch_id: Option<i64>,
    should_audit: bool,
    reason: &str,
) {
    if !should_audit {
        return;
    }
    let audit_reason = format!("automatic control final interlock failed: {reason}");
    if let Err(err) = db
        .insert_control_event_sqlx(
            active_batch_id,
            "automatic_final_interlock_blocked",
            None,
            &audit_reason,
            &AuditActor::system(),
        )
        .await
    {
        tracing::warn!("failed to persist automatic_final_interlock_blocked event: {err}");
    }
}

fn automatic_final_interlock_fault_reason(decision: &ControlDecision) -> Option<String> {
    match decision {
        ControlDecision::Blocked(
            ControlBlockReason::MissingSensorSample
            | ControlBlockReason::SensorStale
            | ControlBlockReason::MissingDeviceStatus
            | ControlBlockReason::DeviceStatusFault
            | ControlBlockReason::DownstreamCommandFault
            | ControlBlockReason::ForbiddenControlZone,
        ) => Some(format!(
            "automatic control final interlock blocked by {decision:?}"
        )),
        ControlDecision::Write(command) => Some(format!(
            "automatic control final interlock produced a different command fingerprint: {command:?}"
        )),
        ControlDecision::Blocked(
            ControlBlockReason::AutoDisabled
            | ControlBlockReason::ManualLock
            | ControlBlockReason::EmergencyStop
            | ControlBlockReason::ControlFault,
        ) => None,
    }
}

fn ensure_automatic_control_write_still_current(
    safety: &reactor_edge_daemon::config::SafetyConfig,
    state: &RuntimeState,
    safety_guard: Option<&std::path::Path>,
    command: &reactor_edge_daemon::control::SafeCommand,
) -> Result<Option<i64>, ControlDecision> {
    let decision = decide_control_with_optional_guard(safety, state, safety_guard);
    match decision {
        ControlDecision::Write(current) if current == *command => Ok(state.active_batch_id),
        other => Err(other),
    }
}

fn automatic_batch_state_is_consistent(
    active_batch_id: Option<i64>,
    unfinished_batches: &[reactor_edge_daemon::db::Batch],
) -> Result<(), String> {
    let unfinished_ids = unfinished_batches
        .iter()
        .map(|batch| batch.id)
        .collect::<Vec<_>>();
    let unexpected_ids = unfinished_batches
        .iter()
        .filter(|batch| active_batch_id != Some(batch.id))
        .map(|batch| batch.id)
        .collect::<Vec<_>>();
    let active_missing = active_batch_id
        .is_some_and(|active_id| !unfinished_batches.iter().any(|batch| batch.id == active_id));
    if unexpected_ids.is_empty() && !active_missing {
        return Ok(());
    }
    Err(format!(
        "database has unfinished batch records {:?} while runtime active batch is {:?}",
        unfinished_ids, active_batch_id
    ))
}

fn decide_control_with_optional_guard(
    safety: &reactor_edge_daemon::config::SafetyConfig,
    state: &RuntimeState,
    safety_guard: Option<&std::path::Path>,
) -> ControlDecision {
    let request = reactor_edge_daemon::control::SafetyGuardRequest::DecideControl {
        safety: safety.clone(),
        sample: state.latest_sample.clone(),
        targets: state.targets.clone(),
        auto_enabled: state.auto_enabled,
        manual_lock: state.manual_lock,
        emergency_stop: state.emergency_stop,
        control_fault: state.last_control_error.clone(),
        device_status: state.device_status.clone(),
    };
    if let Some(guard) = safety_guard {
        let timeout = Duration::from_millis(safety.control.safety_guard_timeout_ms);
        match evaluate_with_process(guard, &request, timeout) {
            Ok(reactor_edge_daemon::control::SafetyGuardResponse::ControlDecision(decision)) => {
                return decision;
            }
            Ok(other) => {
                tracing::warn!("safety guard returned unexpected response: {other:?}");
            }
            Err(err) => {
                tracing::warn!(
                    "safety guard process failed; falling back to in-process safety: {err}"
                );
            }
        }
    }
    match reactor_edge_daemon::control::evaluate_safety_request(request) {
        reactor_edge_daemon::control::SafetyGuardResponse::ControlDecision(decision) => decision,
        // A DecideControl request must yield a ControlDecision. If the shared
        // evaluator ever returns ClampedTargets here (e.g. after a future
        // refactor of evaluate_safety_request), fail safe instead of panicking
        // inside the control-loop task — a panic there would only be caught by
        // the termination monitor above, not by the caller of this function.
        reactor_edge_daemon::control::SafetyGuardResponse::ClampedTargets(_) => {
            tracing::error!(
                "in-process safety evaluation returned clamped targets for a decide-control request; failing safe"
            );
            ControlDecision::Blocked(ControlBlockReason::ControlFault)
        }
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

#[derive(Debug, Clone)]
struct LastWrittenCommand {
    fingerprint: SafeCommandFingerprint,
    written_at: DateTime<Utc>,
}

impl LastWrittenCommand {
    fn matches_recent(
        &self,
        fingerprint: &SafeCommandFingerprint,
        safety: &reactor_edge_daemon::config::SafetyConfig,
    ) -> bool {
        self.fingerprint == *fingerprint
            && Utc::now().signed_duration_since(self.written_at)
                <= chrono::Duration::milliseconds(safety.control.sensor_timeout_ms)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use reactor_edge_daemon::{
        config::{
            ControlConfig, ForbiddenControlZone, OptimizerBounds, SafetyConfig, StirrerSafety,
            TemperatureSafety,
        },
        db::Batch,
        device::{PipelineDevice, ReactorDevice},
        state::{ControlTargets, SensorSnapshot},
    };

    fn test_safety() -> SafetyConfig {
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

    fn automatic_runtime(safety: &SafetyConfig) -> RuntimeState {
        let mut runtime = RuntimeState::from_safety(safety);
        runtime.auto_enabled = true;
        runtime.active_batch_id = Some(42);
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
        runtime.targets = ControlTargets {
            temperature_c: 120.0,
            heat_time_s: 300.0,
            hold_time_s: 600.0,
            cool_time_s: 180.0,
            stirrer_rpm: 900.0,
            shake_speed_cpm: 35.0,
            target_pressure_mpa: 0.5,
        };
        runtime
    }

    fn current_command(
        safety: &SafetyConfig,
        runtime: &RuntimeState,
    ) -> reactor_edge_daemon::control::SafeCommand {
        match decide_control_with_optional_guard(safety, runtime, None) {
            ControlDecision::Write(command) => command,
            decision => panic!("expected write decision, got {decision:?}"),
        }
    }

    fn unfinished_batch(id: i64) -> Batch {
        Batch {
            id,
            process_id: None,
            name: format!("unfinished-{id}"),
            started_at: Utc::now(),
            finished_at: None,
            target_temperature_c: 60.0,
            target_stirrer_rpm: 300.0,
            heating_minutes: 10.0,
            stirring_minutes: 10.0,
        }
    }

    #[test]
    fn automatic_control_final_interlock_requires_same_current_write_decision() {
        let safety = test_safety();
        let runtime = automatic_runtime(&safety);
        let command = current_command(&safety, &runtime);

        assert_eq!(
            ensure_automatic_control_write_still_current(&safety, &runtime, None, &command),
            Ok(Some(42))
        );
    }

    #[test]
    fn automatic_control_final_interlock_requires_persisted_batch_match() {
        assert!(automatic_batch_state_is_consistent(Some(42), &[unfinished_batch(42)]).is_ok());
        assert!(automatic_batch_state_is_consistent(None, &[]).is_ok());

        let orphan = automatic_batch_state_is_consistent(None, &[unfinished_batch(42)])
            .expect_err("orphan DB batch must block automatic writes");
        assert!(orphan.contains("unfinished batch records [42]"));

        let missing = automatic_batch_state_is_consistent(Some(42), &[])
            .expect_err("runtime active batch missing from DB must block automatic writes");
        assert!(missing.contains("runtime active batch is Some(42)"));

        let extra = automatic_batch_state_is_consistent(
            Some(42),
            &[unfinished_batch(43), unfinished_batch(42)],
        )
        .expect_err("extra unfinished DB batch must block automatic writes");
        assert!(extra.contains("unfinished batch records [43, 42]"));
    }

    #[test]
    fn classify_ack_outcome_confirmed_has_no_fault() {
        let outcome = classify_ack_outcome(&AckStatus::Confirmed, true, 2000);
        assert!(outcome.confirmed);
        assert!(!outcome.retry);
        assert!(outcome.fault.is_none());
    }

    #[test]
    fn classify_ack_outcome_rejected_latches_without_retry() {
        let outcome = classify_ack_outcome(
            &AckStatus::Rejected("target out of range".to_string()),
            true,
            2000,
        );
        assert!(!outcome.confirmed);
        assert!(!outcome.retry, "a hard reject must not auto-retry");
        let (reason, event_type) = outcome.fault.expect("rejected must produce a fault");
        assert!(reason.contains("downstream rejected command"));
        assert!(reason.contains("target out of range"));
        assert_eq!(event_type, "device_write_rejected");
    }

    #[test]
    fn classify_ack_outcome_timeout_latches_with_retry() {
        let outcome = classify_ack_outcome(&AckStatus::Timeout, true, 2000);
        assert!(!outcome.confirmed);
        assert!(outcome.retry, "timeout must schedule a retry backoff");
        let (reason, event_type) = outcome.fault.expect("timeout must produce a fault");
        assert!(reason.contains("command ack timeout after 2000 ms"));
        assert_eq!(event_type, "device_write_unconfirmed");
    }

    #[test]
    fn classify_ack_outcome_unverified_under_require_fails_closed() {
        // require_command_ack=true + a device with no real handshake (Unverified)
        // is a configuration error: fail closed, do NOT silently treat as success.
        let outcome = classify_ack_outcome(&AckStatus::Unverified, true, 2000);
        assert!(
            !outcome.confirmed,
            "must not confirm when a handshake is required but missing"
        );
        assert!(!outcome.retry);
        let (reason, event_type) = outcome
            .fault
            .expect("missing required handshake must fault");
        assert!(reason.contains("require_command_ack is enabled"));
        assert_eq!(event_type, "device_write_unconfirmed");
    }

    #[test]
    fn classify_ack_outcome_unverified_legacy_is_treated_as_confirmed() {
        // require_command_ack=false preserves the legacy fire-and-forget path: an
        // Unverified device whose write Ok returned is accepted, not latched.
        let outcome = classify_ack_outcome(&AckStatus::Unverified, false, 2000);
        assert!(
            outcome.confirmed,
            "legacy unverified must be accepted when ack is not required"
        );
        assert!(outcome.fault.is_none());
    }

    #[test]
    fn ack_status_token_maps_each_status_for_audit_reason() {
        assert_eq!(ack_status_token(&AckStatus::Confirmed), "confirmed");
        assert_eq!(
            ack_status_token(&AckStatus::Rejected(String::new())),
            "rejected"
        );
        assert_eq!(ack_status_token(&AckStatus::Timeout), "timeout");
        assert_eq!(ack_status_token(&AckStatus::Unverified), "unverified");
    }

    #[tokio::test]
    async fn pipeline_device_handshake_reports_confirmed_no_op() {
        // Pipeline mode never emits commands; the handshake is a no-op Confirmed
        // so require_command_ack does not spuriously latch pipeline deployments.
        let dev = PipelineDevice;
        let command = reactor_edge_daemon::control::SafeCommand {
            target_temperature_c: 50.0,
            heat_time_s: 300.0,
            hold_time_s: 600.0,
            cool_time_s: 180.0,
            target_stirrer_rpm: 300.0,
            target_shake_speed_cpm: 30.0,
            target_pressure_mpa: 0.5,
            reason: "test".to_string(),
        };
        let ack = dev
            .write_targets_acknowledged(&command, "req-pipeline", Duration::from_millis(100))
            .await
            .unwrap();
        assert_eq!(ack.request_id, "req-pipeline");
        assert!(matches!(ack.status, AckStatus::Confirmed));
        assert!(ack.accepted_targets.is_none());
    }

    #[test]
    fn automatic_control_command_dedup_expires_after_field_freshness_window() {
        let safety = test_safety();
        let runtime = automatic_runtime(&safety);
        let command = current_command(&safety, &runtime);
        let fingerprint = SafeCommandFingerprint::from(&command);
        let recent = LastWrittenCommand {
            fingerprint: fingerprint.clone(),
            written_at: Utc::now(),
        };
        assert!(recent.matches_recent(&fingerprint, &safety));

        let expired = LastWrittenCommand {
            fingerprint: fingerprint.clone(),
            written_at: Utc::now()
                - chrono::Duration::milliseconds(safety.control.sensor_timeout_ms + 1),
        };
        assert!(!expired.matches_recent(&fingerprint, &safety));
    }

    #[test]
    fn automatic_control_final_interlock_blocks_state_changes_after_decision() {
        let safety = test_safety();
        let runtime = automatic_runtime(&safety);
        let command = current_command(&safety, &runtime);

        let mut emergency = runtime.clone();
        emergency.emergency_stop = true;
        assert_eq!(
            ensure_automatic_control_write_still_current(&safety, &emergency, None, &command),
            Err(ControlDecision::Blocked(ControlBlockReason::EmergencyStop))
        );

        let mut stale_sample = runtime.clone();
        stale_sample.latest_sample.as_mut().unwrap().captured_at =
            Utc::now() - chrono::Duration::seconds(30);
        assert_eq!(
            ensure_automatic_control_write_still_current(&safety, &stale_sample, None, &command),
            Err(ControlDecision::Blocked(ControlBlockReason::SensorStale))
        );

        let mut changed_targets = runtime.clone();
        changed_targets.targets.temperature_c = 40.0;
        let rejection =
            ensure_automatic_control_write_still_current(&safety, &changed_targets, None, &command)
                .unwrap_err();
        assert_ne!(rejection, ControlDecision::Write(command));
    }

    #[test]
    fn automatic_final_interlock_fault_reason_latches_unproven_or_changed_field_state() {
        assert!(
            automatic_final_interlock_fault_reason(&ControlDecision::Blocked(
                ControlBlockReason::MissingSensorSample
            ))
            .unwrap()
            .contains("MissingSensorSample")
        );
        assert!(
            automatic_final_interlock_fault_reason(&ControlDecision::Blocked(
                ControlBlockReason::SensorStale
            ))
            .is_some()
        );
        assert!(
            automatic_final_interlock_fault_reason(&ControlDecision::Blocked(
                ControlBlockReason::MissingDeviceStatus
            ))
            .is_some()
        );
        assert!(
            automatic_final_interlock_fault_reason(&ControlDecision::Blocked(
                ControlBlockReason::DeviceStatusFault
            ))
            .is_some()
        );
        assert!(
            automatic_final_interlock_fault_reason(&ControlDecision::Blocked(
                ControlBlockReason::DownstreamCommandFault
            ))
            .is_some()
        );
        assert!(
            automatic_final_interlock_fault_reason(&ControlDecision::Blocked(
                ControlBlockReason::ForbiddenControlZone
            ))
            .is_some()
        );

        let safety = test_safety();
        let runtime = automatic_runtime(&safety);
        let changed_command = current_command(&safety, &runtime);
        assert!(
            automatic_final_interlock_fault_reason(&ControlDecision::Write(changed_command))
                .unwrap()
                .contains("different command fingerprint")
        );
    }

    #[test]
    fn automatic_final_interlock_fault_reason_does_not_relabel_operator_blocks() {
        for reason in [
            ControlBlockReason::AutoDisabled,
            ControlBlockReason::ManualLock,
            ControlBlockReason::EmergencyStop,
            ControlBlockReason::ControlFault,
        ] {
            assert_eq!(
                automatic_final_interlock_fault_reason(&ControlDecision::Blocked(reason)),
                None
            );
        }
    }

    #[test]
    fn automatic_control_final_interlock_requires_device_status_when_configured() {
        let mut safety = test_safety();
        safety.control.require_device_status_for_control = true;
        let runtime = automatic_runtime(&safety);
        let fallback_command = reactor_edge_daemon::control::SafeCommand {
            target_temperature_c: runtime.targets.temperature_c,
            heat_time_s: runtime.targets.heat_time_s,
            hold_time_s: runtime.targets.hold_time_s,
            cool_time_s: runtime.targets.cool_time_s,
            target_stirrer_rpm: runtime.targets.stirrer_rpm,
            target_shake_speed_cpm: runtime.targets.shake_speed_cpm,
            target_pressure_mpa: runtime.targets.target_pressure_mpa,
            reason: "stale command should not pass without status proof".to_string(),
        };

        assert_eq!(
            ensure_automatic_control_write_still_current(
                &safety,
                &runtime,
                None,
                &fallback_command
            ),
            Err(ControlDecision::Blocked(
                ControlBlockReason::MissingDeviceStatus
            ))
        );
    }
}
