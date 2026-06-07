param(
  [int]$Port = 18240,
  [string]$TargetDir = ""
)

$ErrorActionPreference = "Stop"
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
$outDir = Join-Path $root "output\acceptance\restore-drill"
$logDir = Join-Path $outDir "logs"
New-Item -ItemType Directory -Force -Path $outDir, $logDir | Out-Null

$sourceDb = Join-Path $outDir "source.sqlite3"
$restoredDb = Join-Path $outDir "restored.sqlite3"
$backup = Join-Path $outDir "reactor.sqlite3.snapshot"
$integrationTmp = Join-Path $outDir "integration.restore-drill.toml"
$reportJson = Join-Path $outDir "restore-drill-report.json"
$reportMd = Join-Path $outDir "restore-drill-report.md"
$daemonOut = Join-Path $logDir "daemon.out.log"
$daemonErr = Join-Path $logDir "daemon.err.log"
$restoredOut = Join-Path $logDir "restored-daemon.out.log"
$restoredErr = Join-Path $logDir "restored-daemon.err.log"
$targetDebugDir = if ($TargetDir) {
  Join-Path $TargetDir "debug"
} else {
  Join-Path $root "target\debug"
}
$xingshuBin = Join-Path $targetDebugDir "xingshu.exe"
$daemonBin = Join-Path $targetDebugDir "reactor-edge-daemon.exe"
$daemonProc = $null

function Normalize-ProcessPathEnvironment {
  $processEnv = [System.Environment]::GetEnvironmentVariables("Process")
  $pathKeys = @()
  foreach ($key in $processEnv.Keys) {
    if ([string]::Equals([string]$key, "Path", [System.StringComparison]::OrdinalIgnoreCase)) {
      $pathKeys += [string]$key
    }
  }
  if ($pathKeys.Count -le 1) { return }

  $preferred = if ($pathKeys -contains "Path") { "Path" } else { $pathKeys[0] }
  $preferredValue = [string]$processEnv[$preferred]
  foreach ($key in $pathKeys) {
    if ($key -ne $preferred) {
      [System.Environment]::SetEnvironmentVariable($key, $null, "Process")
      Remove-Item -Path "Env:\$key" -ErrorAction SilentlyContinue
    }
  }
  [System.Environment]::SetEnvironmentVariable("Path", $preferredValue, "Process")
  Remove-Item -Path "Env:\PATH" -ErrorAction SilentlyContinue
  Set-Item -Path "Env:\Path" -Value $preferredValue
}

Normalize-ProcessPathEnvironment

function Remove-DbFiles {
  param([string]$Path)
  Remove-Item -LiteralPath $Path, "$Path-wal", "$Path-shm", "$Path-journal" -Force -ErrorAction SilentlyContinue
}

function Stop-Daemon {
  if ($script:daemonProc -and -not $script:daemonProc.HasExited) {
    Stop-Process -Id $script:daemonProc.Id -Force -ErrorAction SilentlyContinue
    $script:daemonProc.WaitForExit(5000) | Out-Null
  }
  $script:daemonProc = $null
}

function Wait-HttpOk {
  param(
    [string]$Url,
    [int]$TimeoutSeconds = 30
  )
  $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
  while ((Get-Date) -lt $deadline) {
    try {
      $response = Invoke-WebRequest -Uri $Url -UseBasicParsing -TimeoutSec 2
      if ($response.StatusCode -eq 200) { return $true }
    } catch {
      Start-Sleep -Milliseconds 500
    }
  }
  return $false
}

function Invoke-Json {
  param(
    [ValidateSet("GET", "POST")]
    [string]$Method,
    [string]$Path,
    [string]$Token = "",
    [object]$Body = $null
  )
  $headers = @{}
  if ($Token) { $headers.Authorization = "Bearer $Token" }
  $uri = "http://127.0.0.1:$Port$Path"
  if ($null -eq $Body) {
    return Invoke-RestMethod -Method $Method -Uri $uri -Headers $headers -TimeoutSec 10
  }
  $json = $Body | ConvertTo-Json -Compress -Depth 8
  return Invoke-RestMethod -Method $Method -Uri $uri -Headers $headers -ContentType "application/json" -Body $json -TimeoutSec 10
}

function Invoke-HttpNoContent {
  param(
    [string]$Path,
    [string]$Token
  )
  $headers = @{ Authorization = "Bearer $Token" }
  $uri = "http://127.0.0.1:$Port$Path"
  $response = Invoke-WebRequest -Method Post -Uri $uri -Headers $headers -UseBasicParsing -TimeoutSec 10
  if ($response.StatusCode -lt 200 -or $response.StatusCode -ge 300) {
    throw "unexpected status for ${Path}: $($response.StatusCode)"
  }
}

function Start-DrillDaemon {
  param(
    [string]$DbPath,
    [string]$StdoutPath,
    [string]$StderrPath
  )
  Normalize-ProcessPathEnvironment
  $script:daemonProc = Start-Process -FilePath $daemonBin -ArgumentList @(
    "--config", (Join-Path $root "config\device.toml"),
    "--safety", (Join-Path $root "config\safety.toml"),
    "--memory", (Join-Path $root "config\ai_memory.toml"),
    "--integration", $integrationTmp,
    "--db", $DbPath,
    "--assets", "auto",
    "--bind", "127.0.0.1:$Port",
    "--enable-test-reset"
  ) -WorkingDirectory $root -RedirectStandardOutput $StdoutPath -RedirectStandardError $StderrPath -PassThru -WindowStyle Hidden
  if (-not (Wait-HttpOk -Url "http://127.0.0.1:$Port/health" -TimeoutSeconds 35)) {
    throw "daemon did not become healthy on port $Port; see $StdoutPath and $StderrPath"
  }
}

function Login-Role {
  param(
    [string]$Username,
    [string]$Password
  )
  $login = Invoke-Json -Method POST -Path "/api/auth/login" -Body @{
    username = $Username
    password = $Password
  }
  if (-not $login.data.token) { throw "login for $Username did not return a token" }
  return [string]$login.data.token
}

$savedEnv = @{
  XINGSHU_AUTH_SECRET = $env:XINGSHU_AUTH_SECRET
  XINGSHU_OPERATOR_PASSWORD = $env:XINGSHU_OPERATOR_PASSWORD
  XINGSHU_ENGINEER_PASSWORD = $env:XINGSHU_ENGINEER_PASSWORD
  XINGSHU_ADMIN_PASSWORD = $env:XINGSHU_ADMIN_PASSWORD
}

try {
  if (-not (Test-Path -LiteralPath $xingshuBin) -or -not (Test-Path -LiteralPath $daemonBin)) {
    $buildArgs = @("build")
    if ($TargetDir) {
      $buildArgs += @("--target-dir", $TargetDir)
    }
    $buildArgs += @("--bin", "xingshu", "--bin", "reactor-edge-daemon")
    & cargo @buildArgs | Out-Null
  }
  if (-not (Test-Path -LiteralPath $xingshuBin)) { throw "xingshu binary missing: $xingshuBin" }
  if (-not (Test-Path -LiteralPath $daemonBin)) { throw "daemon binary missing: $daemonBin" }

  foreach ($path in @($sourceDb, $restoredDb)) { Remove-DbFiles -Path $path }
  Remove-Item -LiteralPath $backup, "$backup.sha256" -Force -ErrorAction SilentlyContinue

@"
[mqtt]
enabled = false
host = "127.0.0.1"
port = 1883
use_tls = false
client_id = "xingshu-restore-drill"
keep_alive_s = 30
queue_capacity = 100
status_topic = "xingshu/restore/status"
task_topic = "xingshu/restore/tasks"
receipt_topic = "xingshu/restore/task_receipts"
alert_topic = "xingshu/restore/alerts"
alert_interval_s = 5

[modbus_tcp]
enabled = false
bind = "127.0.0.1:0"
require_tls = false
unit_id = 1
max_pdu_bytes = 260
"@ | Set-Content -LiteralPath $integrationTmp -Encoding UTF8

  $env:XINGSHU_AUTH_SECRET = "restore-drill-auth-secret-0123456789abcdef"
  $env:XINGSHU_OPERATOR_PASSWORD = "operator123"
  $env:XINGSHU_ENGINEER_PASSWORD = "engineer123"
  $env:XINGSHU_ADMIN_PASSWORD = "admin123"

  Start-DrillDaemon -DbPath $sourceDb -StdoutPath $daemonOut -StderrPath $daemonErr
  $operatorToken = Login-Role -Username "operator" -Password "operator123"
  $engineerToken = Login-Role -Username "engineer" -Password "engineer123"
  $adminToken = Login-Role -Username "admin" -Password "admin123"

  $batch = Invoke-Json -Method POST -Path "/api/batches/start" -Token $operatorToken -Body @{
    name = "restore drill batch"
    target_temperature_c = 68.5
    target_stirrer_rpm = 360
    target_shake_speed_cpm = 22
    heating_minutes = 12
    stirring_minutes = 18
  }
  $batchId = [int64]$batch.id
  if ($batchId -le 0) { throw "batch start did not return a valid id" }
  Invoke-HttpNoContent -Path "/api/batches/$batchId/finish" -Token $operatorToken
  $null = Invoke-Json -Method POST -Path "/api/product-results" -Token $engineerToken -Body @{
    batch_id = $batchId
    yield_percent = 88.4
    product_ratio = 0.93
    notes = "restore drill product result"
  }

  $beforeBatches = Invoke-Json -Method GET -Path "/api/batches" -Token $operatorToken
  $beforeAudit = Invoke-Json -Method GET -Path "/api/audit/logs?page=1&page_size=50" -Token $adminToken
  if (-not ($beforeBatches.data.batches | Where-Object { $_.id -eq $batchId })) {
    throw "source daemon does not expose batch $batchId before backup"
  }
  if (-not ($beforeBatches.data.outcomes | Where-Object { $_.batch_id -eq $batchId -and $_.yield_percent -eq 88.4 })) {
    throw "source daemon does not expose product result for batch $batchId before backup"
  }
  if (-not $beforeAudit.data.chain.window_valid) {
    throw "source audit chain window is invalid before backup"
  }

  $backupOutput = & $xingshuBin ops backup --db $sourceDb --out $backup 2>&1 | Out-String
  $backupOutput | Set-Content -LiteralPath (Join-Path $logDir "backup.log") -Encoding UTF8
  if (-not (Test-Path -LiteralPath $backup)) { throw "backup was not created: $backup" }
  if (-not (Test-Path -LiteralPath "$backup.sha256")) { throw "backup sha256 sidecar was not created" }

  Stop-Daemon
  $restoreOutput = & $xingshuBin ops restore --backup $backup --db $restoredDb --yes 2>&1 | Out-String
  $restoreOutput | Set-Content -LiteralPath (Join-Path $logDir "restore.log") -Encoding UTF8
  if ($restoreOutput -notmatch "integrity:\s+ok") {
    throw "restore output did not report integrity ok: $restoreOutput"
  }

  Start-DrillDaemon -DbPath $restoredDb -StdoutPath $restoredOut -StderrPath $restoredErr
  $operatorToken2 = Login-Role -Username "operator" -Password "operator123"
  $adminToken2 = Login-Role -Username "admin" -Password "admin123"
  $afterHealth = Invoke-Json -Method GET -Path "/health"
  $afterBatches = Invoke-Json -Method GET -Path "/api/batches" -Token $operatorToken2
  $afterDetail = Invoke-Json -Method GET -Path "/api/batches/$batchId" -Token $operatorToken2
  $afterAudit = Invoke-Json -Method GET -Path "/api/audit/logs?page=1&page_size=50" -Token $adminToken2

  $restoredBatch = $afterBatches.data.batches | Where-Object { $_.id -eq $batchId } | Select-Object -First 1
  $restoredOutcome = $afterBatches.data.outcomes | Where-Object { $_.batch_id -eq $batchId } | Select-Object -First 1
  if (-not $restoredBatch) { throw "restored daemon does not expose batch $batchId" }
  if (-not $restoredOutcome) { throw "restored daemon does not expose product result for batch $batchId" }
  if ([double]$restoredOutcome.yield_percent -ne 88.4) { throw "restored yield mismatch: $($restoredOutcome.yield_percent)" }
  if (-not $afterDetail.data.events -or $afterDetail.data.events.Count -lt 2) {
    throw "restored batch detail should include lifecycle audit events"
  }
  if (-not $afterAudit.data.chain.window_valid) { throw "restored audit chain window is invalid" }

  $backupSha256Line = (Get-Content -LiteralPath "$backup.sha256" -Raw).Trim()
  $backupSha256 = ($backupSha256Line -split "\s+")[0]
  $report = [pscustomobject]@{
    status = "ok"
    action = "backup-restore-drill"
    source_db = $sourceDb
    backup = $backup
    restored_db = $restoredDb
    batch_id = $batchId
    health_ok = [bool]$afterHealth.ok
    health_service = [string]$afterHealth.service
    audit_total_before = $beforeAudit.data.total
    audit_total_after = $afterAudit.data.total
    audit_chain_window_valid = $afterAudit.data.chain.window_valid
    audit_chain_valid = $afterAudit.data.chain.valid
    backup_sha256 = $backupSha256
    restored_yield_percent = [double]$restoredOutcome.yield_percent
    restored_product_ratio = [double]$restoredOutcome.product_ratio
    logs = $logDir
  }
  $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $reportJson -Encoding UTF8

  @(
    "# Backup Restore Drill Report",
    "",
    "- status: **OK**",
    "- source db: ``$sourceDb``",
    "- backup: ``$backup``",
    "- restored db: ``$restoredDb``",
    "- batch id: ``$batchId``",
    "- restored health: ``$($afterHealth.service)`` / ``ok=$($afterHealth.ok)``",
    "- audit events before/after: ``$($beforeAudit.data.total)`` / ``$($afterAudit.data.total)``",
    "- audit chain window valid: ``$($afterAudit.data.chain.window_valid)``",
    "- backup sha256: ``$backupSha256``",
    "- logs: ``$logDir``"
  ) | Set-Content -LiteralPath $reportMd -Encoding UTF8

  Write-Host "backup restore drill ok"
  Write-Host "report -> $reportJson"
  Write-Host "report -> $reportMd"
} finally {
  Stop-Daemon
  foreach ($entry in $savedEnv.GetEnumerator()) {
    if ($null -eq $entry.Value) {
      Remove-Item -Path "Env:\$($entry.Key)" -ErrorAction SilentlyContinue
    } else {
      Set-Item -Path "Env:\$($entry.Key)" -Value $entry.Value
    }
  }
}
