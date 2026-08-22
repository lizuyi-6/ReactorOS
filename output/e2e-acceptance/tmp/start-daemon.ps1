
$ErrorActionPreference = 'Stop'
Copy-Item X:\rust-target\reactor-os\debug\reactor-edge-daemon.exe X:\rust-target\reactor-os\debug\reactor-edge-daemon-e2e.exe -Force
Get-Content .env | ForEach-Object {
  if ($_ -match '^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$' -and $_ -notmatch '^\s*#') {
    [Environment]::SetEnvironmentVariable($Matches[1], $Matches[2].Trim().Trim('"').Trim("'"), 'Process')
  }
}
& X:\rust-target\reactor-os\debug\reactor-edge-daemon-e2e.exe --config config/device.simulation.toml --safety config/safety.toml --memory config/ai_memory.toml --db output/e2e-acceptance/e2e.sqlite3 --assets frontend/dist --bind 127.0.0.1:8000 --enable-test-reset --seed-demo-context
