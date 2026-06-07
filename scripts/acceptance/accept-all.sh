#!/usr/bin/env bash
# One-button upper-computer acceptance: bring up the daemon, start the
# local sample simulator, run the core verification matrix, and probe the
# shipped AINAS / STM32 mock entrypoints. The mosquitto broker drill remains
# a dedicated script because it requires Docker and an external broker port.
#
# Usage:
#   bash scripts/acceptance/accept-all.sh
#
# Exit code is 0 only if every step in the matrix passes.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

OUT_DIR="$ROOT/output/acceptance"
REPORT_JSON="$OUT_DIR/acceptance-report.json"
REPORT_MD="$OUT_DIR/acceptance-report.md"
STEPS_JSONL="$OUT_DIR/acceptance-steps.jsonl"
LOG_DIR="$OUT_DIR/logs"
mkdir -p "$LOG_DIR" "$ROOT/output/local-run"
: > "$STEPS_JSONL"

record() {
  local name="$1"
  local status="$2"
  local info="$3"
  STEP_NAME="$name" STEP_STATUS="$status" STEP_INFO="$info" python - <<'PY' >> "$STEPS_JSONL"
import json
import os

print(json.dumps({
    "step": os.environ["STEP_NAME"],
    "status": os.environ["STEP_STATUS"],
    "info": os.environ.get("STEP_INFO", ""),
}, ensure_ascii=False))
PY
  echo "[$status] $name :: $info"
}

tail_summary() {
  local path="$1"
  if [[ -f "$path" ]]; then
    tail -3 "$path" | head -1 | tr -d '\r'
  else
    echo "log file not found: $path"
  fi
}

PORT=18300
VUE_PORT=15173
DATA_DB="$OUT_DIR/acceptance.sqlite3"
VITE_LOG="$LOG_DIR/vite.log"
DAEMON_LOG="$LOG_DIR/daemon.log"
SIM_LOG="$LOG_DIR/sim.log"
VITE_PID=""
DAEMON_PID=""
SIM_PID=""
AINAS_PID=""
STM32_PID=""

cleanup() {
  for pid in "$VITE_PID" "$SIM_PID" "$AINAS_PID" "$STM32_PID" "$DAEMON_PID"; do
    if [[ -n "${pid:-}" ]]; then
      kill "$pid" 2>/dev/null || true
    fi
  done
  rm -f "$DATA_DB" "$DATA_DB-wal" "$DATA_DB-shm" "$DATA_DB-journal"
}
trap cleanup EXIT

resolve_daemon_bin() {
  local native="$ROOT/target/debug/reactor-edge-daemon"
  local windows="$ROOT/target/debug/reactor-edge-daemon.exe"
  local cargo_bin=""
  local kernel
  kernel="$(uname -s 2>/dev/null || echo unknown)"
  if [[ -x "$native" ]]; then
    echo "$native"
    return 0
  fi
  case "$kernel" in
    MINGW*|MSYS*|CYGWIN*)
      if [[ -f "$windows" ]]; then
        echo "$windows"
        return 0
      fi
      ;;
  esac
  if command -v cargo >/dev/null 2>&1; then
    cargo_bin="cargo"
  elif [[ -x "$HOME/.cargo/bin/cargo" ]]; then
    cargo_bin="$HOME/.cargo/bin/cargo"
  fi
  if [[ -n "$cargo_bin" && "${XINGSHU_ACCEPTANCE_BUILD_NATIVE:-0}" == "1" ]]; then
    echo "native daemon binary missing for ${kernel}; building with current cargo..." >&2
    CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}" "$cargo_bin" build --bin reactor-edge-daemon >&2 || return 1
  fi
  if [[ -x "$native" ]]; then
    echo "$native"
    return 0
  fi
  case "$kernel" in
    MINGW*|MSYS*|CYGWIN*)
      if [[ -f "$windows" ]]; then
        echo "$windows"
        return 0
      fi
      ;;
  esac
  if [[ "$kernel" == Linux* && -f "$windows" ]]; then
    echo "found Windows daemon binary but this Bash runtime cannot execute it; run scripts/acceptance/accept-all.ps1 from PowerShell or set XINGSHU_ACCEPTANCE_BUILD_NATIVE=1 after installing a working Linux linker" >&2
  fi
  return 1
}

DAEMON_BIN="$(resolve_daemon_bin || true)"
if [[ ! -x "$DAEMON_BIN" ]]; then
  echo "daemon binary not built; run: cargo build --bin reactor-edge-daemon" >&2
  exit 1
fi

api_pass=0
api_fail=0

vue_release_log="$LOG_DIR/vue-release-assets.log"
if node scripts/verify-vue-release-assets.mjs > "$vue_release_log" 2>&1; then
  record "verify-vue-release-assets" "ok" "release package and systemd paths prefer Vue dist with legacy fallback"
  api_pass=$((api_pass + 1))
else
  record "verify-vue-release-assets" "fail" "$(tail_summary "$vue_release_log")"
  api_fail=$((api_fail + 1))
fi

safety_guard_log="$LOG_DIR/production-safety-guard.log"
if node scripts/verify-production-safety-guard.mjs > "$safety_guard_log" 2>&1; then
  record "verify-production-safety-guard" "ok" "release package and systemd launch the isolated safety guard"
  api_pass=$((api_pass + 1))
else
  record "verify-production-safety-guard" "fail" "$(tail_summary "$safety_guard_log")"
  api_fail=$((api_fail + 1))
fi

backup_schedule_log="$LOG_DIR/production-backup-schedule.log"
if node scripts/verify-production-backup-schedule.mjs > "$backup_schedule_log" 2>&1; then
  record "verify-production-backup-schedule" "ok" "systemd timer schedules online SQLite backup snapshots"
  api_pass=$((api_pass + 1))
else
  record "verify-production-backup-schedule" "fail" "$(tail_summary "$backup_schedule_log")"
  api_fail=$((api_fail + 1))
fi

backup_script_log="$LOG_DIR/production-backup-script.log"
if powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-production-backup-script.ps1 > "$backup_script_log" 2>&1; then
  record "verify-production-backup-script" "ok" "backup script writes timestamped SQLite snapshots and latest link"
  api_pass=$((api_pass + 1))
else
  record "verify-production-backup-script" "fail" "$(tail_summary "$backup_script_log")"
  api_fail=$((api_fail + 1))
fi

restore_drill_log="$LOG_DIR/backup-restore-drill.log"
if powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify-backup-restore-drill.ps1 > "$restore_drill_log" 2>&1; then
  record "verify-backup-restore-drill" "ok" "restored snapshot boots a fresh daemon with batch, product result, and audit chain intact"
  api_pass=$((api_pass + 1))
else
  record "verify-backup-restore-drill" "fail" "$(tail_summary "$restore_drill_log")"
  api_fail=$((api_fail + 1))
fi

training_deliverables_log="$LOG_DIR/training-deliverables.log"
if node scripts/verify-training-deliverables.mjs > "$training_deliverables_log" 2>&1; then
  record "verify-training-deliverables" "ok" "training deck, PPTX package, image assets, UAT script, and preview manifest passed"
  api_pass=$((api_pass + 1))
else
  record "verify-training-deliverables" "fail" "$(tail_summary "$training_deliverables_log")"
  api_fail=$((api_fail + 1))
fi

preflight_log="$LOG_DIR/xingshu-ops-preflight.log"
if XINGSHU_AUTH_SECRET="0123456789abcdef0123456789abcdef" \
  XINGSHU_OPERATOR_PASSWORD="operator-password-123" \
  XINGSHU_ENGINEER_PASSWORD="engineer-password-123" \
  XINGSHU_ADMIN_PASSWORD="admin-password-123" \
  XINGSHU_DB_ENCRYPTION_KEY="0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" \
  cargo run --quiet --bin xingshu -- ops preflight --production --json > "$preflight_log" 2>&1; then
  record "xingshu-ops-preflight" "ok" "production secrets, TLS paths, and backup timer files checked"
  api_pass=$((api_pass + 1))
else
  record "xingshu-ops-preflight" "fail" "$(tail_summary "$preflight_log")"
  api_fail=$((api_fail + 1))
fi

start_vite() {
  XINGSHU_VITE_API_TARGET="http://127.0.0.1:${PORT}" npm run frontend:dev -- --port "$VUE_PORT" --strictPort > "$VITE_LOG" 2>&1 &
  VITE_PID=$!
  for _ in $(seq 1 30); do
    if ! kill -0 "$VITE_PID" 2>/dev/null; then
      return 1
    fi
    if curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:${VUE_PORT}/" | grep -q 200; then
      return 0
    fi
    sleep 1
  done
  return 1
}

INTEGRATION_DIR="$(mktemp -d)"
INTEGRATION_TMP="$INTEGRATION_DIR/integration.toml"
cat > "$INTEGRATION_TMP" <<EOF
[mqtt]
enabled = false
host = "127.0.0.1"
port = 1883
use_tls = false
client_id = "xingshu-acceptance"
keep_alive_s = 30
queue_capacity = 100
status_topic = "xingshu/reactor_001/status"
task_topic = "xingshu/reactor_001/tasks"
receipt_topic = "xingshu/reactor_001/task_receipts"
alert_topic = "xingshu/reactor_001/alerts"
alert_interval_s = 5

[modbus_tcp]
enabled = false
bind = "0.0.0.0:502"
require_tls = false
unit_id = 1
max_pdu_bytes = 260
EOF

"$DAEMON_BIN" \
  --config config/device.toml \
  --safety config/safety.toml \
  --memory config/ai_memory.toml \
  --integration "$INTEGRATION_TMP" \
  --db "$DATA_DB" \
  --assets auto \
  --bind "127.0.0.1:${PORT}" \
  --enable-test-reset \
  > "$DAEMON_LOG" 2>&1 &
DAEMON_PID=$!

for _ in $(seq 1 30); do
  if curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:${PORT}/health" | grep -q 200; then
    break
  fi
  sleep 1
done
DAEMON_OK=$(curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:${PORT}/health")
if [[ "$DAEMON_OK" != "200" ]]; then
  echo "daemon did not become healthy on ${PORT}; see $DAEMON_LOG" >&2
  exit 1
fi

if start_vite; then
  record "vite-dev" "ok" "vite dev on ${VUE_PORT} proxied to ${PORT}"
  api_pass=$((api_pass + 1))
else
  record "vite-dev" "fail" "vite dev did not start; see $VITE_LOG"
  api_fail=$((api_fail + 1))
  exit 1
fi

node scripts/simulate-device.js \
  --url "http://127.0.0.1:${PORT}" \
  --profile production \
  --interval-ms 1000 \
  > "$SIM_LOG" 2>&1 &
SIM_PID=$!
sleep 4

verify_rbac() {
  local log="$LOG_DIR/load-and-rbac.log"
  if powershell -NoProfile -File "$ROOT/scripts/verify-load-and-rbac.ps1" -Base "http://127.0.0.1:${PORT}" > "$log" 2>&1; then
    record "verify-load-and-rbac" "ok" "RBAC matrix all-pass; see $log"
    api_pass=$((api_pass + 1))
  else
    record "verify-load-and-rbac" "fail" "$(tail_summary "$log")"
    api_fail=$((api_fail + 1))
  fi
}

verify_vue_parity() {
  local log="$LOG_DIR/vue-parity.log"
  if E2E_BASE_URL="http://127.0.0.1:${PORT}" VUE_URL="http://127.0.0.1:${VUE_PORT}/" node scripts/verify-vue-parity.mjs > "$log" 2>&1; then
    record "verify-vue-parity" "ok" "Vue 7 routes and bilingual checks passed"
    api_pass=$((api_pass + 1))
  else
    record "verify-vue-parity" "fail" "$(tail_summary "$log")"
    api_fail=$((api_fail + 1))
  fi
}

verify_vue_history_xlsx() {
  local log="$LOG_DIR/vue-history-xlsx.log"
  if E2E_BASE_URL="http://127.0.0.1:${PORT}" VUE_URL="http://127.0.0.1:${VUE_PORT}/" node scripts/verify-vue-history-xlsx.mjs > "$log" 2>&1; then
    record "verify-vue-history-xlsx" "ok" "History CSV/XLSX downloads and bilingual buttons passed"
    api_pass=$((api_pass + 1))
  else
    record "verify-vue-history-xlsx" "fail" "$(tail_summary "$log")"
    api_fail=$((api_fail + 1))
  fi
}

verify_vue_lifecycle() {
  local log="$LOG_DIR/vue-lifecycle.log"
  if E2E_BASE_URL="http://127.0.0.1:${PORT}" VUE_URL="http://127.0.0.1:${VUE_PORT}/" node scripts/verify-vue-process-lifecycle.mjs > "$log" 2>&1; then
    record "verify-vue-process-lifecycle" "ok" "process lifecycle and bilingual checks passed"
    api_pass=$((api_pass + 1))
  else
    record "verify-vue-process-lifecycle" "fail" "$(tail_summary "$log")"
    api_fail=$((api_fail + 1))
  fi
}

verify_vue_mobile() {
  local log="$LOG_DIR/vue-mobile.log"
  if E2E_BASE_URL="http://127.0.0.1:${PORT}" VUE_URL="http://127.0.0.1:${VUE_PORT}/" node scripts/verify-vue-mobile.mjs > "$log" 2>&1; then
    record "verify-vue-mobile" "ok" "phone and tablet viewport bilingual navigation checks passed"
    api_pass=$((api_pass + 1))
  else
    record "verify-vue-mobile" "fail" "$(tail_summary "$log")"
    api_fail=$((api_fail + 1))
  fi
}

verify_vue_browser_matrix() {
  local log="$LOG_DIR/vue-browser-matrix.log"
  if E2E_BASE_URL="http://127.0.0.1:${PORT}" VUE_URL="http://127.0.0.1:${VUE_PORT}/" PLAYWRIGHT_BROWSER_MATRIX_STRICT=1 node scripts/verify-vue-browser-matrix.mjs > "$log" 2>&1; then
    local info
    info="$(python - <<'PY'
import json
from pathlib import Path

path = Path("output/playwright/vue-browser-matrix-verification.json")
if not path.exists():
    print("browser matrix passed; report missing")
else:
    report = json.loads(path.read_text(encoding="utf-8"))
    passed = [item["name"] for item in report.get("browsers", []) if item.get("status") == "ok"]
    skipped = [item["name"] for item in report.get("browsers", []) if item.get("skipped")]
    page_checks = sum(len(item.get("pages", [])) for item in report.get("browsers", []))
    console_errors = len(report.get("unexpectedConsoleMessages", []))
    print(f"passed browsers: {', '.join(passed)}; skipped: {', '.join(skipped) or 'none'}; page checks: {page_checks}; console errors: {console_errors}")
PY
)"
    record "verify-vue-browser-matrix" "ok" "$info"
    api_pass=$((api_pass + 1))
  else
    record "verify-vue-browser-matrix" "fail" "$(tail_summary "$log")"
    api_fail=$((api_fail + 1))
  fi
}

verify_probe_cli_ops() {
  local log="$LOG_DIR/probe-cli-ops.log"
  if powershell -NoProfile -File "$ROOT/scripts/probe-cli-ops.ps1" > "$log" 2>&1; then
    record "probe-cli-ops" "ok" "real SQLite backup/restore/wipe/key generate/key rekey"
    api_pass=$((api_pass + 1))
  else
    record "probe-cli-ops" "fail" "$(tail_summary "$log")"
    api_fail=$((api_fail + 1))
  fi
}

verify_ainas_mqtt() {
  local log="$LOG_DIR/ainas-mqtt.log"
  if E2E_BASE_URL="http://127.0.0.1:${PORT}" node scripts/verify-ainas-mqtt.mjs > "$log" 2>&1; then
    record "verify-ainas-mqtt" "ok" "AINAS API and integration config summary passed"
    api_pass=$((api_pass + 1))
  else
    record "verify-ainas-mqtt" "fail" "$(tail_summary "$log")"
    api_fail=$((api_fail + 1))
  fi
}

verify_mosquitto_broker() {
  local log="$LOG_DIR/mqtt-broker.log"
  if ! command -v docker >/dev/null 2>&1; then
    record "verify-mosquitto-broker" "skipped" "Docker not available; run scripts/mocks/verify-mosquitto-broker.sh when Docker is installed"
    return
  fi
  if bash scripts/mocks/verify-mosquitto-broker.sh > "$log" 2>&1; then
    local status
    status="$(python - <<'PY'
import json
from pathlib import Path

path = Path("output/acceptance/mqtt-broker-report.json")
if not path.exists():
    print("missing")
else:
    print(json.loads(path.read_text(encoding="utf-8")).get("status", "missing"))
PY
)"
    case "$status" in
      ok)
        record "verify-mosquitto-broker" "ok" "real broker status/task/receipt round-trip passed"
        api_pass=$((api_pass + 1))
        ;;
      skipped)
        record "verify-mosquitto-broker" "skipped" "broker drill skipped; see $log"
        ;;
      *)
        record "verify-mosquitto-broker" "fail" "$(tail_summary "$log")"
        api_fail=$((api_fail + 1))
        ;;
    esac
  else
    record "verify-mosquitto-broker" "fail" "$(tail_summary "$log")"
    api_fail=$((api_fail + 1))
  fi
}

verify_local_mocks() {
  local log="$LOG_DIR/mock-entrypoints.log"
  if node --check scripts/mocks/ainas-mock-server.mjs > "$log" 2>&1 \
    && node --check scripts/mocks/stm32-modbus-tcp-mock.mjs >> "$log" 2>&1 \
    && bash -n scripts/mocks/verify-mosquitto-broker.sh >> "$log" 2>&1; then
    record "mock-entrypoints-parse" "ok" "AINAS/STM32/mosquitto entrypoints parse"
    api_pass=$((api_pass + 1))
  else
    record "mock-entrypoints-parse" "fail" "$(tail_summary "$log")"
    api_fail=$((api_fail + 1))
    return
  fi

  local ainas_log="$LOG_DIR/ainas-mock.log"
  node scripts/mocks/ainas-mock-server.mjs --listen 127.0.0.1:5599 > "$ainas_log" 2>&1 &
  AINAS_PID=$!
  local ainas_ok=0
  for _ in $(seq 1 20); do
    if curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:5599/health" | grep -q 200; then
      ainas_ok=1
      break
    fi
    sleep 1
  done
  if [[ "$ainas_ok" -eq 1 ]]; then
    record "ainas-mock-health" "ok" "AINAS mock /health returned 200 on 127.0.0.1:5599"
    api_pass=$((api_pass + 1))
  else
    record "ainas-mock-health" "fail" "AINAS mock did not become healthy; see $ainas_log"
    api_fail=$((api_fail + 1))
    return
  fi

  local stm32_log="$LOG_DIR/stm32-modbus-mock.log"
  node scripts/mocks/stm32-modbus-tcp-mock.mjs --listen 127.0.0.1:15502 --registers config/device.toml > "$stm32_log" 2>&1 &
  STM32_PID=$!
  local modbus_ok=0
  for _ in $(seq 1 20); do
    if node - <<'NODE'
const net = require("node:net");
const frame = Buffer.from([
  0x00, 0x01, 0x00, 0x00, 0x00, 0x06, 0x01,
  0x03, 0x00, 0x00, 0x00, 0x01
]);
const socket = net.createConnection({ host: "127.0.0.1", port: 15502 });
const timer = setTimeout(() => {
  socket.destroy();
  process.exit(1);
}, 1000);
socket.on("connect", () => socket.write(frame));
socket.on("data", (data) => {
  clearTimeout(timer);
  const ok = data.length >= 11 && data[7] === 0x03 && data[8] === 0x02;
  socket.end();
  process.exit(ok ? 0 : 1);
});
socket.on("error", () => {
  clearTimeout(timer);
  process.exit(1);
});
NODE
    then
      modbus_ok=1
      break
    fi
    sleep 1
  done
  if [[ "$modbus_ok" -eq 1 ]]; then
    record "stm32-modbus-mock-fc03" "ok" "STM32 mock answered Modbus TCP FC03 on 127.0.0.1:15502"
    api_pass=$((api_pass + 1))
  else
    record "stm32-modbus-mock-fc03" "fail" "STM32 mock did not answer FC03 on 15502; see $stm32_log"
    api_fail=$((api_fail + 1))
  fi
}

verify_rbac
verify_vue_parity
verify_vue_history_xlsx
verify_vue_lifecycle
verify_vue_mobile
verify_vue_browser_matrix
verify_probe_cli_ops
verify_ainas_mqtt
verify_mosquitto_broker
verify_local_mocks

TOTAL=$((api_pass + api_fail))
STATUS="ok"
if [[ $api_fail -gt 0 ]]; then
  STATUS="fail"
fi
STEPS_JSON="$(python - "$STEPS_JSONL" <<'PY'
import json
import sys

items = []
with open(sys.argv[1], "r", encoding="utf-8") as fh:
    for line in fh:
        if line.strip():
            items.append(json.loads(line))
print(json.dumps(items, ensure_ascii=False, indent=4))
PY
)"

{
  echo "{"
  echo "  \"status\": \"$STATUS\","
  echo "  \"base_url\": \"http://127.0.0.1:${PORT}\","
  echo "  \"steps_pass\": $api_pass,"
  echo "  \"steps_fail\": $api_fail,"
  echo "  \"total\": $TOTAL,"
  echo "  \"commit\": \"$(git rev-parse HEAD 2>/dev/null || echo unknown)\","
  echo "  \"steps\": $STEPS_JSON"
  echo
  echo "}"
} > "$REPORT_JSON"

{
  echo "# Upper-Computer Acceptance Report"
  echo
  echo "- commit: \`$(git rev-parse --short HEAD 2>/dev/null || echo unknown)\`"
  echo "- base URL: \`http://127.0.0.1:${PORT}\`"
  echo "- steps pass / fail / total: **${api_pass} / ${api_fail} / ${TOTAL}**"
  echo "- final status: **${STATUS^^}**"
  echo
  echo "## Steps"
  echo
  echo "| Step | Status | Info |"
  echo "|---|---|---|"
  python - "$STEPS_JSONL" <<'PY'
import json
import sys

def esc(value):
    return str(value).replace("|", "\\|").replace("\n", " ")

with open(sys.argv[1], "r", encoding="utf-8") as fh:
    for line in fh:
        if not line.strip():
            continue
        item = json.loads(line)
        print(f"| `{esc(item['step'])}` | {esc(item['status'])} | {esc(item.get('info', ''))} |")
PY
  echo
  echo "## Report Files"
  echo
  echo "- JSON: \`output/acceptance/acceptance-report.json\`"
  echo "- Markdown: \`output/acceptance/acceptance-report.md\`"
  echo "- logs: \`output/acceptance/logs/\`"
} > "$REPORT_MD"

echo
echo "report -> $REPORT_JSON"
echo "report -> $REPORT_MD"

if [[ $api_fail -gt 0 ]]; then
  exit 1
fi
exit 0
