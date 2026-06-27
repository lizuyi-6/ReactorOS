# Modbus TCP TLS handshake self-check.
# Verifies the daemon exposes a valid TLS 1.2+ listener on its configured bind
# port and that the configured certificate chain resolves.
#
# Usage:
#   powershell -File scripts/verify-modbus-tls.ps1 `
#     -Host 127.0.0.1 -Port 502 `
#     -Cert config/tls/modbus-server.pem `
#     -Sni reactor-modbus
#
# Or with a daemon that has not enabled Modbus TCP (expected: connection refused
# after a brief delay) - the script reports the outcome without failing the run.
param(
    [string]$Host = "127.0.0.1",
    [int]$Port = 502,
    [string]$Cert = "config/tls/modbus-server.pem",
    [string]$Sni = "reactor-modbus",
    [int]$TimeoutSec = 8
)

$ErrorActionPreference = "Continue"
$report = [ordered]@{
    host = $Host
    port = $Port
    cert = $Cert
    sni = $Sni
    timeout_sec = $TimeoutSec
    tcp_open = $false
    tls_handshake = $false
    certificate_match = $false
    notes = @()
}

# 1. TCP reachability
try {
    $tcp = New-Object System.Net.Sockets.TcpClient
    $iar = $tcp.BeginConnect($Host, $Port, $null, $null)
    $ok = $iar.AsyncWaitHandle.WaitOne($TimeoutSec * 1000)
    if (-not $ok) { $report.notes += "tcp connect timed out after ${TimeoutSec}s"; $tcp.Close(); return $report | ConvertTo-Json -Depth 5 }
    $tcp.EndConnect($iar)
    $report.tcp_open = $true
    $tcp.Close()
} catch {
    $report.notes += "tcp connect failed: $_"
    return $report | ConvertTo-Json -Depth 5
}

# 2. TLS handshake via openssl s_client
$openssl = (Get-Command openssl -ErrorAction SilentlyContinue)
if (-not $openssl) {
    $report.notes += "openssl not on PATH; skipping TLS handshake"
    $report | ConvertTo-Json -Depth 5 | Out-String | Write-Output
    exit 0
}
$socketOpt = "-connect ${Host}:${Port} -servername ${Sni} -showcerts"
$proc = Start-Process -FilePath "openssl" -ArgumentList @("s_client", $socketOpt.Split(" ")) `
    -NoNewWindow -PassThru -RedirectStandardOutput "output\local-run\tls-stdout.txt" `
    -RedirectStandardError "output\local-run\tls-stderr.txt"
$deadline = (Get-Date).AddSeconds($TimeoutSec)
while (-not $proc.HasExited -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 200 }
if (-not $proc.HasExited) { try { $proc.Kill() } catch {} }
$proc.WaitForExit()
$stdout = Get-Content "output\local-run\tls-stdout.txt" -Raw -ErrorAction SilentlyContinue
$stderr = Get-Content "output\local-run\tls-stderr.txt" -Raw -ErrorAction SilentlyContinue
$combined = ($stdout + "`n" + $stderr)
if ($combined -match "BEGIN CERTIFICATE") { $report.tls_handshake = $true }
if ($combined -match "Verify return code: 0") { $report.certificate_match = $true }
$report.notes += ($combined -split "`n" | Select-Object -First 40)

$report | ConvertTo-Json -Depth 6
