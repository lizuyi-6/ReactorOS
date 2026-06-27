#!/usr/bin/env bash
set -euo pipefail

INSTALL_ROOT="${REACTOR_EDGE_INSTALL_ROOT:-}"
if [[ -n "$INSTALL_ROOT" ]]; then
  PREFIX="${INSTALL_ROOT}/opt/reactor-edge"
  ETC_DIR="${INSTALL_ROOT}/etc/reactor-edge"
  DATA_DIR="${INSTALL_ROOT}/var/lib/reactor-edge"
  PROJECT_DIR="${INSTALL_ROOT}/project"
  SYSTEMD_DIR="${INSTALL_ROOT}/etc/systemd/system"
else
  PREFIX="/opt/reactor-edge"
  ETC_DIR="/etc/reactor-edge"
  DATA_DIR="/var/lib/reactor-edge"
  PROJECT_DIR="/project"
  SYSTEMD_DIR="/etc/systemd/system"
fi
SLOTS_DIR="${PREFIX}/slots"
INITIAL_SLOT="${REACTOR_EDGE_INITIAL_SLOT:-a}"
ENABLE_KIOSK=1
INSTALL_DEPS=0
SEED_DEMO_CONTEXT=0
START_NOW=1

usage() {
  cat <<'EOF'
Install ReactorOS on an ARM64 Debian board and enable boot autostart.

Run inside an extracted ReactorOS package:
  sudo ./install.sh [options]

Options:
  --no-kiosk           Install backend only; do not enable Chromium kiosk.
  --install-deps       Install runtime apt dependencies and Chromium.
  --seed-demo-context  Start backend with demo process/history/AI context.
  --no-start           Install and enable services, but do not start now.
  -h, --help           Show this help.

After install:
  systemctl status reactor-edge
  systemctl status reactor-os-chromium
  curl http://127.0.0.1:8000/health
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --no-kiosk)
      ENABLE_KIOSK=0
      shift
      ;;
    --install-deps)
      INSTALL_DEPS=1
      shift
      ;;
    --seed-demo-context)
      SEED_DEMO_CONTEXT=1
      shift
      ;;
    --no-start)
      START_NOW=0
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "${EUID:-$(id -u)}" -ne 0 && -z "$INSTALL_ROOT" ]]; then
  echo "This installer must run as root. Use: sudo ./install.sh" >&2
  exit 1
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

require_file() {
  local path="$1"
  [[ -f "$path" ]] || {
    echo "Missing required package file: $path" >&2
    exit 1
  }
}

require_executable() {
  local path="$1"
  [[ -x "$path" ]] || {
    echo "Missing required executable package file: $path" >&2
    exit 1
  }
}

require_dir() {
  local path="$1"
  [[ -d "$path" ]] || {
    echo "Missing required package directory: $path" >&2
    exit 1
  }
}

validate_package_before_stopping_services() {
  [[ "$INITIAL_SLOT" == "a" || "$INITIAL_SLOT" == "b" ]] || {
    echo "REACTOR_EDGE_INITIAL_SLOT must be a or b" >&2
    exit 2
  }

  require_dir "${ROOT}/bin"
  require_dir "${ROOT}/static"
  require_dir "${ROOT}/kiosk"
  require_dir "${ROOT}/config"
  require_dir "${ROOT}/deploy"

  require_executable "${ROOT}/bin/reactor-edge-daemon"
  require_executable "${ROOT}/bin/reactor-safety-guard"
  require_executable "${ROOT}/bin/xingshu"
  require_executable "${ROOT}/kiosk/run-chromium-kiosk.sh"
  require_executable "${ROOT}/backup.sh"
  require_executable "${ROOT}/health-check.sh"
  require_executable "${ROOT}/ota-update.sh"
  require_executable "${ROOT}/ota-rollback.sh"
  require_executable "${ROOT}/ota-lib.sh"
  require_executable "${ROOT}/ota-boot-check.sh"

  require_file "${ROOT}/BUILD-METADATA.properties"
  require_file "${ROOT}/deploy/reactor-edge.service"
  require_file "${ROOT}/deploy/reactor-edge-ota-boot-check.service"
  require_file "${ROOT}/deploy/reactor-edge-backup.service"
  require_file "${ROOT}/deploy/reactor-edge-backup.timer"
  require_file "${ROOT}/deploy/reactor-os-chromium.service"
  require_file "${ROOT}/config/reactor-edge.env"
  require_file "${ROOT}/config/device.toml"
  require_file "${ROOT}/config/safety.toml"
  require_file "${ROOT}/config/ai_memory.toml"
  require_file "${ROOT}/config/integration.toml"

  if [[ ! -f "${ROOT}/frontend/dist/index.html" && ! -f "${ROOT}/static/index.html" ]]; then
    echo "Missing HMI assets: expected frontend/dist/index.html or static/index.html" >&2
    exit 1
  fi
}

validate_package_before_stopping_services

service_user() {
  awk -F= '/^User=/{print $2; exit}' "${ROOT}/deploy/reactor-edge.service"
}

service_group() {
  awk -F= '/^Group=/{print $2; exit}' "${ROOT}/deploy/reactor-edge.service"
}

SERVICE_USER="$(service_user)"
SERVICE_GROUP="$(service_group)"
if [[ -z "${SERVICE_USER}" ]]; then SERVICE_USER="pi"; fi
if [[ -z "${SERVICE_GROUP}" ]]; then SERVICE_GROUP="${SERVICE_USER}"; fi

install_deps() {
  export DEBIAN_FRONTEND=noninteractive
  apt-get update
  apt-get install -y --no-install-recommends ca-certificates libudev1 curl x11-xserver-utils
  apt-get install -y --no-install-recommends chromium || \
    apt-get install -y --no-install-recommends chromium-browser
  apt-get install -y --no-install-recommends unclutter || true
}

copy_tree() {
  local src="$1"
  local dst="$2"
  mkdir -p "$dst"
  cp -a "${src}/." "$dst/"
}

link_or_preserve_existing() {
  local path="$1"
  local target="$2"
  local legacy_dir="${PREFIX}/legacy-before-slots-$(date -u +%Y%m%d-%H%M%S)"
  if [[ -L "$path" ]]; then
    rm -f "$path"
  elif [[ -e "$path" ]]; then
    mkdir -p "$legacy_dir"
    mv "$path" "${legacy_dir}/$(basename "$path")"
  fi
  ln -sfnT "$target" "$path"
}

if [[ "${INSTALL_DEPS}" -eq 1 ]]; then
  install_deps
fi

if command -v systemctl >/dev/null 2>&1; then
  systemctl stop reactor-os-chromium >/dev/null 2>&1 || true
  systemctl stop reactor-edge >/dev/null 2>&1 || true
fi

SLOT_DIR="${SLOTS_DIR}/${INITIAL_SLOT}"

install -d -m 0755 "$PREFIX" "$SLOTS_DIR" "$SLOT_DIR" "$SLOT_DIR/bin" "$SLOT_DIR/static" "$SLOT_DIR/frontend" "$SLOT_DIR/kiosk" "$ETC_DIR" "$DATA_DIR" "$PROJECT_DIR"
install -d -m 0750 "$DATA_DIR/backups"
copy_tree "${ROOT}/bin" "${SLOT_DIR}/bin"
copy_tree "${ROOT}/static" "${SLOT_DIR}/static"
if [[ -d "${ROOT}/frontend" ]]; then
  copy_tree "${ROOT}/frontend" "${SLOT_DIR}/frontend"
fi
copy_tree "${ROOT}/kiosk" "${SLOT_DIR}/kiosk"
copy_tree "${ROOT}/config" "$ETC_DIR"
install -m 0755 "${ROOT}/health-check.sh" "${SLOT_DIR}/health-check.sh"
install -m 0755 "${ROOT}/backup.sh" "${SLOT_DIR}/backup.sh"
install -m 0755 "${ROOT}/ota-update.sh" "${SLOT_DIR}/ota-update.sh"
install -m 0755 "${ROOT}/ota-rollback.sh" "${SLOT_DIR}/ota-rollback.sh"
install -m 0755 "${ROOT}/ota-lib.sh" "${SLOT_DIR}/ota-lib.sh"
install -m 0755 "${ROOT}/ota-boot-check.sh" "${SLOT_DIR}/ota-boot-check.sh"
install -m 0644 "${ROOT}/BUILD-METADATA.properties" "${SLOT_DIR}/BUILD-METADATA.properties"
install -d -m 0755 "$SYSTEMD_DIR"
install -m 0644 "${ROOT}/deploy/reactor-edge-ota-boot-check.service" "${SYSTEMD_DIR}/reactor-edge-ota-boot-check.service"
install -m 0644 "${ROOT}/deploy/reactor-edge.service" "${SYSTEMD_DIR}/reactor-edge.service"
install -m 0644 "${ROOT}/deploy/reactor-edge-backup.service" "${SYSTEMD_DIR}/reactor-edge-backup.service"
install -m 0644 "${ROOT}/deploy/reactor-edge-backup.timer" "${SYSTEMD_DIR}/reactor-edge-backup.timer"
install -m 0644 "${ROOT}/deploy/reactor-os-chromium.service" "${SYSTEMD_DIR}/reactor-os-chromium.service"

chmod +x "${SLOT_DIR}/bin/reactor-edge-daemon" "${SLOT_DIR}/bin/reactor-safety-guard" "${SLOT_DIR}/bin/xingshu" "${SLOT_DIR}/kiosk/run-chromium-kiosk.sh"
if [[ -f "${SLOT_DIR}/health-check.sh" ]]; then
  chmod +x "${SLOT_DIR}/health-check.sh"
fi
if [[ -f "${SLOT_DIR}/ota-boot-check.sh" ]]; then
  chmod +x "${SLOT_DIR}/ota-boot-check.sh"
fi
if [[ "$INITIAL_SLOT" == "a" ]]; then
  PREVIOUS_SLOT_DIR="${SLOTS_DIR}/b"
else
  PREVIOUS_SLOT_DIR="${SLOTS_DIR}/a"
fi
link_or_preserve_existing "${PREFIX}/current" "$SLOT_DIR"
link_or_preserve_existing "${PREFIX}/previous" "$PREVIOUS_SLOT_DIR"
link_or_preserve_existing "${PREFIX}/bin" "current/bin"
link_or_preserve_existing "${PREFIX}/static" "current/static"
link_or_preserve_existing "${PREFIX}/frontend" "current/frontend"
link_or_preserve_existing "${PREFIX}/kiosk" "current/kiosk"
link_or_preserve_existing "${PREFIX}/backup.sh" "current/backup.sh"
link_or_preserve_existing "${PREFIX}/health-check.sh" "current/health-check.sh"
install -m 0755 "${SLOT_DIR}/ota-update.sh" "${PREFIX}/ota-update.sh"
install -m 0755 "${SLOT_DIR}/ota-rollback.sh" "${PREFIX}/ota-rollback.sh"
install -m 0755 "${SLOT_DIR}/ota-lib.sh" "${PREFIX}/ota-lib.sh"
install -m 0755 "${SLOT_DIR}/ota-boot-check.sh" "${PREFIX}/ota-boot-check.sh"
chown -R "${SERVICE_USER}:${SERVICE_GROUP}" "$DATA_DIR" "$PROJECT_DIR" || true
chmod 0750 "$DATA_DIR/backups" || true

if [[ "${SEED_DEMO_CONTEXT}" -eq 1 ]]; then
  mkdir -p "${SYSTEMD_DIR}/reactor-edge.service.d"
  cat >"${SYSTEMD_DIR}/reactor-edge.service.d/10-demo-context.conf" <<'EOF'
[Service]
Environment=REACTOR_OS_EXTRA_ARGS=--seed-demo-context
EOF
else
  rm -f "${SYSTEMD_DIR}/reactor-edge.service.d/10-demo-context.conf"
fi

systemctl daemon-reload
systemctl enable reactor-edge-ota-boot-check
systemctl enable reactor-edge
if [[ -f "${SYSTEMD_DIR}/reactor-edge-backup.timer" ]]; then
  systemctl enable reactor-edge-backup.timer
fi
if [[ "${ENABLE_KIOSK}" -eq 1 ]]; then
  systemctl enable reactor-os-chromium
else
  systemctl disable reactor-os-chromium >/dev/null 2>&1 || true
fi

if [[ "${START_NOW}" -eq 1 ]]; then
  systemctl restart reactor-edge
  if [[ -f "${SYSTEMD_DIR}/reactor-edge-backup.timer" ]]; then
    systemctl restart reactor-edge-backup.timer
  fi
  if [[ "${ENABLE_KIOSK}" -eq 1 ]]; then
    systemctl restart reactor-os-chromium || {
      echo "Backend installed, but Chromium kiosk failed to start. Check: journalctl -u reactor-os-chromium -e" >&2
      exit 1
    }
  fi
fi

cat <<EOF
ReactorOS installed.

Backend:
  systemctl status reactor-edge
  systemctl status reactor-edge-backup.timer
  curl http://127.0.0.1:8000/health

Kiosk:
  systemctl status reactor-os-chromium

JSON bridge:
  state:   ${PROJECT_DIR}/state.json
  control: ${PROJECT_DIR}/control.json

Service user:
  ${SERVICE_USER}:${SERVICE_GROUP}

Application slot:
  current: ${PREFIX}/current -> ${SLOT_DIR}
  OTA:     sudo ${PREFIX}/ota-update.sh <release.tar.gz> --sha256 <release.tar.gz.sha256>
EOF
