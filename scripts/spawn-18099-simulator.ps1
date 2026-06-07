$root = "X:\tianhks"
$logDir = Join-Path $root "output\local-run"
New-Item -ItemType Directory -Force -Path $logDir | Out-Null
$proc = Start-Process -FilePath "node" -ArgumentList @(
    "scripts/simulate-device.js",
    "--url", "http://127.0.0.1:18099",
    "--profile", "production",
    "--interval-ms", "1000"
) -WorkingDirectory $root `
  -RedirectStandardOutput (Join-Path $logDir "sim-18099-v3.out.log") `
  -RedirectStandardError (Join-Path $logDir "sim-18099-v3.err.log") `
  -PassThru -WindowStyle Hidden
Write-Output "sim pid=$($proc.Id)"
