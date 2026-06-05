use std::{
    io::{Read, Write},
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{Context, Result};
use wait_timeout::ChildExt;

use crate::control::{SafetyGuardRequest, SafetyGuardResponse};

pub fn evaluate_with_process(
    guard_executable: &Path,
    request: &SafetyGuardRequest,
    timeout: Duration,
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
        let mut stdin = child
            .stdin
            .take()
            .context("safety guard stdin was not available")?;
        serde_json::to_writer(&mut stdin, request)?;
        stdin.write_all(b"\n")?;
    }

    let mut stdout = child
        .stdout
        .take()
        .context("safety guard stdout was not available")?;
    let mut stderr = child
        .stderr
        .take()
        .context("safety guard stderr was not available")?;

    let status = match child
        .wait_timeout(timeout)
        .context("failed while waiting for safety guard process")?
    {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!(
                "safety guard process exceeded timeout of {}ms",
                timeout.as_millis()
            );
        }
    };

    let mut stdout_bytes = Vec::new();
    stdout
        .read_to_end(&mut stdout_bytes)
        .context("failed to read safety guard stdout")?;
    let mut stderr_bytes = Vec::new();
    stderr
        .read_to_end(&mut stderr_bytes)
        .context("failed to read safety guard stderr")?;

    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr_bytes);
        anyhow::bail!(
            "safety guard process exited with {}: {}",
            status,
            stderr.trim()
        );
    }

    serde_json::from_slice(&stdout_bytes).context("failed to parse safety guard process response")
}
