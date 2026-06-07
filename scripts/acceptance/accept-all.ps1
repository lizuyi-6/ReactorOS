param(
  [int]$Port = 18300,
  [int]$VitePort = 15173,
  [int]$AinasPort = 5599,
  [int]$Stm32Port = 15502
)

$ErrorActionPreference = "Stop"
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\..")).Path
Set-Location $root

$outDir = Join-Path $root "output\acceptance"
$logDir = Join-Path $outDir "logs"
New-Item -ItemType Directory -Force -Path $outDir, $logDir | Out-Null

$reportJson = Join-Path $outDir "acceptance-report.json"
$reportMd = Join-Path $outDir "acceptance-report.md"
$dataDb = Join-Path $outDir "acceptance.sqlite3"
$daemonLog = Join-Path $logDir "daemon.log"
$viteLog = Join-Path $logDir "vite.log"
$simLog = Join-Path $logDir "sim.log"
$steps = New-Object System.Collections.Generic.List[object]
$children = New-Object System.Collections.Generic.List[System.Diagnostics.Process]
$script:exitCode = 0

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

function Record-Step {
  param(
    [string]$Name,
    [string]$Status,
    [string]$Info
  )
  $steps.Add([pscustomobject]@{ step = $Name; status = $Status; info = $Info }) | Out-Null
  Write-Host "[$Status] $Name :: $Info"
}

function Last-Log-Line {
  param([string]$Path)
  if (-not (Test-Path -LiteralPath $Path)) { return "log file not found: $Path" }
  $lines = Get-Content -LiteralPath $Path -Tail 3
  if ($lines.Count -eq 0) { return "log file is empty: $Path" }
  return [string]$lines[0]
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
      Start-Sleep -Seconds 1
    }
  }
  return $false
}

function Start-LoggedProcess {
  param(
    [string]$FilePath,
    [string[]]$ArgumentList,
    [string]$LogPath,
    [hashtable]$Env = @{}
  )
  Normalize-ProcessPathEnvironment
  $startInfo = New-Object System.Diagnostics.ProcessStartInfo
  $startInfo.FileName = $FilePath
  $startInfo.Arguments = ($ArgumentList | ForEach-Object {
    if ($_ -match '[\s"]') {
      '"' + ($_ -replace '"', '\"') + '"'
    } else {
      $_
    }
  }) -join " "
  $startInfo.WorkingDirectory = $root
  $startInfo.UseShellExecute = $false
  $startInfo.RedirectStandardOutput = $true
  $startInfo.RedirectStandardError = $true
  foreach ($entry in $Env.GetEnumerator()) {
    $startInfo.Environment[$entry.Key] = [string]$entry.Value
  }
  $proc = New-Object System.Diagnostics.Process
  $proc.StartInfo = $startInfo
  [void]$proc.Start()
  $stdoutTask = $proc.StandardOutput.ReadToEndAsync()
  $stderrTask = $proc.StandardError.ReadToEndAsync()
  $proc | Add-Member -NotePropertyName XingshuLogPath -NotePropertyValue $LogPath
  $proc | Add-Member -NotePropertyName XingshuStdoutTask -NotePropertyValue $stdoutTask
  $proc | Add-Member -NotePropertyName XingshuStderrTask -NotePropertyValue $stderrTask
  $children.Add($proc) | Out-Null
  return $proc
}

function Flush-ProcessLogs {
  foreach ($proc in $children) {
    $logPath = $proc.XingshuLogPath
    if (-not $logPath) { continue }
    try {
      $stdout = if ($proc.XingshuStdoutTask.IsCompleted) { $proc.XingshuStdoutTask.Result } else { "" }
      $stderr = if ($proc.XingshuStderrTask.IsCompleted) { $proc.XingshuStderrTask.Result } else { "" }
      Set-Content -LiteralPath $logPath -Value ($stdout + $stderr) -Encoding UTF8
    } catch {
      Set-Content -LiteralPath $logPath -Value "failed to collect process log: $($_.Exception.Message)" -Encoding UTF8
    }
  }
}

function Stop-Children {
  foreach ($proc in $children) {
    try {
      if (-not $proc.HasExited) {
        Stop-ProcessTree -ProcessId $proc.Id
        $proc.WaitForExit(5000) | Out-Null
      }
    } catch {}
  }
  Flush-ProcessLogs
  Stop-ProcessesByCommandPattern -Pattern "scripts/mocks/ainas-mock-server.mjs" -Port $AinasPort
  Stop-ProcessesByCommandPattern -Pattern "scripts/mocks/stm32-modbus-tcp-mock.mjs" -Port $Stm32Port
  Stop-ProcessesByCommandPattern -Pattern "frontend:dev -- --port $VitePort" -Port $VitePort
  Remove-Item -LiteralPath $dataDb, "$dataDb-wal", "$dataDb-shm", "$dataDb-journal" -Force -ErrorAction SilentlyContinue
}

function Stop-ProcessTree {
  param([int]$ProcessId)
  $childProcesses = @(Get-CimInstance Win32_Process -Filter "ParentProcessId=$ProcessId" -ErrorAction SilentlyContinue)
  foreach ($child in $childProcesses) {
    Stop-ProcessTree -ProcessId ([int]$child.ProcessId)
  }
  try {
    Stop-Process -Id $ProcessId -Force -ErrorAction SilentlyContinue
  } catch {}
}

function Stop-ProcessesByCommandPattern {
  param(
    [string]$Pattern,
    [int]$Port
  )
  $listeners = @(Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue | Select-Object -ExpandProperty OwningProcess -Unique)
  $processes = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue | Where-Object {
      ($_.CommandLine -like "*$Pattern*") -or ($listeners -contains $_.ProcessId)
    })
  foreach ($proc in $processes) {
    Stop-ProcessTree -ProcessId ([int]$proc.ProcessId)
  }
}

function Test-ModbusFc03 {
  param([int]$Port)
  try {
    $client = [System.Net.Sockets.TcpClient]::new()
    $async = $client.BeginConnect("127.0.0.1", $Port, $null, $null)
    if (-not $async.AsyncWaitHandle.WaitOne(1000)) { $client.Close(); return $false }
    $client.EndConnect($async)
    $stream = $client.GetStream()
    $frame = [byte[]](0x00,0x01,0x00,0x00,0x00,0x06,0x01,0x03,0x00,0x00,0x00,0x01)
    $stream.Write($frame, 0, $frame.Length)
    $buffer = New-Object byte[] 32
    $stream.ReadTimeout = 1000
    $read = $stream.Read($buffer, 0, $buffer.Length)
    $client.Close()
    return ($read -ge 11 -and $buffer[7] -eq 0x03 -and $buffer[8] -eq 0x02)
  } catch {
    return $false
  }
}

function Invoke-CapturedCommand {
  param(
    [string]$LogPath,
    [scriptblock]$Command
  )
  Normalize-ProcessPathEnvironment
  $previousPreference = $ErrorActionPreference
  $ErrorActionPreference = "Continue"
  try {
    $output = & $Command 2>&1
    $exit = $LASTEXITCODE
    $output | Out-String | Set-Content -LiteralPath $LogPath -Encoding UTF8
    return $exit
  } finally {
    $ErrorActionPreference = $previousPreference
  }
}

try {
  $daemonBin = Join-Path $root "target\debug\reactor-edge-daemon.exe"
  if (-not (Test-Path -LiteralPath $daemonBin)) {
    & cargo build --bin reactor-edge-daemon
    if ($LASTEXITCODE -ne 0) { throw "cargo build --bin reactor-edge-daemon failed" }
  }
  if (-not (Test-Path -LiteralPath $daemonBin)) { throw "daemon binary missing: $daemonBin" }

  $pass = 0
  $fail = 0

  $vueReleaseLog = Join-Path $logDir "vue-release-assets.log"
  $vueReleaseExit = Invoke-CapturedCommand -LogPath $vueReleaseLog -Command {
    & node scripts/verify-vue-release-assets.mjs
  }
  if ($vueReleaseExit -eq 0) {
    Record-Step "verify-vue-release-assets" "ok" "release package and systemd paths prefer Vue dist with legacy fallback"
    $pass++
  } else {
    Record-Step "verify-vue-release-assets" "fail" (Last-Log-Line $vueReleaseLog)
    $fail++
  }

  $safetyGuardLog = Join-Path $logDir "production-safety-guard.log"
  $safetyGuardExit = Invoke-CapturedCommand -LogPath $safetyGuardLog -Command {
    & node scripts/verify-production-safety-guard.mjs
  }
  if ($safetyGuardExit -eq 0) {
    Record-Step "verify-production-safety-guard" "ok" "release package and systemd launch the isolated safety guard"
    $pass++
  } else {
    Record-Step "verify-production-safety-guard" "fail" (Last-Log-Line $safetyGuardLog)
    $fail++
  }

  $backupScheduleLog = Join-Path $logDir "production-backup-schedule.log"
  $backupScheduleExit = Invoke-CapturedCommand -LogPath $backupScheduleLog -Command {
    & node scripts/verify-production-backup-schedule.mjs
  }
  if ($backupScheduleExit -eq 0) {
    Record-Step "verify-production-backup-schedule" "ok" "systemd timer schedules online SQLite backup snapshots"
    $pass++
  } else {
    Record-Step "verify-production-backup-schedule" "fail" (Last-Log-Line $backupScheduleLog)
    $fail++
  }

  $backupScriptLog = Join-Path $logDir "production-backup-script.log"
  $backupScriptExit = Invoke-CapturedCommand -LogPath $backupScriptLog -Command {
    & powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-production-backup-script.ps1
  }
  if ($backupScriptExit -eq 0) {
    Record-Step "verify-production-backup-script" "ok" "backup script writes timestamped SQLite snapshots and latest link"
    $pass++
  } else {
    Record-Step "verify-production-backup-script" "fail" (Last-Log-Line $backupScriptLog)
    $fail++
  }

  $restoreDrillLog = Join-Path $logDir "backup-restore-drill.log"
  $restoreDrillExit = Invoke-CapturedCommand -LogPath $restoreDrillLog -Command {
    & powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-backup-restore-drill.ps1
  }
  if ($restoreDrillExit -eq 0) {
    Record-Step "verify-backup-restore-drill" "ok" "restored snapshot boots a fresh daemon with batch, product result, and audit chain intact"
    $pass++
  } else {
    Record-Step "verify-backup-restore-drill" "fail" (Last-Log-Line $restoreDrillLog)
    $fail++
  }

  $trainingDeliverablesLog = Join-Path $logDir "training-deliverables.log"
  $trainingDeliverablesExit = Invoke-CapturedCommand -LogPath $trainingDeliverablesLog -Command {
    & node scripts/verify-training-deliverables.mjs
  }
  if ($trainingDeliverablesExit -eq 0) {
    Record-Step "verify-training-deliverables" "ok" "training deck, PPTX package, image assets, UAT script, and preview manifest passed"
    $pass++
  } else {
    Record-Step "verify-training-deliverables" "fail" (Last-Log-Line $trainingDeliverablesLog)
    $fail++
  }

  $preflightLog = Join-Path $logDir "xingshu-ops-preflight.log"
  $preflightExit = Invoke-CapturedCommand -LogPath $preflightLog -Command {
    $savedEnv = @{
      XINGSHU_AUTH_SECRET = $env:XINGSHU_AUTH_SECRET
      XINGSHU_OPERATOR_PASSWORD = $env:XINGSHU_OPERATOR_PASSWORD
      XINGSHU_ENGINEER_PASSWORD = $env:XINGSHU_ENGINEER_PASSWORD
      XINGSHU_ADMIN_PASSWORD = $env:XINGSHU_ADMIN_PASSWORD
      XINGSHU_DB_ENCRYPTION_KEY = $env:XINGSHU_DB_ENCRYPTION_KEY
    }
    try {
      $env:XINGSHU_AUTH_SECRET = "0123456789abcdef0123456789abcdef"
      $env:XINGSHU_OPERATOR_PASSWORD = "operator-password-123"
      $env:XINGSHU_ENGINEER_PASSWORD = "engineer-password-123"
      $env:XINGSHU_ADMIN_PASSWORD = "admin-password-123"
      $env:XINGSHU_DB_ENCRYPTION_KEY = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
      & cargo run --quiet --bin xingshu -- ops preflight --production --json
    } finally {
      foreach ($entry in $savedEnv.GetEnumerator()) {
        if ($null -eq $entry.Value) {
          Remove-Item -Path "Env:\$($entry.Key)" -ErrorAction SilentlyContinue
        } else {
          Set-Item -Path "Env:\$($entry.Key)" -Value $entry.Value
        }
      }
    }
  }
  if ($preflightExit -eq 0) {
    Record-Step "xingshu-ops-preflight" "ok" "production secrets, TLS paths, and backup timer files checked"
    $pass++
  } else {
    Record-Step "xingshu-ops-preflight" "fail" (Last-Log-Line $preflightLog)
    $fail++
  }

  Remove-Item -LiteralPath $dataDb, "$dataDb-wal", "$dataDb-shm", "$dataDb-journal" -Force -ErrorAction SilentlyContinue
  $integrationDir = Join-Path $outDir "tmp"
  New-Item -ItemType Directory -Force -Path $integrationDir | Out-Null
  $integrationTmp = Join-Path $integrationDir "integration.acceptance.toml"
  @"
[mqtt]
enabled = false
host = "127.0.0.1"
port = 1883
use_tls = false
client_id = "xingshu-acceptance"
keep_alive_s = 30
queue_capacity = 100
status_topic = "xingshu/reactor_001/status"
task_topic = "xingshu/reactor_001/tasks"
receipt_topic = "xingshu/reactor_001/task_receipts"
alert_topic = "xingshu/reactor_001/alerts"
alert_interval_s = 5

[modbus_tcp]
enabled = false
bind = "0.0.0.0:502"
require_tls = false
unit_id = 1
max_pdu_bytes = 260
"@ | Set-Content -LiteralPath $integrationTmp -Encoding UTF8

  $daemon = Start-LoggedProcess -FilePath $daemonBin -ArgumentList @(
    "--config", "config/device.toml",
    "--safety", "config/safety.toml",
    "--memory", "config/ai_memory.toml",
    "--integration", $integrationTmp,
    "--db", $dataDb,
    "--assets", "auto",
    "--bind", "127.0.0.1:$Port",
    "--enable-test-reset"
  ) -LogPath $daemonLog
  if (-not (Wait-HttpOk -Url "http://127.0.0.1:$Port/health" -TimeoutSeconds 30)) {
    throw "daemon did not become healthy on $Port"
  }

  $vite = Start-LoggedProcess -FilePath "cmd.exe" -ArgumentList @(
    "/c", "npm", "run", "frontend:dev", "--", "--port", "$VitePort", "--strictPort"
  ) -LogPath $viteLog -Env @{ XINGSHU_VITE_API_TARGET = "http://127.0.0.1:$Port" }

  if (Wait-HttpOk -Url "http://127.0.0.1:$VitePort/" -TimeoutSeconds 30) {
    Record-Step "vite-dev" "ok" "vite dev on $VitePort proxied to $Port"
    $pass++
  } else {
    Record-Step "vite-dev" "fail" "vite dev did not become healthy on $VitePort"
    $fail++
    throw "vite dev did not become healthy on $VitePort"
  }

  $sim = Start-LoggedProcess -FilePath "node.exe" -ArgumentList @(
    "scripts/simulate-device.js",
    "--url", "http://127.0.0.1:$Port",
    "--profile", "production",
    "--interval-ms", "1000"
  ) -LogPath $simLog
  Start-Sleep -Seconds 4

  $rbacLog = Join-Path $logDir "load-and-rbac.log"
  $rbacExit = Invoke-CapturedCommand -LogPath $rbacLog -Command {
    & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $root "scripts\verify-load-and-rbac.ps1") -Base "http://127.0.0.1:$Port"
  }
  if ($rbacExit -eq 0) {
    Record-Step "verify-load-and-rbac" "ok" "RBAC matrix all-pass; see $rbacLog"; $pass++
  } else {
    Record-Step "verify-load-and-rbac" "fail" (Last-Log-Line $rbacLog); $fail++
  }

  if ($sim -and -not $sim.HasExited) {
    try { Stop-Process -Id $sim.Id -Force -ErrorAction SilentlyContinue } catch {}
    Start-Sleep -Milliseconds 800
  }

  $parityLog = Join-Path $logDir "vue-parity.log"
  $env:E2E_BASE_URL = "http://127.0.0.1:$Port"
  $env:VUE_URL = "http://127.0.0.1:$VitePort/"
  $parityExit = Invoke-CapturedCommand -LogPath $parityLog -Command {
    & node scripts/verify-vue-parity.mjs
  }
  if ($parityExit -eq 0) {
    Record-Step "verify-vue-parity" "ok" "Vue 7 routes and bilingual checks passed"; $pass++
  } else {
    Record-Step "verify-vue-parity" "fail" (Last-Log-Line $parityLog); $fail++
  }

  $historyXlsxLog = Join-Path $logDir "vue-history-xlsx.log"
  $historyXlsxExit = Invoke-CapturedCommand -LogPath $historyXlsxLog -Command {
    & node scripts/verify-vue-history-xlsx.mjs
  }
  if ($historyXlsxExit -eq 0) {
    Record-Step "verify-vue-history-xlsx" "ok" "History CSV/XLSX downloads and bilingual buttons passed"; $pass++
  } else {
    Record-Step "verify-vue-history-xlsx" "fail" (Last-Log-Line $historyXlsxLog); $fail++
  }

  $lifecycleLog = Join-Path $logDir "vue-lifecycle.log"
  $lifecycleExit = Invoke-CapturedCommand -LogPath $lifecycleLog -Command {
    & node scripts/verify-vue-process-lifecycle.mjs
  }
  if ($lifecycleExit -eq 0) {
    Record-Step "verify-vue-process-lifecycle" "ok" "process lifecycle and bilingual checks passed"; $pass++
  } else {
    Record-Step "verify-vue-process-lifecycle" "fail" (Last-Log-Line $lifecycleLog); $fail++
  }

  $mobileLog = Join-Path $logDir "vue-mobile.log"
  $mobileExit = Invoke-CapturedCommand -LogPath $mobileLog -Command {
    & node scripts/verify-vue-mobile.mjs
  }
  if ($mobileExit -eq 0) {
    Record-Step "verify-vue-mobile" "ok" "phone and tablet viewport bilingual navigation checks passed"; $pass++
  } else {
    Record-Step "verify-vue-mobile" "fail" (Last-Log-Line $mobileLog); $fail++
  }

  $browserMatrixLog = Join-Path $logDir "vue-browser-matrix.log"
  $browserMatrixExit = Invoke-CapturedCommand -LogPath $browserMatrixLog -Command {
    $env:PLAYWRIGHT_BROWSER_MATRIX_STRICT = "1"
    & node scripts/verify-vue-browser-matrix.mjs
  }
  if ($browserMatrixExit -eq 0) {
    $browserMatrixReport = Join-Path $root "output\playwright\vue-browser-matrix-verification.json"
    $browserMatrixInfo = "available Playwright browsers passed 7-route bilingual layout checks"
    try {
      if (Test-Path -LiteralPath $browserMatrixReport) {
        $browserMatrix = Get-Content -LiteralPath $browserMatrixReport -Raw | ConvertFrom-Json
        $passedBrowsers = @($browserMatrix.browsers | Where-Object { $_.status -eq "ok" } | ForEach-Object { $_.name })
        $skippedBrowsers = @($browserMatrix.browsers | Where-Object { $_.status -eq "skipped" } | ForEach-Object { "$($_.name): $($_.skipReason)" })
        $pageChecks = 0
        foreach ($browserResult in $browserMatrix.browsers) {
          $pageChecks += @($browserResult.pages).Count
        }
        $browserMatrixInfo = "passed browsers: $($passedBrowsers -join ', '); skipped: $($skippedBrowsers -join '; '); page checks: $pageChecks; console errors: $(@($browserMatrix.unexpectedConsoleMessages).Count)"
      }
    } catch {
      $browserMatrixInfo = "$browserMatrixInfo; report summary parse failed: $($_.Exception.Message)"
    }
    Record-Step "verify-vue-browser-matrix" "ok" $browserMatrixInfo; $pass++
  } else {
    Record-Step "verify-vue-browser-matrix" "fail" (Last-Log-Line $browserMatrixLog); $fail++
  }

  $probeLog = Join-Path $logDir "probe-cli-ops.log"
  $probeExit = Invoke-CapturedCommand -LogPath $probeLog -Command {
    & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $root "scripts\probe-cli-ops.ps1")
  }
  if ($probeExit -eq 0) {
    Record-Step "probe-cli-ops" "ok" "real SQLite backup/restore/wipe/key generate/key rekey"; $pass++
  } else {
    Record-Step "probe-cli-ops" "fail" (Last-Log-Line $probeLog); $fail++
  }

  $ainasMqttLog = Join-Path $logDir "ainas-mqtt.log"
  $ainasMqttExit = Invoke-CapturedCommand -LogPath $ainasMqttLog -Command {
    & node scripts/verify-ainas-mqtt.mjs
  }
  if ($ainasMqttExit -eq 0) {
    Record-Step "verify-ainas-mqtt" "ok" "AINAS API and integration config summary passed"; $pass++
  } else {
    Record-Step "verify-ainas-mqtt" "fail" (Last-Log-Line $ainasMqttLog); $fail++
  }

  $mqttBrokerLog = Join-Path $logDir "mqtt-broker.log"
  if (Get-Command docker -ErrorAction SilentlyContinue) {
    $mqttBrokerExit = Invoke-CapturedCommand -LogPath $mqttBrokerLog -Command {
      & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $root "scripts\mocks\verify-mosquitto-broker.ps1")
    }
    if ($mqttBrokerExit -eq 0) {
      $mqttReportPath = Join-Path $root "output\acceptance\mqtt-broker-report.json"
      $mqttReport = if (Test-Path -LiteralPath $mqttReportPath) {
        Get-Content -Raw -LiteralPath $mqttReportPath | ConvertFrom-Json
      } else {
        $null
      }
      if ($mqttReport -and $mqttReport.status -eq "ok") {
        Record-Step "verify-mosquitto-broker" "ok" "real broker status/task/receipt round-trip passed"; $pass++
      } elseif ($mqttReport -and $mqttReport.status -eq "skipped") {
        Record-Step "verify-mosquitto-broker" "skipped" "Docker present but broker drill skipped: $($mqttReport.reason)"
      } else {
        Record-Step "verify-mosquitto-broker" "fail" (Last-Log-Line $mqttBrokerLog); $fail++
      }
    } else {
      Record-Step "verify-mosquitto-broker" "fail" (Last-Log-Line $mqttBrokerLog); $fail++
    }
  } else {
    Record-Step "verify-mosquitto-broker" "skipped" "Docker not available; run scripts\mocks\verify-mosquitto-broker.ps1 when Docker is installed"
  }

  $mockParseLog = Join-Path $logDir "mock-entrypoints.log"
  $parseExit = Invoke-CapturedCommand -LogPath $mockParseLog -Command {
    & node --check scripts/mocks/ainas-mock-server.mjs
  }
  $parseOk = $parseExit -eq 0
  if ($parseOk) {
    $stm32ParseExit = Invoke-CapturedCommand -LogPath "$mockParseLog.tmp" -Command {
      & node --check scripts/mocks/stm32-modbus-tcp-mock.mjs
    }
    Get-Content -LiteralPath "$mockParseLog.tmp" | Add-Content -LiteralPath $mockParseLog
    Remove-Item -LiteralPath "$mockParseLog.tmp" -Force -ErrorAction SilentlyContinue
    $parseOk = $stm32ParseExit -eq 0
  }
  if ($parseOk) {
    Record-Step "mock-entrypoints-parse" "ok" "AINAS/STM32 mock entrypoints parse"; $pass++
  } else {
    Record-Step "mock-entrypoints-parse" "fail" (Last-Log-Line $mockParseLog); $fail++
  }

  $ainasLog = Join-Path $logDir "ainas-mock.log"
  $ainasMock = Start-LoggedProcess -FilePath "node.exe" -ArgumentList @(
    "scripts/mocks/ainas-mock-server.mjs", "--listen", "127.0.0.1:$AinasPort"
  ) -LogPath $ainasLog
  if (Wait-HttpOk -Url "http://127.0.0.1:$AinasPort/health" -TimeoutSeconds 20) {
    Record-Step "ainas-mock-health" "ok" "AINAS mock /health returned 200 on 127.0.0.1:$AinasPort"; $pass++
  } else {
    Record-Step "ainas-mock-health" "fail" "AINAS mock did not become healthy; see $ainasLog"; $fail++
  }

  $stm32Log = Join-Path $logDir "stm32-modbus-mock.log"
  $stm32Mock = Start-LoggedProcess -FilePath "node.exe" -ArgumentList @(
    "scripts/mocks/stm32-modbus-tcp-mock.mjs", "--listen", "127.0.0.1:$Stm32Port", "--registers", "config/device.toml"
  ) -LogPath $stm32Log
  $modbusOk = $false
  $deadline = (Get-Date).AddSeconds(20)
  while ((Get-Date) -lt $deadline) {
    if (Test-ModbusFc03 -Port $Stm32Port) { $modbusOk = $true; break }
    Start-Sleep -Seconds 1
  }
  if ($modbusOk) {
    Record-Step "stm32-modbus-mock-fc03" "ok" "STM32 mock answered Modbus TCP FC03 on 127.0.0.1:$Stm32Port"; $pass++
  } else {
    Record-Step "stm32-modbus-mock-fc03" "fail" "STM32 mock did not answer FC03; see $stm32Log"; $fail++
  }

  $status = if ($fail -gt 0) { "fail" } else { "ok" }
  $stepArray = @($steps.ToArray())
  $payload = [pscustomobject]@{
    status = $status
    base_url = "http://127.0.0.1:$Port"
    vue_url = "http://127.0.0.1:$VitePort/"
    steps_pass = $pass
    steps_fail = $fail
    total = $pass + $fail
    commit = (& git rev-parse HEAD 2>$null)
    steps = $stepArray
  }
  $utf8NoBom = [System.Text.UTF8Encoding]::new($false)
  [System.IO.File]::WriteAllText($reportJson, (($payload | ConvertTo-Json -Depth 8) + [Environment]::NewLine), $utf8NoBom)

  $md = New-Object System.Collections.Generic.List[string]
  $md.Add("# Upper-Computer Acceptance Report")
  $md.Add("")
  $md.Add("- commit: ``$((& git rev-parse --short HEAD 2>$null))``")
  $md.Add("- base URL: ``http://127.0.0.1:$Port``")
  $md.Add("- Vue URL: ``http://127.0.0.1:$VitePort/``")
  $md.Add("- steps pass / fail / total: **$pass / $fail / $($pass + $fail)**")
  $md.Add("- final status: **$($status.ToUpperInvariant())**")
  $md.Add("")
  $md.Add("## Steps")
  $md.Add("")
  $md.Add("| Step | Status | Info |")
  $md.Add("|---|---|---|")
  foreach ($step in $steps) {
    $info = ([string]$step.info).Replace("|", "\|").Replace("`r", " ").Replace("`n", " ")
    $md.Add("| ``$($step.step)`` | $($step.status) | $info |")
  }
  $md.Add("")
  $md.Add("## Report Files")
  $md.Add("")
  $md.Add("- JSON: ``output/acceptance/acceptance-report.json``")
  $md.Add("- Markdown: ``output/acceptance/acceptance-report.md``")
  $md.Add("- logs: ``output/acceptance/logs/``")
  [System.IO.File]::WriteAllText($reportMd, (($md -join [Environment]::NewLine) + [Environment]::NewLine), $utf8NoBom)

  Write-Host ""
  Write-Host "report -> $reportJson"
  Write-Host "report -> $reportMd"
  if ($fail -gt 0) { $script:exitCode = 1 }
} catch {
  $script:exitCode = 1
  $errorLog = Join-Path $logDir "acceptance-run-error.log"
  $_ | Format-List * -Force | Out-String | Set-Content -LiteralPath $errorLog -Encoding UTF8
  Write-Error "acceptance failed: $($_.Exception.Message); see $errorLog"
} finally {
  Stop-Children
}

exit $script:exitCode
