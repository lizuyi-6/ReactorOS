$env:CARGO_TARGET_DIR = "C:\tmp\xingshu-target-v3"
New-Item -ItemType Directory -Force -Path $env:CARGO_TARGET_DIR | Out-Null
cargo build --target-dir $env:CARGO_TARGET_DIR --bin xingshu 2>&1 | Select-Object -Last 5
Get-ChildItem (Join-Path $env:CARGO_TARGET_DIR "debug\xingshu*")
