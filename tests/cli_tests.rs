use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use reactor_edge_daemon::{
    config::load_safety_config, control::SafetyGuardRequest, safety_guard::evaluate_with_process,
    state::ControlTargets,
};

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
    std::fs::write(&path, "@echo off\r\nping -n 6 127.0.0.1 >NUL\r\n").unwrap();
    path
}

#[cfg(unix)]
fn write_slow_guard_script(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = dir.join("slow-guard.sh");
    std::fs::write(&path, "#!/bin/sh\nsleep 5\n").unwrap();
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
fn xingshu_ai_train_reports_current_lora_gap() {
    let output = xingshu()
        .args(["ai", "train"])
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
        stderr.contains("local LoRA training is not exposed"),
        "unexpected stderr: {stderr}"
    );
    assert!(
        stderr.contains("XINGSHU_LOCAL_AI_GGUF"),
        "stderr should list missing GGUF model env: {stderr}"
    );
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
    assert!(
        started_at.elapsed() < Duration::from_secs(3),
        "timeout should return before the slow guard script finishes"
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
