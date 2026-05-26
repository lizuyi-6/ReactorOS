param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$Args
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

if (-not (Get-Command wsl -ErrorAction SilentlyContinue)) {
    throw "WSL was not found. Install WSL or run scripts/run-lubancat2-qemu.sh inside Linux."
}

function ConvertTo-WslPath([string]$Path) {
    $resolved = (Resolve-Path $Path).Path
    if ($resolved -match '^([A-Za-z]):\\(.*)$') {
        $drive = $matches[1].ToLowerInvariant()
        $rest = $matches[2] -replace '\\', '/'
        return "/mnt/$drive/$rest"
    }

    $converted = (& wsl.exe -e wslpath -a $resolved 2>$null)
    if ($LASTEXITCODE -eq 0 -and $converted) {
        return $converted.Trim()
    }

    throw "Failed to convert path to WSL path: $resolved"
}

$wslRepo = ConvertTo-WslPath $repo

function Quote-BashArg([string]$Value) {
    return "'" + ($Value -replace "'", "'\''") + "'"
}

$quotedRepo = Quote-BashArg $wslRepo
$quotedArgs = ($Args | ForEach-Object { Quote-BashArg $_ }) -join " "
$command = "cd $quotedRepo && ./scripts/run-lubancat2-qemu.sh $quotedArgs"

Write-Host "Starting LubanCat 2 / A55 QEMU emulation through WSL..."
Write-Host "Repo: $wslRepo"
& wsl -e bash -lc $command
if ($LASTEXITCODE -ne 0) {
    throw "LubanCat 2 QEMU emulation failed with exit code $LASTEXITCODE"
}
