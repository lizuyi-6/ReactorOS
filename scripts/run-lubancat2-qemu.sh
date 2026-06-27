#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNTIME_DIR="${ROOT}/data/lubancat2-qemu"
PACKAGE_PATH=""
BIND="${REACTOR_OS_QEMU_BIND:-127.0.0.1:8000}"
ASSETS_PATH="${REACTOR_OS_QEMU_ASSETS:-}"
QEMU_BIN="${QEMU_AARCH64:-}"
SYSROOT="${AARCH64_SYSROOT:-}"
WITH_SIMULATOR=1
SEED_DEMO_CONTEXT=1
SMOKE=0

usage() {
  cat <<'EOF'
Run the LubanCat 2 / Cortex-A55 ARM64 ReactorOS package under QEMU user-mode.

Usage:
  scripts/run-lubancat2-qemu.sh [options]

Options:
  --package PATH       Package directory or .tar.gz. Defaults to dist/latest-lubancat2-debian10-package.txt
  --bind ADDR:PORT     Backend bind address. Default: 127.0.0.1:8000
  --assets PATH        HMI assets. Default: package frontend/dist when present, otherwise package static/
  --sysroot PATH       ARM64 sysroot containing /lib/ld-linux-aarch64.so.1
  --qemu PATH          qemu-aarch64 or qemu-aarch64-static path
  --no-simulator       Do not start the local JSON bridge simulator
  --no-demo-context    Do not seed demo processes/history/AI context
  --smoke              Start, probe /health /api/live /api/devices/status, then stop
  -h, --help           Show this help

Install dependencies in WSL/Ubuntu if missing:
  sudo apt-get update
  sudo apt-get install -y qemu-user gcc-aarch64-linux-gnu libc6-arm64-cross

This is user-mode emulation. It validates the ARM64 binary, Debian 10 runtime
dependencies, HTTP API, JSON bridge, and component control path. It does not
emulate RK3568 board peripherals, GPU, UART timing, or touch hardware.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --package)
      PACKAGE_PATH="${2:?missing value for --package}"
      shift 2
      ;;
    --bind)
      BIND="${2:?missing value for --bind}"
      shift 2
      ;;
    --assets)
      ASSETS_PATH="${2:?missing value for --assets}"
      shift 2
      ;;
    --sysroot)
      SYSROOT="${2:?missing value for --sysroot}"
      shift 2
      ;;
    --qemu)
      QEMU_BIN="${2:?missing value for --qemu}"
      shift 2
      ;;
    --no-simulator)
      WITH_SIMULATOR=0
      shift
      ;;
    --no-demo-context)
      SEED_DEMO_CONTEXT=0
      shift
      ;;
    --smoke)
      SMOKE=1
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

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing command: $1" >&2
    return 1
  fi
}

find_qemu() {
  if [[ -n "${QEMU_BIN}" ]]; then
    [[ -x "${QEMU_BIN}" ]] || { echo "QEMU binary is not executable: ${QEMU_BIN}" >&2; exit 1; }
    return
  fi
  QEMU_BIN="$(command -v qemu-aarch64 || true)"
  if [[ -z "${QEMU_BIN}" ]]; then
    QEMU_BIN="$(command -v qemu-aarch64-static || true)"
  fi
  if [[ -z "${QEMU_BIN}" ]]; then
    cat >&2 <<'EOF'
qemu-aarch64 was not found.

Install in WSL/Ubuntu:
  sudo apt-get update
  sudo apt-get install -y qemu-user
EOF
    exit 1
  fi
}

find_sysroot() {
  if [[ -n "${SYSROOT}" ]]; then
    [[ -e "${SYSROOT}/lib/ld-linux-aarch64.so.1" ]] || {
      echo "ARM64 loader not found under --sysroot: ${SYSROOT}/lib/ld-linux-aarch64.so.1" >&2
      exit 1
    }
    return
  fi
  for candidate in /usr/aarch64-linux-gnu /usr/aarch64-linux-gnu/libc; do
    if [[ -e "${candidate}/lib/ld-linux-aarch64.so.1" ]]; then
      SYSROOT="${candidate}"
      return
    fi
  done
  cat >&2 <<'EOF'
ARM64 sysroot was not found.

Install in WSL/Ubuntu:
  sudo apt-get update
  sudo apt-get install -y gcc-aarch64-linux-gnu libc6-arm64-cross

Then retry, or pass:
  --sysroot /usr/aarch64-linux-gnu
EOF
  exit 1
}

resolve_package() {
  if [[ -z "${PACKAGE_PATH}" ]]; then
    local pointer="${ROOT}/dist/latest-lubancat2-debian10-package.txt"
    [[ -f "${pointer}" ]] || {
      echo "Missing latest package pointer: ${pointer}" >&2
      echo "Build first: powershell -ExecutionPolicy Bypass -File scripts\\build-lubancat2-debian10.ps1" >&2
      exit 1
    }
    PACKAGE_PATH="$(tr -d '\r\n' < "${pointer}")"
  fi

  if [[ "${PACKAGE_PATH}" != /* ]]; then
    PACKAGE_PATH="${ROOT}/${PACKAGE_PATH}"
  fi

  if [[ "${PACKAGE_PATH}" == *.tar.gz ]]; then
    mkdir -p "${RUNTIME_DIR}/package"
    tar -xzf "${PACKAGE_PATH}" -C "${RUNTIME_DIR}/package"
    PACKAGE_PATH="$(find "${RUNTIME_DIR}/package" -mindepth 1 -maxdepth 1 -type d | sort | tail -n 1)"
  fi

  [[ -d "${PACKAGE_PATH}" ]] || { echo "Package directory not found: ${PACKAGE_PATH}" >&2; exit 1; }
  [[ -x "${PACKAGE_PATH}/bin/reactor-edge-daemon" ]] || {
    echo "ARM64 binary not found or not executable: ${PACKAGE_PATH}/bin/reactor-edge-daemon" >&2
    exit 1
  }
  if [[ -z "${ASSETS_PATH}" ]]; then
    if [[ -f "${PACKAGE_PATH}/frontend/dist/index.html" ]]; then
      ASSETS_PATH="${PACKAGE_PATH}/frontend/dist"
    else
      ASSETS_PATH="${PACKAGE_PATH}/static"
    fi
  elif [[ "${ASSETS_PATH}" != /* ]]; then
    ASSETS_PATH="${ROOT}/${ASSETS_PATH}"
  fi
  [[ -d "${ASSETS_PATH}" ]] || { echo "Assets directory not found: ${ASSETS_PATH}" >&2; exit 1; }
}

prepare_runtime_config() {
  mkdir -p "${RUNTIME_DIR}"
  local config="${RUNTIME_DIR}/device.qemu.json_bridge.toml"
  cp "${PACKAGE_PATH}/config/device.json_bridge.toml" "${config}"
  sed -i \
    -e "s|^state_path = .*|state_path = \"${RUNTIME_DIR}/state.json\"|" \
    -e "s|^control_path = .*|control_path = \"${RUNTIME_DIR}/control.json\"|" \
    "${config}"
  echo "${config}"
}

start_json_bridge_simulator() {
  need_cmd python3
  python3 - "${RUNTIME_DIR}/state.json" "${RUNTIME_DIR}/control.json" <<'PY' &
import json
import math
import os
import sys
import tempfile
import time

state_path, control_path = sys.argv[1], sys.argv[2]
os.makedirs(os.path.dirname(state_path), exist_ok=True)

started_ms = int(time.time() * 1000)
last_request_id = None
last_command = None
last_command_ok = None
last_command_error = None
relay = 0
motor = 1
tilt = 0
target_temp = 60.0
target_stir = 300.0
shake_cpm = 30.0
temperature = 31.11
pressure = 0.50
stirrer = 125.18
flow = 1.05
concentration = 11.10
ph = 6.15

def clamp(value, low, high):
    return min(high, max(low, value))

def approach(current, target, step):
    delta = target - current
    if abs(delta) <= step:
        return target
    return current + step * (1 if delta > 0 else -1)

def atomic_write(path, payload):
    directory = os.path.dirname(path)
    fd, tmp = tempfile.mkstemp(prefix=".state.", suffix=".tmp", dir=directory)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(payload, handle, ensure_ascii=False, indent=2)
            handle.write("\n")
        os.replace(tmp, path)
    finally:
        if os.path.exists(tmp):
            os.unlink(tmp)

def read_control():
    global last_request_id, last_command, last_command_ok, last_command_error
    global relay, motor, target_stir, shake_cpm
    try:
        with open(control_path, "r", encoding="utf-8") as handle:
            control = json.load(handle)
    except FileNotFoundError:
        return
    except Exception as exc:
        last_command_ok = False
        last_command_error = f"invalid control.json: {exc}"
        return
    request_id = control.get("request_id")
    if not request_id or request_id == last_request_id:
        return
    last_request_id = request_id
    command = str(control.get("command", ""))
    value = control.get("value")
    try:
        if command == "motor":
            motor = 1 if value else 0
        elif command == "relay":
            relay = 1 if value else 0
        elif command == "speed":
            if value == "up":
                shake_cpm = clamp(shake_cpm + 5, 0, 60)
                motor = 1 if shake_cpm > 0 else 0
            elif value == "down":
                shake_cpm = clamp(shake_cpm - 5, 0, 60)
                motor = 1 if shake_cpm > 0 else 0
            else:
                raise ValueError("speed command value must be up or down")
        elif command == "stir_speed":
            target_stir = clamp(float(value), 0, 2000)
        else:
            raise ValueError(f"unsupported command {command}")
        last_command = f"{command}:{json.dumps(value, ensure_ascii=False)}"
        last_command_ok = True
        last_command_error = None
    except Exception as exc:
        last_command = f"{command}:{json.dumps(value, ensure_ascii=False)}"
        last_command_ok = False
        last_command_error = str(exc)

while True:
    read_control()
    now_ms = int(time.time() * 1000)
    elapsed = (now_ms - started_ms) / 1000.0
    desired_temp = max(target_temp, 72.0) if relay else target_temp
    temperature = approach(temperature, desired_temp + math.sin(elapsed / 12.0) * 0.25, 0.35)
    stirrer = approach(stirrer, target_stir + math.sin(elapsed / 7.0) * 4.0, 12.0)
    pressure = approach(pressure, 0.42 + max(0, temperature - 35) * 0.002 + stirrer * 0.00003, 0.01)
    flow = approach(flow, 1.1 + (shake_cpm if motor else 0) * 0.015, 0.03)
    concentration = clamp(concentration + (0.025 if 45 <= temperature <= 95 else 0.006), 0, 98.5)
    ph = approach(ph, 6.35 - concentration * 0.004, 0.01)
    if motor and shake_cpm > 0.01:
        period_ms = 60000.0 / shake_cpm
        tilt = int((now_ms % period_ms) >= period_ms / 2)
    else:
        tilt = 0

    status = (1 if relay else 0) | (2 if motor else 0) | (4 if tilt else 0)
    sample = {
        "connected": True,
        "last_seen_ms": now_ms,
        "last_frame_hex": "QEMU_LUBANCAT2_JSON_BRIDGE_SIM",
        "last_frame_ok": True,
        "adc": round(clamp(concentration, 0, 100) / 0.0244200244),
        "status": status,
        "relay": relay,
        "motor": motor,
        "tilt": tilt,
        "speed_delay_us": None if shake_cpm <= 0.01 else round(60000000 / (shake_cpm * 200)),
        "last_command": last_command,
        "last_command_request_id": last_request_id,
        "last_command_sent_ms": now_ms if last_request_id else None,
        "last_command_ok": last_command_ok,
        "last_command_error": last_command_error,
        "port": "qemu-json-bridge",
        "baudrate": 115200,
        "bridge_started_ms": started_ms,
        "temperature_c": round(temperature, 2),
        "pressure_mpa": round(pressure, 2),
        "stirrer_rpm": round(stirrer, 2),
        "shake_speed_cpm": round(shake_cpm if motor else 0.0, 2),
        "flow_rate_l_min": round(flow, 2),
        "product_concentration_percent": round(concentration, 2),
        "ph": round(ph, 2),
    }
    atomic_write(state_path, sample)
    time.sleep(1.0)
PY
  SIM_PID=$!
}

poll_http() {
  local url="$1"
  local attempts="${2:-30}"
  for _ in $(seq 1 "${attempts}"); do
    if curl -fsS "${url}" >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  return 1
}

find_qemu
find_sysroot
resolve_package
CONFIG_PATH="$(prepare_runtime_config)"

echo "LubanCat 2 QEMU user-mode runtime"
echo "  package:  ${PACKAGE_PATH}"
echo "  qemu:     ${QEMU_BIN}"
echo "  sysroot:  ${SYSROOT}"
echo "  config:   ${CONFIG_PATH}"
echo "  assets:   ${ASSETS_PATH}"
echo "  state:    ${RUNTIME_DIR}/state.json"
echo "  control:  ${RUNTIME_DIR}/control.json"
echo "  bind:     ${BIND}"

SIM_PID=""
APP_PID=""
cleanup() {
  if [[ -n "${APP_PID}" ]] && kill -0 "${APP_PID}" >/dev/null 2>&1; then
    kill "${APP_PID}" >/dev/null 2>&1 || true
    wait "${APP_PID}" >/dev/null 2>&1 || true
  fi
  if [[ -n "${SIM_PID}" ]] && kill -0 "${SIM_PID}" >/dev/null 2>&1; then
    kill "${SIM_PID}" >/dev/null 2>&1 || true
    wait "${SIM_PID}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT INT TERM

if [[ "${WITH_SIMULATOR}" -eq 1 ]]; then
  start_json_bridge_simulator
  echo "  simulator pid: ${SIM_PID}"
fi

EXTRA_ARGS=()
if [[ "${SEED_DEMO_CONTEXT}" -eq 1 ]]; then
  EXTRA_ARGS+=(--seed-demo-context)
fi

if [[ -f "${PACKAGE_PATH}/config/reactor-edge.env" ]]; then
  set -a
  # shellcheck disable=SC1091
  source "${PACKAGE_PATH}/config/reactor-edge.env"
  set +a
fi

"${QEMU_BIN}" -L "${SYSROOT}" \
  -E "LD_LIBRARY_PATH=/usr/lib/aarch64-linux-gnu:/lib/aarch64-linux-gnu" \
  "${PACKAGE_PATH}/bin/reactor-edge-daemon" \
  --config "${CONFIG_PATH}" \
  --safety "${PACKAGE_PATH}/config/safety.toml" \
  --memory "${PACKAGE_PATH}/config/ai_memory.toml" \
  --db "${RUNTIME_DIR}/reactor.sqlite3" \
  --assets "${ASSETS_PATH}" \
  --bind "${BIND}" \
  "${EXTRA_ARGS[@]}" &
APP_PID="$!"

if [[ "${SMOKE}" -eq 1 ]]; then
  need_cmd curl
  if ! poll_http "http://${BIND}/health" 30; then
    echo "Backend did not become healthy on http://${BIND}/health" >&2
    exit 1
  fi
  echo "health:"
  curl -fsS "http://${BIND}/health"
  echo
  echo "live:"
  curl -fsS "http://${BIND}/api/live"
  echo
  echo "devices:"
  curl -fsS "http://${BIND}/api/devices/status"
  echo
  exit 0
fi

echo
echo "Open the emulated HMI:"
echo "  http://${BIND}/"
echo
echo "Press Ctrl+C to stop QEMU and the local JSON bridge simulator."
wait "${APP_PID}"
