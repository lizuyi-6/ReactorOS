use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Stdio},
    time::Instant,
};

use anyhow::{anyhow, Context, Result};
use clap::{Args, Parser, Subcommand};
use reactor_edge_daemon::{
    config::{load_device_config, load_safety_config},
    control::{evaluate_safety_request, SafetyGuardRequest, SafetyGuardResponse},
    db::Db,
    local_ai::LocalAiStatus,
    mqtt::load_integration_config,
    safety_guard::evaluate_with_process,
    state::ControlTargets,
};
use reqwest::{Client, Method, StatusCode};
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
    #[arg(long, default_value = "static")]
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
    /// Check whether local LoRA training is available.
    Train,
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
        Commands::Ai(args) => ai(&client, &cli.api, token, args).await,
        Commands::Audit(args) => audit(&client, &cli.api, token, args).await,
        Commands::Modbus(args) => modbus(&client, &cli.api, token, args).await,
        Commands::Safety(args) => safety_guard_check(args),
        Commands::Perf(args) => perf(&client, &cli.api, args).await,
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
        "ph": round2_value(6.15 + (phase * 0.09).cos() * 0.05)
    })
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

fn round2_value(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
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
    args: &AiArgs,
) -> Result<CommandOutput> {
    match &args.command {
        AiCommand::Suggest => {
            let value = request_json(
                client,
                Method::GET,
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
        AiCommand::Train => {
            let status = LocalAiStatus::from_env();
            let missing = if status.missing.is_empty() {
                "daemon training endpoint".to_string()
            } else {
                status.missing.join(", ")
            };
            Err(anyhow!(
                "local LoRA training is not exposed by the current daemon API yet; missing local AI assets: {missing}"
            ))
        }
    }
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
            let response = evaluate_with_process(&guard_path, &request)?;
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
                let response = evaluate_with_process(&guard_path, &guard_request)?;
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
