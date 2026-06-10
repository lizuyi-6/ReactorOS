param(
  [string]$XingshuBin = (Join-Path $PSScriptRoot "..\target\debug\xingshu.exe")
)

$ErrorActionPreference = "Stop"
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
$workDir = Join-Path $root "output\acceptance\backup-script"
$db = Join-Path $workDir "reactor.sqlite3"
$backupDir = Join-Path $workDir "backups"
$realBackup = Join-Path $backupDir "manual-vacuum.snapshot"

Remove-Item -LiteralPath $workDir -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $workDir, $backupDir | Out-Null

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

function Convert-ToBashPath {
  param(
    [string]$Path,
    [string]$BashPath
  )
  $resolved = if (Test-Path -LiteralPath $Path) {
    (Resolve-Path -LiteralPath $Path).Path
  } else {
    [System.IO.Path]::GetFullPath($Path)
  }
  if ($resolved -match '^([A-Za-z]):\\(.*)$') {
    $drive = $matches[1].ToLowerInvariant()
    $rest = $matches[2].Replace('\', '/')
    if ($BashPath -match '\\Windows\\System32\\bash\.exe$' -or $BashPath -match '\\WindowsApps\\bash\.exe$') {
      return "/mnt/$drive/$rest"
    }
    return "/$drive/$rest"
  }
  return $resolved.Replace('\', '/')
}

function Quote-Bash {
  param([string]$Value)
  return "'" + $Value.Replace("'", "'\''") + "'"
}

function Find-UsableBash {
  $candidates = @()
  foreach ($command in @(Get-Command bash -All -ErrorAction SilentlyContinue)) {
    if ($command.Source) { $candidates += $command.Source }
  }
  $candidates += @(
    "C:\Program Files\Git\bin\bash.exe",
    "C:\Program Files\Git\usr\bin\bash.exe",
    "C:\msys64\usr\bin\bash.exe"
  )

  foreach ($candidate in ($candidates | Where-Object { $_ } | Select-Object -Unique)) {
    if (-not (Test-Path -LiteralPath $candidate)) { continue }
    try {
      $output = & $candidate -lc "printf ok" 2>$null
      if ($LASTEXITCODE -eq 0 -and (($output | Out-String).Trim()) -eq "ok") {
        return $candidate
      }
    } catch {
      continue
    }
  }
  throw "usable bash is required to verify deploy/reactor-edge-backup.sh; install Git Bash or MSYS2 Bash"
}

if (-not (Test-Path -LiteralPath $XingshuBin)) {
  & cargo build --bin xingshu
  if ($LASTEXITCODE -ne 0) { throw "cargo build --bin xingshu failed" }
}
if (-not (Test-Path -LiteralPath $XingshuBin)) { throw "missing xingshu binary: $XingshuBin" }

& $XingshuBin --db $db data delete --yes --confirm-daemon-stopped | Out-Null
if ($LASTEXITCODE -ne 0) { throw "failed to create/migrate temporary database" }

& $XingshuBin --db $db ops backup --out $realBackup | Out-Host
if ($LASTEXITCODE -ne 0) { throw "xingshu ops backup failed" }
if (-not (Test-Path -LiteralPath $realBackup)) { throw "missing real backup: $realBackup" }
$realBytes = [System.IO.File]::ReadAllBytes($realBackup)
if ($realBytes.Length -le 16) { throw "real backup is too small" }
$realMagic = [System.Text.Encoding]::ASCII.GetString($realBytes[0..15])
if (-not $realMagic.StartsWith("SQLite format 3")) {
  throw "real backup is not SQLite: $realBackup"
}
if (-not (Test-Path -LiteralPath "$realBackup.sha256")) {
  throw "real backup missing sha256 sidecar"
}

$bash = Find-UsableBash

$mockBin = Join-Path $workDir "mock-xingshu.sh"
$mockLog = Join-Path $workDir "mock-xingshu-args.log"
$mockToolDir = Join-Path $workDir "mock-bin"
$mockSync = Join-Path $mockToolDir "sync"
$mockSyncLog = Join-Path $workDir "mock-sync.log"
New-Item -ItemType Directory -Force -Path $mockToolDir | Out-Null
$mockText = @'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$MOCK_XINGSHU_LOG"
out=""
prev=""
for arg in "$@"; do
  if [[ "$prev" == "--out" ]]; then
    out="$arg"
    break
  fi
  prev="$arg"
done
if [[ -z "$out" ]]; then
  echo "missing --out" >&2
  exit 2
fi
mkdir -p "$(dirname "$out")"
printf 'SQLite format 3\000mock-backup\n' > "$out"
sha256sum "$out" > "${out}.sha256"
'@
[System.IO.File]::WriteAllText($mockBin, $mockText, [System.Text.UTF8Encoding]::new($false))
$mockSyncText = @'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "sync" >> "$MOCK_SYNC_LOG"
'@
[System.IO.File]::WriteAllText($mockSync, $mockSyncText, [System.Text.UTF8Encoding]::new($false))

$bashScript = Convert-ToBashPath -Path (Join-Path $root "deploy\reactor-edge-backup.sh") -BashPath $bash
$mockBinWsl = Convert-ToBashPath -Path $mockBin -BashPath $bash
$mockToolDirWsl = Convert-ToBashPath -Path $mockToolDir -BashPath $bash
$mockSyncWsl = Convert-ToBashPath -Path $mockSync -BashPath $bash
$mockSyncLogWsl = Convert-ToBashPath -Path $mockSyncLog -BashPath $bash
$mockLogWsl = Convert-ToBashPath -Path $mockLog -BashPath $bash
$dbWsl = Convert-ToBashPath -Path $db -BashPath $bash
$backupDirWsl = Convert-ToBashPath -Path $backupDir -BashPath $bash
$lockWsl = "$backupDirWsl/.reactor-edge-backup.lock"

$cmd = "chmod +x $(Quote-Bash $mockBinWsl) $(Quote-Bash $mockSyncWsl) && " + (@(
    "MOCK_XINGSHU_LOG=$(Quote-Bash $mockLogWsl)",
    "MOCK_SYNC_LOG=$(Quote-Bash $mockSyncLogWsl)",
    "PATH=$(Quote-Bash $mockToolDirWsl):/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    "REACTOR_EDGE_DB=$(Quote-Bash $dbWsl)",
    "REACTOR_EDGE_BACKUP_DIR=$(Quote-Bash $backupDirWsl)",
    "REACTOR_EDGE_XINGSHU_BIN=$(Quote-Bash $mockBinWsl)",
    "REACTOR_EDGE_BACKUP_RETAIN_DAYS=30",
    "$(Quote-Bash $bashScript)"
  ) -join " ")
& $bash -lc $cmd
if ($LASTEXITCODE -ne 0) { throw "reactor-edge-backup.sh exited with $LASTEXITCODE" }

$snapshots = @(Get-ChildItem -LiteralPath $backupDir -Filter "reactor.sqlite3.*.snapshot" -File)
if ($snapshots.Count -ne 1) {
  throw "expected exactly one timestamped snapshot from backup.sh, found $($snapshots.Count)"
}
$snapshot = $snapshots[0]
if ($snapshot.Name -notmatch '^reactor\.sqlite3\.\d{8}-\d{6}\.snapshot$') {
  throw "snapshot name is not timestamped: $($snapshot.Name)"
}
if (-not (Test-Path -LiteralPath "$($snapshot.FullName).sha256")) {
  throw "timestamped snapshot missing sha256 sidecar"
}
$latest = Join-Path $backupDir "latest.snapshot"
if (-not (Test-Path -LiteralPath $latest)) { throw "missing latest snapshot link/file" }
$latestSha = Join-Path $backupDir "latest.snapshot.sha256"
if (-not (Test-Path -LiteralPath $latestSha)) { throw "missing latest snapshot sha256 link/file" }
if (Get-ChildItem -LiteralPath $backupDir -Filter "*.tmp.*" -File) {
  throw "backup script left temporary files in backup directory"
}
$sidecarLine = Get-Content -LiteralPath "$($snapshot.FullName).sha256" -Raw
if ($sidecarLine -match "\.tmp\.") {
  throw "published sha256 sidecar still references temporary path: $sidecarLine"
}
$mockArgs = Get-Content -LiteralPath $mockLog -Raw
if ($mockArgs -notmatch "--db" -or $mockArgs -notmatch "ops backup" -or $mockArgs -notmatch "--out") {
  throw "mock xingshu did not receive expected backup args: $mockArgs"
}
if ($mockArgs -notmatch "\.tmp\.") {
  throw "backup script did not write through a temporary snapshot path: $mockArgs"
}
$syncCalls = @(Get-Content -LiteralPath $mockSyncLog)
if ($syncCalls.Count -lt 2) {
  throw "backup script did not call sync after publishing snapshot and latest links"
}

$contentBeforeLockCheck = @(Get-ChildItem -LiteralPath $backupDir -Filter "reactor.sqlite3.*.snapshot" -File).Count
$lockedCmd = "flock -n $(Quote-Bash $lockWsl) -c " + (Quote-Bash ((@(
      "MOCK_XINGSHU_LOG=$(Quote-Bash $mockLogWsl)",
      "MOCK_SYNC_LOG=$(Quote-Bash $mockSyncLogWsl)",
      "PATH=$(Quote-Bash $mockToolDirWsl):/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
      "REACTOR_EDGE_DB=$(Quote-Bash $dbWsl)",
      "REACTOR_EDGE_BACKUP_DIR=$(Quote-Bash $backupDirWsl)",
      "REACTOR_EDGE_XINGSHU_BIN=$(Quote-Bash $mockBinWsl)",
      "REACTOR_EDGE_BACKUP_RETAIN_DAYS=30",
      "$(Quote-Bash $bashScript)"
    ) -join " ")))
& $bash -lc $lockedCmd
if ($LASTEXITCODE -ne 75) {
  throw "backup script did not fail with 75 while lock was held; exit=$LASTEXITCODE"
}
$contentAfterLockCheck = @(Get-ChildItem -LiteralPath $backupDir -Filter "reactor.sqlite3.*.snapshot" -File).Count
if ($contentAfterLockCheck -ne $contentBeforeLockCheck) {
  throw "locked backup attempt wrote a new timestamped snapshot"
}

Write-Host "production backup script ok: $($snapshot.FullName)"
