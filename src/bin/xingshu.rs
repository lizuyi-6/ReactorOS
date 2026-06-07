use std::{
    env, fs,
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Stdio},
    time::{Duration, Instant},
};

use anyhow::{anyhow, Context, Result};
use base64::{
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD},
    Engine as _,
};
use clap::{Args, Parser, Subcommand};
use rand::rngs::StdRng;
use rand::{rngs::OsRng, Rng, RngCore, SeedableRng};
use reactor_edge_daemon::{
    config::{load_device_config, load_safety_config},
    control::{evaluate_safety_request, SafetyGuardRequest, SafetyGuardResponse},
    db::{parse_encryption_key, Db, DbEncryption, DB_ENCRYPTION_KEY_ENV, ENCRYPTED_JSON_PREFIX},
    local_ai::{run_training_from_env, LocalAiTrainingRequest},
    mqtt::load_integration_config,
    number::{round1, round2, round3},
    safety_guard::evaluate_with_process,
    state::ControlTargets,
};
use reqwest::{Client, Method, StatusCode};
use rusqlite::{params, Connection};
use serde_json::{json, Value};

const DEFAULT_API: &str = "http://127.0.0.1:8000";
const DEFAULT_DEVICE_ID: &str = "reactor_001";

#[derive(Debug, Parser)]
#[command(
    name = "xingshu",
    version,
    about = "Xingshu intelligent reactor upper-computer CLI"
)]
struct Cli {
    #[arg(long, global = true, default_value = DEFAULT_API)]
    api: String,
    #[arg(long, global = true)]
    token: Option<String>,
    #[arg(long, global = true, default_value = "data/reactor.sqlite3")]
    db: PathBuf,
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Start the reactor-edge daemon in the foreground.
    Start(DaemonStartArgs),
    /// Safely stop the active reactor process and disable automatic control.
    Stop(SafeStopArgs),
    /// Print service, device, runtime, and model status.
    Status,
    /// Show runtime configuration from the API or local TOML files.
    Config(ConfigArgs),
    /// Login and print a bearer token for protected operations.
    Auth(AuthArgs),
    /// Manage experiment and history data.
    Data(DataArgs),
    /// Send control commands through the safety-gated API.
    Control(ControlArgs),
    /// Request AI suggestions and inspect model state.
    Ai(AiArgs),
    /// Query or export the tamper-evident audit log.
    Audit(AuditArgs),
    /// Inspect and test Modbus register mappings.
    Modbus(ModbusArgs),
    /// Run local safety-guard checks through the isolated guard process.
    Safety(SafetyArgs),
    /// Run local upper-computer performance smoke checks.
    Perf(PerfArgs),
    /// Backup, restore, and securely wipe the local reactor data store.
    Ops(OpsArgs),
    /// Rotate sensitive keys used by the local data store and integration adapters.
    Key(KeyArgs),
}

#[derive(Debug, Args)]
struct DaemonStartArgs {
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
    bind: String,
    #[arg(long)]
    enable_test_reset: bool,
    #[arg(long)]
    seed_demo_context: bool,
}

#[derive(Debug, Args)]
struct SafeStopArgs {
    #[arg(long)]
    reason: Option<String>,
}

#[derive(Debug, Args)]
struct ConfigArgs {
    #[arg(long)]
    local: bool,
    #[arg(long, default_value = "config/device.toml")]
    config: PathBuf,
    #[arg(long, default_value = "config/safety.toml")]
    safety: PathBuf,
    #[arg(long, default_value = "config/integration.toml")]
    integration: PathBuf,
}

#[derive(Debug, Args)]
struct AuthArgs {
    #[command(subcommand)]
    command: AuthCommand,
}

#[derive(Debug, Subcommand)]
enum AuthCommand {
    /// Exchange a local username/password for a bearer token.
    Login {
        #[arg(long, default_value = "operator")]
        username: String,
        #[arg(long)]
        password: String,
    },
    /// Show the current bearer session user.
    Me,
}

#[derive(Debug, Args)]
struct DataArgs {
    #[command(subcommand)]
    command: DataCommand,
}

#[derive(Debug, Subcommand)]
enum DataCommand {
    /// List recent experiment batches.
    List {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Export batch history as CSV.
    Export {
        #[arg(long, default_value = "reactor-batches.csv")]
        out: PathBuf,
    },
    /// Export batch history as an Excel workbook.
    ExportXlsx {
        #[arg(long, default_value = "reactor-batches.xlsx")]
        out: PathBuf,
    },
    /// Export one batch as a Markdown experiment report.
    Report {
        #[arg(long)]
        batch_id: i64,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Push demo pipeline samples through the external v1 sample ingest API.
    Sample {
        #[arg(long, default_value_t = 1)]
        count: usize,
        #[arg(long)]
        duration_s: Option<u64>,
        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,
        #[arg(long, default_value = DEFAULT_DEVICE_ID)]
        device_id: String,
    },
    /// Delete local runtime data from the SQLite database.
    Delete {
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Args)]
struct ControlArgs {
    #[command(subcommand)]
    command: ControlCommand,
}

#[derive(Debug, Subcommand)]
enum ControlCommand {
    /// Set target temperature and stirrer speed.
    Set {
        #[arg(long)]
        temp: f64,
        #[arg(long)]
        rpm: f64,
        #[arg(long)]
        shake: Option<f64>,
    },
    /// Start a stored process or create a basic batch.
    Start {
        #[arg(long)]
        process_id: Option<i64>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        temp: Option<f64>,
        #[arg(long)]
        rpm: Option<f64>,
        #[arg(long)]
        heat_minutes: Option<f64>,
        #[arg(long)]
        stir_minutes: Option<f64>,
    },
    /// Stop the current process.
    Stop(SafeStopArgs),
    /// Trigger or reset emergency stop.
    Estop {
        #[arg(long)]
        reset: bool,
    },
}

#[derive(Debug, Args)]
struct AiArgs {
    #[command(subcommand)]
    command: AiCommand,
}

#[derive(Debug, Subcommand)]
enum AiCommand {
    /// Generate or fetch the latest parameter suggestion.
    Suggest,
    /// Draft a safety-gated experiment plan and SOP from current recommendation evidence.
    Plan,
    /// Show active AI provider and memory profile.
    Model,
    /// Export local experiment data and optionally invoke the LoRA training entrypoint.
    Train {
        /// Write the generated JSONL dataset here. Defaults to output/local-ai/lora-training-dataset.jsonl.
        #[arg(long)]
        dataset: Option<PathBuf>,
        /// Directory passed through to the configured LoRA training entrypoint.
        #[arg(long)]
        output_dir: Option<PathBuf>,
        /// Write the training manifest here. Defaults beside the dataset as *.manifest.json.
        #[arg(long)]
        manifest: Option<PathBuf>,
        /// Number of completed product-result batches to include.
        #[arg(long, default_value_t = 50)]
        max_batches: usize,
        /// Maximum sensor samples per batch included in each JSONL row.
        #[arg(long, default_value_t = 128)]
        sample_limit: usize,
        /// Maximum control/audit events per batch included in each JSONL row.
        #[arg(long, default_value_t = 64)]
        event_limit: usize,
        /// Only export the training dataset; do not require or invoke model assets.
        #[arg(long)]
        export_only: bool,
        /// Pass --dry-run through to the configured training entrypoint.
        #[arg(long)]
        dry_run: bool,
        /// Promote a passing candidate adapter into XINGSHU_LOCAL_AI_LORA.
        #[arg(long)]
        promote: bool,
        /// Minimum evaluation score required for --promote.
        #[arg(long, default_value_t = 0.0)]
        min_eval_score: f64,
        /// Training command timeout in seconds.
        #[arg(long, default_value_t = 1800)]
        timeout_s: u64,
    },
}

#[derive(Debug, Args)]
struct AuditArgs {
    #[command(subcommand)]
    command: AuditCommand,
}

#[derive(Debug, Subcommand)]
enum AuditCommand {
    /// List audit events.
    List {
        #[arg(long, default_value_t = 20)]
        page_size: usize,
        #[arg(long)]
        event_type: Option<String>,
    },
    /// Export audit events as CSV.
    Export {
        #[arg(long, default_value = "reactor-audit-log.csv")]
        out: PathBuf,
        #[arg(long)]
        event_type: Option<String>,
    },
}

#[derive(Debug, Args)]
struct ModbusArgs {
    #[command(subcommand)]
    command: ModbusCommand,
}

#[derive(Debug, Subcommand)]
enum ModbusCommand {
    /// Show configured register mapping.
    Map,
    /// Read one mapped register.
    Read { register: String },
    /// Write one mapped register through the safety gate.
    Write {
        register: String,
        value: f64,
        #[arg(long)]
        reason: Option<String>,
    },
}

#[derive(Debug, Args)]
struct SafetyArgs {
    #[command(subcommand)]
    command: SafetyCommand,
}

#[derive(Debug, Subcommand)]
enum SafetyCommand {
    /// Clamp target values through the isolated safety guard process.
    Check {
        #[arg(long, default_value = "config/safety.toml")]
        safety: PathBuf,
        #[arg(long)]
        temp: f64,
        #[arg(long)]
        rpm: f64,
        #[arg(long, default_value_t = 30.0)]
        shake: f64,
        #[arg(long, default_value_t = 0.5)]
        pressure: f64,
        #[arg(long, default_value = "reactor-safety-guard")]
        guard: PathBuf,
    },
}

#[derive(Debug, Args)]
struct PerfArgs {
    #[command(subcommand)]
    command: PerfCommand,
}

#[derive(Debug, Subcommand)]
enum PerfCommand {
    /// Measure read-only API latency and isolated safety-guard latency.
    Smoke {
        #[arg(long, default_value_t = 20)]
        iterations: usize,
        #[arg(long, default_value_t = 100)]
        api_threshold_ms: u64,
        #[arg(long, default_value_t = 100)]
        safety_threshold_ms: u64,
        #[arg(long, default_value = "config/safety.toml")]
        safety: PathBuf,
        #[arg(long, default_value = "reactor-safety-guard")]
        guard: PathBuf,
    },
}

struct CommandOutput {
    human: String,
    json: Value,
}

#[derive(Debug, Args)]
struct OpsArgs {
    #[command(subcommand)]
    command: OpsCommand,
}

#[derive(Debug, Subcommand)]
enum OpsCommand {
    /// Run production-readiness checks for secrets, TLS paths, and backup timer files.
    Preflight {
        #[arg(long, default_value = "config/device.toml")]
        config: PathBuf,
        #[arg(long, default_value = "config/safety.toml")]
        safety: PathBuf,
        #[arg(long, default_value = "config/integration.toml")]
        integration: PathBuf,
        #[arg(long, default_value = "deploy/reactor-edge-backup.service")]
        backup_service: PathBuf,
        #[arg(long, default_value = "deploy/reactor-edge-backup.timer")]
        backup_timer: PathBuf,
        #[arg(long, default_value = "deploy/reactor-edge-backup.sh")]
        backup_script: PathBuf,
        /// Treat fail-level findings as errors. Keep disabled for local audits.
        #[arg(long)]
        production: bool,
    },
    /// Take a SQLite online snapshot using VACUUM INTO. Ciphertext is stored
    /// inside the same database file, so the snapshot is sufficient for
    /// restoring encrypted rows when the matching key is available.
    Backup {
        #[arg(long, default_value = "data/reactor.sqlite3")]
        db: PathBuf,
        #[arg(long, default_value = "backups/reactor.sqlite3.snapshot")]
        out: PathBuf,
        #[arg(long, default_value_t = false, hide = true)]
        include_ciphertext: bool,
    },
    /// Restore a previously captured SQLite snapshot into the target database path.
    Restore {
        #[arg(long, default_value = "backups/reactor.sqlite3.snapshot")]
        backup: PathBuf,
        #[arg(long, default_value = "data/reactor.sqlite3")]
        db: PathBuf,
        #[arg(long)]
        yes: bool,
    },
    /// Securely overwrite and remove the SQLite database, WAL/SHM/JOURNAL
    /// sidecars, <db>.key, and matching snapshots under the sibling backups/
    /// directory. Run this only when the daemon is stopped.
    Wipe {
        #[arg(long, default_value = "data/reactor.sqlite3")]
        db: PathBuf,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Args)]
struct KeyArgs {
    #[command(subcommand)]
    command: KeyCommand,
}

#[derive(Debug, Subcommand)]
enum KeyCommand {
    /// Generate a new AES-256 key into <db>.key (mode 0600) and print only the
    /// environment variable name. Run `rekey-integration-tasks` before
    /// switching the daemon to the new key when existing encrypted rows must
    /// remain readable.
    Generate {
        #[arg(long, default_value = "data/reactor.sqlite3")]
        db: PathBuf,
        #[arg(long)]
        yes: bool,
    },
    /// Re-encrypt integration_tasks.request_json/response_json with a new
    /// AES-256 key. Run this offline while the daemon is stopped.
    RekeyIntegrationTasks {
        #[arg(long, default_value = "data/reactor.sqlite3")]
        db: PathBuf,
        #[arg(long)]
        old_key: Option<String>,
        #[arg(long)]
        old_key_file: Option<PathBuf>,
        #[arg(long)]
        new_key: Option<String>,
        #[arg(long)]
        new_key_file: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        yes: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = Client::new();
    let env_token = env::var("XINGSHU_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let token = cli.token.as_deref().or(env_token.as_deref());
    let output = match &cli.command {
        Commands::Start(args) => start_daemon(args),
        Commands::Stop(args) => safe_stop(&client, &cli.api, token, args).await,
        Commands::Status => status(&client, &cli.api).await,
        Commands::Config(args) => config(&client, &cli.api, args).await,
        Commands::Auth(args) => auth(&client, &cli.api, token, args).await,
        Commands::Data(args) => data(&client, &cli.api, token, &cli.db, args).await,
        Commands::Control(args) => control(&client, &cli.api, token, args).await,
        Commands::Ai(args) => ai(&client, &cli.api, token, &cli.db, args).await,
        Commands::Audit(args) => audit(&client, &cli.api, token, args).await,
        Commands::Modbus(args) => modbus(&client, &cli.api, token, args).await,
        Commands::Safety(args) => safety_guard_check(args),
        Commands::Perf(args) => perf(&client, &cli.api, args).await,
        Commands::Ops(args) => ops(args),
        Commands::Key(args) => key(args),
    }?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&output.json)?);
    } else if !output.human.is_empty() {
        println!("{}", output.human);
    }
    Ok(())
}

fn start_daemon(args: &DaemonStartArgs) -> Result<CommandOutput> {
    let daemon = daemon_executable()?;
    let mut command = ProcessCommand::new(&daemon);
    command
        .arg("--config")
        .arg(&args.config)
        .arg("--safety")
        .arg(&args.safety)
        .arg("--memory")
        .arg(&args.memory)
        .arg("--integration")
        .arg(&args.integration)
        .arg("--db")
        .arg(&args.db)
        .arg("--assets")
        .arg(&args.assets)
        .arg("--bind")
        .arg(&args.bind);
    if args.enable_test_reset {
        command.arg("--enable-test-reset");
    }
    if args.seed_demo_context {
        command.arg("--seed-demo-context");
    }
    let status = command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("failed to start {}", daemon.display()))?;
    if !status.success() {
        return Err(anyhow!("reactor-edge daemon exited with {status}"));
    }
    Ok(CommandOutput {
        human: "reactor-edge daemon stopped".to_string(),
        json: json!({ "status": "stopped" }),
    })
}

fn daemon_executable() -> Result<PathBuf> {
    sibling_executable("reactor-edge-daemon")
}

fn sibling_executable(name: &str) -> Result<PathBuf> {
    let current = env::current_exe().context("failed to locate current executable")?;
    let dir = current
        .parent()
        .ok_or_else(|| anyhow!("current executable has no parent directory"))?;
    let exe_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let sibling = dir.join(exe_name);
    if sibling.exists() {
        return Ok(sibling);
    }
    Ok(PathBuf::from(name))
}

async fn status(client: &Client, api: &str) -> Result<CommandOutput> {
    let health = request_json(client, Method::GET, api, "/health", None, None, &[]).await?;
    let devices = request_json(
        client,
        Method::GET,
        api,
        "/api/devices/status",
        None,
        None,
        &[],
    )
    .await?;
    let live = request_json(
        client,
        Method::GET,
        api,
        "/api/live",
        None,
        None,
        &[("sample_limit", "1")],
    )
    .await
    .ok();

    let devices_data = unwrap_data(&devices);
    let online = devices_data
        .get("online_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let total = devices_data
        .get("total_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let runtime = live.as_ref().and_then(|value| value.get("runtime"));
    let emergency = runtime
        .and_then(|value| value.get("emergency_stop"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let auto = runtime
        .and_then(|value| value.get("auto_enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let model = live
        .as_ref()
        .and_then(|value| value.get("ai_provider"))
        .and_then(|value| value.get("model"))
        .and_then(Value::as_str)
        .unwrap_or("--");

    Ok(CommandOutput {
        human: format!(
            "service: {}\ndevices: {online}/{total} online\nauto: {}\nemergency_stop: {}\nai_model: {model}",
            health
                .get("service")
                .and_then(Value::as_str)
                .unwrap_or("reactor-edge-daemon"),
            if auto { "enabled" } else { "standby" },
            if emergency { "active" } else { "clear" },
        ),
        json: json!({
            "health": health,
            "devices": devices,
            "live": live,
        }),
    })
}

async fn auth(
    client: &Client,
    api: &str,
    token: Option<&str>,
    args: &AuthArgs,
) -> Result<CommandOutput> {
    match &args.command {
        AuthCommand::Login { username, password } => {
            let value = request_json(
                client,
                Method::POST,
                api,
                "/api/auth/login",
                None,
                Some(json!({ "username": username, "password": password })),
                &[],
            )
            .await?;
            let data = unwrap_data(&value);
            let token = data
                .get("token")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("login response did not include token"))?;
            let role = data
                .get("user")
                .and_then(|user| user.get("role"))
                .and_then(Value::as_str)
                .unwrap_or("--");
            Ok(CommandOutput {
                human: format!(
                    "logged in as {username} ({role})\nset XINGSHU_TOKEN={token} for protected commands"
                ),
                json: value,
            })
        }
        AuthCommand::Me => {
            let value =
                request_json(client, Method::GET, api, "/api/auth/me", token, None, &[]).await?;
            Ok(CommandOutput {
                human: serde_json::to_string_pretty(unwrap_data(&value))?,
                json: value,
            })
        }
    }
}

async fn config(client: &Client, api: &str, args: &ConfigArgs) -> Result<CommandOutput> {
    let value = if args.local {
        let device = load_device_config(&args.config)?;
        let safety = load_safety_config(&args.safety)?;
        let integration = load_integration_config(&args.integration)?;
        json!({ "device": device, "safety": safety, "integration": integration })
    } else {
        request_json(
            client,
            Method::GET,
            api,
            "/api/config/summary",
            None,
            None,
            &[],
        )
        .await?
    };
    Ok(CommandOutput {
        human: serde_json::to_string_pretty(&value)?,
        json: value,
    })
}

async fn data(
    client: &Client,
    api: &str,
    token: Option<&str>,
    db_path: &Path,
    args: &DataArgs,
) -> Result<CommandOutput> {
    match &args.command {
        DataCommand::List { limit } => {
            let value =
                request_json(client, Method::GET, api, "/api/batches", None, None, &[]).await?;
            let data = unwrap_data(&value);
            let batches = data
                .get("batches")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let human = batches
                .iter()
                .rev()
                .take(*limit)
                .map(batch_line)
                .collect::<Vec<_>>()
                .join("\n");
            Ok(CommandOutput {
                human: if human.is_empty() {
                    "no batches".to_string()
                } else {
                    human
                },
                json: value,
            })
        }
        DataCommand::Export { out } => {
            let csv = request_text(client, api, "/api/batches/export.csv", token, &[]).await?;
            fs::write(out, csv).with_context(|| format!("failed to write {}", out.display()))?;
            Ok(CommandOutput {
                human: format!("wrote {}", out.display()),
                json: json!({ "output": out, "source": "/api/batches/export.csv" }),
            })
        }
        DataCommand::ExportXlsx { out } => {
            let bytes = request_bytes(client, api, "/api/batches/export.xlsx", token, &[]).await?;
            fs::write(out, bytes).with_context(|| format!("failed to write {}", out.display()))?;
            Ok(CommandOutput {
                human: format!("wrote {}", out.display()),
                json: json!({ "output": out, "source": "/api/batches/export.xlsx" }),
            })
        }
        DataCommand::Report { batch_id, out } => {
            let path = format!("/api/batches/{batch_id}/report.md");
            let report = request_text(client, api, &path, token, &[]).await?;
            let out = out
                .clone()
                .unwrap_or_else(|| PathBuf::from(format!("reactor-batch-{batch_id}-report.md")));
            fs::write(&out, report)
                .with_context(|| format!("failed to write {}", out.display()))?;
            Ok(CommandOutput {
                human: format!("wrote {}", out.display()),
                json: json!({ "output": out, "source": path }),
            })
        }
        DataCommand::Sample {
            count,
            duration_s,
            interval_ms,
            device_id,
        } => {
            let interval_ms = (*interval_ms).max(100);
            let count = duration_s
                .map(|seconds| {
                    let samples = (seconds.saturating_mul(1000) / interval_ms).max(1);
                    samples as usize
                })
                .unwrap_or(*count)
                .clamp(1, 10_000);
            let mut last = Value::Null;
            for index in 0..count {
                let path = format!("/api/v1/reactor/{device_id}/samples");
                let body = demo_pipeline_sample(index);
                last =
                    request_json(client, Method::POST, api, &path, None, Some(body), &[]).await?;
                if index + 1 < count {
                    tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
                }
            }
            Ok(CommandOutput {
                human: format!(
                    "pushed {count} pipeline sample{} to {device_id}",
                    if count == 1 { "" } else { "s" }
                ),
                json: json!({
                    "device_id": device_id,
                    "samples_pushed": count,
                    "interval_ms": interval_ms,
                    "duration_s": duration_s,
                    "last_response": last
                }),
            })
        }
        DataCommand::Delete { yes } => {
            if !yes {
                return Err(anyhow!(
                    "data delete is destructive; rerun with --yes to clear runtime data"
                ));
            }
            let db = Db::open(db_path)?;
            db.clear_runtime_data_for_tests()?;
            Ok(CommandOutput {
                human: format!("cleared runtime data from {}", db_path.display()),
                json: json!({ "deleted": true, "db": db_path }),
            })
        }
    }
}

fn demo_pipeline_sample(index: usize) -> Value {
    let phase = index as f64;
    json!({
        "temperature_c": round1(31.0 + (phase * 0.18).sin() * 1.8 + phase * 0.03),
        "pressure_mpa": round3(0.50 + (phase * 0.17).cos() * 0.03),
        "stirrer_rpm": round1(125.0 + (phase * 0.11).sin() * 8.0),
        "shake_speed_cpm": round1(30.0 + (phase * 0.21).cos() * 2.0),
        "tilt_state": if index % 2 == 0 { 1 } else { 0 },
        "flow_rate_l_min": round3(2.42 + (phase * 0.13).sin() * 0.08),
        "product_concentration_percent": round1(11.0 + phase * 0.04),
        "ph": round2(6.15 + (phase * 0.09).cos() * 0.05)
    })
}

async fn control(
    client: &Client,
    api: &str,
    token: Option<&str>,
    args: &ControlArgs,
) -> Result<CommandOutput> {
    match &args.command {
        ControlCommand::Set { temp, rpm, shake } => {
            let mut body = json!({
                "temperature_c": temp,
                "stirrer_rpm": rpm,
            });
            if let Some(shake) = shake {
                body["shake_speed_cpm"] = json!(shake);
            }
            let value = request_json(
                client,
                Method::POST,
                api,
                "/api/control/targets",
                token,
                Some(body),
                &[],
            )
            .await?;
            Ok(CommandOutput {
                human: "targets updated through safety gate".to_string(),
                json: value,
            })
        }
        ControlCommand::Start {
            process_id,
            name,
            temp,
            rpm,
            heat_minutes,
            stir_minutes,
        } => {
            let (path, body) = if let Some(process_id) = process_id {
                (format!("/api/processes/{process_id}/start"), None)
            } else {
                let mut body = json!({});
                if let Some(name) = name {
                    body["name"] = json!(name);
                }
                if let Some(temp) = temp {
                    body["target_temperature_c"] = json!(temp);
                }
                if let Some(rpm) = rpm {
                    body["target_stirrer_rpm"] = json!(rpm);
                }
                if let Some(minutes) = heat_minutes {
                    body["heating_minutes"] = json!(minutes);
                }
                if let Some(minutes) = stir_minutes {
                    body["stirring_minutes"] = json!(minutes);
                }
                ("/api/batches/start".to_string(), Some(body))
            };
            let value = request_json(client, Method::POST, api, &path, token, body, &[]).await?;
            Ok(CommandOutput {
                human: "process or batch started".to_string(),
                json: value,
            })
        }
        ControlCommand::Stop(args) => safe_stop(client, api, token, args).await,
        ControlCommand::Estop { reset } => {
            let path = if *reset {
                "/api/control/emergency-stop/reset"
            } else {
                "/api/control/emergency-stop"
            };
            let value = request_json(client, Method::POST, api, path, token, None, &[]).await?;
            Ok(CommandOutput {
                human: if *reset {
                    "emergency stop reset"
                } else {
                    "emergency stop triggered"
                }
                .to_string(),
                json: value,
            })
        }
    }
}

async fn safe_stop(
    client: &Client,
    api: &str,
    token: Option<&str>,
    _args: &SafeStopArgs,
) -> Result<CommandOutput> {
    match request_json(
        client,
        Method::POST,
        api,
        "/api/processes/current/stop",
        token,
        None,
        &[],
    )
    .await
    {
        Ok(value) => Ok(CommandOutput {
            human: "active process stopped".to_string(),
            json: value,
        }),
        Err(err) => {
            let value = request_json(
                client,
                Method::POST,
                api,
                "/api/control/auto",
                token,
                Some(json!({ "enabled": false })),
                &[],
            )
            .await
            .with_context(|| {
                format!("process stop failed and auto-disable fallback failed: {err}")
            })?;
            Ok(CommandOutput {
                human: "no active process stop completed; automatic control disabled".to_string(),
                json: json!({ "fallback": "auto_disabled", "response": value }),
            })
        }
    }
}

async fn ai(
    client: &Client,
    api: &str,
    _token: Option<&str>,
    db_path: &Path,
    args: &AiArgs,
) -> Result<CommandOutput> {
    match &args.command {
        AiCommand::Suggest => {
            let value = request_json(
                client,
                Method::POST,
                api,
                "/api/recommendations/latest",
                None,
                None,
                &[],
            )
            .await?;
            let rec = unwrap_data(&value)
                .get("recommendation")
                .cloned()
                .unwrap_or_else(|| value.clone());
            Ok(CommandOutput {
                human: recommendation_line(&rec),
                json: value,
            })
        }
        AiCommand::Plan => {
            let value = request_json(
                client,
                Method::GET,
                api,
                "/api/ai/experiment-plan",
                None,
                None,
                &[],
            )
            .await?;
            let data = unwrap_data(&value);
            let title = data
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("AI plan");
            let status = data.get("status").and_then(Value::as_str).unwrap_or("--");
            let summary = data
                .get("sop_summary")
                .and_then(Value::as_str)
                .unwrap_or("no SOP summary");
            let steps = data
                .get("steps")
                .and_then(Value::as_array)
                .map(|steps| {
                    steps
                        .iter()
                        .map(|step| {
                            format!(
                                "{}. {}: {} degC / {} RPM / {} min",
                                step.get("step_no").and_then(Value::as_u64).unwrap_or(0),
                                step.get("name").and_then(Value::as_str).unwrap_or("step"),
                                fmt(step.get("target_temperature_c")),
                                fmt(step.get("target_stirrer_rpm")),
                                fmt(step.get("duration_minutes")),
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            Ok(CommandOutput {
                human: format!("{title}\nstatus: {status}\n{summary}\n{steps}"),
                json: value,
            })
        }
        AiCommand::Model => {
            let value = request_json(
                client,
                Method::GET,
                api,
                "/api/config/summary",
                None,
                None,
                &[],
            )
            .await?;
            let data = unwrap_data(&value);
            let provider = data.get("ai_provider").cloned().unwrap_or(Value::Null);
            let local_ai = data.get("local_ai").cloned().unwrap_or(Value::Null);
            let memory = data.get("ai_memory").cloned().unwrap_or(Value::Null);
            Ok(CommandOutput {
                human: format!(
                    "provider: {}\nlocal_ai: {}\nmemory: {}",
                    serde_json::to_string_pretty(&provider)?,
                    serde_json::to_string_pretty(&local_ai)?,
                    serde_json::to_string_pretty(&memory)?,
                ),
                json: json!({ "provider": provider, "local_ai": local_ai, "memory": memory }),
            })
        }
        AiCommand::Train {
            dataset,
            output_dir,
            manifest,
            max_batches,
            sample_limit,
            event_limit,
            export_only,
            dry_run,
            promote,
            min_eval_score,
            timeout_s,
        } => ai_train(
            db_path,
            dataset.as_deref(),
            output_dir.as_deref(),
            manifest.as_deref(),
            *max_batches,
            *sample_limit,
            *event_limit,
            *export_only,
            *dry_run,
            *promote,
            *min_eval_score,
            *timeout_s,
        ),
    }
}

fn ai_train(
    db_path: &Path,
    dataset: Option<&Path>,
    output_dir: Option<&Path>,
    manifest: Option<&Path>,
    max_batches: usize,
    sample_limit: usize,
    event_limit: usize,
    export_only: bool,
    dry_run: bool,
    promote: bool,
    min_eval_score: f64,
    timeout_s: u64,
) -> Result<CommandOutput> {
    let dataset_path = dataset
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("output/local-ai/lora-training-dataset.jsonl"));
    let db = Db::open(db_path).with_context(|| {
        format!(
            "failed to open local SQLite database for LoRA dataset export: {}",
            db_path.display()
        )
    })?;
    let export = export_lora_training_dataset(
        &db,
        &dataset_path,
        max_batches.max(1),
        sample_limit.max(1),
        event_limit.max(1),
    )?;
    if export_only {
        return Ok(CommandOutput {
            human: format!(
                "local LoRA dataset exported\n  dataset: {}\n  rows:    {}\n  mode:    export-only",
                export.dataset.display(),
                export.rows
            ),
            json: json!({
                "action": "local-ai-train",
                "mode": "export_only",
                "dataset": export.dataset.display().to_string(),
                "rows": export.rows
            }),
        });
    }

    let report = run_training_from_env(LocalAiTrainingRequest {
        dataset: Some(export.dataset.clone()),
        output_dir: output_dir.map(PathBuf::from),
        dry_run,
        timeout: Duration::from_secs(timeout_s.max(1)),
    })
    .with_context(|| {
        format!(
            "local LoRA dataset was exported to {}, but training could not start",
            export.dataset.display()
        )
    })?;
    let manifest_path = manifest
        .map(PathBuf::from)
        .unwrap_or_else(|| default_training_manifest_path(&export.dataset));
    let manifest_doc =
        write_training_manifest(&manifest_path, &export, &report, promote, min_eval_score)?;
    if promote
        && !manifest_doc["promotion"]["promoted"]
            .as_bool()
            .unwrap_or(false)
    {
        return Err(anyhow!(
            "local LoRA promotion requested but not performed: {} (manifest: {})",
            manifest_doc["promotion"]["reason"]
                .as_str()
                .unwrap_or("see manifest"),
            manifest_path.display()
        ));
    }
    Ok(CommandOutput {
        human: format!(
            "local LoRA training command completed\n  dataset:  {}\n  rows:     {}\n  manifest: {}\n  program:  {}\n  exit:     {}",
            export.dataset.display(),
            export.rows,
            manifest_path.display(),
            report.program,
            report.exit_code.unwrap_or_default()
        ),
        json: json!({
            "action": "local-ai-train",
            "mode": "train",
            "dataset": export.dataset.display().to_string(),
            "rows": export.rows,
            "manifest": manifest_path.display().to_string(),
            "evaluation": manifest_doc["evaluation"].clone(),
            "promotion": manifest_doc["promotion"].clone(),
            "training": report
        }),
    })
}

struct LoraDatasetExport {
    dataset: PathBuf,
    rows: usize,
}

fn default_training_manifest_path(dataset: &Path) -> PathBuf {
    let mut path = dataset.to_path_buf();
    let extension = dataset
        .extension()
        .and_then(|ext| ext.to_str())
        .filter(|ext| !ext.is_empty())
        .map(|ext| format!("{ext}.manifest.json"))
        .unwrap_or_else(|| "manifest.json".to_string());
    path.set_extension(extension);
    path
}

fn write_training_manifest(
    manifest_path: &Path,
    export: &LoraDatasetExport,
    report: &reactor_edge_daemon::local_ai::LocalAiCommandReport,
    promote: bool,
    min_eval_score: f64,
) -> Result<Value> {
    if let Some(parent) = manifest_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }
    let parsed = report.parsed_stdout.as_ref();
    let evaluation_score = parsed.and_then(extract_evaluation_score);
    let candidate_adapter = parsed
        .and_then(extract_candidate_adapter)
        .map(PathBuf::from);
    let promotion = maybe_promote_lora_adapter(
        promote,
        min_eval_score,
        evaluation_score,
        candidate_adapter.as_deref(),
    )?;
    let manifest = json!({
        "schema": "xingshu.local_ai.training_manifest.v1",
        "created_at": chrono::Utc::now(),
        "dataset": export.dataset.display().to_string(),
        "rows": export.rows,
        "training": {
            "program": report.program.clone(),
            "args": report.args.clone(),
            "exit_code": report.exit_code,
            "stdout_bytes": report.stdout.len(),
            "stderr_bytes": report.stderr.len(),
            "parsed_stdout": report.parsed_stdout.clone(),
        },
        "evaluation": {
            "score": evaluation_score,
            "min_score_for_promotion": min_eval_score,
            "metrics": parsed.and_then(extract_metrics).cloned(),
            "candidate_adapter": candidate_adapter.as_ref().map(|path| path.display().to_string()),
        },
        "promotion": promotion,
        "audit": {
            "decision": if promote { "promotion_requested" } else { "manifest_only" },
            "note": "Promotion only copies a candidate adapter into XINGSHU_LOCAL_AI_LORA after explicit --promote and a passing evaluation score."
        }
    });
    fs::write(manifest_path, serde_json::to_string_pretty(&manifest)?)
        .with_context(|| format!("failed to write {}", manifest_path.display()))?;
    Ok(manifest)
}

fn extract_evaluation_score(value: &Value) -> Option<f64> {
    for path in [
        &["evaluation", "score"][..],
        &["eval", "score"][..],
        &["metrics", "eval_score"][..],
        &["metrics", "score"][..],
        &["score"][..],
    ] {
        if let Some(score) = json_path(value, path).and_then(Value::as_f64) {
            return Some(score);
        }
    }
    None
}

fn extract_candidate_adapter(value: &Value) -> Option<String> {
    for path in [
        &["candidate_adapter"][..],
        &["adapter"][..],
        &["adapter_path"][..],
        &["artifacts", "adapter"][..],
        &["artifacts", "adapter_path"][..],
        &["artifacts", "lora_adapter"][..],
        &["output", "adapter"][..],
    ] {
        if let Some(adapter) = json_path(value, path)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(adapter.to_string());
        }
    }
    None
}

fn extract_metrics(value: &Value) -> Option<&Value> {
    json_path(value, &["metrics"])
        .or_else(|| json_path(value, &["evaluation", "metrics"]))
        .or_else(|| json_path(value, &["eval", "metrics"]))
}

fn json_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

fn maybe_promote_lora_adapter(
    promote: bool,
    min_eval_score: f64,
    evaluation_score: Option<f64>,
    candidate_adapter: Option<&Path>,
) -> Result<Value> {
    if !promote {
        return Ok(json!({
            "requested": false,
            "promoted": false,
            "reason": "promotion not requested"
        }));
    }
    let Some(score) = evaluation_score else {
        return Ok(json!({
            "requested": true,
            "promoted": false,
            "reason": "training output did not include an evaluation score"
        }));
    };
    if score < min_eval_score {
        return Ok(json!({
            "requested": true,
            "promoted": false,
            "reason": format!("evaluation score {score:.4} is below required {min_eval_score:.4}"),
            "score": score,
            "min_score": min_eval_score
        }));
    }
    let Some(candidate_adapter) = candidate_adapter else {
        return Ok(json!({
            "requested": true,
            "promoted": false,
            "reason": "training output did not include a candidate adapter path",
            "score": score,
            "min_score": min_eval_score
        }));
    };
    if !candidate_adapter.is_file() {
        return Ok(json!({
            "requested": true,
            "promoted": false,
            "reason": format!("candidate adapter is not a readable file: {}", candidate_adapter.display()),
            "score": score,
            "min_score": min_eval_score
        }));
    }
    let target = env::var("XINGSHU_LOCAL_AI_LORA")
        .ok()
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty());
    let Some(target) = target else {
        return Ok(json!({
            "requested": true,
            "promoted": false,
            "reason": "XINGSHU_LOCAL_AI_LORA is not configured as a promotion target",
            "score": score,
            "min_score": min_eval_score
        }));
    };
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }
    let backup = if target.is_file() {
        let backup = promoted_backup_path(&target);
        fs::copy(&target, &backup).with_context(|| {
            format!(
                "failed to preserve existing LoRA adapter {} as {}",
                target.display(),
                backup.display()
            )
        })?;
        Some(backup)
    } else {
        None
    };
    fs::copy(candidate_adapter, &target).with_context(|| {
        format!(
            "failed to promote candidate adapter {} to {}",
            candidate_adapter.display(),
            target.display()
        )
    })?;
    Ok(json!({
        "requested": true,
        "promoted": true,
        "reason": "candidate adapter promoted",
        "score": score,
        "min_score": min_eval_score,
        "source": candidate_adapter.display().to_string(),
        "target": target.display().to_string(),
        "backup": backup.map(|path| path.display().to_string())
    }))
}

fn promoted_backup_path(target: &Path) -> PathBuf {
    let stamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("adapter.gguf");
    target.with_file_name(format!("{file_name}.pre-promote-{stamp}.bak"))
}

fn export_lora_training_dataset(
    db: &Db,
    dataset: &Path,
    max_batches: usize,
    sample_limit: usize,
    event_limit: usize,
) -> Result<LoraDatasetExport> {
    let mut outcomes = db.batch_outcomes()?;
    if outcomes.len() > max_batches {
        outcomes = outcomes[outcomes.len() - max_batches..].to_vec();
    }
    if outcomes.is_empty() {
        return Err(anyhow!(
            "no completed product-result batches are available for local LoRA training export"
        ));
    }
    if let Some(parent) = dataset.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }

    let mut rows = Vec::new();
    for outcome in outcomes {
        let batch = db.batch_by_id(outcome.batch_id)?.ok_or_else(|| {
            anyhow!(
                "batch {} disappeared during dataset export",
                outcome.batch_id
            )
        })?;
        let samples = db.sample_records_for_batch(outcome.batch_id, sample_limit)?;
        let events = db.control_events_for_batch(outcome.batch_id, event_limit)?;
        let sample_summary = summarize_samples(&samples);
        let event_summary = events
            .iter()
            .map(|event| {
                json!({
                    "id": event.id,
                    "event_type": event.event_type,
                    "target_temperature_c": event.target_temperature_c,
                    "target_stirrer_rpm": event.target_stirrer_rpm,
                    "target_shake_speed_cpm": event.target_shake_speed_cpm,
                    "reason": event.reason,
                    "created_at": event.created_at
                })
            })
            .collect::<Vec<_>>();
        let prompt = format!(
            "Given reactor batch history, recommend safe next parameters. Batch name: {}. Product yield: {:.2}%. Product ratio: {:.4}. Recent sample count: {}.",
            batch.name,
            outcome.yield_percent,
            outcome.product_ratio,
            samples.len()
        );
        let completion = json!({
            "target_temperature_c": outcome.target_temperature_c,
            "target_stirrer_rpm": outcome.target_stirrer_rpm,
            "heating_minutes": outcome.heating_minutes,
            "stirring_minutes": outcome.stirring_minutes,
            "expected_yield_percent": outcome.yield_percent,
            "expected_product_ratio": outcome.product_ratio,
            "rationale": "Supervised target reconstructed from a completed batch with recorded product result."
        });
        rows.push(json!({
            "schema": "xingshu.local_ai.lora_dataset.v1",
            "messages": [
                {
                    "role": "system",
                    "content": "You are the Xingshu intelligent reactor local Qwen LoRA assistant. Return safe, auditable reactor parameter suggestions as JSON."
                },
                {
                    "role": "user",
                    "content": prompt
                },
                {
                    "role": "assistant",
                    "content": completion.to_string()
                }
            ],
            "input": {
                "batch": {
                    "id": batch.id,
                    "name": batch.name,
                    "started_at": batch.started_at,
                    "finished_at": batch.finished_at,
                    "target_temperature_c": batch.target_temperature_c,
                    "target_stirrer_rpm": batch.target_stirrer_rpm,
                    "heating_minutes": batch.heating_minutes,
                    "stirring_minutes": batch.stirring_minutes
                },
                "samples": samples.iter().map(|sample| json!({
                    "temperature_c": sample.sample.temperature_c,
                    "pressure_mpa": sample.sample.pressure_mpa,
                    "stirrer_rpm": sample.sample.stirrer_rpm,
                    "shake_speed_cpm": sample.sample.shake_speed_cpm,
                    "flow_rate_l_min": sample.sample.flow_rate_l_min,
                    "product_concentration_percent": sample.sample.product_concentration_percent,
                    "ph": sample.sample.ph,
                    "captured_at": sample.sample.captured_at
                })).collect::<Vec<_>>(),
                "sample_summary": sample_summary,
                "control_events": event_summary
            },
            "output": completion,
            "metadata": {
                "batch_id": outcome.batch_id,
                "source": "sqlite",
                "created_by": "xingshu ai train",
                "dataset_role": "supervised_parameter_recommendation"
            }
        }));
    }

    let mut data = String::new();
    for row in &rows {
        data.push_str(&serde_json::to_string(row)?);
        data.push('\n');
    }
    fs::write(dataset, data).with_context(|| format!("failed to write {}", dataset.display()))?;
    Ok(LoraDatasetExport {
        dataset: dataset.to_path_buf(),
        rows: rows.len(),
    })
}

fn summarize_samples(samples: &[reactor_edge_daemon::db::SensorSampleRecord]) -> Value {
    fn avg(values: impl Iterator<Item = f64>) -> Option<f64> {
        let mut count = 0usize;
        let mut sum = 0.0;
        for value in values {
            count += 1;
            sum += value;
        }
        (count > 0).then_some(sum / count as f64)
    }

    json!({
        "count": samples.len(),
        "temperature_c_avg": avg(samples.iter().map(|sample| sample.sample.temperature_c)).map(round2),
        "pressure_mpa_avg": avg(samples.iter().map(|sample| sample.sample.pressure_mpa)).map(round3),
        "stirrer_rpm_avg": avg(samples.iter().map(|sample| sample.sample.stirrer_rpm)).map(round1),
        "flow_rate_l_min_avg": avg(samples.iter().map(|sample| sample.sample.flow_rate_l_min)).map(round2),
        "product_concentration_percent_avg": avg(samples.iter().map(|sample| sample.sample.product_concentration_percent)).map(round2),
        "ph_avg": avg(samples.iter().map(|sample| sample.sample.ph)).map(round2)
    })
}

async fn audit(
    client: &Client,
    api: &str,
    token: Option<&str>,
    args: &AuditArgs,
) -> Result<CommandOutput> {
    match &args.command {
        AuditCommand::List {
            page_size,
            event_type,
        } => {
            let mut query = vec![("page_size", page_size.to_string())];
            if let Some(event_type) = event_type {
                query.push(("event_type", event_type.clone()));
            }
            let query_refs = query
                .iter()
                .map(|(key, value)| (*key, value.as_str()))
                .collect::<Vec<_>>();
            let value = request_json(
                client,
                Method::GET,
                api,
                "/api/audit/logs",
                token,
                None,
                &query_refs,
            )
            .await?;
            let data = unwrap_data(&value);
            let events = data
                .get("events")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            Ok(CommandOutput {
                human: events.iter().map(audit_line).collect::<Vec<_>>().join("\n"),
                json: value,
            })
        }
        AuditCommand::Export { out, event_type } => {
            let mut query = Vec::new();
            if let Some(event_type) = event_type {
                query.push(("event_type", event_type.as_str()));
            }
            let text = request_text(client, api, "/api/audit/export.csv", token, &query).await?;
            fs::write(out, text).with_context(|| format!("failed to write {}", out.display()))?;
            Ok(CommandOutput {
                human: format!("wrote {}", out.display()),
                json: json!({ "output": out, "source": "/api/audit/export.csv" }),
            })
        }
    }
}

async fn modbus(
    client: &Client,
    api: &str,
    token: Option<&str>,
    args: &ModbusArgs,
) -> Result<CommandOutput> {
    match &args.command {
        ModbusCommand::Map => {
            let value = request_json(
                client,
                Method::GET,
                api,
                "/api/modbus/registers",
                None,
                None,
                &[],
            )
            .await?;
            Ok(CommandOutput {
                human: serde_json::to_string_pretty(unwrap_data(&value))?,
                json: value,
            })
        }
        ModbusCommand::Read { register } => {
            let path = format!("/api/modbus/registers/{register}/read");
            let value = request_json(client, Method::GET, api, &path, None, None, &[]).await?;
            Ok(CommandOutput {
                human: serde_json::to_string_pretty(unwrap_data(&value))?,
                json: value,
            })
        }
        ModbusCommand::Write {
            register,
            value,
            reason,
        } => {
            let path = format!("/api/modbus/registers/{register}/write");
            let body = json!({
                "value": value,
                "reason": reason.clone().unwrap_or_else(|| format!("xingshu modbus write {register}")),
            });
            let value =
                request_json(client, Method::POST, api, &path, token, Some(body), &[]).await?;
            Ok(CommandOutput {
                human: "register write accepted through safety gate".to_string(),
                json: value,
            })
        }
    }
}

fn safety_guard_check(args: &SafetyArgs) -> Result<CommandOutput> {
    match &args.command {
        SafetyCommand::Check {
            safety,
            temp,
            rpm,
            shake,
            pressure,
            guard,
        } => {
            let safety_config = load_safety_config(safety)?;
            let safety_guard_timeout_ms = safety_config.control.safety_guard_timeout_ms;
            let guard_path = if guard == &PathBuf::from("reactor-safety-guard") {
                sibling_executable("reactor-safety-guard")?
            } else {
                guard.clone()
            };
            let request = SafetyGuardRequest::ClampTargets {
                safety: safety_config,
                targets: ControlTargets {
                    temperature_c: *temp,
                    heat_time_s: 300.0,
                    hold_time_s: 600.0,
                    cool_time_s: 180.0,
                    stirrer_rpm: *rpm,
                    shake_speed_cpm: *shake,
                    target_pressure_mpa: *pressure,
                },
            };
            let response = evaluate_with_process(
                &guard_path,
                &request,
                std::time::Duration::from_millis(safety_guard_timeout_ms),
            )?;
            let SafetyGuardResponse::ClampedTargets(targets) = response else {
                return Err(anyhow!(
                    "safety guard returned a non-clamp response for clamp request"
                ));
            };
            Ok(CommandOutput {
                human: format!(
                    "safety guard {} clamped targets: temp={:.2} rpm={:.2} shake={:.2} pressure={:.2}",
                    guard_path.display(),
                    targets.temperature_c,
                    targets.stirrer_rpm,
                    targets.shake_speed_cpm,
                    targets.target_pressure_mpa
                ),
                json: json!({
                    "guard": guard_path,
                    "targets": targets,
                    "isolation": "external_process"
                }),
            })
        }
    }
}

async fn perf(client: &Client, api: &str, args: &PerfArgs) -> Result<CommandOutput> {
    match &args.command {
        PerfCommand::Smoke {
            iterations,
            api_threshold_ms,
            safety_threshold_ms,
            safety,
            guard,
        } => {
            let iterations = (*iterations).clamp(1, 500);
            let endpoints = [
                PerfEndpoint {
                    name: "health",
                    path: "/health",
                    query: Vec::new(),
                    success_only: true,
                },
                PerfEndpoint {
                    name: "config_summary",
                    path: "/api/config/summary",
                    query: Vec::new(),
                    success_only: true,
                },
                PerfEndpoint {
                    name: "devices_status",
                    path: "/api/devices/status",
                    query: Vec::new(),
                    success_only: true,
                },
                PerfEndpoint {
                    name: "live_light",
                    path: "/api/live",
                    query: vec![
                        ("sample_limit", "1"),
                        ("include_processes", "false"),
                        ("include_batches", "false"),
                        ("include_events", "false"),
                    ],
                    success_only: false,
                },
            ];
            let mut endpoint_reports = Vec::new();
            for endpoint in endpoints {
                let mut samples = Vec::with_capacity(iterations);
                for _ in 0..iterations {
                    samples.push(
                        measure_get(
                            client,
                            api,
                            endpoint.path,
                            &endpoint.query,
                            endpoint.success_only,
                        )
                        .await?,
                    );
                }
                endpoint_reports.push(endpoint_report(
                    endpoint.name,
                    endpoint.path,
                    endpoint.success_only,
                    samples,
                    *api_threshold_ms,
                ));
            }

            let safety_config = load_safety_config(safety)?;
            let safety_guard_timeout_ms = safety_config.control.safety_guard_timeout_ms;
            let guard_path = if guard == &PathBuf::from("reactor-safety-guard") {
                sibling_executable("reactor-safety-guard")?
            } else {
                guard.clone()
            };
            let guard_request = SafetyGuardRequest::ClampTargets {
                safety: safety_config,
                targets: ControlTargets {
                    temperature_c: 999.0,
                    heat_time_s: 300.0,
                    hold_time_s: 600.0,
                    cool_time_s: 180.0,
                    stirrer_rpm: 9999.0,
                    shake_speed_cpm: 99.0,
                    target_pressure_mpa: 99.0,
                },
            };
            let mut compute_samples = Vec::with_capacity(iterations);
            for _ in 0..iterations {
                let started = Instant::now();
                let response = evaluate_safety_request(guard_request.clone());
                let elapsed_ms = started.elapsed().as_micros().div_ceil(1_000) as u64;
                let SafetyGuardResponse::ClampedTargets(_) = response else {
                    return Err(anyhow!("safety compute returned a non-clamp response"));
                };
                compute_samples.push(PerfSample {
                    elapsed_ms,
                    status: "ok".to_string(),
                    ok: true,
                });
            }
            let compute_report = endpoint_report(
                "safety_compute",
                "reactor_edge_daemon::control::evaluate_safety_request",
                true,
                compute_samples,
                *safety_threshold_ms,
            );

            let mut guard_samples = Vec::with_capacity(iterations);
            for _ in 0..iterations {
                let started = Instant::now();
                let response = evaluate_with_process(
                    &guard_path,
                    &guard_request,
                    std::time::Duration::from_millis(safety_guard_timeout_ms),
                )?;
                let elapsed_ms = started.elapsed().as_millis() as u64;
                let SafetyGuardResponse::ClampedTargets(_) = response else {
                    return Err(anyhow!("safety guard returned a non-clamp response"));
                };
                guard_samples.push(PerfSample {
                    elapsed_ms,
                    status: "ok".to_string(),
                    ok: true,
                });
            }
            let process_report = endpoint_report(
                "safety_guard_process_spawn",
                &guard_path.display().to_string(),
                true,
                guard_samples,
                u64::MAX,
            );

            let api_pass = endpoint_reports
                .iter()
                .all(|report| report["pass"].as_bool().unwrap_or(false));
            let safety_pass = compute_report["pass"].as_bool().unwrap_or(false);
            let overall_pass = api_pass && safety_pass;
            let json = json!({
                "status": if overall_pass { "pass" } else { "fail" },
                "iterations": iterations,
                "thresholds": {
                    "api_ms": api_threshold_ms,
                    "safety_guard_ms": safety_threshold_ms
                },
                "api": endpoint_reports,
                "safety_compute": compute_report,
                "safety_guard_process_spawn": {
                    "diagnostic_only": true,
                    "note": "This includes Windows process spawn, stdin/stdout JSON, and parse overhead; it is not used for the <100ms in-process safety compute verdict.",
                    "report": process_report
                },
                "scope": {
                    "proves": [
                        "local read-only HTTP round-trip latency on this machine",
                        "in-process safety clamp computation latency on this machine",
                        "isolated safety guard process spawn round-trip as a diagnostic only"
                    ],
                    "does_not_prove": [
                        "STM32/RS485 acquisition latency",
                        "real actuator control latency",
                        "RK-side Qwen/LoRA inference or training latency",
                        "7x24 reliability, MTBF, or external broker/tool performance"
                    ]
                }
            });
            let human = format!(
                "performance smoke: {}\napi p95 max={} ms threshold={} ms\nsafety_compute p95={} ms threshold={} ms\nsafety_guard_process_spawn p95={} ms diagnostic only",
                if overall_pass { "pass" } else { "fail" },
                json["api"]
                    .as_array()
                    .and_then(|reports| reports.iter().filter_map(|report| report["p95_ms"].as_u64()).max())
                    .unwrap_or(0),
                api_threshold_ms,
                json["safety_compute"]["p95_ms"].as_u64().unwrap_or(0),
                safety_threshold_ms,
                json["safety_guard_process_spawn"]["report"]["p95_ms"].as_u64().unwrap_or(0),
            );
            Ok(CommandOutput { human, json })
        }
    }
}

struct PerfEndpoint {
    name: &'static str,
    path: &'static str,
    query: Vec<(&'static str, &'static str)>,
    success_only: bool,
}

#[derive(Clone)]
struct PerfSample {
    elapsed_ms: u64,
    status: String,
    ok: bool,
}

async fn measure_get(
    client: &Client,
    api: &str,
    path: &str,
    query: &[(&str, &str)],
    success_only: bool,
) -> Result<PerfSample> {
    let url = endpoint(api, path);
    let started = Instant::now();
    let response = client
        .get(&url)
        .query(query)
        .send()
        .await
        .with_context(|| format!("failed to request {url}"))?;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if success_only && !status.is_success() {
        return Err(api_error(status, &url, &text));
    }
    Ok(PerfSample {
        elapsed_ms,
        status: status.as_u16().to_string(),
        ok: status.is_success(),
    })
}

fn endpoint_report(
    name: &str,
    path: &str,
    success_only: bool,
    samples: Vec<PerfSample>,
    threshold_ms: u64,
) -> Value {
    let mut elapsed = samples
        .iter()
        .map(|sample| sample.elapsed_ms)
        .collect::<Vec<_>>();
    elapsed.sort_unstable();
    let success_count = samples.iter().filter(|sample| sample.ok).count();
    let pass = !elapsed.is_empty() && percentile(&elapsed, 95) <= threshold_ms;
    let statuses = samples
        .iter()
        .fold(serde_json::Map::new(), |mut map, sample| {
            let count = map.get(&sample.status).and_then(Value::as_u64).unwrap_or(0) + 1;
            map.insert(sample.status.clone(), json!(count));
            map
        });
    json!({
        "name": name,
        "path": path,
        "samples": samples.len(),
        "success_count": success_count,
        "success_only": success_only,
        "status_counts": statuses,
        "min_ms": elapsed.first().copied().unwrap_or(0),
        "p50_ms": percentile(&elapsed, 50),
        "p95_ms": percentile(&elapsed, 95),
        "max_ms": elapsed.last().copied().unwrap_or(0),
        "threshold_ms": threshold_ms,
        "pass": pass
    })
}

fn percentile(sorted: &[u64], percentile: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let index = ((sorted.len() - 1) * percentile).div_ceil(100);
    sorted[index.min(sorted.len() - 1)]
}

async fn request_json(
    client: &Client,
    method: Method,
    api: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
    query: &[(&str, &str)],
) -> Result<Value> {
    let url = endpoint(api, path);
    let mut request = client.request(method, &url).query(query);
    if let Some(token) = token.filter(|value| !value.trim().is_empty()) {
        request = request.bearer_auth(token.trim());
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("failed to request {url}"))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(api_error(status, &url, &text));
    }
    if text.trim().is_empty() {
        return Ok(json!({ "status": "ok" }));
    }
    serde_json::from_str(&text).with_context(|| format!("failed to parse JSON response from {url}"))
}

async fn request_text(
    client: &Client,
    api: &str,
    path: &str,
    token: Option<&str>,
    query: &[(&str, &str)],
) -> Result<String> {
    let url = endpoint(api, path);
    let mut request = client.get(&url).query(query);
    if let Some(token) = token.filter(|value| !value.trim().is_empty()) {
        request = request.bearer_auth(token.trim());
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("failed to request {url}"))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(api_error(status, &url, &text));
    }
    Ok(text)
}

async fn request_bytes(
    client: &Client,
    api: &str,
    path: &str,
    token: Option<&str>,
    query: &[(&str, &str)],
) -> Result<Vec<u8>> {
    let url = endpoint(api, path);
    let mut request = client.get(&url).query(query);
    if let Some(token) = token.filter(|value| !value.trim().is_empty()) {
        request = request.bearer_auth(token.trim());
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("failed to request {url}"))?;
    let status = response.status();
    let bytes = response.bytes().await.unwrap_or_default();
    if !status.is_success() {
        let text = String::from_utf8_lossy(&bytes);
        return Err(api_error(status, &url, &text));
    }
    Ok(bytes.to_vec())
}

fn api_error(status: StatusCode, url: &str, text: &str) -> anyhow::Error {
    let message = serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| text.trim().to_string());
    anyhow!("{url} returned HTTP {status}: {message}")
}

fn endpoint(api: &str, path: &str) -> String {
    format!(
        "{}/{}",
        api.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

fn unwrap_data(value: &Value) -> &Value {
    value.get("data").unwrap_or(value)
}

fn batch_line(batch: &Value) -> String {
    format!(
        "#{:<4} {:<24} temp={} rpm={} status={}",
        batch.get("id").and_then(Value::as_i64).unwrap_or_default(),
        batch.get("name").and_then(Value::as_str).unwrap_or("batch"),
        fmt(batch.get("target_temperature_c")),
        fmt(batch.get("target_stirrer_rpm")),
        if batch
            .get("finished_at")
            .is_some_and(|value| !value.is_null())
        {
            "finished"
        } else {
            "running"
        }
    )
}

fn audit_line(event: &Value) -> String {
    format!(
        "#{:<4} {:<24} {}",
        event.get("id").and_then(Value::as_i64).unwrap_or_default(),
        event
            .get("event_type")
            .and_then(Value::as_str)
            .unwrap_or("event"),
        event.get("reason").and_then(Value::as_str).unwrap_or("")
    )
}

fn recommendation_line(value: &Value) -> String {
    format!(
        "temp={} rpm={} heat_min={} stir_min={} score={}\n{}",
        fmt(value.get("target_temperature_c")),
        fmt(value.get("target_stirrer_rpm")),
        fmt(value.get("heating_minutes")),
        fmt(value.get("stirring_minutes")),
        fmt(value.get("expected_score")),
        value.get("rationale").and_then(Value::as_str).unwrap_or("")
    )
}

fn fmt(value: Option<&Value>) -> String {
    match value {
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => "--".to_string(),
        Some(other) => other.to_string(),
    }
}

#[allow(dead_code)]
fn default_device_id() -> &'static str {
    DEFAULT_DEVICE_ID
}

fn ops(args: &OpsArgs) -> Result<CommandOutput> {
    match &args.command {
        OpsCommand::Preflight {
            config,
            safety,
            integration,
            backup_service,
            backup_timer,
            backup_script,
            production,
        } => ops_preflight(
            config,
            safety,
            integration,
            backup_service,
            backup_timer,
            backup_script,
            *production,
        ),
        OpsCommand::Backup {
            db,
            out,
            include_ciphertext,
        } => ops_backup(db, out, *include_ciphertext),
        OpsCommand::Restore { backup, db, yes } => ops_restore(backup, db, *yes),
        OpsCommand::Wipe { db, yes } => ops_wipe(db, *yes),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreflightLevel {
    Pass,
    Warn,
    Fail,
}

impl PreflightLevel {
    fn as_str(self) -> &'static str {
        match self {
            PreflightLevel::Pass => "pass",
            PreflightLevel::Warn => "warn",
            PreflightLevel::Fail => "fail",
        }
    }
}

struct PreflightFinding {
    level: PreflightLevel,
    check: &'static str,
    detail: String,
}

impl PreflightFinding {
    fn pass(check: &'static str, detail: impl Into<String>) -> Self {
        Self {
            level: PreflightLevel::Pass,
            check,
            detail: detail.into(),
        }
    }

    fn warn(check: &'static str, detail: impl Into<String>) -> Self {
        Self {
            level: PreflightLevel::Warn,
            check,
            detail: detail.into(),
        }
    }

    fn fail(check: &'static str, detail: impl Into<String>) -> Self {
        Self {
            level: PreflightLevel::Fail,
            check,
            detail: detail.into(),
        }
    }
}

fn ops_preflight(
    config_path: &Path,
    safety_path: &Path,
    integration_path: &Path,
    backup_service: &Path,
    backup_timer: &Path,
    backup_script: &Path,
    production: bool,
) -> Result<CommandOutput> {
    let mut findings = Vec::new();

    let device_config = match load_device_config(config_path) {
        Ok(config) => {
            findings.push(PreflightFinding::pass(
                "device_config",
                format!("parsed {}", config_path.display()),
            ));
            Some(config)
        }
        Err(err) => {
            findings.push(PreflightFinding::fail(
                "device_config",
                format!("{}: {err}", config_path.display()),
            ));
            None
        }
    };

    match load_safety_config(safety_path) {
        Ok(safety) => {
            if safety.control.safety_guard_timeout_ms == 0 {
                findings.push(PreflightFinding::fail(
                    "safety_config",
                    "safety_guard_timeout_ms must be greater than 0",
                ));
            } else {
                findings.push(PreflightFinding::pass(
                    "safety_config",
                    format!("parsed {}", safety_path.display()),
                ));
            }
        }
        Err(err) => findings.push(PreflightFinding::fail(
            "safety_config",
            format!("{}: {err}", safety_path.display()),
        )),
    }

    let integration = if !integration_path.is_file() {
        findings.push(PreflightFinding::fail(
            "integration_config",
            format!("{} does not exist", integration_path.display()),
        ));
        None
    } else {
        match load_integration_config(integration_path) {
            Ok(config) => {
                findings.push(PreflightFinding::pass(
                    "integration_config",
                    format!("parsed {}", integration_path.display()),
                ));
                Some(config)
            }
            Err(err) => {
                findings.push(PreflightFinding::fail(
                    "integration_config",
                    format!("{}: {err}", integration_path.display()),
                ));
                None
            }
        }
    };

    if let Some(device) = device_config.as_ref() {
        if matches!(
            device.mode,
            reactor_edge_daemon::config::DeviceMode::Pipeline
        ) {
            findings.push(PreflightFinding::warn(
                "device_mode",
                "device mode is pipeline; production hardware should use modbus, esp32_serial, or json_bridge with real input",
            ));
        } else {
            findings.push(PreflightFinding::pass(
                "device_mode",
                format!("device mode is {:?}", device.mode),
            ));
        }
    }

    check_secret_env(
        &mut findings,
        "auth_secret",
        "XINGSHU_AUTH_SECRET",
        "xingshu-local-rbac-session-secret",
        32,
    );
    check_password_env(
        &mut findings,
        "operator_password",
        "XINGSHU_OPERATOR_PASSWORD",
        "operator123",
    );
    check_password_env(
        &mut findings,
        "engineer_password",
        "XINGSHU_ENGINEER_PASSWORD",
        "engineer123",
    );
    check_password_env(
        &mut findings,
        "admin_password",
        "XINGSHU_ADMIN_PASSWORD",
        "admin123",
    );
    check_db_key_env(&mut findings);

    if let Some(integration) = integration.as_ref() {
        check_mqtt_preflight(&mut findings, &integration.mqtt);
        check_modbus_tcp_preflight(&mut findings, &integration.modbus_tcp);
    }

    check_required_file(
        &mut findings,
        "backup_service",
        backup_service,
        "systemd backup service is missing from the release path",
    );
    check_required_file(
        &mut findings,
        "backup_timer",
        backup_timer,
        "systemd backup timer is missing from the release path",
    );
    check_required_file(
        &mut findings,
        "backup_script",
        backup_script,
        "backup helper script is missing from the release path",
    );

    let fail_count = findings
        .iter()
        .filter(|finding| finding.level == PreflightLevel::Fail)
        .count();
    let warn_count = findings
        .iter()
        .filter(|finding| finding.level == PreflightLevel::Warn)
        .count();
    let pass_count = findings
        .iter()
        .filter(|finding| finding.level == PreflightLevel::Pass)
        .count();
    let status = if fail_count > 0 {
        "fail"
    } else if warn_count > 0 {
        "warn"
    } else {
        "ok"
    };

    let mut human = format!(
        "production preflight {status}\n  pass: {pass_count}\n  warn: {warn_count}\n  fail: {fail_count}\n"
    );
    for finding in &findings {
        human.push_str(&format!(
            "  [{}] {}: {}\n",
            finding.level.as_str(),
            finding.check,
            finding.detail
        ));
    }
    if production && fail_count > 0 {
        human.push_str("  result: refusing production preflight with fail-level findings");
    } else if production && warn_count > 0 {
        human.push_str("  result: production preflight passed with warnings to review");
    } else {
        human.push_str("  result: preflight completed");
    }

    let json_findings = findings
        .iter()
        .map(|finding| {
            json!({
                "level": finding.level.as_str(),
                "check": finding.check,
                "detail": finding.detail,
            })
        })
        .collect::<Vec<_>>();
    let output = CommandOutput {
        human,
        json: json!({
            "action": "production-preflight",
            "status": status,
            "production": production,
            "counts": {
                "pass": pass_count,
                "warn": warn_count,
                "fail": fail_count
            },
            "findings": json_findings
        }),
    };
    if production && fail_count > 0 {
        return Err(anyhow!(output.human));
    }
    Ok(output)
}

fn check_secret_env(
    findings: &mut Vec<PreflightFinding>,
    check: &'static str,
    name: &'static str,
    default_value: &'static str,
    min_chars: usize,
) {
    match env::var(name) {
        Ok(value) if value.trim().is_empty() => findings.push(PreflightFinding::fail(
            check,
            format!("{name} is set but empty"),
        )),
        Ok(value) if value == default_value => findings.push(PreflightFinding::fail(
            check,
            format!("{name} still uses the documented local default"),
        )),
        Ok(value) if value.chars().count() < min_chars => findings.push(PreflightFinding::fail(
            check,
            format!("{name} is shorter than {min_chars} characters"),
        )),
        Ok(_) => findings.push(PreflightFinding::pass(
            check,
            format!("{name} is configured"),
        )),
        Err(env::VarError::NotPresent) => findings.push(PreflightFinding::fail(
            check,
            format!("{name} is not set; daemon would use the local default"),
        )),
        Err(env::VarError::NotUnicode(_)) => findings.push(PreflightFinding::fail(
            check,
            format!("{name} is not valid unicode"),
        )),
    }
}

fn check_password_env(
    findings: &mut Vec<PreflightFinding>,
    check: &'static str,
    name: &'static str,
    default_value: &'static str,
) {
    match env::var(name) {
        Ok(value) if value.trim().is_empty() => findings.push(PreflightFinding::fail(
            check,
            format!("{name} is set but empty"),
        )),
        Ok(value) if value == default_value => findings.push(PreflightFinding::fail(
            check,
            format!("{name} still uses the documented local default"),
        )),
        Ok(value) if value.chars().count() < 12 => findings.push(PreflightFinding::warn(
            check,
            format!("{name} is configured but shorter than 12 characters"),
        )),
        Ok(_) => findings.push(PreflightFinding::pass(
            check,
            format!("{name} is configured"),
        )),
        Err(env::VarError::NotPresent) => findings.push(PreflightFinding::fail(
            check,
            format!("{name} is not set; daemon would use the local default"),
        )),
        Err(env::VarError::NotUnicode(_)) => findings.push(PreflightFinding::fail(
            check,
            format!("{name} is not valid unicode"),
        )),
    }
}

fn check_db_key_env(findings: &mut Vec<PreflightFinding>) {
    match env::var("XINGSHU_DB_ENCRYPTION_KEY") {
        Ok(value) if valid_db_key_value(&value) => findings.push(PreflightFinding::pass(
            "db_encryption_key",
            "XINGSHU_DB_ENCRYPTION_KEY is configured with a valid length/encoding",
        )),
        Ok(_) => findings.push(PreflightFinding::fail(
            "db_encryption_key",
            "XINGSHU_DB_ENCRYPTION_KEY must be 32 raw bytes, 64 hex chars, or base64-encoded 32 bytes",
        )),
        Err(env::VarError::NotPresent) => findings.push(PreflightFinding::fail(
            "db_encryption_key",
            "XINGSHU_DB_ENCRYPTION_KEY is not set; integration task payloads would be stored without AES-GCM encryption",
        )),
        Err(env::VarError::NotUnicode(_)) => findings.push(PreflightFinding::fail(
            "db_encryption_key",
            "XINGSHU_DB_ENCRYPTION_KEY is not valid unicode",
        )),
    }
}

fn valid_db_key_value(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.len() == 32
        || (trimmed.len() == 64
            && trimmed
                .as_bytes()
                .iter()
                .all(|byte| byte.is_ascii_hexdigit()))
        || STANDARD
            .decode(trimmed)
            .or_else(|_| STANDARD_NO_PAD.decode(trimmed))
            .is_ok_and(|bytes| bytes.len() == 32)
}

fn check_mqtt_preflight(
    findings: &mut Vec<PreflightFinding>,
    config: &reactor_edge_daemon::mqtt::MqttBridgeConfig,
) {
    if !config.enabled {
        findings.push(PreflightFinding::warn(
            "mqtt_enabled",
            "MQTT bridge is disabled; skip only if the site does not require broker integration yet",
        ));
        return;
    }
    if config.use_tls {
        check_optional_cert_file(
            findings,
            "mqtt_ca_cert",
            &config.ca_cert,
            "MQTT TLS is enabled but ca_cert is missing",
        );
        match (&config.client_cert, &config.client_key) {
            (Some(cert), Some(key)) => {
                check_required_cert_file(
                    findings,
                    "mqtt_client_cert",
                    cert,
                    "MQTT client certificate file is missing",
                );
                check_required_key_file(
                    findings,
                    "mqtt_client_key",
                    key,
                    "MQTT client key file is missing",
                );
            }
            (None, None) => findings.push(PreflightFinding::warn(
                "mqtt_client_auth",
                "MQTT TLS has no client certificate/key; broker must explicitly allow username/password or anonymous client auth",
            )),
            _ => findings.push(PreflightFinding::fail(
                "mqtt_client_auth",
                "MQTT client_cert and client_key must be configured together",
            )),
        }
    } else {
        findings.push(PreflightFinding::fail(
            "mqtt_tls",
            "MQTT bridge is enabled without TLS",
        ));
    }
    if config.username.is_none() && config.client_cert.is_none() {
        findings.push(PreflightFinding::warn(
            "mqtt_auth",
            "MQTT bridge has neither username nor client certificate configured",
        ));
    }
}

fn check_modbus_tcp_preflight(
    findings: &mut Vec<PreflightFinding>,
    config: &reactor_edge_daemon::modbus_tcp::ModbusTcpConfig,
) {
    if !config.enabled {
        findings.push(PreflightFinding::warn(
            "modbus_tcp_enabled",
            "Modbus TCP server is disabled; skip only if external Modbus TCP is out of scope for this deployment",
        ));
        return;
    }
    if config.require_tls {
        match (&config.tls_cert, &config.tls_key) {
            (Some(cert), Some(key)) => {
                check_required_cert_file(
                    findings,
                    "modbus_tcp_tls_cert",
                    cert,
                    "Modbus TCP TLS certificate file is missing",
                );
                check_required_key_file(
                    findings,
                    "modbus_tcp_tls_key",
                    key,
                    "Modbus TCP TLS key file is missing",
                );
            }
            _ => findings.push(PreflightFinding::fail(
                "modbus_tcp_tls",
                "Modbus TCP require_tls=true but tls_cert/tls_key are not both configured",
            )),
        }
    } else {
        findings.push(PreflightFinding::fail(
            "modbus_tcp_tls",
            "Modbus TCP server is enabled without require_tls=true",
        ));
    }
}

fn check_optional_cert_file(
    findings: &mut Vec<PreflightFinding>,
    check: &'static str,
    path: &Option<PathBuf>,
    missing: &'static str,
) {
    match path {
        Some(path) => check_required_cert_file(findings, check, path, missing),
        None => findings.push(PreflightFinding::fail(check, missing)),
    }
}

fn check_required_cert_file(
    findings: &mut Vec<PreflightFinding>,
    check: &'static str,
    path: &Path,
    missing: &'static str,
) {
    if !path.is_file() {
        findings.push(PreflightFinding::fail(
            check,
            format!("{missing}: {}", path.display()),
        ));
        return;
    }
    match reactor_edge_daemon::tls::load_cert_chain(path) {
        Ok(certs) => findings.push(PreflightFinding::pass(
            check,
            format!(
                "parsed {} certificate(s) from {}",
                certs.len(),
                path.display()
            ),
        )),
        Err(err) => findings.push(PreflightFinding::fail(
            check,
            format!("failed to parse certificate {}: {err}", path.display()),
        )),
    }
}

fn check_required_key_file(
    findings: &mut Vec<PreflightFinding>,
    check: &'static str,
    path: &Path,
    missing: &'static str,
) {
    if !path.is_file() {
        findings.push(PreflightFinding::fail(
            check,
            format!("{missing}: {}", path.display()),
        ));
        return;
    }
    match reactor_edge_daemon::tls::load_private_key(path) {
        Ok(_) => findings.push(PreflightFinding::pass(
            check,
            format!("parsed private key {}", path.display()),
        )),
        Err(err) => findings.push(PreflightFinding::fail(
            check,
            format!("failed to parse private key {}: {err}", path.display()),
        )),
    }
}

fn check_required_file(
    findings: &mut Vec<PreflightFinding>,
    check: &'static str,
    path: &Path,
    missing: &'static str,
) {
    if path.is_file() {
        findings.push(PreflightFinding::pass(
            check,
            format!("found {}", path.display()),
        ));
    } else {
        findings.push(PreflightFinding::fail(
            check,
            format!("{missing}: {}", path.display()),
        ));
    }
}

fn key(args: &KeyArgs) -> Result<CommandOutput> {
    match &args.command {
        KeyCommand::Generate { db, yes } => key_rotate(db, *yes),
        KeyCommand::RekeyIntegrationTasks {
            db,
            old_key,
            old_key_file,
            new_key,
            new_key_file,
            dry_run,
            yes,
        } => key_rekey_integration_tasks(
            db,
            old_key.as_deref(),
            old_key_file.as_deref(),
            new_key.as_deref(),
            new_key_file.as_deref(),
            *dry_run,
            *yes,
        ),
    }
}

fn ops_backup(db: &Path, out: &Path, _include_ciphertext: bool) -> Result<CommandOutput> {
    if !db.is_file() {
        return Err(anyhow!("source database {} does not exist", db.display()));
    }
    let store =
        Db::open(db).with_context(|| format!("failed to open source database {}", db.display()))?;
    let report = store.backup_to(out)?;
    let hash_path = out.with_extension(format!(
        "{}.sha256",
        out.extension().and_then(|s| s.to_str()).unwrap_or("bin")
    ));
    fs::write(
        &hash_path,
        format!("{}  {}\n", report.sha256, out.display()),
    )
    .ok();

    let human = format!(
        "backup complete (SQLite VACUUM INTO online snapshot)\n  source:      {}\n  output:      {}\n  size:        {} bytes\n  sha256:      {}\n  hash:        {}\n  duration_ms: {}\n  note:        VACUUM INTO creates a compact SQLite image; restore still requires the daemon to be stopped before replacing the live database file.",
        report.source,
        report.destination,
        report.size_bytes,
        &report.sha256[..16],
        hash_path.display(),
        report.duration_ms
    );
    Ok(CommandOutput {
        human,
        json: json!({
            "action": "backup",
            "kind": "sqlite_vacuum_into",
            "source": report.source,
            "output": report.destination,
            "size_bytes": report.size_bytes,
            "duration_ms": report.duration_ms,
            "sha256": report.sha256,
            "hash_sidecar": hash_path.to_string_lossy(),
            "include_ciphertext": "no-op: ciphertext lives inside the SQLite file; VACUUM INTO captures the encrypted rows"
        }),
    })
}

fn ops_restore(backup: &Path, db: &Path, yes: bool) -> Result<CommandOutput> {
    if !backup.is_file() {
        return Err(anyhow!("backup file {} does not exist", backup.display()));
    }
    if !yes {
        return Err(anyhow!(
            "refusing to restore without --yes (this overwrites {})",
            db.display()
        ));
    }
    let report = Db::restore_file(backup, db, true)?;
    let human = format!(
        "restore complete (validated SQLite image)\n  backup:     {}\n  target:     {}\n  preserved:  {}\n  sidecars:   {}\n  integrity:  {}\n  size:       {} bytes\n  sha256:     {}\n  tables:     {}\n  note:       run this only while the daemon is stopped; restart daemon after restore so migrations and pools reopen cleanly.",
        report.source,
        report.destination,
        report.preserved_existing.as_deref().unwrap_or("none"),
        if report.removed_sidecars.is_empty() {
            "none".to_string()
        } else {
            report.removed_sidecars.join(", ")
        },
        report.integrity_check,
        report.size_bytes,
        &report.sha256[..16],
        report.tables.join(", ")
    );
    Ok(CommandOutput {
        human,
        json: json!({
            "action": "restore",
            "backup": report.source,
            "target": report.destination,
            "preserved_pre_restore": report.preserved_existing,
            "removed_sidecars": report.removed_sidecars,
            "integrity_check": report.integrity_check,
            "size_bytes": report.size_bytes,
            "sha256": report.sha256,
            "tables": report.tables,
            "validated": "sqlite_magic_header_and_schema_open"
        }),
    })
}

fn ops_wipe(db: &Path, yes: bool) -> Result<CommandOutput> {
    if !db.exists() {
        return Err(anyhow!("database {} does not exist", db.display()));
    }
    if !yes {
        return Err(anyhow!(
            "refusing to wipe without --yes (this overwrites and removes the SQLite main file, WAL/SHM sidecars, the <db>.key file, and any *.backup/*.snapshot files in the backups/ subdirectory next to {})",
            db.display()
        ));
    }
    let original_size = fs::metadata(db).map(|m| m.len()).unwrap_or(0);
    let mut rng = StdRng::from_entropy();
    let mut scope = Vec::new();
    let mut bytes_overwritten: u64 = 0;
    let mut files_removed: u64 = 0;

    // 1. SQLite main file.
    bytes_overwritten += overwrite_file_with_random(db, &mut rng, original_size, 3)?;
    fs::remove_file(db).with_context(|| format!("failed to remove {}", db.display()))?;
    files_removed += 1;
    scope.push("sqlite_main_file".to_string());

    // 2. WAL/SHM sidecars.
    for suffix in ["-wal", "-shm", "-journal"] {
        let side = with_suffix(db, suffix);
        if side.is_file() {
            let size = fs::metadata(&side).map(|m| m.len()).unwrap_or(0);
            bytes_overwritten += overwrite_file_with_random(&side, &mut rng, size, 1)?;
            fs::remove_file(&side).ok();
            files_removed += 1;
            scope.push(format!("sqlite_{}", suffix.trim_start_matches('-')));
        }
    }

    // 3. <db>.key file.
    let key_path = with_extension(db, "key");
    if key_path.is_file() {
        let size = fs::metadata(&key_path).map(|m| m.len()).unwrap_or(0);
        bytes_overwritten += overwrite_file_with_random(&key_path, &mut rng, size, 1)?;
        fs::remove_file(&key_path).ok();
        files_removed += 1;
        scope.push("db_key_file".to_string());
    }

    // 4. Backup snapshots in <db parent>/backups/ matching the db stem.
    if let Some(parent) = db.parent() {
        let backup_dir = parent.join("backups");
        if backup_dir.is_dir() {
            if let Some(stem) = db.file_stem().map(|s| s.to_string_lossy().into_owned()) {
                if let Ok(entries) = fs::read_dir(&backup_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                        if !name.starts_with(&stem) {
                            continue;
                        }
                        if path.is_file() {
                            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                            bytes_overwritten +=
                                overwrite_file_with_random(&path, &mut rng, size, 1)?;
                            fs::remove_file(&path).ok();
                            files_removed += 1;
                            scope.push(format!("backup_snapshot:{}", name));
                        }
                    }
                }
            }
        }
    }

    let human = format!(
        "wipe complete\n  bytes_overwritten: {} (3 passes on main file, 1 pass on each sidecar/backup/key)\n  files_removed:        {}\n  scope:                 {}\n  physical_erase:        SSD/NVMe overwrite is NOT a physical erase. For physical retirement run blkdiscard /hdparm --security-erase after this command. See docs/upper_computer_production_operations.md.",
        bytes_overwritten,
        files_removed,
        scope.join(", ")
    );
    Ok(CommandOutput {
        human,
        json: json!({
            "action": "wipe",
            "path": db.to_string_lossy(),
            "bytes_overwritten": bytes_overwritten,
            "files_removed": files_removed,
            "scope": scope,
            "physical_erase_required": "blkdiscard or hdparm --security-erase for SSD/NVMe retirement"
        }),
    })
}

fn overwrite_file_with_random(
    path: &Path,
    rng: &mut StdRng,
    size: u64,
    passes: usize,
) -> Result<u64> {
    if size == 0 {
        // Truncate to zero so the filesystem releases the inode.
        std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(path)
            .with_context(|| format!("failed to open {} for truncate", path.display()))?;
        return Ok(0);
    }
    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("failed to open {} for overwrite", path.display()))?;
    for _ in 0..passes {
        let mut buffer = vec![0u8; 64 * 1024];
        let mut remaining = size as usize;
        file.seek(SeekFrom::Start(0))?;
        while remaining > 0 {
            let chunk = remaining.min(buffer.len());
            rng.fill(&mut buffer[..chunk]);
            file.write_all(&mut buffer[..chunk])?;
            remaining -= chunk;
        }
        file.sync_all().ok();
    }
    file.set_len(0).ok();
    Ok(size * passes as u64)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut p = path.to_path_buf();
    let mut name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.push_str(suffix);
    p.set_file_name(name);
    p
}

fn with_extension(path: &Path, ext: &str) -> PathBuf {
    let p = path.to_path_buf();
    let mut name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Some(dot) = name.rfind('.') {
        name.truncate(dot);
    }
    name.push('.');
    name.push_str(ext);
    p.with_file_name(name)
}

fn key_rotate(db: &Path, yes: bool) -> Result<CommandOutput> {
    if !db.exists() {
        return Err(anyhow!("database {} does not exist", db.display()));
    }
    if !yes {
        return Err(anyhow!(
            "refusing to generate a new key without --yes (this overwrites <db>.key and changes the AES-GCM key used for new integration task ciphertext; previously encrypted rows will become unreadable once the daemon is restarted with the new env var)"
        ));
    }
    let mut new_key_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut new_key_bytes);
    let new_key_hex = new_key_bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let key_path = db.with_extension("key");
    fs::write(
        &key_path,
        format!("XINGSHU_DB_ENCRYPTION_KEY={new_key_hex}\n"),
    )
    .with_context(|| format!("failed to write {}", key_path.display()))?;
    // Tighten permissions to 0600 on Unix. Windows ignores mode bits, so the
    // NTFS ACL must be set out of band; the docs call this out.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&key_path, perms)
            .with_context(|| format!("failed to chmod 0600 {}", key_path.display()))?;
    }
    // Print only the env var NAME, not the key value, on the human channel.
    // The key lives in the file; the operator reads it themselves and
    // exports it into the daemon's env. JSON intentionally omits the key
    // material as well so logs do not leak the secret.
    let human = format!(
        "key material generated (not printed)\n  path:           {}\n  new_key_file:   {}\n  next_step:      read the file with restricted permissions, then export XINGSHU_DB_ENCRYPTION_KEY before starting the daemon",
        db.display(),
        key_path.display()
    );
    Ok(CommandOutput {
        human,
        json: json!({
            "action": "key-generate",
            "database": db.to_string_lossy(),
            "new_key_file": key_path.to_string_lossy(),
            "new_key_env_var": "XINGSHU_DB_ENCRYPTION_KEY",
            "warning": "operator must restart the daemon with the new env var; rows previously encrypted with the old key will become unreadable. Re-encryption of existing rows is NOT performed by this command."
        }),
    })
}

#[derive(Debug, Default)]
struct RekeyIntegrationReport {
    rows_scanned: usize,
    fields_scanned: usize,
    encrypted_fields_seen: usize,
    plaintext_fields_seen: usize,
    fields_reencrypted: usize,
    plaintext_fields_encrypted: usize,
    fields_changed: usize,
}

fn key_rekey_integration_tasks(
    db: &Path,
    old_key_arg: Option<&str>,
    old_key_file: Option<&Path>,
    new_key_arg: Option<&str>,
    new_key_file: Option<&Path>,
    dry_run: bool,
    yes: bool,
) -> Result<CommandOutput> {
    if !db.is_file() {
        return Err(anyhow!("database {} does not exist", db.display()));
    }
    if !dry_run && !yes {
        return Err(anyhow!(
            "refusing to re-encrypt integration task payloads without --yes (run with --dry-run first, then stop the daemon and rerun with --yes)"
        ));
    }

    let old_key = load_cli_key("old", old_key_arg, old_key_file, true)?;
    let new_key = load_cli_key("new", new_key_arg, new_key_file, false)?;
    if old_key == new_key {
        return Err(anyhow!(
            "old and new encryption keys are identical; refusing no-op rekey"
        ));
    }
    let old_cipher = DbEncryption::from_key(old_key, "cli-old-key");
    let new_cipher = DbEncryption::from_key(new_key, "cli-new-key");

    let mut conn =
        Connection::open(db).with_context(|| format!("failed to open {}", db.display()))?;
    conn.busy_timeout(Duration::from_secs(15))?;
    let tx = conn.transaction()?;
    let mut rows = Vec::new();
    {
        let mut stmt = tx.prepare(
            r#"
            SELECT id, request_json, response_json
            FROM integration_tasks
            ORDER BY id ASC
            "#,
        )?;
        let iter = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        for row in iter {
            rows.push(row?);
        }
    }

    let mut report = RekeyIntegrationReport::default();
    for (id, request_json, response_json) in rows {
        report.rows_scanned += 1;
        let request = rekey_integration_field(
            &old_cipher,
            &new_cipher,
            &request_json,
            &mut report,
            "request_json",
            id,
        )?;
        let response = rekey_integration_field(
            &old_cipher,
            &new_cipher,
            &response_json,
            &mut report,
            "response_json",
            id,
        )?;
        if !dry_run && (request.changed || response.changed) {
            tx.execute(
                r#"
                UPDATE integration_tasks
                SET request_json = ?1, response_json = ?2
                WHERE id = ?3
                "#,
                params![request.value, response.value, id],
            )?;
        }
    }

    if dry_run {
        tx.rollback()?;
    } else {
        tx.commit()?;
        compact_rekeyed_database(&conn)?;
    }

    let mode = if dry_run { "dry-run" } else { "committed" };
    let human = format!(
        "integration task payload rekey {mode}\n  database:                    {}\n  rows_scanned:                {}\n  fields_scanned:              {}\n  encrypted_fields_seen:       {}\n  plaintext_fields_seen:       {}\n  fields_reencrypted:          {}\n  plaintext_fields_encrypted:  {}\n  fields_changed:              {}\n  next_step:                   restart daemon with {} set to the new key after verifying reads",
        db.display(),
        report.rows_scanned,
        report.fields_scanned,
        report.encrypted_fields_seen,
        report.plaintext_fields_seen,
        report.fields_reencrypted,
        report.plaintext_fields_encrypted,
        report.fields_changed,
        DB_ENCRYPTION_KEY_ENV
    );
    Ok(CommandOutput {
        human,
        json: json!({
            "action": "key-rekey-integration-tasks",
            "mode": mode,
            "database": db.to_string_lossy(),
            "rows_scanned": report.rows_scanned,
            "fields_scanned": report.fields_scanned,
            "encrypted_fields_seen": report.encrypted_fields_seen,
            "plaintext_fields_seen": report.plaintext_fields_seen,
            "fields_reencrypted": report.fields_reencrypted,
            "plaintext_fields_encrypted": report.plaintext_fields_encrypted,
            "fields_changed": report.fields_changed,
            "encrypted_fields": [
                "integration_tasks.request_json",
                "integration_tasks.response_json"
            ],
            "new_key_env_var": DB_ENCRYPTION_KEY_ENV,
            "requires_daemon_stopped": true,
            "secret_material_printed": false
        }),
    })
}

fn compact_rekeyed_database(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA wal_checkpoint(TRUNCATE);
        VACUUM;
        PRAGMA wal_checkpoint(TRUNCATE);
        "#,
    )
    .context("failed to compact database and truncate WAL after key rekey")
}

struct RekeyedField {
    value: String,
    changed: bool,
}

fn rekey_integration_field(
    old_cipher: &DbEncryption,
    new_cipher: &DbEncryption,
    value: &str,
    report: &mut RekeyIntegrationReport,
    field: &str,
    row_id: i64,
) -> Result<RekeyedField> {
    report.fields_scanned += 1;
    let was_encrypted = value.starts_with(ENCRYPTED_JSON_PREFIX);
    if was_encrypted {
        report.encrypted_fields_seen += 1;
    } else {
        report.plaintext_fields_seen += 1;
    }
    let plaintext = old_cipher
        .decrypt_json_if_needed_anyhow(value)
        .with_context(|| {
            format!("failed to decrypt integration_tasks.{field} for row id {row_id}")
        })?;
    serde_json::from_str::<Value>(&plaintext).with_context(|| {
        format!("integration_tasks.{field} for row id {row_id} is not valid JSON")
    })?;
    let encrypted = new_cipher.encrypt_json(&plaintext).with_context(|| {
        format!("failed to encrypt integration_tasks.{field} for row id {row_id}")
    })?;
    if was_encrypted {
        report.fields_reencrypted += 1;
    } else {
        report.plaintext_fields_encrypted += 1;
    }
    report.fields_changed += 1;
    Ok(RekeyedField {
        value: encrypted,
        changed: true,
    })
}

fn load_cli_key(
    label: &str,
    direct: Option<&str>,
    file: Option<&Path>,
    allow_env: bool,
) -> Result<[u8; 32]> {
    match (direct, file) {
        (Some(_), Some(_)) => {
            return Err(anyhow!(
                "provide either --{label}-key or --{label}-key-file, not both"
            ));
        }
        (Some(value), None) => parse_encryption_key(value)
            .with_context(|| format!("--{label}-key must be 32 bytes, 64 hex chars, or base64")),
        (None, Some(path)) => {
            let content = fs::read_to_string(path)
                .with_context(|| format!("failed to read --{label}-key-file {}", path.display()))?;
            let value = parse_key_file_content(&content);
            parse_encryption_key(value).with_context(|| {
                format!(
                    "--{label}-key-file {} must contain a raw key or {DB_ENCRYPTION_KEY_ENV}=...",
                    path.display()
                )
            })
        }
        (None, None) if allow_env => {
            let value = env::var(DB_ENCRYPTION_KEY_ENV).with_context(|| {
                format!(
                    "missing old key: pass --old-key, --old-key-file, or set {DB_ENCRYPTION_KEY_ENV}"
                )
            })?;
            parse_encryption_key(&value).with_context(|| {
                format!("{DB_ENCRYPTION_KEY_ENV} must be 32 bytes, 64 hex chars, or base64")
            })
        }
        (None, None) => Err(anyhow!(
            "missing {label} key: pass --{label}-key or --{label}-key-file"
        )),
    }
}

fn parse_key_file_content(content: &str) -> &str {
    let trimmed = content.trim();
    trimmed
        .strip_prefix(&format!("{DB_ENCRYPTION_KEY_ENV}="))
        .unwrap_or(trimmed)
        .trim()
}
