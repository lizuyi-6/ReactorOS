$root = (Resolve-Path -LiteralPath $PSScriptRoot\..).Path
$logDir = Join-Path $root "output\local-run"
New-Item -ItemType Directory -Force -Path $logDir | Out-Null
if (-not $env:XINGSHU_TOKEN) {
    throw "Set XINGSHU_TOKEN to an engineer/admin bearer token before starting the pipeline simulator."
}
$proc = Start-Process -FilePath "node" -ArgumentList @(
    "scripts/simulate-device.js",
    "--url", "http://127.0.0.1:8000",
    "--token", $env:XINGSHU_TOKEN,
    "--profile", "production",
    "--interval-ms", "1000"
) -WorkingDirectory $root `
  -RedirectStandardOutput (Join-Path $logDir "sim-8000.out.log") `
  -RedirectStandardError (Join-Path $logDir "sim-8000.err.log") `
  -PassThru -WindowStyle Hidden
Write-Output "sim pid=$($proc.Id)"
