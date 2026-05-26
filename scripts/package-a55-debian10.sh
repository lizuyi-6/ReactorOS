#!/usr/bin/env bash
set -euo pipefail

cd /work

TARGET="${TARGET:-aarch64-unknown-linux-gnu}"
PROFILE="${PROFILE:-release}"
TARGET_DIR="${TARGET_DIR:-target-a55-arm64-buster}"
PKG_PREFIX="${PKG_PREFIX:-reactor-os-a55-arm64-debian10-chromium-kiosk}"
DIST_DIR="${DIST_DIR:-dist}"
CONFIG_NAME="${CONFIG_NAME:-device.json_bridge.toml}"
A55_RUSTFLAGS="${A55_RUSTFLAGS:--C target-cpu=cortex-a55}"
BOARD_NAME="${BOARD_NAME:-ARM64 Cortex-A55 Debian 10 board}"
SERVICE_USER="${SERVICE_USER:-pi}"
SERVICE_GROUP="${SERVICE_GROUP:-${SERVICE_USER}}"
SERVICE_HOME="${SERVICE_HOME:-/home/${SERVICE_USER}}"
DIST_POINTER="${DIST_POINTER:-latest-a55-debian10-package.txt}"
PACKAGE_README="${PACKAGE_README:-README-A55-CHROMIUM.md}"
STEPFUN_AI_ENABLED="${STEPFUN_AI_ENABLED:-false}"
STEPFUN_API_KEY="${STEPFUN_API_KEY:-}"
STEPFUN_BASE_URL="${STEPFUN_BASE_URL:-https://api.stepfun.com/v1}"
STEPFUN_API_TYPE="${STEPFUN_API_TYPE:-chat_completions}"
STEPFUN_MODEL="${STEPFUN_MODEL:-step-3.6}"
STEPFUN_REASONING_EFFORT="${STEPFUN_REASONING_EFFORT:-medium}"
STEPFUN_TIMEOUT_SECONDS="${STEPFUN_TIMEOUT_SECONDS:-20}"

echo "==> Formatting and testing host code"
cargo fmt --check
CARGO_TARGET_DIR="${HOST_TARGET_DIR:-/tmp/reactor-host-target}" cargo test

echo "==> Cross-compiling ReactorOS for ${TARGET} on Debian 10 sysroot"
PKG_CONFIG_ALLOW_CROSS=1 \
PKG_CONFIG_PATH="/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig" \
PKG_CONFIG_SYSROOT_DIR="/" \
CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
RUSTFLAGS="${A55_RUSTFLAGS}" \
cargo build --locked --release --target "${TARGET}" --target-dir "${TARGET_DIR}"

BIN="${TARGET_DIR}/${TARGET}/${PROFILE}/reactor-edge-daemon"
if [[ ! -x "${BIN}" ]]; then
  echo "Missing binary: ${BIN}" >&2
  exit 1
fi

GIT_SHA="$(git rev-parse --short HEAD 2>/dev/null || printf 'nogit')"
STAMP="$(date +%Y%m%d-%H%M%S)"
PACKAGE_NAME="${PKG_PREFIX}-${STAMP}-${GIT_SHA}"
PACKAGE_DIR="${DIST_DIR}/${PACKAGE_NAME}"

echo "==> Creating package ${PACKAGE_DIR}"
rm -rf "${PACKAGE_DIR}"
mkdir -p \
  "${PACKAGE_DIR}/bin" \
  "${PACKAGE_DIR}/config" \
  "${PACKAGE_DIR}/data" \
  "${PACKAGE_DIR}/deploy" \
  "${PACKAGE_DIR}/docs" \
  "${PACKAGE_DIR}/kiosk" \
  "${PACKAGE_DIR}/static"

cp "${BIN}" "${PACKAGE_DIR}/bin/reactor-edge-daemon"
cp config/*.toml "${PACKAGE_DIR}/config/"
cp -r static/. "${PACKAGE_DIR}/static/"
cp kiosk/run-chromium-kiosk.sh "${PACKAGE_DIR}/kiosk/"
cp deploy/reactor-edge.service "${PACKAGE_DIR}/deploy/"
cp deploy/reactor-os-chromium.service "${PACKAGE_DIR}/deploy/"
cp deploy/install-board.sh "${PACKAGE_DIR}/install.sh"
cp deploy/uninstall-board.sh "${PACKAGE_DIR}/uninstall.sh"
cp deploy/board-health.sh "${PACKAGE_DIR}/health-check.sh"
cp docs/json_bridge_protocol.md "${PACKAGE_DIR}/docs/"
cp docs/chromium_kiosk.md "${PACKAGE_DIR}/docs/"

cat >"${PACKAGE_DIR}/config/reactor-edge.env" <<EOF
STEPFUN_AI_ENABLED=${STEPFUN_AI_ENABLED}
STEPFUN_API_KEY=${STEPFUN_API_KEY}
STEPFUN_BASE_URL=${STEPFUN_BASE_URL}
STEPFUN_API_TYPE=${STEPFUN_API_TYPE}
STEPFUN_MODEL=${STEPFUN_MODEL}
STEPFUN_REASONING_EFFORT=${STEPFUN_REASONING_EFFORT}
STEPFUN_TIMEOUT_SECONDS=${STEPFUN_TIMEOUT_SECONDS}
EOF

sed -i \
  -e "s/^User=.*/User=${SERVICE_USER}/" \
  -e "s/^Group=.*/Group=${SERVICE_GROUP}/" \
  -e "s|/home/pi/.Xauthority|${SERVICE_HOME}/.Xauthority|g" \
  -e "s|--config /etc/reactor-edge/device.toml|--config /etc/reactor-edge/${CONFIG_NAME}|g" \
  "${PACKAGE_DIR}/deploy/reactor-edge.service" \
  "${PACKAGE_DIR}/deploy/reactor-os-chromium.service"

cat >"${PACKAGE_DIR}/run.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONFIG="${1:-${ROOT}/config/device.json_bridge.toml}"

exec "${ROOT}/bin/reactor-edge-daemon" \
  --config "${CONFIG}" \
  --safety "${ROOT}/config/safety.toml" \
  --memory "${ROOT}/config/ai_memory.toml" \
  --db "${ROOT}/data/reactor.sqlite3" \
  --assets "${ROOT}/static" \
  --bind "${REACTOR_OS_BIND:-0.0.0.0:8000}" \
  ${REACTOR_OS_EXTRA_ARGS:-}
EOF
chmod +x \
  "${PACKAGE_DIR}/run.sh" \
  "${PACKAGE_DIR}/install.sh" \
  "${PACKAGE_DIR}/uninstall.sh" \
  "${PACKAGE_DIR}/health-check.sh" \
  "${PACKAGE_DIR}/bin/reactor-edge-daemon" \
  "${PACKAGE_DIR}/kiosk/run-chromium-kiosk.sh"

cat >"${PACKAGE_DIR}/README-A55-CHROMIUM.md" <<EOF
# ReactorOS A55 Debian 10 Chromium Package

Board profile: ${BOARD_NAME}

This package is built on the PC side for ARM64 / Cortex-A55 boards running Debian 10.
The board should only install runtime dependencies and run the prepared binary.

## Runtime Dependencies

\`\`\`bash
sudo apt-get update
sudo apt-get install -y ca-certificates libudev1 curl x11-xserver-utils
sudo apt-get install -y chromium || sudo apt-get install -y chromium-browser
\`\`\`

Optional:

\`\`\`bash
sudo apt-get install -y unclutter
\`\`\`

## Manual Run

\`\`\`bash
tar -xzf ${PACKAGE_NAME}.tar.gz
cd ${PACKAGE_NAME}
./run.sh ./config/device.json_bridge.toml
\`\`\`

Then open:

\`\`\`text
http://127.0.0.1:8000/
\`\`\`

Chromium kiosk:

\`\`\`bash
./kiosk/run-chromium-kiosk.sh
\`\`\`

Low-load board health check:

\`\`\`bash
./health-check.sh
sudo /opt/reactor-edge/health-check.sh
\`\`\`

## Demo Context For Customer Presentation

Demo mode can seed process definitions, process steps, historical batch outcomes,
AI recommendation context, and non-sensor demo alarms:

\`\`\`bash
REACTOR_OS_EXTRA_ARGS=--seed-demo-context ./run.sh ./config/device.json_bridge.toml
\`\`\`

Important production rule: demo context never writes \`sensor_samples\` and never
fabricates runtime sensor values. Without real \`state.json\`, ESP32, or pipeline
samples, \`/api/live\` still returns 503 and sensor widgets must show the real
error state.

## JSON Bridge

Default paths in \`config/device.json_bridge.toml\`:

\`\`\`text
/project/state.json    downstream state, read by ReactorOS
/project/control.json  downstream control request, written by ReactorOS
\`\`\`

See \`docs/json_bridge_protocol.md\` for the exact JSON fields.

## Systemd Install

\`\`\`bash
sudo ./install.sh
\`\`\`

Install runtime apt dependencies at the same time:

\`\`\`bash
sudo ./install.sh --install-deps
\`\`\`

Install backend only without Chromium kiosk:

\`\`\`bash
sudo ./install.sh --no-kiosk
\`\`\`

For customer presentation packages, seed demo process/history/AI context while
keeping runtime sensor data strict:

\`\`\`bash
sudo ./install.sh --seed-demo-context
\`\`\`

Manual equivalent:

\`\`\`bash
sudo mkdir -p /opt/reactor-edge /etc/reactor-edge /var/lib/reactor-edge
sudo cp -r bin static kiosk /opt/reactor-edge/
sudo cp health-check.sh /opt/reactor-edge/
sudo cp config/*.toml /etc/reactor-edge/
sudo cp config/reactor-edge.env /etc/reactor-edge/
sudo cp deploy/reactor-edge.service deploy/reactor-os-chromium.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now reactor-edge
sudo systemctl enable --now reactor-os-chromium
\`\`\`

This package generated systemd units for display user \`${SERVICE_USER}\`,
group \`${SERVICE_GROUP}\`, and XAuthority \`${SERVICE_HOME}/.Xauthority\`.
If the board image uses another desktop user, rebuild with \`SERVICE_USER\`,
\`SERVICE_GROUP\`, and \`SERVICE_HOME\` set, or edit the two service files before
installing them.

## StepFun AI Provider

The backend reads optional StepFun settings from:

\`\`\`text
/etc/reactor-edge/reactor-edge.env
\`\`\`

Set these values on the board when real AI control is required:

\`\`\`bash
sudo tee /etc/reactor-edge/reactor-edge.env >/dev/null <<'ENV'
STEPFUN_AI_ENABLED=true
STEPFUN_API_KEY=replace-with-real-key
STEPFUN_BASE_URL=https://api.stepfun.com/v1
STEPFUN_API_TYPE=chat_completions
STEPFUN_MODEL=step-3.6
STEPFUN_REASONING_EFFORT=medium
STEPFUN_TIMEOUT_SECONDS=20
ENV
sudo systemctl restart reactor-edge
\`\`\`

Production behavior is strict: when \`STEPFUN_AI_ENABLED=true\`, a missing key,
request failure, invalid model output, or out-of-bounds recommendation returns a
JSON error code instead of silently using local optimizer rules.

This package includes \`config/reactor-edge.env\` generated at packaging time.
For demo builds, it may already contain StepFun settings; keep the package
private and do not commit that file to git.

## Build Metadata

- Board profile: ${BOARD_NAME}
- Git: ${GIT_SHA}
- Built: ${STAMP}
- Target: ${TARGET}
- CPU hint: cortex-a55
- Debian baseline: Debian 10 / glibc 2.28
- Service user: ${SERVICE_USER}
- Service config: /etc/reactor-edge/${CONFIG_NAME}
EOF

if [[ "${PACKAGE_README}" != "README-A55-CHROMIUM.md" ]]; then
  cp "${PACKAGE_DIR}/README-A55-CHROMIUM.md" "${PACKAGE_DIR}/${PACKAGE_README}"
fi

echo "==> Validating binary"
file "${PACKAGE_DIR}/bin/reactor-edge-daemon" | tee "${PACKAGE_DIR}/BUILD-VALIDATION.txt"
{
  echo
  echo "glibc symbol versions:"
  aarch64-linux-gnu-readelf --version-info "${PACKAGE_DIR}/bin/reactor-edge-daemon" \
    | grep -o 'GLIBC_[0-9.]*' \
    | sort -Vu || true
} | tee -a "${PACKAGE_DIR}/BUILD-VALIDATION.txt"

echo "==> Archiving"
tar -C "${DIST_DIR}" -czf "${PACKAGE_DIR}.tar.gz" "${PACKAGE_NAME}"
if command -v zip >/dev/null 2>&1; then
  (cd "${DIST_DIR}" && zip -qr "${PACKAGE_NAME}.zip" "${PACKAGE_NAME}")
fi

echo "${PACKAGE_DIR}" >"${DIST_DIR}/${DIST_POINTER}"
echo "DONE: ${PACKAGE_DIR}"
echo "TAR:  ${PACKAGE_DIR}.tar.gz"
