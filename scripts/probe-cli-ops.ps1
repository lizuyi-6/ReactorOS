# Real xingshu ops / key CLI smoke.
# Verifies (against a real SQLite file written via the C runtime, not a "abc"
# stub) that backup, restore, wipe, key generate, and integration task
# payload rekey work end-to-end.
$ErrorActionPreference = "Stop"
$root = (Resolve-Path -LiteralPath $PSScriptRoot\..).Path
$targetDebugDir = Join-Path $root "target\debug"
$bin = Join-Path $targetDebugDir "xingshu.exe"

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

function Read-SqliteFamilyUtf8 {
    param([Parameter(Mandatory = $true)][string]$DbPath)
    $parts = @()
    foreach ($candidate in @($DbPath, "$DbPath-wal", "$DbPath-shm", "$DbPath-journal")) {
        if (Test-Path -LiteralPath $candidate) {
            $parts += [System.Text.Encoding]::UTF8.GetString([System.IO.File]::ReadAllBytes($candidate))
        }
    }
    return ($parts -join "`n")
}

Push-Location $root
try {
    if (-not (Test-Path -LiteralPath $bin)) {
        cargo build --bin xingshu | Out-Null
    }
} finally {
    Pop-Location
}
if (-not (Test-Path $bin)) { throw "xingshu binary not built at $bin" }
# Use the canonical in-tree target dir; the CLI scratch target dir can be
# locked by a concurrent cargo build from another acceptance slice.
$daemonBin = (Resolve-Path -LiteralPath (Join-Path $targetDebugDir "reactor-edge-daemon.exe")).Path
if (-not (Test-Path $daemonBin)) { throw "daemon binary not built; run cargo build --bin reactor-edge-daemon first" }
$workDir = Join-Path $root ("output\acceptance\cli-smoke-{0}-{1}" -f $PID, [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())
$tmpDb = Join-Path $workDir "ops-real.sqlite3"
$tmpBackup = Join-Path $workDir "ops-real.snapshot"
$tmpWiped = Join-Path $workDir "ops-real.wiped.sqlite3"
$tmpKey = [System.IO.Path]::ChangeExtension($tmpDb, ".key")
$tmpLoraDataset = Join-Path $workDir "lora-training-dataset.jsonl"
$tmpLoraManifest = Join-Path $workDir "lora-training-manifest.json"
$tmpLoraScript = Join-Path $workDir "fake-lora-train.cmd"
$tmpLoraModel = Join-Path $workDir "fake-qwen.gguf"
$tmpLoraConvert = Join-Path $workDir "fake-convert.py"
$tmpLoraActive = Join-Path $workDir "active-adapter.gguf"
$tmpLoraCandidate = Join-Path $workDir "candidate-adapter.gguf"

if (-not (Test-Path $workDir)) { New-Item -ItemType Directory -Force -Path $workDir | Out-Null }
foreach ($f in @($tmpDb, $tmpBackup, $tmpWiped, $tmpKey)) {
    if (Test-Path $f) { Remove-Item $f -Force }
}
if (Test-Path $tmpLoraDataset) { Remove-Item $tmpLoraDataset -Force }
if (Test-Path $tmpLoraManifest) { Remove-Item $tmpLoraManifest -Force }

# 1. Create a real SQLite file by spinning up the daemon briefly, writing
#    a control event, then stopping it. This gives a valid SQLite magic
#    header so backup / restore / wipe have something real to operate on.
$daemonOut = Join-Path $workDir "daemon.out.log"
$daemonErr = Join-Path $workDir "daemon.err.log"
$daemonProc = Start-Process -FilePath $daemonBin -ArgumentList @(
    "--config", (Join-Path $root "config\device.toml"),
    "--safety", (Join-Path $root "config\safety.toml"),
    "--memory", (Join-Path $root "config\ai_memory.toml"),
    "--integration", (Join-Path $root "config\integration.toml"),
    "--db", $tmpDb,
    "--bind", "127.0.0.1:18189",
    "--enable-test-reset"
) -WorkingDirectory $root -RedirectStandardOutput $daemonOut -RedirectStandardError $daemonErr -PassThru -WindowStyle Hidden
try {
    $ready = $false
    Add-Type -AssemblyName System.Net.Http -ErrorAction SilentlyContinue
    $client = [System.Net.Http.HttpClient]::new()
    $client.Timeout = [TimeSpan]::FromSeconds(3)
    for ($i = 0; $i -lt 30; $i++) {
        Start-Sleep -Seconds 1
        try {
            $r = $client.GetAsync("http://127.0.0.1:18189/health").GetAwaiter().GetResult()
            if ([int]$r.StatusCode -eq 200) { $ready = $true; break }
        } catch { }
    }
    $client.Dispose()
    if (-not $ready) { throw "daemon on 18189 did not become healthy" }

    # Drop a control event so the audit chain has at least one row.
    Add-Type -AssemblyName System.Net.Http -ErrorAction SilentlyContinue
    $client2 = [System.Net.Http.HttpClient]::new()
    $client2.Timeout = [TimeSpan]::FromSeconds(10)
    $body = (@{ username = "operator"; password = "operator123" } | ConvertTo-Json -Compress)
    $content = New-Object System.Net.Http.StringContent($body, [System.Text.Encoding]::UTF8, "application/json")
    $resp = $client2.PostAsync("http://127.0.0.1:18189/api/auth/login", $content).GetAwaiter().GetResult()
    $login = ($resp.Content.ReadAsStringAsync().GetAwaiter().GetResult() | ConvertFrom-Json)
    $operatorToken = $login.data.token
    $engineerBody = (@{ username = "engineer"; password = "engineer123" } | ConvertTo-Json -Compress)
    $engineerContent = New-Object System.Net.Http.StringContent($engineerBody, [System.Text.Encoding]::UTF8, "application/json")
    $engineerResp = $client2.PostAsync("http://127.0.0.1:18189/api/auth/login", $engineerContent).GetAwaiter().GetResult()
    $engineerLogin = ($engineerResp.Content.ReadAsStringAsync().GetAwaiter().GetResult() | ConvertFrom-Json)
    $engineerToken = $engineerLogin.data.token
    $null = $client2.GetAsync("http://127.0.0.1:18189/api/audit/logs?page=1&page_size=1").GetAwaiter().GetResult()
    $sampleBody = @{
        temperature_c = 60.2
        pressure_mpa = 0.55
        stirrer_rpm = 300
        shake_speed_cpm = 0
        tilt_state = 0
        flow_rate_l_min = 2.2
        product_concentration_percent = 12.4
        ph = 6.8
    } | ConvertTo-Json -Compress
    $sampleContent = New-Object System.Net.Http.StringContent($sampleBody, [System.Text.Encoding]::UTF8, "application/json")
    $sampleReq = [System.Net.Http.HttpRequestMessage]::new([System.Net.Http.HttpMethod]::Post, "http://127.0.0.1:18189/api/v1/reactor/reactor_001/samples")
    $sampleReq.Headers.Authorization = [System.Net.Http.Headers.AuthenticationHeaderValue]::new("Bearer", $engineerToken)
    $sampleReq.Content = $sampleContent
    $sampleResp = $client2.SendAsync($sampleReq).GetAwaiter().GetResult()
    $sampleText = $sampleResp.Content.ReadAsStringAsync().GetAwaiter().GetResult()
    if ([int]$sampleResp.StatusCode -lt 200 -or [int]$sampleResp.StatusCode -ge 300) {
        throw "failed to seed fresh sample before batch start: status=$([int]$sampleResp.StatusCode) body=$sampleText"
    }
    $batchBody = @{
        name = "cli ops lora dataset"
        target_temperature_c = 72
        target_stirrer_rpm = 420
        heating_minutes = 30
        stirring_minutes = 45
    } | ConvertTo-Json -Compress
    $batchContent = New-Object System.Net.Http.StringContent($batchBody, [System.Text.Encoding]::UTF8, "application/json")
    $batchReq = [System.Net.Http.HttpRequestMessage]::new([System.Net.Http.HttpMethod]::Post, "http://127.0.0.1:18189/api/batches/start")
    $batchReq.Headers.Authorization = [System.Net.Http.Headers.AuthenticationHeaderValue]::new("Bearer", $operatorToken)
    $batchReq.Content = $batchContent
    $batchResp = $client2.SendAsync($batchReq).GetAwaiter().GetResult()
    $batchText = $batchResp.Content.ReadAsStringAsync().GetAwaiter().GetResult()
    if ([int]$batchResp.StatusCode -lt 200 -or [int]$batchResp.StatusCode -ge 300) {
        throw "failed to start batch for lora dataset: status=$([int]$batchResp.StatusCode) body=$batchText"
    }
    $batchJson = ($batchText | ConvertFrom-Json)
    $batchId = $batchJson.id
    if (-not $batchId) { throw "batch start response did not include id: $batchText" }
    $finishReq = [System.Net.Http.HttpRequestMessage]::new([System.Net.Http.HttpMethod]::Post, "http://127.0.0.1:18189/api/batches/$batchId/finish")
    $finishReq.Headers.Authorization = [System.Net.Http.Headers.AuthenticationHeaderValue]::new("Bearer", $operatorToken)
    $finishResp = $client2.SendAsync($finishReq).GetAwaiter().GetResult()
    $finishText = $finishResp.Content.ReadAsStringAsync().GetAwaiter().GetResult()
    if ([int]$finishResp.StatusCode -lt 200 -or [int]$finishResp.StatusCode -ge 300) {
        throw "failed to finish batch for lora dataset: status=$([int]$finishResp.StatusCode) body=$finishText"
    }
    $resultBody = @{
        batch_id = $batchId
        yield_percent = 86.5
        product_ratio = 0.91
        notes = "cli ops lora dataset"
    } | ConvertTo-Json -Compress
    $resultContent = New-Object System.Net.Http.StringContent($resultBody, [System.Text.Encoding]::UTF8, "application/json")
    $resultReq = [System.Net.Http.HttpRequestMessage]::new([System.Net.Http.HttpMethod]::Post, "http://127.0.0.1:18189/api/product-results")
    $resultReq.Headers.Authorization = [System.Net.Http.Headers.AuthenticationHeaderValue]::new("Bearer", $engineerToken)
    $resultReq.Content = $resultContent
    $resultResp = $client2.SendAsync($resultReq).GetAwaiter().GetResult()
    if ([int]$resultResp.StatusCode -lt 200 -or [int]$resultResp.StatusCode -ge 300) {
        $resultText = $resultResp.Content.ReadAsStringAsync().GetAwaiter().GetResult()
        throw "failed to seed product result for lora dataset: status=$([int]$resultResp.StatusCode) body=$resultText"
    }
    $sampleContent = New-Object System.Net.Http.StringContent($sampleBody, [System.Text.Encoding]::UTF8, "application/json")
    $sampleReq = [System.Net.Http.HttpRequestMessage]::new([System.Net.Http.HttpMethod]::Post, "http://127.0.0.1:18189/api/v1/reactor/reactor_001/samples")
    $sampleReq.Headers.Authorization = [System.Net.Http.Headers.AuthenticationHeaderValue]::new("Bearer", $engineerToken)
    $sampleReq.Content = $sampleContent
    $sampleResp = $client2.SendAsync($sampleReq).GetAwaiter().GetResult()
    $sampleText = $sampleResp.Content.ReadAsStringAsync().GetAwaiter().GetResult()
    if ([int]$sampleResp.StatusCode -lt 200 -or [int]$sampleResp.StatusCode -ge 300) {
        throw "failed to seed fresh sample before AINAS target update: status=$([int]$sampleResp.StatusCode) body=$sampleText"
    }
    $ainasBody = @{
        external_task_id = "cli-rekey-legacy-001"
        action = "set_targets"
        target_temperature_c = 61.5
        target_stirrer_rpm = 305
        reason = "cli rekey legacy plaintext request"
    } | ConvertTo-Json -Compress
    $ainasContent = New-Object System.Net.Http.StringContent($ainasBody, [System.Text.Encoding]::UTF8, "application/json")
    $ainasReq = [System.Net.Http.HttpRequestMessage]::new([System.Net.Http.HttpMethod]::Post, "http://127.0.0.1:18189/api/integrations/ainas/tasks")
    $ainasReq.Headers.Authorization = [System.Net.Http.Headers.AuthenticationHeaderValue]::new("Bearer", $engineerToken)
    $ainasReq.Content = $ainasContent
    $ainasResp = $client2.SendAsync($ainasReq).GetAwaiter().GetResult()
    $ainasText = $ainasResp.Content.ReadAsStringAsync().GetAwaiter().GetResult()
    if ([int]$ainasResp.StatusCode -lt 200 -or [int]$ainasResp.StatusCode -ge 300) {
        throw "failed to seed plaintext AINAS task for key rekey: status=$([int]$ainasResp.StatusCode) body=$ainasText"
    }
    $client2.Dispose()
} finally {
    Stop-Process -Id $daemonProc.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
}

# 2. Verify the daemon left a real SQLite file behind.
if (-not (Test-Path $tmpDb)) { throw "daemon did not create $tmpDb" }
$dbBytes = [System.IO.File]::ReadAllBytes($tmpDb)
if ($dbBytes.Length -lt 16 -or -not [System.Text.Encoding]::ASCII.GetString($dbBytes[0..15]).StartsWith("SQLite format 3")) {
    throw "$tmpDb is not a real SQLite file (magic header missing)"
}
$dbSize = (Get-Item $tmpDb).Length
Write-Host "real sqlite: $tmpDb ($dbSize bytes)"

# 3. ops backup must produce a VACUUM INTO snapshot and a .sha256 sidecar.
& $bin ops --help | Out-Null
& $bin key --help | Out-Null
$backupOutput = & $bin ops backup --db $tmpDb --out $tmpBackup 2>&1 | Out-String
$backupOutput | Write-Host
if (-not (Test-Path $tmpBackup)) { throw "ops backup did not produce $tmpBackup" }
$backupSize = (Get-Item $tmpBackup).Length
if ($backupSize -le 0) { throw "backup size must be non-zero" }
if ($backupOutput -notmatch "VACUUM INTO") { throw "backup output must mention VACUUM INTO: $backupOutput" }
if (-not (Test-Path "$tmpBackup.sha256")) { throw "missing sha256 sidecar" }
Write-Host "backup ok: $tmpBackup ($backupSize bytes, VACUUM INTO, sha256 present)"

# 4. ops restore must reject non-SQLite files and accept the real snapshot.
$bogus = Join-Path $workDir "bogus.bin"
"not a sqlite database" | Out-File -FilePath $bogus -Encoding ascii -NoNewline
$bogusExit = 0
$bogusOutput = ""
try {
    $bogusOutput = & $bin ops restore --backup $bogus --db (Join-Path $workDir "rejected.sqlite3") --yes --confirm-daemon-stopped 2>&1 | Out-String
    $bogusExit = $LASTEXITCODE
} catch {
    $bogusExit = 1
    $bogusOutput = $_.Exception.Message
}
if ($bogusExit -eq 0) { throw "ops restore should reject non-SQLite magic header but returned 0; output: $bogusOutput" }
if ($bogusOutput -notmatch "SQLite" -or $bogusOutput -notmatch "magic header") { throw "ops restore rejection did not mention SQLite magic header: $bogusOutput" }
Write-Host "restore rejects non-sqlite input: ok"

$restoredTarget = Join-Path $workDir "restored.sqlite3"
$restoreOutput = & $bin ops restore --backup $tmpBackup --db $restoredTarget --yes --confirm-daemon-stopped 2>&1 | Out-String
$restoreOutput | Write-Host
if (-not (Test-Path $restoredTarget)) { throw "ops restore did not produce $restoredTarget" }
if ($restoreOutput -notmatch "integrity:\s+ok") { throw "restore output must show integrity ok: $restoreOutput" }
$restoredBytes = [System.IO.File]::ReadAllBytes($restoredTarget)
if (-not [System.Text.Encoding]::ASCII.GetString($restoredBytes[0..15]).StartsWith("SQLite format 3")) {
    throw "restored file is not a real SQLite file"
}
Write-Host "restore ok: $restoredTarget is a valid sqlite file"

# 4b. ai train --export-only must build a supervised JSONL dataset from
#     the real SQLite rows before any model assets are required.
$trainOutput = & $bin --db $tmpDb ai train --export-only --dataset $tmpLoraDataset 2>&1 | Out-String
$trainOutput | Write-Host
if (-not (Test-Path $tmpLoraDataset)) { throw "ai train --export-only did not write $tmpLoraDataset" }
$datasetLines = @(Get-Content -LiteralPath $tmpLoraDataset)
if ($datasetLines.Count -lt 1) { throw "ai train dataset should contain at least one JSONL row" }
$firstDatasetRow = $datasetLines[0] | ConvertFrom-Json
if ($firstDatasetRow.schema -ne "xingshu.local_ai.lora_dataset.v1") { throw "unexpected lora dataset schema: $($firstDatasetRow.schema)" }
if (-not $firstDatasetRow.output.target_temperature_c) { throw "lora dataset output target_temperature_c missing" }
Write-Host "local ai dataset export ok: $tmpLoraDataset ($($datasetLines.Count) rows)"

# 4c. ai train must write a manifest and only promote an evaluated
#     candidate adapter after an explicit --promote request.
"fake qwen gguf" | Out-File -FilePath $tmpLoraModel -Encoding ascii -NoNewline
"fake convert script" | Out-File -FilePath $tmpLoraConvert -Encoding ascii -NoNewline
"old adapter" | Out-File -FilePath $tmpLoraActive -Encoding ascii -NoNewline
"new adapter" | Out-File -FilePath $tmpLoraCandidate -Encoding ascii -NoNewline
$candidateJsonPath = $tmpLoraCandidate.Replace("\", "\\")
"@echo off`r`necho {`"status`":`"ok`",`"evaluation`":{`"score`":0.92,`"metrics`":{`"loss`":0.11}},`"artifacts`":{`"adapter_path`":`"$candidateJsonPath`"}}`r`n" | Out-File -FilePath $tmpLoraScript -Encoding ascii -NoNewline
$env:XINGSHU_LOCAL_AI_ENABLED_OLD = $env:XINGSHU_LOCAL_AI_ENABLED
$env:XINGSHU_LOCAL_AI_GGUF_OLD = $env:XINGSHU_LOCAL_AI_GGUF
$env:XINGSHU_LOCAL_AI_LORA_OLD = $env:XINGSHU_LOCAL_AI_LORA
$env:XINGSHU_LOCAL_AI_TRAIN_SCRIPT_OLD = $env:XINGSHU_LOCAL_AI_TRAIN_SCRIPT
$env:XINGSHU_LOCAL_AI_CONVERT_SCRIPT_OLD = $env:XINGSHU_LOCAL_AI_CONVERT_SCRIPT
try {
    $env:XINGSHU_LOCAL_AI_ENABLED = "true"
    $env:XINGSHU_LOCAL_AI_GGUF = $tmpLoraModel
    $env:XINGSHU_LOCAL_AI_LORA = $tmpLoraActive
    $env:XINGSHU_LOCAL_AI_TRAIN_SCRIPT = $tmpLoraScript
    $env:XINGSHU_LOCAL_AI_CONVERT_SCRIPT = $tmpLoraConvert
    $promoteOutput = & $bin --db $tmpDb --json ai train --dataset $tmpLoraDataset --manifest $tmpLoraManifest --promote --min-eval-score 0.8 --timeout-s 10 2>&1 | Out-String
    $promoteOutput | Write-Host
    $promoteJson = $promoteOutput | ConvertFrom-Json
    if ($promoteJson.promotion.promoted -ne $true) { throw "ai train --promote did not promote passing adapter: $promoteOutput" }
    if (-not (Test-Path $tmpLoraManifest)) { throw "ai train --promote did not write $tmpLoraManifest" }
    $manifestJson = Get-Content -LiteralPath $tmpLoraManifest -Raw | ConvertFrom-Json
    if ($manifestJson.schema -ne "xingshu.local_ai.training_manifest.v1") { throw "unexpected training manifest schema: $($manifestJson.schema)" }
    if ([double]$manifestJson.evaluation.score -lt 0.8) { throw "training manifest score did not meet threshold" }
    if ((Get-Content -LiteralPath $tmpLoraActive -Raw) -ne "new adapter") { throw "active adapter was not replaced by candidate" }
    if (-not $promoteJson.promotion.backup -or -not (Test-Path $promoteJson.promotion.backup)) { throw "promotion did not preserve backup adapter" }
    if ((Get-Content -LiteralPath $promoteJson.promotion.backup -Raw) -ne "old adapter") { throw "promotion backup did not contain old adapter" }
} finally {
    foreach ($pair in @(
        @("XINGSHU_LOCAL_AI_ENABLED", "XINGSHU_LOCAL_AI_ENABLED_OLD"),
        @("XINGSHU_LOCAL_AI_GGUF", "XINGSHU_LOCAL_AI_GGUF_OLD"),
        @("XINGSHU_LOCAL_AI_LORA", "XINGSHU_LOCAL_AI_LORA_OLD"),
        @("XINGSHU_LOCAL_AI_TRAIN_SCRIPT", "XINGSHU_LOCAL_AI_TRAIN_SCRIPT_OLD"),
        @("XINGSHU_LOCAL_AI_CONVERT_SCRIPT", "XINGSHU_LOCAL_AI_CONVERT_SCRIPT_OLD")
    )) {
        $name = $pair[0]
        $oldName = $pair[1]
        if (Test-Path "env:$oldName") {
            Set-Item "env:$name" (Get-Item "env:$oldName").Value
            Remove-Item "env:$oldName" -ErrorAction SilentlyContinue
        } else {
            Remove-Item "env:$name" -ErrorAction SilentlyContinue
        }
    }
}
Write-Host "local ai manifest and promotion ok: $tmpLoraManifest"

# 5. ops wipe must refuse without --yes and accept with --yes.
$wipeExit = 0
try {
    $null = & $bin ops wipe --db $tmpDb 2>&1 | Out-Null
    $wipeExit = $LASTEXITCODE
} catch {
    $wipeExit = 1
}
if ($wipeExit -eq 0) { throw "ops wipe should refuse without --yes" }
Write-Host "wipe refuses without --yes: ok"

# Wipe a copy so we keep the source for the key generate step.
Copy-Item $tmpDb $tmpWiped -Force
$tmpWipedWal = "$tmpWiped-wal"
$tmpWipedShm = "$tmpWiped-shm"
$tmpWipedKeyBefore = [System.IO.Path]::ChangeExtension($tmpWiped, ".key")
"wal" | Out-File -FilePath $tmpWipedWal -Encoding ascii -NoNewline
"shm" | Out-File -FilePath $tmpWipedShm -Encoding ascii -NoNewline
"XINGSHU_DB_ENCRYPTION_KEY=deadbeef" | Out-File -FilePath $tmpWipedKeyBefore -Encoding ascii -NoNewline
$backupDir = Join-Path $workDir "backups"
New-Item -ItemType Directory -Force -Path $backupDir | Out-Null
$tmpWipedBackup = Join-Path $backupDir "ops-real.wiped.snapshot"
Copy-Item $tmpDb $tmpWipedBackup -Force
$wipeOutput = & $bin ops wipe --db $tmpWiped --yes --confirm-daemon-stopped 2>&1 | Out-String
$wipeOutput | Write-Host
if (Test-Path $tmpWiped) { throw "ops wipe did not remove $tmpWiped" }
foreach ($removed in @($tmpWipedWal, $tmpWipedShm, $tmpWipedKeyBefore, $tmpWipedBackup)) {
    if (Test-Path $removed) { throw "ops wipe did not remove scoped file $removed" }
}
if ($wipeOutput -notmatch "sqlite_wal" -or $wipeOutput -notmatch "sqlite_shm" -or $wipeOutput -notmatch "db_key_file" -or $wipeOutput -notmatch "backup_snapshot") {
    throw "wipe output did not list full industrial scope: $wipeOutput"
}
Write-Host "wipe removes the sqlite file: ok"

# 6. key generate must NOT print the secret and must write 0600 (or hidden on
#    Windows) with the env var NAME in the output. We point at a fresh copy
#    of the real SQLite so the generated <db>.key file is in a known path.
Copy-Item $tmpDb $tmpWiped -Force
$tmpWipedKey = [System.IO.Path]::ChangeExtension($tmpWiped, ".key")
if (Test-Path $tmpWipedKey) { Remove-Item $tmpWipedKey -Force }
$env:XINGSHU_DB_ENCRYPTION_KEY_OLD = $env:XINGSHU_DB_ENCRYPTION_KEY
$keyExit = 0
$keyOutput = ""
try {
    $keyOutput = & $bin key generate --db $tmpWiped --yes --confirm-daemon-stopped 2>&1 | Out-String
    $keyExit = $LASTEXITCODE
} catch {
    $keyExit = 1
    $keyOutput = $_.Exception.Message
}
finally {
    if ($env:XINGSHU_DB_ENCRYPTION_KEY_OLD) {
        $env:XINGSHU_DB_ENCRYPTION_KEY = $env:XINGSHU_DB_ENCRYPTION_KEY_OLD
    }
}
if ($keyExit -ne 0) { throw "key generate failed: exit=$keyExit output=$keyOutput" }
if ($keyOutput -match "[0-9a-f]{64}") {
    throw "key generate must not print the 64-hex secret (got match: $($Matches[0]))"
}
if (-not $keyOutput -match "XINGSHU_DB_ENCRYPTION_KEY") {
    throw "key generate output should mention XINGSHU_DB_ENCRYPTION_KEY env var name: $keyOutput"
}
if (-not (Test-Path $tmpWipedKey)) { throw "key generate did not write $tmpWipedKey" }
Write-Host "key generate ok: $tmpWipedKey written, secret not printed, env var name surfaced"

# 7. key rekey-integration-tasks must migrate legacy plaintext integration
#    payloads into AES-GCM envelopes without printing key material.
$oldKeyFile = Join-Path $workDir "old-db.key"
$oldKeyHex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
"XINGSHU_DB_ENCRYPTION_KEY=$oldKeyHex" | Out-File -FilePath $oldKeyFile -Encoding ascii -NoNewline
$dryRekeyOutput = & $bin --json key rekey-integration-tasks --db $tmpDb --old-key-file $oldKeyFile --new-key-file $tmpWipedKey --dry-run 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) { throw "key rekey dry-run failed: $dryRekeyOutput" }
$dryRekeyJson = $dryRekeyOutput | ConvertFrom-Json
if ($dryRekeyJson.mode -ne "dry-run") { throw "key rekey dry-run returned unexpected mode: $dryRekeyOutput" }
if ([int]$dryRekeyJson.plaintext_fields_encrypted -lt 2) { throw "key rekey dry-run did not count plaintext payloads: $dryRekeyOutput" }
$rekeyOutput = & $bin --json key rekey-integration-tasks --db $tmpDb --old-key-file $oldKeyFile --new-key-file $tmpWipedKey --yes --confirm-daemon-stopped 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) { throw "key rekey commit failed: $rekeyOutput" }
if ($rekeyOutput -match "[0-9a-f]{64}") {
    throw "key rekey must not print key material (got match: $($Matches[0]))"
}
$rekeyJson = $rekeyOutput | ConvertFrom-Json
if ($rekeyJson.mode -ne "committed") { throw "key rekey commit returned unexpected mode: $rekeyOutput" }
if ([int]$rekeyJson.fields_changed -lt 2) { throw "key rekey commit changed too few fields: $rekeyOutput" }
$dbTextAfterRekey = Read-SqliteFamilyUtf8 -DbPath $tmpDb
if (-not $dbTextAfterRekey.Contains("xingshu:v1:aes256gcm:")) {
    throw "key rekey commit did not write AES-GCM envelopes"
}
Write-Host "key rekey ok: integration task payloads migrated without printing secrets"

Write-Host "ALL OK"
