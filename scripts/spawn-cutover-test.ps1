$env:CARGO_TARGET_DIR = "C:\tmp\xingshu-target-cutover-test"
$root = (Resolve-Path -LiteralPath $PSScriptRoot\..).Path
$logDir = Join-Path $root "output\local-run"
New-Item -ItemType Directory -Force -Path $logDir | Out-Null
$proc = Start-Process -FilePath "cargo" -ArgumentList @(
    "run", "--bin", "reactor-edge-daemon", "--",
    "--config", "config/device.toml",
    "--safety", "config/safety.toml",
    "--memory", "config/ai_memory.toml",
    "--integration", "config/integration.toml",
    "--db", "data\cutover-test.sqlite3",
    "--bind", "127.0.0.1:18099",
    "--enable-test-reset"
) -WorkingDirectory $root `
  -RedirectStandardOutput (Join-Path $logDir "cutover-test.out.log") `
  -RedirectStandardError (Join-Path $logDir "cutover-test.err.log") `
  -PassThru -WindowStyle Hidden
Write-Output "spawned pid=$($proc.Id)"
