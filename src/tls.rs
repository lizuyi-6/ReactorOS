use std::{
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

pub fn install_rustls_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

pub fn load_cert_chain(path: impl AsRef<Path>) -> Result<Vec<CertificateDer<'static>>> {
    let path = path.as_ref();
    let mut reader = BufReader::new(
        File::open(path)
            .with_context(|| format!("failed to open certificate {}", path.display()))?,
    );
    let certs = rustls_pemfile::certs(&mut reader)
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("failed to parse certificate {}", path.display()))?;
    if certs.is_empty() {
        bail!("no certificate found in {}", path.display());
    }
    Ok(certs)
}

pub fn load_private_key(path: impl AsRef<Path>) -> Result<PrivateKeyDer<'static>> {
    let path = path.as_ref();
    let mut reader = BufReader::new(
        File::open(path)
            .with_context(|| format!("failed to open private key {}", path.display()))?,
    );
    rustls_pemfile::private_key(&mut reader)
        .with_context(|| format!("failed to parse private key {}", path.display()))?
        .ok_or_else(|| anyhow::anyhow!("no private key found in {}", path.display()))
}

pub fn paired_paths(
    cert: &Option<PathBuf>,
    key: &Option<PathBuf>,
    label: &str,
) -> Result<Option<(PathBuf, PathBuf)>> {
    match (cert, key) {
        (Some(cert), Some(key)) => Ok(Some((cert.clone(), key.clone()))),
        (None, None) => Ok(None),
        _ => bail!("{label} certificate and key must be provided together"),
    }
}
