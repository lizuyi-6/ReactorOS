use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use reactor_edge_daemon::{
    config::load_safety_config,
    control::SafetyGuardRequest,
    db::{Db, ProductResult},
    safety_guard::evaluate_with_process,
    state::{ControlTargets, SensorSnapshot},
};

// Some Windows test cases spawn real subprocesses (ping / sleep) whose
// IO contention is amplified by cargo test's default multi-thread runner.
// Serialize them through a single global mutex so the slow-script timing
// in `safety_guard_external_process_timeout` is not perturbed by sibling
// test threads inside the same test binary.
fn windows_subprocess_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn xingshu() -> Command {
    Command::new(env!("CARGO_BIN_EXE_xingshu"))
}

fn daemon() -> Command {
    Command::new(env!("CARGO_BIN_EXE_reactor-edge-daemon"))
}

fn safety_guard() -> Command {
    Command::new(env!("CARGO_BIN_EXE_reactor-safety-guard"))
}

#[cfg(windows)]
fn write_slow_guard_script(dir: &Path) -> PathBuf {
    let path = dir.join("slow-guard.cmd");
    // The slow script must run for at least 10 seconds. If the safety
    // guard timeout really fires, evaluate_with_process should return in
    // tens of milliseconds — well before the script's natural exit. The
    // hard floor is 10s so the test cannot accidentally pass just because
    // the slow script finished before the assertion ran.
    std::fs::write(&path, "@echo off\r\nping -n 15 127.0.0.1 >NUL\r\n").unwrap();
    path
}

#[cfg(unix)]
fn write_slow_guard_script(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("slow-guard.sh");
    std::fs::write(&path, "#!/bin/sh\nsleep 15\n").unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

#[test]
fn xingshu_help_exposes_prd_command_surface() {
    let output = xingshu().arg("--help").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    for command in [
        "start", "stop", "status", "config", "data", "control", "ai", "audit", "modbus", "safety",
        "perf",
    ] {
        assert!(
            stdout.contains(command),
            "help output should include {command}: {stdout}"
        );
    }
}

#[test]
fn xingshu_data_help_exposes_excel_export() {
    let output = xingshu().args(["data", "--help"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("export-xlsx"),
        "data help should expose Excel export: {stdout}"
    );
    assert!(
        stdout.contains("sample"),
        "data help should expose pipeline sample ingest: {stdout}"
    );
    assert!(
        stdout.contains("delete"),
        "data help should expose local runtime data deletion: {stdout}"
    );
}

#[test]
fn xingshu_ai_help_exposes_experiment_plan() {
    let output = xingshu().args(["ai", "--help"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("plan"),
        "ai help should expose experiment plan draft: {stdout}"
    );
}

#[test]
fn xingshu_can_print_local_config_as_json_without_daemon() {
    let output = xingshu()
        .args(["config", "--local", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["device"]["mode"], "pipeline");
    assert!(value["safety"]["temperature"]["max_c"].is_number());
    assert!(value["safety"]["stirrer"]["max_rpm"].is_number());
}

#[test]
fn xingshu_perf_help_exposes_smoke_check() {
    let output = xingshu().args(["perf", "--help"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("smoke"),
        "perf help should expose smoke check: {stdout}"
    );
}

#[test]
fn xingshu_ops_preflight_fails_production_defaults() {
    let output = xingshu()
        .args(["ops", "preflight", "--production", "--json"])
        .env_remove("XINGSHU_AUTH_SECRET")
        .env_remove("XINGSHU_OPERATOR_PASSWORD")
        .env_remove("XINGSHU_ENGINEER_PASSWORD")
        .env_remove("XINGSHU_ADMIN_PASSWORD")
        .env_remove("XINGSHU_DB_ENCRYPTION_KEY")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("XINGSHU_AUTH_SECRET is not set")
            || stderr.contains("XINGSHU_AUTH_SECRET still uses"),
        "preflight should reject default auth secret: {stderr}"
    );
    assert!(
        stderr.contains("XINGSHU_DB_ENCRYPTION_KEY is not set"),
        "preflight should require DB encryption key: {stderr}"
    );
}

#[test]
fn xingshu_ops_preflight_passes_with_production_secrets_and_tls_files() {
    let temp_dir = tempfile::tempdir().unwrap();
    let device_path = temp_dir.path().join("device.toml");
    let integration_path = temp_dir.path().join("integration.toml");
    let backup_service = temp_dir.path().join("reactor-edge-backup.service");
    let backup_timer = temp_dir.path().join("reactor-edge-backup.timer");
    let backup_script = temp_dir.path().join("reactor-edge-backup.sh");
    std::fs::write(&backup_service, "[Service]\nType=oneshot\n").unwrap();
    std::fs::write(&backup_timer, "[Timer]\nOnCalendar=*-*-* 02:17:00\n").unwrap();
    std::fs::write(&backup_script, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::write(
        &device_path,
        r#"
mode = "json_bridge"

[serial]
port = "/dev/ttyUSB0"
baudrate = 9600
parity = "N"
stopbits = 1
bytesize = 8
timeout_ms = 1000

[modbus]
slave_id = 1

[esp32]
frame_prefix = "RX"
command_prefix = "TX"
checksum = true
max_line_bytes = 256

[json_bridge]
state_path = "/project/state.json"
control_path = "/project/control.json"
max_state_age_ms = 6000
request_id_prefix = "reactor-os"
speed_steps_per_cycle = 200.0
speed_deadband_cpm = 1.0
temperature_deadband_c = 1.0
relay_temperature_control = false

[modbus.registers.temperature_c]
address = 0
scale = 0.1
offset = 0.0
min_valid = 0.0
max_valid = 250.0

[modbus.registers.stirrer_rpm]
address = 1
scale = 1.0
offset = 0.0
min_valid = 0.0
max_valid = 2000.0

[modbus.registers.target_temperature_c]
address = 10
scale = 0.1
offset = 0.0

[modbus.registers.target_stirrer_rpm]
address = 11
scale = 1.0
offset = 0.0
"#,
    )
    .unwrap();
    let cert = std::path::Path::new("tests/fixtures/tls/server.crt")
        .canonicalize()
        .unwrap();
    let key = std::path::Path::new("tests/fixtures/tls/server.key")
        .canonicalize()
        .unwrap();
    let cert_text = cert.to_string_lossy().replace('\\', "/");
    let key_text = key.to_string_lossy().replace('\\', "/");
    std::fs::write(
        &integration_path,
        format!(
            r#"
[mqtt]
enabled = true
host = "127.0.0.1"
port = 8883
client_id = "xingshu-preflight-test"
username = "mqtt-user"
password = "mqtt-password"
use_tls = true
ca_cert = "{}"
client_cert = "{}"
client_key = "{}"
keep_alive_s = 30
queue_capacity = 16
task_topic = "xingshu/reactor_001/tasks"
receipt_topic = "xingshu/reactor_001/task_receipts"
status_topic = "xingshu/reactor_001/status"
alert_topic = "xingshu/reactor_001/alerts"
alert_interval_s = 5

[modbus_tcp]
enabled = true
bind = "0.0.0.0:1502"
unit_id = 1
require_tls = true
tls_cert = "{}"
tls_key = "{}"
max_pdu_bytes = 253
"#,
            cert_text, cert_text, key_text, cert_text, key_text
        ),
    )
    .unwrap();

    let output = xingshu()
        .args([
            "ops",
            "preflight",
            "--production",
            "--json",
            "--config",
            device_path.to_str().unwrap(),
            "--safety",
            "config/safety.toml",
            "--integration",
            integration_path.to_str().unwrap(),
            "--backup-service",
            backup_service.to_str().unwrap(),
            "--backup-timer",
            backup_timer.to_str().unwrap(),
            "--backup-script",
            backup_script.to_str().unwrap(),
        ])
        .env("XINGSHU_AUTH_SECRET", "0123456789abcdef0123456789abcdef")
        .env("XINGSHU_OPERATOR_PASSWORD", "operator-password-123")
        .env("XINGSHU_ENGINEER_PASSWORD", "engineer-password-123")
        .env("XINGSHU_ADMIN_PASSWORD", "admin-password-123")
        .env(
            "XINGSHU_DB_ENCRYPTION_KEY",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["status"], "ok");
    assert_eq!(value["counts"]["fail"], 0);
    assert!(
        value["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["check"] == "backup_timer" && finding["level"] == "pass"),
        "preflight should check backup timer path: {value}"
    );
}

#[test]
fn xingshu_ai_train_reports_no_completed_batches_before_asset_checks() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("reactor.sqlite3");
    let output = xingshu()
        .args(["--db", db_path.to_str().unwrap(), "ai", "train"])
        .env_remove("XINGSHU_LOCAL_AI_ENABLED")
        .env_remove("XINGSHU_LOCAL_AI_BIN")
        .env_remove("XINGSHU_LOCAL_AI_GGUF")
        .env_remove("XINGSHU_LOCAL_AI_LORA")
        .env_remove("XINGSHU_LOCAL_AI_TRAIN_SCRIPT")
        .env_remove("XINGSHU_LOCAL_AI_CONVERT_SCRIPT")
        .env_remove("XINGSHU_LOCAL_AI_RK_REPORT")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("no completed product-result batches"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn xingshu_ai_train_export_only_writes_supervised_jsonl_dataset() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("reactor.sqlite3");
    let dataset_path = temp_dir.path().join("lora-dataset.jsonl");
    let db = Db::open(&db_path).unwrap();
    let batch = db
        .create_batch("cli lora export", 72.5, 420.0, 35.0, 55.0)
        .unwrap();
    db.insert_sample(
        Some(batch.id),
        &SensorSnapshot {
            temperature_c: 72.4,
            pressure_mpa: 0.42,
            stirrer_rpm: 419.0,
            shake_speed_cpm: 12.0,
            tilt_state: 1,
            tilt_angle_deg: 3.0,
            flow_rate_l_min: 0.8,
            product_concentration_percent: 48.0,
            ph: 6.8,
            captured_at: chrono::Utc::now(),
        },
    )
    .unwrap();
    db.insert_product_result(&ProductResult {
        batch_id: batch.id,
        yield_percent: 87.5,
        product_ratio: 0.91,
        notes: "cli lora dataset export".to_string(),
    })
    .unwrap();

    let output = xingshu()
        .args([
            "--db",
            db_path.to_str().unwrap(),
            "--json",
            "ai",
            "train",
            "--export-only",
            "--dataset",
            dataset_path.to_str().unwrap(),
        ])
        .env_remove("XINGSHU_LOCAL_AI_ENABLED")
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["mode"], "export_only");
    assert_eq!(value["rows"], 1);
    let jsonl = std::fs::read_to_string(&dataset_path).unwrap();
    let lines = jsonl.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1);
    let row: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert!(
        row["messages"][2]["content"]
            .as_str()
            .unwrap()
            .contains("target_temperature_c"),
        "assistant content should be JSON-like parameter target: {row}"
    );
    assert_eq!(row["output"]["expected_yield_percent"], 87.5);
    assert_eq!(row["input"]["samples"].as_array().unwrap().len(), 1);
}

#[test]
fn xingshu_ai_train_invokes_configured_training_entrypoint_with_dataset() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("reactor.sqlite3");
    let dataset_path = temp_dir.path().join("lora-dataset.jsonl");
    let model_path = temp_dir.path().join("qwen.gguf");
    let convert_path = temp_dir.path().join("convert.py");
    std::fs::write(&model_path, "fake gguf").unwrap();
    std::fs::write(&convert_path, "fake convert").unwrap();
    let train_script = write_local_ai_train_script(temp_dir.path());
    let db = Db::open(&db_path).unwrap();
    let batch = db
        .create_batch("cli lora train", 70.0, 400.0, 30.0, 45.0)
        .unwrap();
    db.insert_product_result(&ProductResult {
        batch_id: batch.id,
        yield_percent: 82.0,
        product_ratio: 0.88,
        notes: "training entrypoint invocation".to_string(),
    })
    .unwrap();

    let output = xingshu()
        .args([
            "--db",
            db_path.to_str().unwrap(),
            "--json",
            "ai",
            "train",
            "--dataset",
            dataset_path.to_str().unwrap(),
            "--dry-run",
            "--timeout-s",
            "10",
        ])
        .env("XINGSHU_LOCAL_AI_ENABLED", "true")
        .env("XINGSHU_LOCAL_AI_GGUF", &model_path)
        .env("XINGSHU_LOCAL_AI_TRAIN_SCRIPT", &train_script)
        .env("XINGSHU_LOCAL_AI_CONVERT_SCRIPT", &convert_path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["mode"], "train");
    assert_eq!(value["rows"], 1);
    let manifest_path = PathBuf::from(value["manifest"].as_str().unwrap());
    assert!(manifest_path.is_file());
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["schema"], "xingshu.local_ai.training_manifest.v1");
    assert_eq!(manifest["promotion"]["promoted"], false);
    assert_eq!(value["training"]["exit_code"], 0);
    assert!(value["training"]["stdout"]
        .as_str()
        .unwrap()
        .contains(dataset_path.to_str().unwrap()));
    assert!(dataset_path.is_file());
}

#[test]
fn xingshu_ai_train_promotes_passing_candidate_adapter_with_backup() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("reactor.sqlite3");
    let dataset_path = temp_dir.path().join("lora-dataset.jsonl");
    let manifest_path = temp_dir.path().join("train-manifest.json");
    let model_path = temp_dir.path().join("qwen.gguf");
    let convert_path = temp_dir.path().join("convert.py");
    let current_adapter = temp_dir.path().join("active-adapter.gguf");
    let candidate_adapter = temp_dir.path().join("candidate-adapter.gguf");
    std::fs::write(&model_path, "fake gguf").unwrap();
    std::fs::write(&convert_path, "fake convert").unwrap();
    std::fs::write(&current_adapter, "old adapter").unwrap();
    std::fs::write(&candidate_adapter, "new adapter").unwrap();
    let train_script = write_local_ai_candidate_train_script(temp_dir.path(), &candidate_adapter);
    let db = Db::open(&db_path).unwrap();
    let batch = db
        .create_batch("cli lora promote", 73.0, 430.0, 32.0, 44.0)
        .unwrap();
    db.insert_product_result(&ProductResult {
        batch_id: batch.id,
        yield_percent: 90.0,
        product_ratio: 0.94,
        notes: "training promotion".to_string(),
    })
    .unwrap();

    let output = xingshu()
        .args([
            "--db",
            db_path.to_str().unwrap(),
            "--json",
            "ai",
            "train",
            "--dataset",
            dataset_path.to_str().unwrap(),
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--promote",
            "--min-eval-score",
            "0.8",
            "--timeout-s",
            "10",
        ])
        .env("XINGSHU_LOCAL_AI_ENABLED", "true")
        .env("XINGSHU_LOCAL_AI_GGUF", &model_path)
        .env("XINGSHU_LOCAL_AI_LORA", &current_adapter)
        .env("XINGSHU_LOCAL_AI_TRAIN_SCRIPT", &train_script)
        .env("XINGSHU_LOCAL_AI_CONVERT_SCRIPT", &convert_path)
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["promotion"]["promoted"], true);
    assert_eq!(
        std::fs::read_to_string(&current_adapter).unwrap(),
        "new adapter"
    );
    let backup = PathBuf::from(value["promotion"]["backup"].as_str().unwrap());
    assert!(backup.is_file());
    assert_eq!(std::fs::read_to_string(&backup).unwrap(), "old adapter");
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["evaluation"]["score"], 0.91);
    assert_eq!(
        manifest["promotion"]["target"],
        current_adapter.display().to_string()
    );
}

#[test]
fn xingshu_key_rekey_integration_tasks_migrates_existing_payloads() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("reactor.sqlite3");
    let old_key = [21_u8; 32];
    let new_key = [22_u8; 32];
    let old_key_hex = hex_key(old_key);
    let new_key_path = temp_dir.path().join("new-db.key");
    std::fs::write(
        &new_key_path,
        format!("XINGSHU_DB_ENCRYPTION_KEY={}\n", hex_key(new_key)),
    )
    .unwrap();

    let db = Db::open_with_encryption_key(&db_path, old_key).unwrap();
    let encrypted_task = db
        .create_integration_task(
            "ainas",
            Some("rekey-encrypted-001"),
            "set_targets",
            &serde_json::json!({
                "action": "set_targets",
                "reason": "old encrypted payload",
                "target_temperature_c": 73.5
            }),
        )
        .unwrap();
    db.update_integration_task(
        encrypted_task.id,
        "executed",
        &serde_json::json!({
            "code": 0,
            "message": "old encrypted response"
        }),
    )
    .unwrap();
    drop(db);

    let now = chrono::Utc::now().to_rfc3339();
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            r#"
            INSERT INTO integration_tasks
                (external_task_id, source, action, status, request_json, response_json, created_at, updated_at)
            VALUES (?1, 'mqtt', 'set_targets', 'executed', ?2, ?3, ?4, ?4)
            "#,
            rusqlite::params![
                "rekey-plaintext-001",
                serde_json::json!({ "reason": "legacy plaintext request" }).to_string(),
                serde_json::json!({ "message": "legacy plaintext response" }).to_string(),
                now
            ],
        )
        .unwrap();
    }
    let before_dry_run = raw_integration_payloads(&db_path);

    let dry_run = xingshu()
        .args([
            "--json",
            "key",
            "rekey-integration-tasks",
            "--db",
            db_path.to_str().unwrap(),
            "--old-key",
            &old_key_hex,
            "--new-key-file",
            new_key_path.to_str().unwrap(),
            "--dry-run",
        ])
        .env_remove("XINGSHU_DB_ENCRYPTION_KEY")
        .output()
        .unwrap();
    assert!(
        dry_run.status.success(),
        "dry-run stderr: {}",
        String::from_utf8_lossy(&dry_run.stderr)
    );
    let dry_run_value: serde_json::Value =
        serde_json::from_slice(&dry_run.stdout).expect("dry-run should emit JSON");
    assert_eq!(dry_run_value["mode"], "dry-run");
    assert_eq!(dry_run_value["rows_scanned"], 2);
    assert_eq!(dry_run_value["fields_reencrypted"], 2);
    assert_eq!(dry_run_value["plaintext_fields_encrypted"], 2);
    assert_eq!(raw_integration_payloads(&db_path), before_dry_run);

    let committed = xingshu()
        .args([
            "--json",
            "key",
            "rekey-integration-tasks",
            "--db",
            db_path.to_str().unwrap(),
            "--new-key-file",
            new_key_path.to_str().unwrap(),
            "--yes",
        ])
        .env("XINGSHU_DB_ENCRYPTION_KEY", &old_key_hex)
        .output()
        .unwrap();
    assert!(
        committed.status.success(),
        "commit stderr: {}",
        String::from_utf8_lossy(&committed.stderr)
    );
    let stdout = String::from_utf8(committed.stdout).unwrap();
    assert!(
        !stdout.contains(&hex_key(new_key)),
        "rekey JSON must not print new key material: {stdout}"
    );
    let committed_value: serde_json::Value =
        serde_json::from_str(&stdout).expect("commit should emit JSON");
    assert_eq!(committed_value["mode"], "committed");
    assert_eq!(committed_value["fields_changed"], 4);

    let raw_after = raw_integration_payloads(&db_path);
    assert!(raw_after.iter().all(|(_, request, response)| request
        .starts_with("xingshu:v1:aes256gcm:")
        && response.starts_with("xingshu:v1:aes256gcm:")));
    assert!(raw_after.iter().all(|(_, request, response)| {
        !request.contains("old encrypted payload")
            && !response.contains("old encrypted response")
            && !request.contains("legacy plaintext request")
            && !response.contains("legacy plaintext response")
    }));
    let sqlite_family = raw_sqlite_family_text(&db_path);
    assert!(
        !sqlite_family.contains("legacy plaintext request"),
        "rekey should vacuum/truncate SQLite family files so old plaintext is not left in WAL or free pages"
    );

    let new_db = Db::open_with_encryption_key(&db_path, new_key).unwrap();
    let tasks = new_db.integration_tasks(None, 10).unwrap();
    assert!(tasks
        .iter()
        .any(|task| task.request["reason"] == "old encrypted payload"
            && task.response["message"] == "old encrypted response"));
    assert!(tasks
        .iter()
        .any(|task| task.request["reason"] == "legacy plaintext request"
            && task.response["message"] == "legacy plaintext response"));

    let old_db = Db::open_with_encryption_key(&db_path, old_key).unwrap();
    let err = old_db.integration_tasks(None, 10).unwrap_err();
    assert!(
        err.to_string()
            .contains("failed to decrypt integration task payload"),
        "old key should fail after rekey, got {err}"
    );
}

#[cfg(windows)]
fn write_local_ai_train_script(dir: &Path) -> PathBuf {
    let path = dir.join("train.cmd");
    std::fs::write(
        &path,
        "@echo off\r\nsetlocal enabledelayedexpansion\r\necho {\"status\":\"ok\",\"args\":\"%*\"}\r\n",
    )
    .unwrap();
    path
}

#[cfg(windows)]
fn write_local_ai_candidate_train_script(dir: &Path, candidate_adapter: &Path) -> PathBuf {
    let path = dir.join("train-candidate.cmd");
    let candidate = candidate_adapter
        .display()
        .to_string()
        .replace('\\', "\\\\");
    std::fs::write(
        &path,
        format!(
            "@echo off\r\necho {{\"status\":\"ok\",\"evaluation\":{{\"score\":0.91,\"metrics\":{{\"loss\":0.12}}}},\"artifacts\":{{\"adapter_path\":\"{candidate}\"}}}}\r\n"
        ),
    )
    .unwrap();
    path
}

#[cfg(unix)]
fn write_local_ai_train_script(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("train.sh");
    std::fs::write(
        &path,
        "#!/bin/sh\nprintf '{\"status\":\"ok\",\"args\":\"'\nprintf '%s' \"$*\"\nprintf '\"}\\n'\n",
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

#[cfg(unix)]
fn write_local_ai_candidate_train_script(dir: &Path, candidate_adapter: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("train-candidate.sh");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\nprintf '%s\\n' '{{\"status\":\"ok\",\"evaluation\":{{\"score\":0.91,\"metrics\":{{\"loss\":0.12}}}},\"artifacts\":{{\"adapter_path\":\"{}\"}}}}'\n",
            candidate_adapter.display()
        ),
    )
    .unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

#[test]
fn daemon_help_exposes_https_tls_options() {
    let output = daemon().arg("--help").output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("--tls-cert"),
        "daemon help should expose --tls-cert: {stdout}"
    );
    assert!(
        stdout.contains("--tls-key"),
        "daemon help should expose --tls-key: {stdout}"
    );
    assert!(
        stdout.contains("--safety-guard"),
        "daemon help should expose --safety-guard: {stdout}"
    );
}

#[test]
fn daemon_rejects_unpaired_tls_options() {
    let output = daemon()
        .args(["--tls-cert", "output/tls-test/server.crt"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("--tls-cert and --tls-key must be provided together"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn safety_guard_cli_clamps_targets_through_external_process() {
    let temp_dir = tempfile::tempdir().unwrap();
    let safety_path = temp_dir.path().join("safety.toml");
    let safety = std::fs::read_to_string("config/safety.toml")
        .unwrap()
        .replace(
            "safety_guard_timeout_ms = 1000",
            "safety_guard_timeout_ms = 10000",
        );
    std::fs::write(&safety_path, safety).unwrap();
    let output = xingshu()
        .args([
            "--json",
            "safety",
            "check",
            "--safety",
            safety_path.to_str().unwrap(),
            "--temp",
            "999",
            "--rpm",
            "9999",
            "--shake",
            "99",
            "--pressure",
            "99",
            "--guard",
            env!("CARGO_BIN_EXE_reactor-safety-guard"),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["isolation"], "external_process");
    assert_eq!(value["targets"]["temperature_c"], 160.0);
    assert_eq!(value["targets"]["stirrer_rpm"], 1200.0);
    assert_eq!(value["targets"]["shake_speed_cpm"], 60.0);
    assert_eq!(value["targets"]["target_pressure_mpa"], 10.0);
}

#[test]
fn safety_guard_external_process_timeout_returns_before_slow_guard_finishes() {
    let _guard = windows_subprocess_lock().lock().unwrap();
    let temp_dir = tempfile::tempdir().unwrap();
    let guard = write_slow_guard_script(temp_dir.path());
    let request = SafetyGuardRequest::ClampTargets {
        safety: load_safety_config("config/safety.toml").unwrap(),
        targets: ControlTargets {
            temperature_c: 80.0,
            heat_time_s: 300.0,
            hold_time_s: 600.0,
            cool_time_s: 180.0,
            stirrer_rpm: 500.0,
            shake_speed_cpm: 30.0,
            target_pressure_mpa: 0.5,
        },
    };

    let started_at = Instant::now();
    let err = evaluate_with_process(&guard, &request, Duration::from_millis(100)).unwrap_err();

    assert!(err
        .to_string()
        .contains("safety guard process exceeded timeout of 100ms"));
    // The slow script sleeps 15s. The 100ms timeout must really fire —
    // evaluate_with_process must return in well under the slow script's
    // natural exit. The 3s upper bound is a tight guarantee: the safety
    // gate returning within 3s of a 100ms timeout (i.e. the timeout
    // path works) is a strong signal that the kill is honored and the
    // process tree is reaped. If the slow script ever completes before
    // 3s, the timeout is being ignored.
    assert!(
        started_at.elapsed() < Duration::from_secs(3),
        "safety guard timeout must return well before the slow script finishes; elapsed={:?}",
        started_at.elapsed()
    );
}

#[test]
fn safety_guard_binary_accepts_json_protocol_on_stdin() {
    let request = serde_json::json!({
        "clamp_targets": {
            "safety": {
                "control": {
                    "auto_enabled_default": false,
                    "manual_lock_default": false,
                    "control_interval_ms": 2000,
                    "sensor_timeout_ms": 6000
                },
                "temperature": {
                    "min_c": 20.0,
                    "max_c": 160.0,
                    "max_step_c": 2.0,
                    "default_target_c": 60.0
                },
                "stirrer": {
                    "min_rpm": 0.0,
                    "max_rpm": 1200.0,
                    "max_step_rpm": 50.0,
                    "default_target_rpm": 300.0
                },
                "optimizer": {
                    "min_temperature_c": 35.0,
                    "max_temperature_c": 140.0,
                    "min_stirrer_rpm": 100.0,
                    "max_stirrer_rpm": 1000.0,
                    "min_heating_minutes": 15.0,
                    "max_heating_minutes": 240.0,
                    "min_stirring_minutes": 15.0,
                    "max_stirring_minutes": 240.0
                }
            },
            "targets": {
                "temperature_c": 999.0,
                "heat_time_s": 5000.0,
                "hold_time_s": 8000.0,
                "cool_time_s": 5000.0,
                "stirrer_rpm": 9999.0,
                "shake_speed_cpm": 99.0,
                "target_pressure_mpa": 99.0
            }
        }
    });
    let mut child = safety_guard()
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(stdin, "{request}").unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["clamped_targets"]["temperature_c"], 160.0);
    assert_eq!(value["clamped_targets"]["stirrer_rpm"], 1200.0);
}

fn hex_key(key: [u8; 32]) -> String {
    key.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn raw_integration_payloads(path: &Path) -> Vec<(i64, String, String)> {
    let conn = rusqlite::Connection::open(path).unwrap();
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, request_json, response_json
            FROM integration_tasks
            ORDER BY id ASC
            "#,
        )
        .unwrap();
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .map(|row| row.unwrap())
        .collect()
}

fn raw_sqlite_family_text(path: &Path) -> String {
    let mut text = String::new();
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let candidate = if suffix.is_empty() {
            path.to_path_buf()
        } else {
            let mut name = path.file_name().unwrap().to_string_lossy().into_owned();
            name.push_str(suffix);
            path.with_file_name(name)
        };
        if candidate.is_file() {
            text.push_str(&String::from_utf8_lossy(
                &std::fs::read(&candidate).unwrap(),
            ));
        }
    }
    text
}
