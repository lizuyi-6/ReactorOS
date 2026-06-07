#!/usr/bin/env bash
set -euo pipefail

API_URL="${REACTOR_OS_HEALTH_URL:-http://127.0.0.1:8000/health}"
STATE_JSON="${REACTOR_OS_STATE_JSON:-/project/state.json}"
CONTROL_JSON="${REACTOR_OS_CONTROL_JSON:-/project/control.json}"

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

print_section "JSON Bridge"
printf 'state:   %s age=%ss\n' "$STATE_JSON" "$(file_age_seconds "$STATE_JSON")"
printf 'control: %s age=%ss\n' "$CONTROL_JSON" "$(file_age_seconds "$CONTROL_JSON")"
if [[ -r "$STATE_JSON" ]]; then
  head -c 600 "$STATE_JSON"
  printf '\n'
fi
