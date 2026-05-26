#!/usr/bin/env bash
set -euo pipefail

IMAGE="${IMAGE:-reactor-os-a55-debian10-builder}"
RUST_VERSION="${RUST_VERSION:-1.90.0}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "Building A55 Debian 10 builder image on this PC..."
docker build \
  -f "${ROOT}/scripts/Dockerfile.a55-debian10" \
  --build-arg "RUST_VERSION=${RUST_VERSION}" \
  -t "${IMAGE}" \
  "${ROOT}"

echo "Cross-compiling and packaging for ARM64 Cortex-A55..."
docker run --rm \
  -v "${ROOT}:/work" \
  -w /work \
  "${IMAGE}"

echo "Latest package:"
cat "${ROOT}/dist/latest-a55-debian10-package.txt"
