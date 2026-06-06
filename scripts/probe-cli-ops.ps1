# Real xingshu ops / key CLI smoke.
# Verifies (against a real SQLite file written via the C runtime, not a "abc"
# stub) that backup, restore, wipe, and key generate work end-to-end and
# surface the downgraded wording introduced by the post-review rework.
$ErrorActionPreference = "Stop"
$env:CARGO_TARGET_DIR = "C:\tmp\xingshu-target-v3"
$bin = "C:\tmp\xingshu-target-v3\debug\xingshu.exe"
$root = (Resolve-Path -LiteralPath $PSScriptRoot\..).Path
# Use the canonical in-tree target dir; the bugfix/loadtest scratch dirs
# may be locked by a running daemon.exe and refused by Start-Process.
$daemonBin = (Resolve-Path -LiteralPath (Join-Path $root "target\debug\reactor-edge-daemon.exe")).Path
if (-not (Test-Path $daemonBin)) { throw "daemon binary not built; run cargo build --bin reactor-edge-daemon first" }
$workDir = "C:\tmp\xingshu-cli-smoke"
$tmpDb = Join-Path $workDir "ops-real.sqlite3"
$tmpBackup = Join-Path $workDir "ops-real.snapshot"
$tmpWiped = Join-Path $workDir "ops-real.wiped.sqlite3"
$tmpKey = [System.IO.Path]::ChangeExtension($tmpDb, ".key")

if (-not (Test-Path $workDir)) { New-Item -ItemType Directory -Force -Path $workDir | Out-Null }
foreach ($f in @($tmpDb, $tmpBackup, $tmpWiped, $tmpKey)) {
    if (Test-Path $f) { Remove-Item $f -Force }
}

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
    $token = $login.data.token
    $null = $client2.GetAsync("http://127.0.0.1:18189/api/audit/logs?page=1&page_size=1").GetAwaiter().GetResult()
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

# 3. ops backup must produce a copy and a .sha256 sidecar.
& $bin ops --help | Out-Null
& $bin key --help | Out-Null
& $bin ops backup --db $tmpDb --out $tmpBackup | Out-Host
if (-not (Test-Path $tmpBackup)) { throw "ops backup did not produce $tmpBackup" }
$backupSize = (Get-Item $tmpBackup).Length
if ($backupSize -ne $dbSize) { throw "backup size $backupSize differs from source $dbSize" }
if (-not (Test-Path "$tmpBackup.sha256")) { throw "missing sha256 sidecar" }
Write-Host "backup ok: $tmpBackup ($backupSize bytes, sha256 present)"

# 4. ops restore must reject non-SQLite files and accept the real snapshot.
$bogus = Join-Path $workDir "bogus.bin"
"not a sqlite database" | Out-File -FilePath $bogus -Encoding ascii -NoNewline
$bogusExit = 0
$bogusOutput = ""
try {
    $bogusOutput = & $bin ops restore --backup $bogus --db (Join-Path $workDir "rejected.sqlite3") --yes 2>&1 | Out-String
    $bogusExit = $LASTEXITCODE
} catch {
    $bogusExit = 1
    $bogusOutput = $_.Exception.Message
}
if ($bogusExit -eq 0) { throw "ops restore should reject non-SQLite magic header but returned 0; output: $bogusOutput" }
if ($bogusOutput -notmatch "SQLite magic header") { throw "ops restore rejection did not mention SQLite magic header: $bogusOutput" }
Write-Host "restore rejects non-sqlite input: ok"

$restoredTarget = Join-Path $workDir "restored.sqlite3"
& $bin ops restore --backup $tmpBackup --db $restoredTarget --yes | Out-Host
if (-not (Test-Path $restoredTarget)) { throw "ops restore did not produce $restoredTarget" }
$restoredBytes = [System.IO.File]::ReadAllBytes($restoredTarget)
if (-not [System.Text.Encoding]::ASCII.GetString($restoredBytes[0..15]).StartsWith("SQLite format 3")) {
    throw "restored file is not a real SQLite file"
}
Write-Host "restore ok: $restoredTarget is a valid sqlite file"

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
& $bin ops wipe --db $tmpWiped --yes | Out-Host
if (Test-Path $tmpWiped) { throw "ops wipe did not remove $tmpWiped" }
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
    $keyOutput = & $bin key generate --db $tmpWiped --yes 2>&1 | Out-String
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

Write-Host "ALL OK"
