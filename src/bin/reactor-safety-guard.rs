use std::io::{self, Read};

use anyhow::{Context, Result};
use reactor_edge_daemon::control::{evaluate_safety_request, SafetyGuardRequest};

fn main() -> Result<()> {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .context("failed to read safety guard request from stdin")?;
    let request: SafetyGuardRequest =
        serde_json::from_str(&input).context("failed to parse safety guard request JSON")?;
    let response = evaluate_safety_request(request);
    println!("{}", serde_json::to_string(&response)?);
    Ok(())
}
