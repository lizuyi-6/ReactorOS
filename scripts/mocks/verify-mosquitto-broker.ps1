param(
  [int]$DaemonPort = 18200,
  [int]$BrokerPort = 1883,
  [string]$Container = "xingshu-mqtt-mosquitto"
)

$ErrorActionPreference = "Stop"
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..\..")).Path
Set-Location $root

$outDir = Join-Path $root "output\acceptance"
$logDir = Join-Path $root "output\local-run"
New-Item -ItemType Directory -Force -Path $outDir, $logDir | Out-Null

$report = Join-Path $outDir "mqtt-broker-report.json"
$daemonLog = Join-Path $logDir "mqtt-acceptance-daemon.log"
$simLog = Join-Path $logDir "mqtt-acceptance-simulator.log"
$statusPayload = Join-Path $logDir "mqtt-status-payload.json"
$receiptPayload = Join-Path $logDir "mqtt-receipt-payload.json"
$integrationDir = Join-Path $outDir "tmp"
$integrationTmp = Join-Path $integrationDir "integration.mqtt-acceptance.toml"
$mosquittoConf = Join-Path $outDir "mosquitto.acceptance.conf"
$dbPath = Join-Path $outDir "mqtt-acceptance.sqlite3"
$daemonProc = $null
$simProc = $null

function Write-Report {
  param([hashtable]$Payload)
  $Payload | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $report -Encoding UTF8
}

function Stop-ProcessTree {
  param([int]$ProcessId)
  $children = @(Get-CimInstance Win32_Process -Filter "ParentProcessId=$ProcessId" -ErrorAction SilentlyContinue)
  foreach ($child in $children) {
    Stop-ProcessTree -ProcessId ([int]$child.ProcessId)
  }
  Stop-Process -Id $ProcessId -Force -ErrorAction SilentlyContinue
}

function Wait-HttpOk {
  param([string]$Url, [int]$TimeoutSeconds = 30)
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

function Test-TcpPort {
  param([int]$Port, [int]$TimeoutMilliseconds = 1000)
  try {
    $client = [System.Net.Sockets.TcpClient]::new()
    $async = $client.BeginConnect("127.0.0.1", $Port, $null, $null)
    if (-not $async.AsyncWaitHandle.WaitOne($TimeoutMilliseconds)) {
      $client.Close()
      return $false
    }
    $client.EndConnect($async)
    $client.Close()
    return $true
  } catch {
    return $false
  }
}

function Wait-TcpPort {
  param([int]$Port, [int]$TimeoutSeconds = 30)
  $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
  while ((Get-Date) -lt $deadline) {
    if (Test-TcpPort -Port $Port) { return $true }
    Start-Sleep -Seconds 1
  }
  return $false
}

function Invoke-DockerMosquittoSub {
  param(
    [string]$Topic,
    [string]$OutFile,
    [int]$WaitSeconds
  )
  & docker exec $Container mosquitto_sub -h 127.0.0.1 -p 1883 -t $Topic -C 1 -W $WaitSeconds |
    Set-Content -LiteralPath $OutFile -Encoding UTF8
  return $LASTEXITCODE -eq 0
}

function Invoke-DockerMosquittoPub {
  param([string]$Topic, [string]$Message)
  $Message | & docker exec -i $Container mosquitto_pub -h 127.0.0.1 -p 1883 -t $Topic -q 1 -l
  return $LASTEXITCODE -eq 0
}

function Remove-MosquittoContainer {
  try {
    & docker rm -f $Container 2>$null | Out-Null
  } catch {
  }
}

try {
  if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
    Write-Report @{ status = "skipped"; reason = "no docker" }
    Write-Host "docker not available; cannot run mosquitto broker acceptance"
    exit 0
  }
  $dockerInfoOk = $false
  try {
    & docker info *> $null
    $dockerInfoOk = $LASTEXITCODE -eq 0
  } catch {
    $dockerInfoOk = $false
  }
  if (-not $dockerInfoOk) {
    Write-Report @{ status = "skipped"; reason = "docker api unavailable or permission denied" }
    Write-Host "docker API unavailable or permission denied; cannot run mosquitto broker acceptance"
    exit 0
  }

  $daemonBin = Join-Path $root "target\debug\reactor-edge-daemon.exe"
  if (-not (Test-Path -LiteralPath $daemonBin)) {
    & cargo build --bin reactor-edge-daemon
    if ($LASTEXITCODE -ne 0) { throw "cargo build --bin reactor-edge-daemon failed" }
  }
  if (-not (Test-Path -LiteralPath $daemonBin)) {
    Write-Report @{ status = "skipped"; reason = "daemon binary missing"; daemon = $daemonBin }
    Write-Host "daemon binary missing: $daemonBin"
    exit 0
  }

  @"
listener 1883 0.0.0.0
allow_anonymous true
"@ | Set-Content -LiteralPath $mosquittoConf -Encoding ASCII
  $containerConf = ($mosquittoConf -replace "\\", "/")
  Remove-MosquittoContainer
  & docker run -d --name $Container -p "$($BrokerPort):1883" `
    -v "$($containerConf):/mosquitto/config/mosquitto.conf:ro" `
    eclipse-mosquitto:2.0 *> $null
  if ($LASTEXITCODE -ne 0) { throw "failed to start mosquitto container" }
  if (-not (Wait-TcpPort -Port $BrokerPort -TimeoutSeconds 30)) {
    throw "mosquitto container did not expose broker port $BrokerPort"
  }

  New-Item -ItemType Directory -Force -Path $integrationDir | Out-Null
  @"
[mqtt]
enabled = true
host = "127.0.0.1"
port = $BrokerPort
use_tls = false
client_id = "xingshu-mqtt-acceptance"
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

  Remove-Item -LiteralPath $dbPath, "$dbPath-wal", "$dbPath-shm", "$dbPath-journal" -Force -ErrorAction SilentlyContinue
  $daemonProc = Start-Process -FilePath $daemonBin -ArgumentList @(
    "--config", "config/device.toml",
    "--safety", "config/safety.toml",
    "--memory", "config/ai_memory.toml",
    "--integration", $integrationTmp,
    "--db", $dbPath,
    "--assets", "auto",
    "--bind", "127.0.0.1:$DaemonPort",
    "--enable-test-reset"
  ) -WorkingDirectory $root -RedirectStandardOutput $daemonLog -RedirectStandardError "$daemonLog.err" -PassThru -WindowStyle Hidden
  if (-not (Wait-HttpOk -Url "http://127.0.0.1:$DaemonPort/health" -TimeoutSeconds 30)) {
    throw "daemon did not become healthy on $DaemonPort"
  }
  $engineerLogin = Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:$DaemonPort/api/auth/login" -ContentType "application/json" -Body (@{ username = "engineer"; password = "engineer123" } | ConvertTo-Json -Compress)
  $engineerToken = $engineerLogin.data.token
  if (-not $engineerToken) { throw "engineer login returned no token for simulator sample ingest" }

  $simProc = Start-Process -FilePath "node.exe" -ArgumentList @(
    "scripts/simulate-device.js",
    "--url", "http://127.0.0.1:$DaemonPort",
    "--token", $engineerToken,
    "--profile", "production",
    "--interval-ms", "1000"
  ) -WorkingDirectory $root -RedirectStandardOutput $simLog -RedirectStandardError "$simLog.err" -PassThru -WindowStyle Hidden
  Start-Sleep -Seconds 4

  $statusSubscribeOk = $false
  for ($i = 0; $i -lt 20; $i++) {
    if (Invoke-DockerMosquittoSub -Topic "xingshu/reactor_001/status" -OutFile $statusPayload -WaitSeconds 3) {
      $statusSubscribeOk = $true
      break
    }
    Start-Sleep -Seconds 1
  }

  $externalTaskId = "mqtt-acceptance-$([DateTimeOffset]::UtcNow.ToUnixTimeSeconds())"
  $taskPayload = @{
    external_task_id = $externalTaskId
    action = "set_targets"
    target_temperature_c = 60
    target_stirrer_rpm = 300
    target_shake_speed_cpm = 0
    reason = "mqtt broker acceptance"
  } | ConvertTo-Json -Compress

  $receiptJob = Start-Job -ScriptBlock {
    param($Container, $ReceiptPayload)
    docker exec $Container mosquitto_sub -h 127.0.0.1 -p 1883 -t "xingshu/reactor_001/task_receipts" -C 1 -W 15 |
      Set-Content -LiteralPath $ReceiptPayload -Encoding UTF8
    [pscustomobject]@{ exit_code = $LASTEXITCODE }
  } -ArgumentList $Container, $receiptPayload
  Start-Sleep -Seconds 1
  $taskPublishOk = Invoke-DockerMosquittoPub -Topic "xingshu/reactor_001/tasks" -Message $taskPayload
  Wait-Job -Job $receiptJob -Timeout 20 | Out-Null
  $receiptJobResult = Receive-Job -Job $receiptJob -ErrorAction SilentlyContinue
  $receiptSubscribeOk = $receiptJob.State -eq "Completed" -and $receiptJobResult.exit_code -eq 0
  Remove-Job -Job $receiptJob -Force -ErrorAction SilentlyContinue

  $payloadValidateOk = $false
  if ($statusSubscribeOk -and $receiptSubscribeOk -and (Test-Path -LiteralPath $statusPayload) -and (Test-Path -LiteralPath $receiptPayload)) {
    $statusJson = Get-Content -Raw -LiteralPath $statusPayload | ConvertFrom-Json
    $receiptJson = Get-Content -Raw -LiteralPath $receiptPayload | ConvertFrom-Json
    $payloadValidateOk = (
      $statusJson.device_id -eq "reactor_001" -and
      $statusJson.status -eq "online" -and
      $statusJson.task_topic -eq "xingshu/reactor_001/tasks" -and
      $receiptJson.ok -eq $true -and
      $receiptJson.source -eq "mqtt" -and
      $receiptJson.external_task_id -eq $externalTaskId -and
      $receiptJson.action -eq "set_targets" -and
      $receiptJson.status -eq "executed"
    )
  }

  $mqttLogLines = 0
  if (Test-Path -LiteralPath $daemonLog) {
    $mqttLogLines = (Select-String -LiteralPath $daemonLog -Pattern "MQTT bridge|mqtt" -SimpleMatch -ErrorAction SilentlyContinue).Count
  }
  $status = if ($statusSubscribeOk -and $taskPublishOk -and $receiptSubscribeOk -and $payloadValidateOk) { "ok" } else { "fail" }
  Write-Report @{
    status = $status
    broker = "mosquitto:2.0 (docker)"
    broker_port = $BrokerPort
    daemon_bound = "127.0.0.1:$DaemonPort"
    log_lines_with_mqtt_keyword = $mqttLogLines
    status_subscribe_ok = $statusSubscribeOk
    task_publish_ok = $taskPublishOk
    receipt_subscribe_ok = $receiptSubscribeOk
    payload_validate_ok = $payloadValidateOk
    status_payload = $statusPayload
    receipt_payload = $receiptPayload
    external_task_id = $externalTaskId
  }
  Write-Host "mqtt broker acceptance report -> $report"
  if ($status -ne "ok") {
    throw "mqtt broker acceptance failed: status_subscribe=$statusSubscribeOk task_publish=$taskPublishOk receipt_subscribe=$receiptSubscribeOk payload_validate=$payloadValidateOk"
  }
} finally {
  if ($simProc -and -not $simProc.HasExited) { Stop-ProcessTree -ProcessId $simProc.Id }
  if ($daemonProc -and -not $daemonProc.HasExited) { Stop-ProcessTree -ProcessId $daemonProc.Id }
  Remove-MosquittoContainer
}
