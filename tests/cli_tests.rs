use std::{
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, OnceLock},
    thread,
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

fn spawn_stop_fallback_server() -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let api = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for index in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            requests.push(request);
            let (status, body) = if index == 0 {
                (
                    "HTTP/1.1 500 Internal Server Error",
                    r#"{"message":"device stop write failed after partial attempt"}"#,
                )
            } else {
                (
                    "HTTP/1.1 200 OK",
                    r#"{"data":{"auto_enabled":false},"message":"auto disabled"}"#,
                )
            };
            let response = format!(
                "{status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
        requests
    });
    (api, handle)
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

#[cfg(windows)]
fn write_active_systemctl_script(dir: &Path) -> PathBuf {
    let path = dir.join("systemctl-active.cmd");
    std::fs::write(&path, "@echo off\r\nexit /b 0\r\n").unwrap();
    path
}

#[cfg(unix)]
fn write_active_systemctl_script(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("systemctl-active.sh");
    std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

#[cfg(windows)]
fn unstartable_systemctl_path(dir: &Path) -> PathBuf {
    let path = dir.join("systemctl-dir");
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[cfg(unix)]
fn unstartable_systemctl_path(dir: &Path) -> PathBuf {
    dir.join("missing-systemctl.sh")
}

fn restore_fixture(dir: &Path) -> (PathBuf, PathBuf, Vec<u8>) {
    let source_db_path = dir.join("source.sqlite3");
    let backup_path = dir.join("reactor.sqlite3.snapshot");
    let target_db_path = dir.join("reactor.sqlite3");

    let source_db = Db::open_with_encryption_key(&source_db_path, [31_u8; 32]).unwrap();
    source_db
        .create_batch("restore source batch", 71.0, 410.0, 31.0, 43.0)
        .unwrap();
    source_db.backup_to(&backup_path).unwrap();
    drop(source_db);

    let target_conn = rusqlite::Connection::open(&target_db_path).unwrap();
    target_conn
        .execute(
            "CREATE TABLE pre_restore_marker (id INTEGER PRIMARY KEY, value TEXT NOT NULL)",
            [],
        )
        .unwrap();
    target_conn
        .execute(
            "INSERT INTO pre_restore_marker (value) VALUES ('must be replaced')",
            [],
        )
        .unwrap();
    drop(target_conn);

    let before_restore = std::fs::read(&target_db_path).unwrap();
    (backup_path, target_db_path, before_restore)
}

fn wipe_fixture(dir: &Path, name: &str) -> (PathBuf, Vec<PathBuf>) {
    let db_path = dir.join(format!("{name}.sqlite3"));
    let db = Db::open_with_encryption_key(&db_path, [33_u8; 32]).unwrap();
    let batch = db
        .create_batch("wipe fixture batch", 69.0, 390.0, 29.0, 41.0)
        .unwrap();
    db.finish_batch(batch.id).unwrap();
    drop(db);

    let wal = PathBuf::from(format!("{}-wal", db_path.display()));
    let shm = PathBuf::from(format!("{}-shm", db_path.display()));
    let key = db_path.with_extension("key");
    std::fs::write(&wal, "wal").unwrap();
    std::fs::write(&shm, "shm").unwrap();
    std::fs::write(&key, "XINGSHU_DB_ENCRYPTION_KEY=deadbeef").unwrap();

    let backup_dir = dir.join("backups");
    std::fs::create_dir_all(&backup_dir).unwrap();
    let backup = backup_dir.join(format!("{name}.snapshot"));
    std::fs::copy(&db_path, &backup).unwrap();

    (db_path, vec![wal, shm, key, backup])
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
fn xingshu_data_sample_requires_ingest_token() {
    let output = xingshu()
        .args(["data", "sample", "--count", "1"])
        .env_remove("XINGSHU_TOKEN")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("data sample requires an engineer/admin bearer token"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn xingshu_control_start_requires_explicit_control_field_for_ad_hoc_batch() {
    for args in [
        vec!["--api", "http://127.0.0.1:1", "control", "start"],
        vec![
            "--api",
            "http://127.0.0.1:1",
            "control",
            "start",
            "--name",
            "label-only",
        ],
    ] {
        let output = xingshu()
            .args(args)
            .env_remove("XINGSHU_TOKEN")
            .output()
            .unwrap();

        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("control start without --process-id must include at least one explicit target or duration flag"),
            "ad-hoc start without control intent should fail locally before contacting the API: {stderr}"
        );
    }
}

#[test]
fn xingshu_data_delete_refuses_when_daemon_service_is_active_even_if_confirmed() {
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("reactor.sqlite3");
    let db = Db::open_with_encryption_key(&db_path, [39_u8; 32]).unwrap();
    db.create_batch("data delete active source", 65.0, 350.0, 25.0, 37.0)
        .unwrap();
    drop(db);
    let before = std::fs::read(&db_path).unwrap();
    let systemctl = write_active_systemctl_script(temp_dir.path());

    let output = xingshu()
        .args([
            "--db",
            db_path.to_str().unwrap(),
            "data",
            "delete",
            "--confirm-daemon-stopped",
            "--yes",
        ])
        .env("XINGSHU_SYSTEMCTL", &systemctl)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("refusing to data delete while reactor-edge is active"),
        "data delete should reject a proven-active production service even with confirmation: {stderr}"
    );
    assert_eq!(
        std::fs::read(&db_path).unwrap(),
        before,
        "data delete must not mutate the database after daemon preflight rejection"
    );
}

#[test]
fn xingshu_data_delete_allows_recorded_confirmation_when_daemon_state_is_unverified() {
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("reactor.sqlite3");
    let db = Db::open_with_encryption_key(&db_path, [40_u8; 32]).unwrap();
    let batch = db
        .create_batch("data delete confirmed source", 64.0, 340.0, 24.0, 36.0)
        .unwrap();
    db.finish_batch(batch.id).unwrap();
    drop(db);
    let unstartable_systemctl = unstartable_systemctl_path(temp_dir.path());

    let output = xingshu()
        .args([
            "--db",
            db_path.to_str().unwrap(),
            "--json",
            "data",
            "delete",
            "--confirm-daemon-stopped",
            "--yes",
        ])
        .env("XINGSHU_SYSTEMCTL", &unstartable_systemctl)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "data delete should proceed only on explicit maintenance confirmation; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["daemon_stop_preflight"], "confirmed_unverified");
    let db = Db::open_with_encryption_key(&db_path, [40_u8; 32]).unwrap();
    assert!(
        db.batch_by_id(batch.id).unwrap().is_none(),
        "confirmed data delete should clear runtime batch rows"
    );
}

#[test]
fn xingshu_data_delete_refuses_unfinished_batch_even_when_daemon_state_is_unverified_and_confirmed()
{
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("reactor.sqlite3");
    let db = Db::open_with_encryption_key(&db_path, [41_u8; 32]).unwrap();
    let batch = db
        .create_batch("data delete unfinished source", 64.0, 340.0, 24.0, 36.0)
        .unwrap();
    drop(db);
    let before = std::fs::read(&db_path).unwrap();
    let unstartable_systemctl = unstartable_systemctl_path(temp_dir.path());

    let output = xingshu()
        .args([
            "--db",
            db_path.to_str().unwrap(),
            "data",
            "delete",
            "--confirm-daemon-stopped",
            "--yes",
        ])
        .env("XINGSHU_SYSTEMCTL", &unstartable_systemctl)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("refusing to data delete while database has unfinished batch records"),
        "data delete should reject unfinished production state even with maintenance confirmation: {stderr}"
    );
    assert_eq!(
        std::fs::read(&db_path).unwrap(),
        before,
        "data delete must not mutate DB when unfinished batch exists"
    );
    let db = Db::open_with_encryption_key(&db_path, [41_u8; 32]).unwrap();
    assert_eq!(db.batch_by_id(batch.id).unwrap().unwrap().id, batch.id);
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
fn xingshu_stop_fallback_reports_unknown_stop_result() {
    let (api, server) = spawn_stop_fallback_server();

    let output = xingshu()
        .args([
            "--json",
            "--api",
            &api,
            "stop",
            "--reason",
            "operator panel requested safe stop",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stop fallback should still disable auto; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["stop_status"], "unknown");
    assert_eq!(value["fallback"], "auto_disabled");
    assert!(
        value["stop_error"]
            .as_str()
            .unwrap()
            .contains("device stop write failed after partial attempt"),
        "fallback JSON should retain the original stop error: {value}"
    );
    assert_eq!(value["response"]["data"]["auto_enabled"], false);

    let requests = server.join().unwrap();
    assert!(
        requests
            .iter()
            .any(|request| request.lines().next().unwrap_or("")
                == "POST /api/processes/current/stop HTTP/1.1"),
        "CLI should attempt process stop first: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(|request| request.contains(r#""reason":"operator panel requested safe stop""#)),
        "CLI should send the operator stop reason for audit: {requests:?}"
    );
    assert!(
        requests.iter().any(
            |request| request.lines().next().unwrap_or("") == "POST /api/control/auto HTTP/1.1"
        ),
        "CLI should disable auto after stop failure: {requests:?}"
    );
}

#[test]
fn xingshu_ops_backup_writes_verified_hash_sidecar() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("reactor.sqlite3");
    let backup_path = temp_dir.path().join("reactor.sqlite3.snapshot");
    let db = Db::open_with_encryption_key(&db_path, [48_u8; 32]).unwrap();
    let batch = db
        .create_batch("backup sidecar source", 63.0, 330.0, 23.0, 35.0)
        .unwrap();
    db.finish_batch(batch.id).unwrap();
    drop(db);

    let output = xingshu()
        .args([
            "--json",
            "ops",
            "backup",
            "--db",
            db_path.to_str().unwrap(),
            "--out",
            backup_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "backup should succeed and write a hash sidecar; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let sidecar = PathBuf::from(value["hash_sidecar"].as_str().unwrap());
    let sha256 = value["sha256"].as_str().unwrap();
    assert!(backup_path.is_file(), "backup file should exist");
    assert!(sidecar.is_file(), "backup hash sidecar should exist");
    let sidecar_text = std::fs::read_to_string(&sidecar).unwrap();
    assert!(
        sidecar_text.contains(&format!("{sha256}  {}", backup_path.display())),
        "sidecar should contain the backup sha256 and exact path: {sidecar_text}"
    );
    assert!(
        !has_backup_hash_tmp_file(temp_dir.path()),
        "successful backup should not leave temporary hash sidecars"
    );
    assert!(
        !has_backup_snapshot_tmp_file(temp_dir.path()),
        "successful backup should not leave temporary snapshots"
    );
}

#[test]
fn xingshu_ops_backup_cleans_temp_hash_sidecar_when_publish_fails() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("reactor.sqlite3");
    let backup_path = temp_dir.path().join("reactor.sqlite3.snapshot");
    let hash_sidecar_path = temp_dir.path().join("reactor.sqlite3.snapshot.sha256");
    std::fs::create_dir(&hash_sidecar_path).unwrap();
    let db = Db::open_with_encryption_key(&db_path, [50_u8; 32]).unwrap();
    let batch = db
        .create_batch("backup sidecar failure source", 63.0, 330.0, 23.0, 35.0)
        .unwrap();
    db.finish_batch(batch.id).unwrap();
    drop(db);

    let output = xingshu()
        .args([
            "--json",
            "ops",
            "backup",
            "--db",
            db_path.to_str().unwrap(),
            "--out",
            backup_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("failed to publish backup hash sidecar")
            || stderr.contains("failed to create backup hash sidecar")
            || stderr.contains("refusing to replace backup hash sidecar directory"),
        "backup should fail when the hash sidecar cannot be atomically published: {stderr}"
    );
    assert!(
        hash_sidecar_path.is_dir(),
        "pre-existing sidecar directory should not be replaced by a partial file"
    );
    assert!(
        !has_backup_hash_tmp_file(temp_dir.path()),
        "failed backup should clean temporary hash sidecars"
    );
    assert!(
        !backup_path.exists(),
        "backup must not publish the snapshot when the sidecar cannot be published"
    );
    assert!(
        !has_backup_snapshot_tmp_file(temp_dir.path()),
        "failed backup should clean temporary snapshots"
    );
}

#[test]
fn xingshu_ops_backup_refuses_to_overwrite_existing_snapshot() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("reactor.sqlite3");
    let backup_path = temp_dir.path().join("reactor.sqlite3.snapshot");
    let existing = b"existing backup must not be overwritten";
    std::fs::write(&backup_path, existing).unwrap();
    let db = Db::open_with_encryption_key(&db_path, [51_u8; 32]).unwrap();
    let batch = db
        .create_batch("backup existing target source", 63.0, 330.0, 23.0, 35.0)
        .unwrap();
    db.finish_batch(batch.id).unwrap();
    drop(db);

    let output = xingshu()
        .args([
            "--json",
            "ops",
            "backup",
            "--db",
            db_path.to_str().unwrap(),
            "--out",
            backup_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("refusing to overwrite existing backup snapshot"),
        "backup should fail before touching an existing output path: {stderr}"
    );
    assert_eq!(
        std::fs::read(&backup_path).unwrap(),
        existing,
        "existing backup output must remain untouched"
    );
    assert!(
        !has_backup_hash_tmp_file(temp_dir.path())
            && !has_backup_snapshot_tmp_file(temp_dir.path()),
        "refused backup should not leave temporary files"
    );
}

fn has_backup_hash_tmp_file(dir: &Path) -> bool {
    std::fs::read_dir(dir).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".sha256.tmp.")
    })
}

fn has_backup_snapshot_tmp_file(dir: &Path) -> bool {
    std::fs::read_dir(dir).unwrap().any(|entry| {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        name.ends_with(".snapshot.tmp.") || name.contains(".snapshot.tmp.")
    })
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
fn xingshu_ops_restore_refuses_when_daemon_service_is_active_even_if_confirmed() {
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempfile::tempdir().unwrap();
    let (backup_path, target_db_path, before_restore) = restore_fixture(temp_dir.path());
    let systemctl = write_active_systemctl_script(temp_dir.path());

    let output = xingshu()
        .args([
            "ops",
            "restore",
            "--backup",
            backup_path.to_str().unwrap(),
            "--db",
            target_db_path.to_str().unwrap(),
            "--confirm-daemon-stopped",
            "--yes",
        ])
        .env("XINGSHU_SYSTEMCTL", &systemctl)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("refusing to restore while reactor-edge is active"),
        "restore should reject a proven-active production service even with confirmation: {stderr}"
    );
    assert_eq!(
        std::fs::read(&target_db_path).unwrap(),
        before_restore,
        "restore must not overwrite the target DB after daemon preflight rejection"
    );
}

#[test]
fn xingshu_ops_restore_allows_recorded_confirmation_when_daemon_state_is_unverified() {
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempfile::tempdir().unwrap();
    let (backup_path, target_db_path, before_restore) = restore_fixture(temp_dir.path());
    let unstartable_systemctl = unstartable_systemctl_path(temp_dir.path());

    let output = xingshu()
        .args([
            "--json",
            "ops",
            "restore",
            "--backup",
            backup_path.to_str().unwrap(),
            "--db",
            target_db_path.to_str().unwrap(),
            "--confirm-daemon-stopped",
            "--yes",
        ])
        .env("XINGSHU_SYSTEMCTL", &unstartable_systemctl)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "restore should proceed only on explicit maintenance confirmation; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["daemon_stop_preflight"], "confirmed_unverified");
    assert_eq!(value["backup_hash_sidecar"], "missing");
    assert!(
        value["preserved_sidecars"].as_array().unwrap().is_empty(),
        "restore JSON should expose preserved sidecar evidence even when none exists"
    );
    assert_ne!(
        std::fs::read(&target_db_path).unwrap(),
        before_restore,
        "confirmed restore should replace the old target DB"
    );
    assert_eq!(
        std::fs::read(&target_db_path).unwrap(),
        std::fs::read(&backup_path).unwrap(),
        "restore should copy the validated SQLite backup into place"
    );
}

#[test]
fn xingshu_ops_restore_verifies_matching_backup_hash_sidecar() {
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("source.sqlite3");
    let backup_path = temp_dir.path().join("reactor.sqlite3.snapshot");
    let target_db_path = temp_dir.path().join("reactor.sqlite3");
    let db = Db::open_with_encryption_key(&db_path, [52_u8; 32]).unwrap();
    let batch = db
        .create_batch("restore sidecar source", 70.0, 400.0, 30.0, 42.0)
        .unwrap();
    db.finish_batch(batch.id).unwrap();
    drop(db);

    let backup_output = xingshu()
        .args([
            "--json",
            "ops",
            "backup",
            "--db",
            db_path.to_str().unwrap(),
            "--out",
            backup_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        backup_output.status.success(),
        "backup stderr: {}",
        String::from_utf8_lossy(&backup_output.stderr)
    );
    std::fs::write(&target_db_path, b"old target marker").unwrap();
    let unstartable_systemctl = unstartable_systemctl_path(temp_dir.path());

    let output = xingshu()
        .args([
            "--json",
            "ops",
            "restore",
            "--backup",
            backup_path.to_str().unwrap(),
            "--db",
            target_db_path.to_str().unwrap(),
            "--confirm-daemon-stopped",
            "--yes",
        ])
        .env("XINGSHU_SYSTEMCTL", &unstartable_systemctl)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "restore should accept a matching backup sidecar; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["backup_hash_sidecar"], "verified");
    assert_eq!(
        std::fs::read(&target_db_path).unwrap(),
        std::fs::read(&backup_path).unwrap(),
        "restore should copy the sidecar-verified backup into place"
    );
}

#[test]
fn xingshu_ops_restore_refuses_mismatched_backup_hash_sidecar_before_overwrite() {
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempfile::tempdir().unwrap();
    let (backup_path, target_db_path, before_restore) = restore_fixture(temp_dir.path());
    let hash_sidecar = backup_path.with_extension("snapshot.sha256");
    std::fs::write(
        &hash_sidecar,
        format!(
            "{}  {}\n",
            "0".repeat(64),
            backup_path.file_name().unwrap().to_string_lossy()
        ),
    )
    .unwrap();
    let unstartable_systemctl = unstartable_systemctl_path(temp_dir.path());

    let output = xingshu()
        .args([
            "ops",
            "restore",
            "--backup",
            backup_path.to_str().unwrap(),
            "--db",
            target_db_path.to_str().unwrap(),
            "--confirm-daemon-stopped",
            "--yes",
        ])
        .env("XINGSHU_SYSTEMCTL", &unstartable_systemctl)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("backup hash sidecar") && stderr.contains("does not match"),
        "restore should reject a mismatched backup sidecar: {stderr}"
    );
    assert_eq!(
        std::fs::read(&target_db_path).unwrap(),
        before_restore,
        "restore must not overwrite the target DB after sidecar mismatch"
    );
}

#[test]
fn xingshu_ops_restore_refuses_to_overwrite_target_with_unfinished_batch_even_when_confirmed() {
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempfile::tempdir().unwrap();
    let source_db_path = temp_dir.path().join("source.sqlite3");
    let backup_path = temp_dir.path().join("reactor.sqlite3.snapshot");
    let source_db = Db::open_with_encryption_key(&source_db_path, [42_u8; 32]).unwrap();
    let source_batch = source_db
        .create_batch("restore replacement source", 70.0, 400.0, 30.0, 42.0)
        .unwrap();
    source_db.finish_batch(source_batch.id).unwrap();
    source_db.backup_to(&backup_path).unwrap();
    drop(source_db);

    let target_db_path = temp_dir.path().join("reactor.sqlite3");
    let target_db = Db::open_with_encryption_key(&target_db_path, [43_u8; 32]).unwrap();
    let unfinished = target_db
        .create_batch("restore target unfinished", 71.0, 410.0, 31.0, 43.0)
        .unwrap();
    drop(target_db);
    let before_restore = std::fs::read(&target_db_path).unwrap();
    let unstartable_systemctl = unstartable_systemctl_path(temp_dir.path());

    let output = xingshu()
        .args([
            "ops",
            "restore",
            "--backup",
            backup_path.to_str().unwrap(),
            "--db",
            target_db_path.to_str().unwrap(),
            "--confirm-daemon-stopped",
            "--yes",
        ])
        .env("XINGSHU_SYSTEMCTL", &unstartable_systemctl)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("refusing to restore over database with unfinished batch records"),
        "restore should reject replacing unfinished production evidence even with maintenance confirmation: {stderr}"
    );
    assert_eq!(
        std::fs::read(&target_db_path).unwrap(),
        before_restore,
        "restore must not overwrite a target DB with unfinished production state"
    );
    let target_db = Db::open_with_encryption_key(&target_db_path, [43_u8; 32]).unwrap();
    assert_eq!(
        target_db.batch_by_id(unfinished.id).unwrap().unwrap().id,
        unfinished.id
    );
}

#[test]
fn xingshu_ops_restore_refuses_production_path_when_daemon_state_is_unverified() {
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempfile::tempdir().unwrap();
    let backup_path = temp_dir.path().join("reactor.sqlite3.snapshot");
    let source_db =
        Db::open_with_encryption_key(temp_dir.path().join("source.sqlite3"), [32_u8; 32]).unwrap();
    source_db
        .create_batch("restore production path source", 70.0, 400.0, 30.0, 42.0)
        .unwrap();
    source_db.backup_to(&backup_path).unwrap();
    drop(source_db);
    let unstartable_systemctl = unstartable_systemctl_path(temp_dir.path());

    let output = xingshu()
        .args([
            "ops",
            "restore",
            "--backup",
            backup_path.to_str().unwrap(),
            "--db",
            "C:\\var\\lib\\reactor-edge\\reactor.sqlite3",
            "--yes",
        ])
        .env("XINGSHU_SYSTEMCTL", &unstartable_systemctl)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot verify daemon service state before restore of production database"),
        "restore should reject an unverified production DB path without maintenance confirmation: {stderr}"
    );
}

#[test]
fn xingshu_ops_wipe_refuses_when_daemon_service_is_active_even_if_confirmed() {
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempfile::tempdir().unwrap();
    let (db_path, scoped_files) = wipe_fixture(temp_dir.path(), "reactor");
    let systemctl = write_active_systemctl_script(temp_dir.path());

    let output = xingshu()
        .args([
            "ops",
            "wipe",
            "--db",
            db_path.to_str().unwrap(),
            "--confirm-daemon-stopped",
            "--yes",
        ])
        .env("XINGSHU_SYSTEMCTL", &systemctl)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("refusing to wipe while reactor-edge is active"),
        "wipe should reject a proven-active production service even with confirmation: {stderr}"
    );
    assert!(
        db_path.exists(),
        "wipe must not remove the target DB after daemon preflight rejection"
    );
    for path in scoped_files {
        assert!(
            path.exists(),
            "wipe must not remove scoped file after daemon preflight rejection: {}",
            path.display()
        );
    }
}

#[test]
fn xingshu_ops_wipe_refuses_unfinished_batch_even_when_daemon_state_is_unverified_and_confirmed() {
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("reactor.sqlite3");
    let db = Db::open_with_encryption_key(&db_path, [44_u8; 32]).unwrap();
    let batch = db
        .create_batch("wipe unfinished source", 68.0, 380.0, 28.0, 40.0)
        .unwrap();
    drop(db);
    let before = std::fs::read(&db_path).unwrap();
    let unstartable_systemctl = unstartable_systemctl_path(temp_dir.path());

    let output = xingshu()
        .args([
            "ops",
            "wipe",
            "--db",
            db_path.to_str().unwrap(),
            "--confirm-daemon-stopped",
            "--yes",
        ])
        .env("XINGSHU_SYSTEMCTL", &unstartable_systemctl)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("refusing to wipe while database has unfinished batch records"),
        "wipe should reject unfinished production state even with maintenance confirmation: {stderr}"
    );
    assert_eq!(
        std::fs::read(&db_path).unwrap(),
        before,
        "wipe must not mutate DB when unfinished batch exists"
    );
    let db = Db::open_with_encryption_key(&db_path, [44_u8; 32]).unwrap();
    assert_eq!(db.batch_by_id(batch.id).unwrap().unwrap().id, batch.id);
}

#[test]
fn xingshu_ops_wipe_allows_recorded_confirmation_when_daemon_state_is_unverified() {
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempfile::tempdir().unwrap();
    let (db_path, scoped_files) = wipe_fixture(temp_dir.path(), "reactor");
    let unstartable_systemctl = unstartable_systemctl_path(temp_dir.path());

    let output = xingshu()
        .args([
            "--json",
            "ops",
            "wipe",
            "--db",
            db_path.to_str().unwrap(),
            "--confirm-daemon-stopped",
            "--yes",
        ])
        .env("XINGSHU_SYSTEMCTL", &unstartable_systemctl)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "wipe should proceed only on explicit maintenance confirmation; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["daemon_stop_preflight"], "confirmed_unverified");
    assert!(!db_path.exists(), "confirmed wipe should remove target DB");
    for path in scoped_files {
        assert!(
            !path.exists(),
            "confirmed wipe should remove scoped file: {}",
            path.display()
        );
    }
}

#[test]
fn xingshu_ops_wipe_refuses_production_path_when_daemon_state_is_unverified() {
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempfile::tempdir().unwrap();
    let production_like_dir = temp_dir.path().join("var/lib/reactor-edge");
    std::fs::create_dir_all(&production_like_dir).unwrap();
    let db_path = production_like_dir.join("reactor.sqlite3");
    let db = Db::open_with_encryption_key(&db_path, [34_u8; 32]).unwrap();
    db.create_batch("wipe production path source", 68.0, 380.0, 28.0, 40.0)
        .unwrap();
    drop(db);
    let unstartable_systemctl = unstartable_systemctl_path(temp_dir.path());

    let output = xingshu()
        .args(["ops", "wipe", "--db", db_path.to_str().unwrap(), "--yes"])
        .env("XINGSHU_SYSTEMCTL", &unstartable_systemctl)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot verify daemon service state before wipe of production database"),
        "wipe should reject an unverified production DB path without maintenance confirmation: {stderr}"
    );
    assert!(
        db_path.exists(),
        "wipe must not remove the target DB after unverified production-path rejection"
    );
}

#[test]
fn xingshu_key_generate_refuses_when_daemon_service_is_active_even_if_confirmed() {
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("reactor.sqlite3");
    let db = Db::open_with_encryption_key(&db_path, [35_u8; 32]).unwrap();
    db.create_batch("key generate active source", 67.0, 370.0, 27.0, 39.0)
        .unwrap();
    drop(db);
    let key_path = db_path.with_extension("key");
    let systemctl = write_active_systemctl_script(temp_dir.path());

    let output = xingshu()
        .args([
            "key",
            "generate",
            "--db",
            db_path.to_str().unwrap(),
            "--confirm-daemon-stopped",
            "--yes",
        ])
        .env("XINGSHU_SYSTEMCTL", &systemctl)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("refusing to key generate while reactor-edge is active"),
        "key generate should reject a proven-active production service even with confirmation: {stderr}"
    );
    assert!(
        !key_path.exists(),
        "key generate must not write a replacement key after daemon preflight rejection"
    );
}

#[test]
fn xingshu_key_generate_allows_recorded_confirmation_when_daemon_state_is_unverified() {
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("reactor.sqlite3");
    let db = Db::open_with_encryption_key(&db_path, [36_u8; 32]).unwrap();
    let batch = db
        .create_batch("key generate confirmed source", 66.0, 360.0, 26.0, 38.0)
        .unwrap();
    db.finish_batch(batch.id).unwrap();
    drop(db);
    let key_path = db_path.with_extension("key");
    let unstartable_systemctl = unstartable_systemctl_path(temp_dir.path());

    let output = xingshu()
        .args([
            "--json",
            "key",
            "generate",
            "--db",
            db_path.to_str().unwrap(),
            "--confirm-daemon-stopped",
            "--yes",
        ])
        .env("XINGSHU_SYSTEMCTL", &unstartable_systemctl)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "key generate should proceed only on explicit maintenance confirmation; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["daemon_stop_preflight"], "confirmed_unverified");
    assert!(
        key_path.is_file(),
        "confirmed key generate should write the key file"
    );
    assert!(
        !stdout.contains(&std::fs::read_to_string(&key_path).unwrap()),
        "key generate JSON must not print key file material"
    );
}

#[test]
fn xingshu_key_generate_refuses_unfinished_batch_even_when_daemon_state_is_unverified_and_confirmed(
) {
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("reactor.sqlite3");
    let db = Db::open_with_encryption_key(&db_path, [45_u8; 32]).unwrap();
    let batch = db
        .create_batch("key generate unfinished source", 66.0, 360.0, 26.0, 38.0)
        .unwrap();
    drop(db);
    let key_path = db_path.with_extension("key");
    let before = std::fs::read(&db_path).unwrap();
    let unstartable_systemctl = unstartable_systemctl_path(temp_dir.path());

    let output = xingshu()
        .args([
            "key",
            "generate",
            "--db",
            db_path.to_str().unwrap(),
            "--confirm-daemon-stopped",
            "--yes",
        ])
        .env("XINGSHU_SYSTEMCTL", &unstartable_systemctl)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("refusing to key generate while database has unfinished batch records"),
        "key generate should reject unfinished production state even with maintenance confirmation: {stderr}"
    );
    assert!(
        !key_path.exists(),
        "key generate must not write a key file when unfinished batch exists"
    );
    assert_eq!(
        std::fs::read(&db_path).unwrap(),
        before,
        "key generate must not mutate DB when unfinished batch exists"
    );
    let db = Db::open_with_encryption_key(&db_path, [45_u8; 32]).unwrap();
    assert_eq!(db.batch_by_id(batch.id).unwrap().unwrap().id, batch.id);
}

#[test]
fn xingshu_key_generate_refuses_existing_encrypted_integration_payloads() {
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("reactor.sqlite3");
    let old_key = [49_u8; 32];
    let old_key_hex = hex_key(old_key);
    let db = Db::open_with_encryption_key(&db_path, old_key).unwrap();
    let batch = db
        .create_batch("key generate encrypted source", 66.0, 360.0, 26.0, 38.0)
        .unwrap();
    db.finish_batch(batch.id).unwrap();
    db.create_integration_task(
        "ainas",
        Some("key-generate-encrypted-001"),
        "set_targets",
        &serde_json::json!({ "reason": "must remain readable after key rotation" }),
    )
    .unwrap();
    drop(db);
    let before = raw_integration_payloads(&db_path);
    let key_path = db_path.with_extension("key");
    let unstartable_systemctl = unstartable_systemctl_path(temp_dir.path());

    let output = xingshu()
        .args([
            "key",
            "generate",
            "--db",
            db_path.to_str().unwrap(),
            "--confirm-daemon-stopped",
            "--yes",
        ])
        .env("XINGSHU_DB_ENCRYPTION_KEY", &old_key_hex)
        .env("XINGSHU_SYSTEMCTL", &unstartable_systemctl)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("refusing to key generate while database already contains"),
        "key generate should refuse to strand existing encrypted integration tasks: {stderr}"
    );
    assert!(
        !key_path.exists(),
        "key generate must not write a replacement key when encrypted rows require rekey"
    );
    assert_eq!(
        raw_integration_payloads(&db_path),
        before,
        "key generate must not mutate encrypted integration payloads"
    );
}

#[test]
fn xingshu_key_rekey_refuses_commit_when_daemon_service_is_active_even_if_confirmed() {
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("reactor.sqlite3");
    let old_key = [37_u8; 32];
    let new_key = [38_u8; 32];
    let old_key_hex = hex_key(old_key);
    let new_key_path = temp_dir.path().join("new-db.key");
    std::fs::write(
        &new_key_path,
        format!("XINGSHU_DB_ENCRYPTION_KEY={}\n", hex_key(new_key)),
    )
    .unwrap();
    let db = Db::open_with_encryption_key(&db_path, old_key).unwrap();
    db.create_integration_task(
        "ainas",
        Some("rekey-active-001"),
        "set_targets",
        &serde_json::json!({ "reason": "must stay old-key encrypted" }),
    )
    .unwrap();
    drop(db);
    let before = raw_integration_payloads(&db_path);
    let systemctl = write_active_systemctl_script(temp_dir.path());

    let output = xingshu()
        .args([
            "key",
            "rekey-integration-tasks",
            "--db",
            db_path.to_str().unwrap(),
            "--new-key-file",
            new_key_path.to_str().unwrap(),
            "--confirm-daemon-stopped",
            "--yes",
        ])
        .env("XINGSHU_DB_ENCRYPTION_KEY", &old_key_hex)
        .env("XINGSHU_SYSTEMCTL", &systemctl)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("refusing to key rekey while reactor-edge is active"),
        "key rekey should reject a proven-active production service even with confirmation: {stderr}"
    );
    assert_eq!(
        raw_integration_payloads(&db_path),
        before,
        "key rekey must not mutate payloads after daemon preflight rejection"
    );
}

#[test]
fn xingshu_key_rekey_refuses_commit_with_unfinished_batch_even_when_confirmed() {
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("reactor.sqlite3");
    let old_key = [46_u8; 32];
    let new_key = [47_u8; 32];
    let old_key_hex = hex_key(old_key);
    let new_key_path = temp_dir.path().join("new-db.key");
    std::fs::write(
        &new_key_path,
        format!("XINGSHU_DB_ENCRYPTION_KEY={}\n", hex_key(new_key)),
    )
    .unwrap();
    let db = Db::open_with_encryption_key(&db_path, old_key).unwrap();
    let batch = db
        .create_batch("key rekey unfinished source", 67.0, 370.0, 27.0, 39.0)
        .unwrap();
    db.create_integration_task(
        "ainas",
        Some("rekey-unfinished-001"),
        "set_targets",
        &serde_json::json!({ "reason": "must stay old-key encrypted" }),
    )
    .unwrap();
    drop(db);
    let before = raw_integration_payloads(&db_path);
    let unstartable_systemctl = unstartable_systemctl_path(temp_dir.path());

    let output = xingshu()
        .args([
            "key",
            "rekey-integration-tasks",
            "--db",
            db_path.to_str().unwrap(),
            "--new-key-file",
            new_key_path.to_str().unwrap(),
            "--confirm-daemon-stopped",
            "--yes",
        ])
        .env("XINGSHU_DB_ENCRYPTION_KEY", &old_key_hex)
        .env("XINGSHU_SYSTEMCTL", &unstartable_systemctl)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("refusing to key rekey while database has unfinished batch records"),
        "key rekey should reject unfinished production state even with maintenance confirmation: {stderr}"
    );
    assert_eq!(
        raw_integration_payloads(&db_path),
        before,
        "key rekey must not mutate payloads when unfinished batch exists"
    );
    let db = Db::open_with_encryption_key(&db_path, old_key).unwrap();
    assert_eq!(db.batch_by_id(batch.id).unwrap().unwrap().id, batch.id);
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
    db.finish_batch(batch.id).unwrap();
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
    db.finish_batch(batch.id).unwrap();
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
fn xingshu_ai_train_promote_refuses_when_daemon_service_is_active_even_if_confirmed() {
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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
        .create_batch("cli lora promote active service", 73.0, 430.0, 32.0, 44.0)
        .unwrap();
    db.finish_batch(batch.id).unwrap();
    db.insert_product_result(&ProductResult {
        batch_id: batch.id,
        yield_percent: 90.0,
        product_ratio: 0.94,
        notes: "training promotion active service".to_string(),
    })
    .unwrap();
    drop(db);
    let systemctl = write_active_systemctl_script(temp_dir.path());

    let output = xingshu()
        .args([
            "--db",
            db_path.to_str().unwrap(),
            "ai",
            "train",
            "--dataset",
            dataset_path.to_str().unwrap(),
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--promote",
            "--confirm-daemon-stopped",
            "--min-eval-score",
            "0.8",
            "--timeout-s",
            "10",
        ])
        .env("XINGSHU_SYSTEMCTL", &systemctl)
        .env("XINGSHU_LOCAL_AI_ENABLED", "true")
        .env("XINGSHU_LOCAL_AI_GGUF", &model_path)
        .env("XINGSHU_LOCAL_AI_LORA", &current_adapter)
        .env("XINGSHU_LOCAL_AI_TRAIN_SCRIPT", &train_script)
        .env("XINGSHU_LOCAL_AI_CONVERT_SCRIPT", &convert_path)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("refusing to promote local AI adapter while reactor-edge is active"),
        "promotion should reject a proven-active production service even with confirmation: {stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(&current_adapter).unwrap(),
        "old adapter"
    );
    assert!(
        !dataset_path.exists(),
        "promotion preflight should fail before exporting a dataset"
    );
    assert!(
        !manifest_path.exists(),
        "promotion preflight should fail before writing a manifest"
    );
    assert!(
        std::fs::read_dir(temp_dir.path()).unwrap().all(|entry| {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            !name.starts_with("active-adapter.gguf.pre-promote-")
        }),
        "promotion preflight must not create an adapter backup"
    );
}

#[test]
fn xingshu_ai_train_promote_refuses_unfinished_batch_even_when_daemon_state_is_unverified_and_confirmed(
) {
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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
    let finished = db
        .create_batch("cli lora promote finished source", 73.0, 430.0, 32.0, 44.0)
        .unwrap();
    db.finish_batch(finished.id).unwrap();
    db.insert_product_result(&ProductResult {
        batch_id: finished.id,
        yield_percent: 90.0,
        product_ratio: 0.94,
        notes: "training promotion finished source".to_string(),
    })
    .unwrap();
    let unfinished = db
        .create_batch(
            "cli lora promote unfinished source",
            74.0,
            440.0,
            33.0,
            45.0,
        )
        .unwrap();
    drop(db);
    let unstartable_systemctl = unstartable_systemctl_path(temp_dir.path());

    let output = xingshu()
        .args([
            "--db",
            db_path.to_str().unwrap(),
            "ai",
            "train",
            "--dataset",
            dataset_path.to_str().unwrap(),
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--promote",
            "--confirm-daemon-stopped",
            "--min-eval-score",
            "0.8",
            "--timeout-s",
            "10",
        ])
        .env("XINGSHU_SYSTEMCTL", &unstartable_systemctl)
        .env("XINGSHU_LOCAL_AI_ENABLED", "true")
        .env("XINGSHU_LOCAL_AI_GGUF", &model_path)
        .env("XINGSHU_LOCAL_AI_LORA", &current_adapter)
        .env("XINGSHU_LOCAL_AI_TRAIN_SCRIPT", &train_script)
        .env("XINGSHU_LOCAL_AI_CONVERT_SCRIPT", &convert_path)
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(
            "refusing to promote local AI adapter while database has unfinished batch records"
        ),
        "promotion should reject unfinished production state even with maintenance confirmation: {stderr}"
    );
    assert_eq!(
        std::fs::read_to_string(&current_adapter).unwrap(),
        "old adapter"
    );
    assert!(
        !dataset_path.exists(),
        "promotion preflight should fail before exporting a dataset"
    );
    assert!(
        !manifest_path.exists(),
        "promotion preflight should fail before writing a manifest"
    );
    let db = Db::open(&db_path).unwrap();
    assert_eq!(
        db.batch_by_id(unfinished.id).unwrap().unwrap().id,
        unfinished.id
    );
}

#[test]
fn xingshu_ai_train_promotes_passing_candidate_adapter_with_backup() {
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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
    db.finish_batch(batch.id).unwrap();
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
            "--confirm-daemon-stopped",
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
        .env(
            "XINGSHU_SYSTEMCTL",
            unstartable_systemctl_path(temp_dir.path()),
        )
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["daemon_stop_preflight"], "confirmed_unverified");
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
            "--confirm-daemon-stopped",
            "--yes",
        ])
        .env("XINGSHU_DB_ENCRYPTION_KEY", &old_key_hex)
        .env(
            "XINGSHU_SYSTEMCTL",
            unstartable_systemctl_path(temp_dir.path()),
        )
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
    assert_eq!(
        committed_value["daemon_stop_preflight"],
        "confirmed_unverified"
    );
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
fn daemon_rejects_test_reset_on_non_loopback_bind() {
    let output = daemon()
        .args(["--enable-test-reset", "--bind", "0.0.0.0:0"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("--enable-test-reset may only be used with a loopback bind address"),
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
    let _guard = windows_subprocess_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
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
    // evaluate_with_process should return in tens of milliseconds, but the
    // assertion only needs to prove the timeout path ran instead of waiting
    // for the script's natural exit. The 8s upper bound keeps a strong
    // margin below the 15s sleep (7s of slack) while tolerating scheduler
    // jitter when this runs alongside the full parallel suite and a busy
    // build host. If the slow script ever completes before this bound, the
    // timeout is being ignored.
    assert!(
        started_at.elapsed() < Duration::from_secs(8),
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
