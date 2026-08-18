#!/usr/bin/env bash
set -euo pipefail

IMAGE="${IMAGE:-reactor-os-lubancat2-debian10-builder}"
RUST_VERSION="${RUST_VERSION:-1.90.0}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SKIP_BUILDER_IMAGE="${SKIP_BUILDER_IMAGE:-0}"

echo "Building production Vue HMI on this PC (frontend/)..."
(cd "${ROOT}" && npm run frontend:build)
test -f "${ROOT}/frontend/dist/index.html"

if [[ "${SKIP_BUILDER_IMAGE}" == "1" ]]; then
  echo "Reusing existing offline builder image ${IMAGE}..."
  docker image inspect "${IMAGE}" >/dev/null
else
  echo "Building LubanCat 2 Debian 10 builder image on this PC..."
  docker build \
    -f "${ROOT}/scripts/Dockerfile.a55-debian10" \
    --build-arg "RUST_VERSION=${RUST_VERSION}" \
    -t "${IMAGE}" \
    "${ROOT}"
fi

echo "Cross-compiling and packaging for LubanCat 2 / RK3568 / ARM64 Cortex-A55..."
docker_cargo_args=()
if [[ -n "${CARGO_HOME:-}" && -d "${CARGO_HOME}/registry" ]]; then
  echo "Mounting host Cargo registry for offline/reproducible dependency reuse..."
  docker_cargo_args+=(
    -e CARGO_NET_OFFLINE=true
    -v "${CARGO_HOME}/registry:/cargo/registry"
  )
fi
docker run --rm \
  -e PKG_PREFIX=reactor-os-lubancat2-rk3568-debian10-chromium-kiosk \
  -e TARGET_DIR=/tmp/target-lubancat2-arm64-buster \
  -e "BOARD_NAME=LubanCat 2 RK3568 ARM64 Cortex-A55 Debian 10" \
  -e SERVICE_USER=cat \
  -e SERVICE_GROUP=cat \
  -e SERVICE_HOME=/home/cat \
  -e "FRONTEND_DIST=frontend/dist" \
  -e "FRONTEND_SOURCE=frontend" \
  -e DIST_POINTER=latest-lubancat2-debian10-package.txt \
  -e PACKAGE_README=README-LUBANCAT2-CHROMIUM.md \
  "${docker_cargo_args[@]}" \
  -v "${ROOT}:/work" \
  -v reactor-host-buster-cache:/tmp/reactor-host-target \
  -v reactor-lubancat2-arm64-buster-cache:/tmp/target-lubancat2-arm64-buster \
  -w /work \
  "${IMAGE}" \
  bash scripts/package-a55-debian10.sh

echo "Latest LubanCat 2 package:"
cat "${ROOT}/dist/latest-lubancat2-debian10-package.txt"
