use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

use anyhow::{Context, Result};

use crate::control::{SafetyGuardRequest, SafetyGuardResponse};

pub fn evaluate_with_process(
    guard_executable: &Path,
    request: &SafetyGuardRequest,
) -> Result<SafetyGuardResponse> {
    let mut child = Command::new(guard_executable)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!(
                "failed to spawn safety guard process {}",
                guard_executable.display()
            )
        })?;

    {
        let stdin = child
            .stdin
            .as_mut()
            .context("safety guard stdin was not available")?;
        serde_json::to_writer(&mut *stdin, request)?;
        stdin.write_all(b"\n")?;
    }

    let output = child
        .wait_with_output()
        .context("failed to wait for safety guard process")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "safety guard process exited with {}: {}",
            output.status,
            stderr.trim()
        );
    }

    serde_json::from_slice(&output.stdout).context("failed to parse safety guard process response")
}
