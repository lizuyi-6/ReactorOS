# 启动带 Vue HMI 的 reactor-edge-daemon。
# 使用方法：
#   powershell -ExecutionPolicy Bypass -File scripts\start-vue-hmi.ps1
# 或带环境变量：
#   $env:XINGSHU_PORT=8000; powershell -File scripts\start-vue-hmi.ps1
param(
    [string]$Port = $env:XINGSHU_PORT ?? "8000",
    [string]$Bind = $env:XINGSHU_BIND ?? "127.0.0.1",
    [string]$DbPath = $env:XINGSHU_DB ?? "data/reactor.sqlite3",
    [switch]$EnableTestReset,
    [switch]$SeedDemoContext
)

$ErrorActionPreference = "Stop"
$root = Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")
Set-Location -LiteralPath $root

# 1. 确保 Vue 产物已构建。
if (-not (Test-Path "frontend/dist/index.html")) {
    Write-Host "frontend/dist/index.html missing; running npm run frontend:build..." -ForegroundColor Yellow
    npm run frontend:build | Out-Host
    if (-not (Test-Path "frontend/dist/index.html")) {
        throw "frontend/dist/index.html still missing after build"
    }
}

# 2. 启动 daemon（auto 模式自动选择 frontend/dist）。
$args = @(
    "run", "--bin", "reactor-edge-daemon", "--",
    "--config", "config/device.toml",
    "--safety", "config/safety.toml",
    "--memory", "config/ai_memory.toml",
    "--integration", "config/integration.toml",
    "--db", $DbPath,
    "--bind", "$($Bind):$($Port)"
)
if ($EnableTestReset) { $args += "--enable-test-reset" }
if ($SeedDemoContext) { $args += "--seed-demo-context" }

$env:CARGO_TARGET_DIR = $env:CARGO_TARGET_DIR ?? "target"
Write-Host "starting daemon on $Bind`:$Port (auto HMI selection, prefers frontend/dist)" -ForegroundColor Cyan
& cargo @args
