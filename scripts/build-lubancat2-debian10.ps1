param(
    [string]$Image = "reactor-os-lubancat2-debian10-builder",
    [string]$RustVersion = "1.90.0",
    [string]$FrontendProject = "workshop\frontend",
    [switch]$SkipBuilderImage
)

$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$frontend = (Resolve-Path (Join-Path $repo $FrontendProject)).Path
$repoPrefix = $repo.TrimEnd("\") + "\"
if (-not $frontend.StartsWith($repoPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "FrontendProject must stay inside the repository: $FrontendProject"
}
$frontendRelative = $frontend.Substring($repoPrefix.Length).Replace("\", "/")

Push-Location $repo
try {
    Write-Host "Building production Workshop HMI on this PC from $frontendRelative..."
    & npm --prefix $frontend run build
    if ($LASTEXITCODE -ne 0) {
        throw "Workshop HMI build failed with exit code $LASTEXITCODE"
    }
    if (-not (Test-Path (Join-Path $frontend "dist\index.html"))) {
        throw "$frontendRelative/dist/index.html missing after Workshop HMI build"
    }
}
finally {
    Pop-Location
}

if ($SkipBuilderImage) {
    Write-Host "Reusing existing offline builder image $Image..."
    docker image inspect $Image *> $null
    if ($LASTEXITCODE -ne 0) {
        throw "Builder image $Image is not available for offline reuse"
    }
}
else {
    Write-Host "Building LubanCat 2 Debian 10 builder image on this PC..."
    docker build `
        -f (Join-Path $repo "scripts/Dockerfile.a55-debian10") `
        --build-arg "RUST_VERSION=$RustVersion" `
        -t $Image `
        $repo
    if ($LASTEXITCODE -ne 0) {
        throw "Docker builder image build failed with exit code $LASTEXITCODE"
    }
}

Write-Host "Cross-compiling and packaging for LubanCat 2 / RK3568 / ARM64 Cortex-A55..."
$dockerRunArgs = @(
    "run", "--rm",
    "-e", "PKG_PREFIX=reactor-os-lubancat2-rk3568-debian10-chromium-kiosk",
    "-e", "TARGET_DIR=/tmp/target-lubancat2-arm64-buster",
    "-e", "BOARD_NAME=LubanCat 2 RK3568 ARM64 Cortex-A55 Debian 10",
    "-e", "SERVICE_USER=cat",
    "-e", "SERVICE_GROUP=cat",
    "-e", "SERVICE_HOME=/home/cat",
    "-e", "FRONTEND_DIST=$frontendRelative/dist",
    "-e", "FRONTEND_SOURCE=$frontendRelative",
    "-e", "DIST_POINTER=latest-lubancat2-debian10-package.txt",
    "-e", "PACKAGE_README=README-LUBANCAT2-CHROMIUM.md",
    "-v", "${repo}:/work",
    "-v", "reactor-host-buster-cache:/tmp/reactor-host-target",
    "-v", "reactor-lubancat2-arm64-buster-cache:/tmp/target-lubancat2-arm64-buster",
    "-w", "/work"
)
$cargoRegistry = if ($env:CARGO_HOME) { Join-Path $env:CARGO_HOME "registry" } else { $null }
if ($cargoRegistry -and (Test-Path $cargoRegistry)) {
    Write-Host "Mounting host Cargo registry for offline/reproducible dependency reuse..."
    $dockerRunArgs += @("-e", "CARGO_NET_OFFLINE=true", "-v", "${cargoRegistry}:/cargo/registry")
}
$dockerRunArgs += @($Image, "bash", "scripts/package-a55-debian10.sh")
& docker @dockerRunArgs
if ($LASTEXITCODE -ne 0) {
    throw "LubanCat 2 Debian 10 package build failed with exit code $LASTEXITCODE"
}

$latest = Join-Path $repo "dist/latest-lubancat2-debian10-package.txt"
if (Test-Path $latest) {
    Write-Host "Latest LubanCat 2 package:"
    Get-Content $latest
}
