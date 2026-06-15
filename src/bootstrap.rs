use std::{net::SocketAddr, path::PathBuf};

use anyhow::{bail, Result};

pub fn resolve_assets_dir(requested: &PathBuf) -> PathBuf {
    let requested_str = requested.to_string_lossy();
    if requested_str != "auto" {
        return requested.clone();
    }
    let candidates = [PathBuf::from("frontend/dist"), PathBuf::from("static")];
    for candidate in candidates.iter() {
        if candidate.join("index.html").is_file() {
            return candidate.clone();
        }
    }
    PathBuf::from("static")
}

pub fn enforce_network_auth_gate(bind: SocketAddr, secret: Option<&str>) -> Result<()> {
    if bind.ip().is_loopback() {
        return Ok(());
    }
    match secret.map(str::trim) {
        Some(secret) if auth_secret_is_exposure_safe(secret) => Ok(()),
        _ => bail!(
            "XINGSHU_AUTH_SECRET is missing, is the public default, or is shorter than 32 chars; refusing to expose the daemon on non-loopback bind {bind}. Set a strong unique XINGSHU_AUTH_SECRET (>= 32 chars), or bind to 127.0.0.1 for local-only operation."
        ),
    }
}

fn auth_secret_is_exposure_safe(secret: &str) -> bool {
    !secret.is_empty() && secret != "xingshu-local-rbac-session-secret" && secret.len() >= 32
}

#[cfg(test)]
mod tests {
    use super::enforce_network_auth_gate;
    use std::net::SocketAddr;

    #[test]
    fn network_auth_gate_allows_loopback_regardless_of_secret() {
        let bind = "127.0.0.1:8000".parse::<SocketAddr>().unwrap();
        assert!(enforce_network_auth_gate(bind, None).is_ok());
        assert!(
            enforce_network_auth_gate(bind, Some("xingshu-local-rbac-session-secret")).is_ok(),
            "loopback binds must remain usable with the default secret for local dev and tests"
        );
    }

    #[test]
    fn network_auth_gate_rejects_non_loopback_without_strong_secret() {
        let bind = "0.0.0.0:8443".parse::<SocketAddr>().unwrap();
        assert!(enforce_network_auth_gate(bind, None).is_err());
        assert!(
            enforce_network_auth_gate(bind, Some("xingshu-local-rbac-session-secret")).is_err(),
            "the public default secret must not be accepted on a network-exposed bind"
        );
        assert!(enforce_network_auth_gate(bind, Some("short")).is_err());
    }

    #[test]
    fn network_auth_gate_accepts_non_loopback_with_strong_secret() {
        let bind = "0.0.0.0:8443".parse::<SocketAddr>().unwrap();
        let strong = "0123456789abcdef0123456789abcdef";
        assert_eq!(strong.len(), 32);
        assert!(enforce_network_auth_gate(bind, Some(strong)).is_ok());
    }
}
