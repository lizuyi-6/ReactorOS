param(
    [string]$Image = "reactor-os-lubancat2-debian10-builder",
    [string]$RustVersion = "1.90.0"
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

Push-Location $repo
try {
    Write-Host "Building Vue HMI on this PC..."
    & npm run frontend:build
    if ($LASTEXITCODE -ne 0) {
        throw "npm run frontend:build failed with exit code $LASTEXITCODE"
    }
    if (-not (Test-Path (Join-Path $repo "frontend\dist\index.html"))) {
        throw "frontend\dist\index.html missing after npm run frontend:build"
    }
}
finally {
    Pop-Location
}

Write-Host "Building LubanCat 2 Debian 10 builder image on this PC..."
docker build `
    -f (Join-Path $repo "scripts/Dockerfile.a55-debian10") `
    --build-arg "RUST_VERSION=$RustVersion" `
    -t $Image `
    $repo
if ($LASTEXITCODE -ne 0) {
    throw "Docker builder image build failed with exit code $LASTEXITCODE"
}

Write-Host "Cross-compiling and packaging for LubanCat 2 / RK3568 / ARM64 Cortex-A55..."
docker run --rm `
    -e "PKG_PREFIX=reactor-os-lubancat2-rk3568-debian10-chromium-kiosk" `
    -e "TARGET_DIR=target-lubancat2-arm64-buster" `
    -e "BOARD_NAME=LubanCat 2 RK3568 ARM64 Cortex-A55 Debian 10" `
    -e "SERVICE_USER=cat" `
    -e "SERVICE_GROUP=cat" `
    -e "SERVICE_HOME=/home/cat" `
    -e "DIST_POINTER=latest-lubancat2-debian10-package.txt" `
    -e "PACKAGE_README=README-LUBANCAT2-CHROMIUM.md" `
    -v "${repo}:/work" `
    -w /work `
    $Image
if ($LASTEXITCODE -ne 0) {
    throw "LubanCat 2 Debian 10 package build failed with exit code $LASTEXITCODE"
}

$latest = Join-Path $repo "dist/latest-lubancat2-debian10-package.txt"
if (Test-Path $latest) {
    Write-Host "Latest LubanCat 2 package:"
    Get-Content $latest
}
