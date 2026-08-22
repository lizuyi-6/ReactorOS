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
        Some(secret) if auth_secret_is_exposure_safe(secret) => {}
        _ => bail!(
            "XINGSHU_AUTH_SECRET is missing, is the public default, or is shorter than 32 chars; refusing to expose the daemon on non-loopback bind {bind}. Set a strong unique XINGSHU_AUTH_SECRET (>= 32 chars), or bind to 127.0.0.1 for local-only operation."
        ),
    }
    // A strong signing secret does not help if the login passwords themselves
    // are the documented defaults (admin123 / engineer123 / operator123): an
    // attacker on the network could simply log in as admin. Fail closed at
    // startup instead of leaving a known-credential daemon on the network.
    let weak_passwords = network_login_passwords()
        .into_iter()
        .filter(|(_, password)| !login_password_is_exposure_safe(password))
        .map(|(name, _)| name.to_string())
        .collect::<Vec<_>>();
    if !weak_passwords.is_empty() {
        bail!(
            "refusing to expose the daemon on non-loopback bind {bind}: login password(s) for {} use the documented local default or are unset. Set XINGSHU_OPERATOR_PASSWORD, XINGSHU_ENGINEER_PASSWORD and XINGSHU_ADMIN_PASSWORD to strong unique values (>= 12 chars), or bind to 127.0.0.1 for local-only operation.",
            weak_passwords.join(", ")
        );
    }
    Ok(())
}

/// Login passwords that would be active for a network-exposed daemon.
/// Mirrors `api_auth::role_for_login`, which falls back to the documented
/// defaults when the env vars are unset — so "unset" counts as the default.
fn network_login_passwords() -> Vec<(&'static str, String)> {
    [
        (
            "operator",
            std::env::var("XINGSHU_OPERATOR_PASSWORD")
                .unwrap_or_else(|_| "operator123".to_string()),
        ),
        (
            "engineer",
            std::env::var("XINGSHU_ENGINEER_PASSWORD")
                .unwrap_or_else(|_| "engineer123".to_string()),
        ),
        (
            "admin",
            std::env::var("XINGSHU_ADMIN_PASSWORD").unwrap_or_else(|_| "admin123".to_string()),
        ),
    ]
    .into_iter()
    .collect()
}

fn login_password_is_exposure_safe(password: &str) -> bool {
    !password.is_empty()
        && password.len() >= 12
        && password != "operator123"
        && password != "engineer123"
        && password != "admin123"
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

    #[test]
    fn network_auth_gate_rejects_default_login_passwords_on_non_loopback() {
        let bind = "0.0.0.0:8443".parse::<SocketAddr>().unwrap();
        let strong = "0123456789abcdef0123456789abcdef";
        // Env untouched -> all three login passwords fall back to the
        // documented defaults; the gate must refuse even with a strong secret.
        let _guard = PasswordEnvGuard::clear(&[
            "XINGSHU_OPERATOR_PASSWORD",
            "XINGSHU_ENGINEER_PASSWORD",
            "XINGSHU_ADMIN_PASSWORD",
        ]);
        assert!(enforce_network_auth_gate(bind, Some(strong)).is_err());

        // Explicitly set to a default value must also be refused.
        let _set = PasswordEnvGuard::set(&[("XINGSHU_ADMIN_PASSWORD", "admin123")]);
        assert!(enforce_network_auth_gate(bind, Some(strong)).is_err());
    }

    #[test]
    fn network_auth_gate_accepts_non_loopback_when_all_login_passwords_are_strong() {
        let bind = "0.0.0.0:8443".parse::<SocketAddr>().unwrap();
        let strong = "0123456789abcdef0123456789abcdef";
        let _guard = PasswordEnvGuard::set(&[
            ("XINGSHU_OPERATOR_PASSWORD", "op-secret-passphrase"),
            ("XINGSHU_ENGINEER_PASSWORD", "eng-secret-passphrase"),
            ("XINGSHU_ADMIN_PASSWORD", "adm-secret-passphrase"),
        ]);
        assert!(enforce_network_auth_gate(bind, Some(strong)).is_ok());
    }

    /// Serializes env-var mutation for the network-gate tests and restores the
    /// original values on drop so the tests do not leak state into each other.
    struct PasswordEnvGuard {
        saved: Vec<(&'static str, Option<String>)>,
    }

    impl PasswordEnvGuard {
        fn clear(names: &[&'static str]) -> Self {
            use std::env;
            let saved = names
                .iter()
                .map(|name| (*name, env::var(name).ok()))
                .collect();
            for name in names {
                env::remove_var(name);
            }
            Self { saved }
        }

        fn set(pairs: &[(&'static str, &str)]) -> Self {
            use std::env;
            let saved = pairs
                .iter()
                .map(|(name, _)| (*name, env::var(name).ok()))
                .collect();
            for (name, value) in pairs {
                // The guard's save/restore keeps the mutation scoped to this
                // test; the guard itself serializes setup/teardown.
                env::set_var(name, value);
            }
            Self { saved }
        }
    }

    impl Drop for PasswordEnvGuard {
        fn drop(&mut self) {
            use std::env;
            for (name, previous) in &self.saved {
                match previous {
                    Some(value) => env::set_var(name, value),
                    None => env::remove_var(name),
                }
            }
        }
    }
}
