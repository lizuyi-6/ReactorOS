#!/usr/bin/env bash
set -euo pipefail

API_URL="${REACTOR_OS_HEALTH_URL:-http://127.0.0.1:8000/health}"
STATUS_URL="${REACTOR_EDGE_STATUS_URL:-http://127.0.0.1:8000/api/devices/status}"
STATE_JSON="${REACTOR_OS_STATE_JSON:-/project/state.json}"
CONTROL_JSON="${REACTOR_OS_CONTROL_JSON:-/project/control.json}"
PRODUCTION_CHECK=0

usage() {
  cat <<'USAGE'
Usage: health-check.sh [--production]

Without options this prints board diagnostics. With --production it also fails
unless the backend reports a safe idle device state through /api/devices/status.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --production)
      PRODUCTION_CHECK=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unexpected argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

print_section() {
  printf '\n== %s ==\n' "$1"
}

file_age_seconds() {
  local file="$1"
  if [[ ! -e "$file" ]]; then
    printf 'missing'
    return 0
  fi

  local now mtime
  now="$(date +%s)"
  mtime="$(stat -c %Y "$file" 2>/dev/null || printf '0')"
  printf '%s' "$((now - mtime))"
}

check_safe_idle_status() {
  local tmp rc
  if ! command -v curl >/dev/null 2>&1; then
    echo "curl unavailable; cannot verify device status" >&2
    return 20
  fi
  if ! command -v python3 >/dev/null 2>&1; then
    echo "python3 unavailable; cannot parse device status" >&2
    return 21
  fi
  tmp="$(mktemp)"
  if ! curl -fsS --max-time 3 "$STATUS_URL" >"$tmp"; then
    rm -f "$tmp"
    echo "device status request failed: $STATUS_URL" >&2
    return 22
  fi
  if python3 - "$tmp" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as fh:
    payload = json.load(fh)

data = payload.get("data", payload)
devices = data.get("devices", [])
if not devices:
    print("device status did not report any devices", file=sys.stderr)
    sys.exit(16)

failures = []
for device in devices:
    device_id = str(device.get("device_id", "unknown"))
    status = str(device.get("status", "")).lower()
    if device.get("online") is not True or status != "idle":
        failures.append(f"{device_id}:not-safe-idle:{status or 'unknown'}")
    if device.get("active_batch_id") is not None:
        failures.append(f"{device_id}:active-batch")
    if device.get("emergency_stop"):
        failures.append(f"{device_id}:emergency-stop")
    if device.get("auto_enabled") is True:
        failures.append(f"{device_id}:auto-enabled")
    if device.get("manual_lock") is True:
        failures.append(f"{device_id}:manual-lock")
    last_control_error = device.get("last_control_error")
    if isinstance(last_control_error, str):
        last_control_error = last_control_error.strip()
    if last_control_error:
        failures.append(f"{device_id}:control-fault")
    if device.get("last_command_ok") is False:
        failures.append(f"{device_id}:downstream-command-fault")

if failures:
    print("production state is not safe idle: " + ",".join(failures), file=sys.stderr)
    sys.exit(14)

print("production_state=safe_idle")
PY
  then
    rc=0
  else
    rc=$?
  fi
  rm -f "$tmp"
  return "$rc"
}

print_section "Board"
date
printf 'uptime: %s\n' "$(uptime -p 2>/dev/null || uptime)"
printf 'load: %s\n' "$(cut -d' ' -f1-3 /proc/loadavg 2>/dev/null || true)"
printf 'kernel: %s\n' "$(uname -a)"

print_section "CPU"
if [[ -r /proc/cpuinfo ]]; then
  awk -F: '
    /model name|Hardware|Processor/ && !seen[$1]++ { gsub(/^ +/, "", $2); print $1 ":" $2 }
    /processor/ { count++ }
    END { if (count > 0) print "cores: " count }
  ' /proc/cpuinfo
fi
if [[ -r /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor ]]; then
  printf 'governor: %s\n' "$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor)"
fi
if [[ -r /sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq ]]; then
  awk '{ printf "cpu0_freq_mhz: %.0f\n", $1 / 1000 }' /sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq
fi

print_section "Memory"
free -h 2>/dev/null || cat /proc/meminfo | head -n 8

print_section "Thermal"
found_thermal=0
for zone in /sys/class/thermal/thermal_zone*; do
  [[ -r "$zone/temp" ]] || continue
  found_thermal=1
  name="$(cat "$zone/type" 2>/dev/null || basename "$zone")"
  awk -v name="$name" '{ printf "%s: %.1f C\n", name, $1 / 1000 }' "$zone/temp"
done
if [[ "$found_thermal" -eq 0 ]]; then
  echo "thermal zones unavailable"
fi

print_section "Disk"
df -h / /opt/reactor-edge /var/lib/reactor-edge /project 2>/dev/null || df -h /

print_section "Services"
if command -v systemctl >/dev/null 2>&1; then
  for svc in reactor-edge reactor-os-chromium; do
    printf '%s: %s\n' "$svc" "$(systemctl is-active "$svc" 2>/dev/null || true)"
  done
  printf 'reactor-edge-backup.timer: %s\n' "$(systemctl is-active reactor-edge-backup.timer 2>/dev/null || true)"
  systemctl list-timers reactor-edge-backup.timer --no-pager 2>/dev/null || true
else
  echo "systemctl unavailable"
fi

print_section "Backend"
if command -v curl >/dev/null 2>&1; then
  curl -fsS "$API_URL" || echo "health request failed: $API_URL"
else
  echo "curl unavailable"
fi
printf '\n'

if [[ "$PRODUCTION_CHECK" -eq 1 ]]; then
  print_section "Production State"
  check_safe_idle_status
fi

print_section "JSON Bridge"
printf 'state:   %s age=%ss\n' "$STATE_JSON" "$(file_age_seconds "$STATE_JSON")"
printf 'control: %s age=%ss\n' "$CONTROL_JSON" "$(file_age_seconds "$CONTROL_JSON")"
if [[ -r "$STATE_JSON" ]]; then
  head -c 600 "$STATE_JSON"
  printf '\n'
fi
