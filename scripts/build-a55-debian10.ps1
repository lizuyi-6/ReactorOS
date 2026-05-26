param(
    [string]$Image = "reactor-os-a55-debian10-builder",
    [string]$RustVersion = "1.90.0"
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

Write-Host "Building A55 Debian 10 builder image on this PC..."
docker build `
    -f (Join-Path $repo "scripts/Dockerfile.a55-debian10") `
    --build-arg "RUST_VERSION=$RustVersion" `
    -t $Image `
    $repo
if ($LASTEXITCODE -ne 0) {
    throw "Docker builder image build failed with exit code $LASTEXITCODE"
}

Write-Host "Cross-compiling and packaging for ARM64 Cortex-A55..."
docker run --rm `
    -v "${repo}:/work" `
    -w /work `
    $Image
if ($LASTEXITCODE -ne 0) {
    throw "A55 Debian 10 package build failed with exit code $LASTEXITCODE"
}

$latest = Join-Path $repo "dist/latest-a55-debian10-package.txt"
if (Test-Path $latest) {
    Write-Host "Latest package:"
    Get-Content $latest
}
