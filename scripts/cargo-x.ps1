param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]] $CargoArgs = @("test")
)

$ErrorActionPreference = "Stop"

$repo = Split-Path -Parent $PSScriptRoot
$env:CARGO_HOME = "X:\rust-toolchain\cargo"
$env:RUSTUP_HOME = "X:\rust-toolchain\rustup"
$env:CARGO_TARGET_DIR = "X:\rust-target\reactor-os"
$env:Path = "X:\msys64\mingw64\bin;X:\rust-toolchain\cargo\bin;$env:Path"

Set-Location $repo
& cargo +1.90.0-x86_64-pc-windows-gnu @CargoArgs
exit $LASTEXITCODE
