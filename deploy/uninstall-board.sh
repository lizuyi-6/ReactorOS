#!/usr/bin/env bash
set -euo pipefail

REMOVE_DATA=0

usage() {
  cat <<'EOF'
Uninstall ReactorOS systemd services from a Debian board.

Usage:
  sudo ./uninstall.sh [--remove-data]

Options:
  --remove-data   Also delete /var/lib/reactor-edge and /project.
  -h, --help      Show this help.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --remove-data)
      REMOVE_DATA=1
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
  echo "This uninstaller must run as root. Use: sudo ./uninstall.sh" >&2
  exit 1
fi

systemctl disable --now reactor-os-chromium >/dev/null 2>&1 || true
systemctl disable --now reactor-edge >/dev/null 2>&1 || true
rm -f /etc/systemd/system/reactor-os-chromium.service /etc/systemd/system/reactor-edge.service
rm -rf /etc/systemd/system/reactor-edge.service.d
systemctl daemon-reload
rm -rf /opt/reactor-edge /etc/reactor-edge

if [[ "${REMOVE_DATA}" -eq 1 ]]; then
  rm -rf /var/lib/reactor-edge /project
fi

echo "ReactorOS services removed."
if [[ "${REMOVE_DATA}" -ne 1 ]]; then
  echo "Data preserved: /var/lib/reactor-edge and /project"
fi
