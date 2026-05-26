#!/usr/bin/env bash
set -euo pipefail

PREFIX="/opt/reactor-edge"
ETC_DIR="/etc/reactor-edge"
DATA_DIR="/var/lib/reactor-edge"
PROJECT_DIR="/project"
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

if [[ "${EUID:-$(id -u)}" -ne 0 ]]; then
  echo "This installer must run as root. Use: sudo ./install.sh" >&2
  exit 1
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ ! -x "${ROOT}/bin/reactor-edge-daemon" ]]; then
  echo "Missing ${ROOT}/bin/reactor-edge-daemon. Run this script inside the extracted package." >&2
  exit 1
fi

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

if [[ "${INSTALL_DEPS}" -eq 1 ]]; then
  install_deps
fi

install -d -m 0755 "$PREFIX" "$PREFIX/bin" "$PREFIX/static" "$PREFIX/kiosk" "$ETC_DIR" "$DATA_DIR" "$PROJECT_DIR"
copy_tree "${ROOT}/bin" "${PREFIX}/bin"
copy_tree "${ROOT}/static" "${PREFIX}/static"
copy_tree "${ROOT}/kiosk" "${PREFIX}/kiosk"
copy_tree "${ROOT}/config" "$ETC_DIR"
if [[ -f "${ROOT}/health-check.sh" ]]; then
  install -m 0755 "${ROOT}/health-check.sh" "${PREFIX}/health-check.sh"
elif [[ -f "${ROOT}/deploy/board-health.sh" ]]; then
  install -m 0755 "${ROOT}/deploy/board-health.sh" "${PREFIX}/health-check.sh"
fi
install -m 0644 "${ROOT}/deploy/reactor-edge.service" /etc/systemd/system/reactor-edge.service
install -m 0644 "${ROOT}/deploy/reactor-os-chromium.service" /etc/systemd/system/reactor-os-chromium.service

chmod +x "${PREFIX}/bin/reactor-edge-daemon" "${PREFIX}/kiosk/run-chromium-kiosk.sh"
if [[ -f "${PREFIX}/health-check.sh" ]]; then
  chmod +x "${PREFIX}/health-check.sh"
fi
chown -R "${SERVICE_USER}:${SERVICE_GROUP}" "$DATA_DIR" "$PROJECT_DIR" || true

if [[ "${SEED_DEMO_CONTEXT}" -eq 1 ]]; then
  mkdir -p /etc/systemd/system/reactor-edge.service.d
  cat >/etc/systemd/system/reactor-edge.service.d/10-demo-context.conf <<'EOF'
[Service]
Environment=REACTOR_OS_EXTRA_ARGS=--seed-demo-context
EOF
else
  rm -f /etc/systemd/system/reactor-edge.service.d/10-demo-context.conf
fi

systemctl daemon-reload
systemctl enable reactor-edge
if [[ "${ENABLE_KIOSK}" -eq 1 ]]; then
  systemctl enable reactor-os-chromium
else
  systemctl disable reactor-os-chromium >/dev/null 2>&1 || true
fi

if [[ "${START_NOW}" -eq 1 ]]; then
  systemctl restart reactor-edge
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
  curl http://127.0.0.1:8000/health

Kiosk:
  systemctl status reactor-os-chromium

JSON bridge:
  state:   ${PROJECT_DIR}/state.json
  control: ${PROJECT_DIR}/control.json

Service user:
  ${SERVICE_USER}:${SERVICE_GROUP}
EOF
