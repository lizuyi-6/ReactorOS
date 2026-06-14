# Regenerates the self-signed TLS certificate used by the modbus_tcp TLS test
# (tests/api_tests.rs -> tests/fixtures/tls/server.{crt,key}). The original
# fixture shipped with a ~7-day validity and silently expired, turning the test
# red with InvalidCertificate(ExpiredContext). Re-run whenever the cert is near
# expiry; it produces a 10-year cert so this does not recur.
#
# Key/cert are generated as a matched pair — always regenerate BOTH, never just
# one, or load_single_cert will fail on a key/cert mismatch.
#
# rustls rejects a cert with basicConstraints CA:TRUE as an end-entity cert
# (InvalidCertificate CaUsedAsEndEntity), and openssl's default config inherits
# CA:TRUE unless overridden — so basicConstraints CA:FALSE is set explicitly.
#
# Usage: scripts\regen-test-tls-cert.ps1

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$certDir = Join-Path $repoRoot 'tests\fixtures\tls'
$crt = Join-Path $certDir 'server.crt'
$key = Join-Path $certDir 'server.key'

if (-not (Get-Command openssl -ErrorAction SilentlyContinue)) {
    throw "openssl not found on PATH. Install it (e.g. via git's mingw or a system openssl) and retry."
}

New-Item -ItemType Directory -Force -Path $certDir | Out-Null

# MSYS/Git-Bash mangles -subj paths like /C=CN/... into C:/Program Files/Git/...;
# we call openssl directly from PowerShell so no POSIX shell sits in between.
openssl req -x509 -newkey rsa:2048 -nodes `
    -keyout $key `
    -out $crt `
    -days 3650 `
    -subj '/C=CN/O=Xingshu Local Test/CN=127.0.0.1' `
    -addext 'subjectAltName=DNS:localhost,IP:127.0.0.1' `
    -addext 'basicConstraints=critical,CA:FALSE' `
    -addext 'keyUsage=critical,digitalSignature,keyEncipherment' `
    -addext 'extendedKeyUsage=serverAuth'

if ($LASTEXITCODE -ne 0) {
    throw "openssl req failed with exit code $LASTEXITCODE"
}

$dates = openssl x509 -in $crt -noout -dates
Write-Host "Regenerated $crt / $key"
Write-Host $dates
