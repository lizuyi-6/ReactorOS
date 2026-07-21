#!/usr/bin/env bash
set -euo pipefail

cd /work

TARGET="${TARGET:-aarch64-unknown-linux-gnu}"
PROFILE="${PROFILE:-release}"
TARGET_DIR="${TARGET_DIR:-target-a55-arm64-buster}"
PKG_PREFIX="${PKG_PREFIX:-reactor-os-a55-arm64-debian10-chromium-kiosk}"
DIST_DIR="${DIST_DIR:-dist}"
CONFIG_NAME="${CONFIG_NAME:-device.json_bridge.toml}"
A55_RUSTFLAGS="${A55_RUSTFLAGS:--C target-cpu=cortex-a55 -C target-feature=+aes,+sha2,+crc,+lse}"
BOARD_NAME="${BOARD_NAME:-ARM64 Cortex-A55 Debian 10 board}"
SERVICE_USER="${SERVICE_USER:-pi}"
SERVICE_GROUP="${SERVICE_GROUP:-${SERVICE_USER}}"
SERVICE_HOME="${SERVICE_HOME:-/home/${SERVICE_USER}}"
FRONTEND_DIST="${FRONTEND_DIST:-frontend/dist}"
FRONTEND_SOURCE="${FRONTEND_SOURCE:-frontend}"
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

echo "==> Verifying Vue HMI build artifact: ${FRONTEND_DIST}/index.html"
if [[ ! -f "${FRONTEND_DIST}/index.html" ]]; then
  cat >&2 <<EOF
Missing ${FRONTEND_DIST}/index.html.

Build ${FRONTEND_SOURCE} on the host before packaging so the ARM64 release
serves the selected Vue HMI by default.
EOF
  exit 1
fi

echo "==> Cross-compiling ReactorOS for ${TARGET} on Debian 10 sysroot"
PKG_CONFIG_ALLOW_CROSS=1 \
PKG_CONFIG_PATH="/usr/lib/aarch64-linux-gnu/pkgconfig:/usr/share/pkgconfig" \
PKG_CONFIG_SYSROOT_DIR="/" \
CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc \
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
RUSTFLAGS="${A55_RUSTFLAGS}" \
cargo build --locked --release --target "${TARGET}" --target-dir "${TARGET_DIR}"

DAEMON_BIN="${TARGET_DIR}/${TARGET}/${PROFILE}/reactor-edge-daemon"
SAFETY_GUARD_BIN="${TARGET_DIR}/${TARGET}/${PROFILE}/reactor-safety-guard"
XINGSHU_BIN="${TARGET_DIR}/${TARGET}/${PROFILE}/xingshu"
if [[ ! -x "${DAEMON_BIN}" ]]; then
  echo "Missing binary: ${DAEMON_BIN}" >&2
  exit 1
fi
if [[ ! -x "${SAFETY_GUARD_BIN}" ]]; then
  echo "Missing binary: ${SAFETY_GUARD_BIN}" >&2
  exit 1
fi
if [[ ! -x "${XINGSHU_BIN}" ]]; then
  echo "Missing binary: ${XINGSHU_BIN}" >&2
  exit 1
fi

GIT_SHA="$(git rev-parse --short HEAD 2>/dev/null || printf 'nogit')"
GIT_FULL_SHA="$(git rev-parse HEAD 2>/dev/null || printf 'nogit')"
if [[ -z "$(git status --porcelain --untracked-files=normal 2>/dev/null)" ]]; then
  GIT_DIRTY="false"
else
  GIT_DIRTY="true"
fi
FRONTEND_SHA256="$(sha256sum "${FRONTEND_DIST}/index.html" | awk '{print $1}')"
STAMP="$(date +%Y%m%d-%H%M%S)"
BUILT_AT_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
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
  "${PACKAGE_DIR}/frontend" \
  "${PACKAGE_DIR}/kiosk" \
  "${PACKAGE_DIR}/static"

cp "${DAEMON_BIN}" "${PACKAGE_DIR}/bin/reactor-edge-daemon"
cp "${SAFETY_GUARD_BIN}" "${PACKAGE_DIR}/bin/reactor-safety-guard"
cp "${XINGSHU_BIN}" "${PACKAGE_DIR}/bin/xingshu"
cp config/*.toml "${PACKAGE_DIR}/config/"
cp -r static/. "${PACKAGE_DIR}/static/"
cp -r "${FRONTEND_DIST}" "${PACKAGE_DIR}/frontend/"
cp kiosk/run-chromium-kiosk.sh "${PACKAGE_DIR}/kiosk/"
cp deploy/reactor-edge.service "${PACKAGE_DIR}/deploy/"
cp deploy/reactor-edge-ota-boot-check.service "${PACKAGE_DIR}/deploy/"
cp deploy/reactor-edge-backup.service "${PACKAGE_DIR}/deploy/"
cp deploy/reactor-edge-backup.timer "${PACKAGE_DIR}/deploy/"
cp deploy/reactor-os-chromium.service "${PACKAGE_DIR}/deploy/"
cp deploy/install-board.sh "${PACKAGE_DIR}/install.sh"
cp deploy/uninstall-board.sh "${PACKAGE_DIR}/uninstall.sh"
cp deploy/board-health.sh "${PACKAGE_DIR}/health-check.sh"
cp deploy/reactor-edge-backup.sh "${PACKAGE_DIR}/backup.sh"
cp deploy/reactor-edge-ota-update.sh "${PACKAGE_DIR}/ota-update.sh"
cp deploy/reactor-edge-ota-rollback.sh "${PACKAGE_DIR}/ota-rollback.sh"
cp deploy/reactor-edge-ota-lib.sh "${PACKAGE_DIR}/ota-lib.sh"
cp deploy/reactor-edge-ota-boot-check.sh "${PACKAGE_DIR}/ota-boot-check.sh"
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

cat >"${PACKAGE_DIR}/BUILD-METADATA.properties" <<EOF
REACTOR_EDGE_BUILD_SCHEMA=reactor-edge.build.v1
REACTOR_EDGE_PACKAGE_NAME=${PACKAGE_NAME}
REACTOR_EDGE_GIT_SHA=${GIT_SHA}
REACTOR_EDGE_GIT_FULL_SHA=${GIT_FULL_SHA}
REACTOR_EDGE_GIT_DIRTY=${GIT_DIRTY}
REACTOR_EDGE_BUILT_AT_UTC=${BUILT_AT_UTC}
REACTOR_EDGE_TARGET=${TARGET}
REACTOR_EDGE_PROFILE=${PROFILE}
REACTOR_EDGE_PKG_PREFIX=${PKG_PREFIX}
REACTOR_EDGE_BOARD_NAME=${BOARD_NAME}
REACTOR_EDGE_SERVICE_USER=${SERVICE_USER}
REACTOR_EDGE_CONFIG_NAME=${CONFIG_NAME}
REACTOR_EDGE_RUSTFLAGS=${A55_RUSTFLAGS}
REACTOR_EDGE_FRONTEND_SOURCE=${FRONTEND_SOURCE}
REACTOR_EDGE_FRONTEND_SHA256=${FRONTEND_SHA256}
EOF

sed -i \
  -e "s/^User=.*/User=${SERVICE_USER}/" \
  -e "s/^Group=.*/Group=${SERVICE_GROUP}/" \
  -e "s|-o pi -g pi|-o ${SERVICE_USER} -g ${SERVICE_GROUP}|g" \
  -e "s|/home/pi/.Xauthority|${SERVICE_HOME}/.Xauthority|g" \
  -e "s|--config /etc/reactor-edge/device.toml|--config /etc/reactor-edge/${CONFIG_NAME}|g" \
  "${PACKAGE_DIR}/deploy/reactor-edge.service" \
  "${PACKAGE_DIR}/deploy/reactor-edge-backup.service" \
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
  --assets auto \
  --safety-guard "${ROOT}/bin/reactor-safety-guard" \
  --bind "${REACTOR_OS_BIND:-0.0.0.0:8000}" \
  ${REACTOR_OS_EXTRA_ARGS:-}
EOF
chmod +x \
  "${PACKAGE_DIR}/run.sh" \
  "${PACKAGE_DIR}/install.sh" \
  "${PACKAGE_DIR}/uninstall.sh" \
  "${PACKAGE_DIR}/health-check.sh" \
  "${PACKAGE_DIR}/backup.sh" \
  "${PACKAGE_DIR}/ota-update.sh" \
  "${PACKAGE_DIR}/ota-rollback.sh" \
  "${PACKAGE_DIR}/ota-lib.sh" \
  "${PACKAGE_DIR}/ota-boot-check.sh" \
  "${PACKAGE_DIR}/bin/reactor-edge-daemon" \
  "${PACKAGE_DIR}/bin/reactor-safety-guard" \
  "${PACKAGE_DIR}/bin/xingshu" \
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

The packaged Vue HMI is built for RK3568 low load by default: 1 Hz WebSocket
realtime snapshots update current readouts, the aggregate fallback refresh runs
every 15 seconds, and the live trend keeps 24 samples. Override only for lab
profiling by setting \`XINGSHU_VITE_REFRESH_MS\` or
\`XINGSHU_VITE_LIVE_SAMPLE_LIMIT\` before \`npm run frontend:build\`.

The Chromium kiosk profile/cache default to the runtime directory and low-load
mode caps renderer processes and disk/media cache. If the board has very small
tmpfs, set \`REACTOR_OS_CHROMIUM_USER_DATA_DIR\` and
\`REACTOR_OS_CHROMIUM_CACHE_DIR\` in a systemd override.

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

Default HMI assets:

- \`frontend/dist/index.html\`: Vue 3 production HMI, built from \`${FRONTEND_SOURCE}\` (sha256 \`${FRONTEND_SHA256}\`).
- \`static/index.html\`: legacy HMI fallback. The daemon is launched with \`--assets auto\` and prefers \`frontend/dist\` when present.

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

The installer initializes application slot \`/opt/reactor-edge/slots/a\`, points
\`/opt/reactor-edge/current\` at that slot, installs compatibility links such as
\`/opt/reactor-edge/bin\` and \`/opt/reactor-edge/backup.sh\`, and copies the OTA
tools to \`/opt/reactor-edge/ota-update.sh\`, \`ota-rollback.sh\`, and
\`ota-lib.sh\`. It also enables \`reactor-edge-ota-boot-check.service\`, which
checks interrupted OTA state before the backend is allowed to start after boot.
The backend service also runs \`/opt/reactor-edge/ota-boot-check.sh\` as
\`ExecStartPre\`, so manual restarts and automatic systemd restarts re-check OTA
state before production control starts. Backend and kiosk services use systemd
start-rate limits so repeated crashes stop for maintenance intervention instead
of looping indefinitely.
The backup helper serializes concurrent timer/OTA backup attempts with a
non-blocking lock, writes snapshots through a temporary file, verifies the
sha256 sidecar and SQLite header, then publishes the timestamped snapshot and
\`latest.snapshot\` links.

## Application A/B Update

The package archive is accompanied by a generated sha256 sidecar:

\`\`\`text
${PACKAGE_NAME}.tar.gz.sha256
\`\`\`

Copy both files to the board, then run a dry-run preflight before switching
slots:

\`\`\`bash
sudo /opt/reactor-edge/ota-update.sh ${PACKAGE_NAME}.tar.gz --sha256 ${PACKAGE_NAME}.tar.gz.sha256 --dry-run

sudo /opt/reactor-edge/ota-update.sh ${PACKAGE_NAME}.tar.gz --sha256 ${PACKAGE_NAME}.tar.gz.sha256
\`\`\`

The updater verifies the checksum sidecar references this tarball basename,
rejects invalid health-check arguments, validates tar members before extraction,
fails closed unless the backend status endpoint proves the reactor is idle,
checks disk space, validates backup availability, requires
\`BUILD-METADATA.properties\`, and validates the unpacked candidate slot
contents. With \`--dry-run\`, it performs those checks without changing
\`current\`/\`previous\`, installing systemd units, or creating a database
snapshot. A real update then creates a pre-update SQLite snapshot, switches
\`/opt/reactor-edge/current\`, records \`from_version\`, \`to_version\`,
\`from_git\`, and \`to_git\` in OTA state, and rolls back automatically if
repeated health checks fail. If power is lost before the \`current\` switch, the
boot check keeps the existing slot running; if power is lost after the switch
but before commit, it restores \`previous\` before production control starts.
The short-lived OTA health-check marker records the updater PID and process
start identity; boot-check removes the marker and fails closed if that process
is no longer alive, so a killed OTA script does not leave a stale bypass.
If an update or manual rollback enters \`failed\`, the OTA scripts clear the
temporary health-check bypass marker and stop backend/kiosk services
immediately.
Use \`--force --confirm-maintenance-window\` only in a confirmed maintenance
window. Unsafe lab/recovery bypasses also require explicit pairing:
\`--skip-backup --confirm-skip-backup\` and
\`--allow-missing-checksum --confirm-unsafe-no-checksum\`.

Manual rollback:

\`\`\`bash
sudo /opt/reactor-edge/ota-rollback.sh
\`\`\`

If the backend/status endpoint is unavailable, rollback also fails closed.
Confirm the reactor is stopped at the field panel, then use
\`sudo /opt/reactor-edge/ota-rollback.sh --force --confirm-maintenance-window\`
during the maintenance window.

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
- Built: ${BUILT_AT_UTC}
- Package: ${PACKAGE_NAME}
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
(cd "${DIST_DIR}" && sha256sum "${PACKAGE_NAME}.tar.gz" >"${PACKAGE_NAME}.tar.gz.sha256")
if command -v zip >/dev/null 2>&1; then
  (cd "${DIST_DIR}" && zip -qr "${PACKAGE_NAME}.zip" "${PACKAGE_NAME}")
fi

echo "${PACKAGE_DIR}" >"${DIST_DIR}/${DIST_POINTER}"
echo "DONE: ${PACKAGE_DIR}"
echo "TAR:  ${PACKAGE_DIR}.tar.gz"
