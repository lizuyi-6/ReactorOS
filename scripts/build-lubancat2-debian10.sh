#!/usr/bin/env bash
set -euo pipefail

IMAGE="${IMAGE:-reactor-os-lubancat2-debian10-builder}"
RUST_VERSION="${RUST_VERSION:-1.90.0}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "Building Vue HMI on this PC..."
(cd "${ROOT}" && npm run frontend:build)
test -f "${ROOT}/frontend/dist/index.html"

echo "Building LubanCat 2 Debian 10 builder image on this PC..."
docker build \
  -f "${ROOT}/scripts/Dockerfile.a55-debian10" \
  --build-arg "RUST_VERSION=${RUST_VERSION}" \
  -t "${IMAGE}" \
  "${ROOT}"

echo "Cross-compiling and packaging for LubanCat 2 / RK3568 / ARM64 Cortex-A55..."
docker run --rm \
  -e PKG_PREFIX=reactor-os-lubancat2-rk3568-debian10-chromium-kiosk \
  -e TARGET_DIR=target-lubancat2-arm64-buster \
  -e "BOARD_NAME=LubanCat 2 RK3568 ARM64 Cortex-A55 Debian 10" \
  -e SERVICE_USER=cat \
  -e SERVICE_GROUP=cat \
  -e SERVICE_HOME=/home/cat \
  -e DIST_POINTER=latest-lubancat2-debian10-package.txt \
  -e PACKAGE_README=README-LUBANCAT2-CHROMIUM.md \
  -v "${ROOT}:/work" \
  -w /work \
  "${IMAGE}"

echo "Latest LubanCat 2 package:"
cat "${ROOT}/dist/latest-lubancat2-debian10-package.txt"
