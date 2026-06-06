#!/usr/bin/env bash
# One-button upper-computer acceptance: bring up the daemon, start every
# external mock the upper computer would otherwise depend on (Modbus
# slave, AINAS server, MQTT broker), and run the full verification
# matrix in order. The script writes a single acceptance-report.md and
# acceptance-report.json into output/acceptance/.
#
# Usage:
#   bash scripts/acceptance/accept-all.sh
#
# Exit code is 0 only if every step in the matrix passes; otherwise the
# script returns the number of failed steps.

set -uo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

OUT_DIR="$ROOT/output/acceptance"
mkdir -p "$OUT_DIR"
REPORT_JSON="$OUT_DIR/acceptance-report.json"
REPORT_MD="$OUT_DIR/acceptance-report.md"
LOG_DIR="$ROOT/output/acceptance/logs"
mkdir -p "$LOG_DIR"

mkdir -p "$ROOT/output/local-run"
STEPS=()
record() {
  local name="$1"; local status="$2"; local info="$3"
  STEPS+=("{\"step\":\"$name\",\"status\":\"$status\",\"info\":\"$info\"}")
  echo "[$status] $name :: $info"
}
DAEMON_BIN="$ROOT/target/debug/reactor-edge-daemon"
if [[ ! -x "$DAEMON_BIN" ]]; then
  echo "daemon binary not built; run: cargo build --bin reactor-edge-daemon" >&2
  exit 1
fi

PORT=18300
DATA_DB="$ROOT/output/acceptance/acceptance.sqlite3"
VITE_LOG="$LOG_DIR/vite.log"
VITE_PID=""

start_vite() {
  # Vite proxies /health and /api via vite.config.ts; the new config
  # honors the XINGSHU_VITE_API_TARGET env var so the same dev server
  # can point at the acceptance daemon (18300) instead of the
  # hard-coded 8000.
  XINGSHU_VITE_API_TARGET="http://127.0.0.1:${PORT}" npm run frontend:dev -- --port 5173 > "$VITE_LOG" 2>&1 &
  VITE_PID=$!
  for i in $(seq 1 30); do
    if curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:5173/" | grep -q 200; then
      return 0
    fi
    sleep 1
  done
  return 1
}
INTEGRATION_TMP="$(mktemp -d)/integration.toml"
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

# 1. Start the daemon.
DAEMON_LOG="$LOG_DIR/daemon.log"
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
trap 'kill $DAEMON_PID $SIM_PID $VITE_PID 2>/dev/null || true; rm -f "$DATA_DB"' EXIT

# 2. Wait for health.
for i in $(seq 1 30); do
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

# 2b. Start Vite dev pointed at the acceptance daemon so the Vue
# acceptance scripts exercise the right backend, not whatever
# happens to be on port 8000.
if start_vite; then
  record "vite-dev" "ok" "vite dev on 5173 proxied to ${PORT}"
else
  record "vite-dev" "fail" "vite dev did not start; see $VITE_LOG"
  exit 1
fi

# 3. Start a sample simulator so control writes see fresh data.
SIM_LOG="$LOG_DIR/sim.log"
node scripts/simulate-device.js \
  --url "http://127.0.0.1:${PORT}" \
  --profile production \
  --interval-ms 1000 \
  > "$SIM_LOG" 2>&1 &
SIM_PID=$!
trap 'kill $DAEMON_PID $SIM_PID 2>/dev/null || true; rm -f "$DATA_DB"' EXIT
sleep 4

# 4. Step matrix. Each helper records its own result; we only use the
# return status to count pass/fail, never piping `record` into a head
# filter (which would route the array += into a subshell and drop it).
api_pass=0; api_fail=0
verify_rbac() {
  local log="$LOG_DIR/load-and-rbac.log"
  local result
  result=$(powershell -NoProfile -File "$ROOT/scripts/verify-load-and-rbac.ps1" -Base "http://127.0.0.1:${PORT}" 2>&1)
  local rc=$?
  echo "$result" > "$log"
  if [[ $rc -eq 0 ]]; then
    record "verify-load-and-rbac" "ok" "RBAC matrix all-pass; see $log"
    api_pass=$((api_pass+1))
  else
    local last=$(tail -1 "$log" | tr -d '\r')
    record "verify-load-and-rbac" "fail" "exit=$rc :: $last"
    api_fail=$((api_fail+1))
  fi
}
verify_vue_parity() {
  local out
  out=$(E2E_BASE_URL="http://127.0.0.1:${PORT}" VUE_URL="http://127.0.0.1:5173/" node scripts/verify-vue-parity.mjs 2>&1)
  local rc=$?
  echo "$out" > "$LOG_DIR/vue-parity.log"
  if [[ $rc -eq 0 ]]; then
    record "verify-vue-parity" "ok" "Vue 7 routes + bilingual ok"
    api_pass=$((api_pass+1))
  else
    record "verify-vue-parity" "fail" "$(tail -3 $LOG_DIR/vue-parity.log | head -1)"
    api_fail=$((api_fail+1))
  fi
}
verify_vue_lifecycle() {
  local out
  out=$(E2E_BASE_URL="http://127.0.0.1:${PORT}" VUE_URL="http://127.0.0.1:5173/" node scripts/verify-vue-process-lifecycle.mjs 2>&1)
  local rc=$?
  echo "$out" > "$LOG_DIR/vue-lifecycle.log"
  if [[ $rc -eq 0 ]]; then
    record "verify-vue-process-lifecycle" "ok" "工艺生命周期 + 中英 ok"
    api_pass=$((api_pass+1))
  else
    record "verify-vue-process-lifecycle" "fail" "$(tail -3 $LOG_DIR/vue-lifecycle.log | head -1)"
    api_fail=$((api_fail+1))
  fi
}
verify_probe_cli_ops() {
  local log="$LOG_DIR/probe-cli-ops.log"
  if powershell -NoProfile -File "$ROOT/scripts/probe-cli-ops.ps1" > "$log" 2>&1; then
    record "probe-cli-ops" "ok" "real SQLite backup/restore/wipe/key generate"
    api_pass=$((api_pass+1))
  else
    record "probe-cli-ops" "fail" "$(tail -3 $log | head -1)"
    api_fail=$((api_fail+1))
  fi
}
verify_ainas_mqtt() {
  local log="$LOG_DIR/ainas-mqtt.log"
  if E2E_BASE_URL="http://127.0.0.1:${PORT}" node scripts/verify-ainas-mqtt.mjs > "$log" 2>&1; then
    record "verify-ainas-mqtt" "ok" "AINAS API + config summary"
    api_pass=$((api_pass+1))
  else
    record "verify-ainas-mqtt" "fail" "$(tail -3 $log | head -1)"
    api_fail=$((api_fail+1))
  fi
}

verify_rbac
verify_vue_parity
verify_vue_lifecycle
verify_probe_cli_ops
verify_ainas_mqtt

# 5. Aggregate.
TOTAL=$((api_pass + api_fail))
STATUS="ok"
if [[ $api_fail -gt 0 ]]; then STATUS="fail"; fi

# 6. JSON report.
{
  echo "{"
  echo "  \"status\": \"$STATUS\","
  echo "  \"base_url\": \"http://127.0.0.1:${PORT}\","
  echo "  \"steps_pass\": $api_pass,"
  echo "  \"steps_fail\": $api_fail,"
  echo "  \"total\": $TOTAL,"
  echo "  \"commit\": \"$(git rev-parse HEAD 2>/dev/null || echo unknown)\","
  echo "  \"steps\": ["
  first=1
  for s in "${STEPS[@]}"; do
    if [[ $first -eq 0 ]]; then echo ","; fi
    echo "    $s"
    first=0
  done
  echo "  ]"
  echo "}"
} > "$REPORT_JSON"

# 7. Markdown report.
{
  echo "# 上位机一键验收报告"
  echo
  echo "- commit: \`$(git rev-parse --short HEAD 2>/dev/null || echo unknown)\`"
  echo "- base URL: \`http://127.0.0.1:${PORT}\`"
  echo "- 步骤通过 / 失败 / 总计: **${api_pass} / ${api_fail} / ${TOTAL}**"
  echo "- 最终状态: **${STATUS^^}**"
  echo
  echo "## 步骤"
  echo
  echo "| 步骤 | 状态 | 说明 |"
  echo "|---|---|---|"
  for s in "${STEPS[@]}"; do
    name=$(echo "$s" | python -c "import sys,json; print(json.load(sys.stdin)['step'])")
    status=$(echo "$s" | python -c "import sys,json; print(json.load(sys.stdin)['status'])")
    info=$(echo "$s" | python -c "import sys,json; print(json.load(sys.stdin).get('info',''))")
    echo "| \`$name\` | $status | $info |"
  done
  echo
  echo "## 报告位置"
  echo
  echo "- JSON: \`output/acceptance/acceptance-report.json\`"
  echo "- Markdown: \`output/acceptance/acceptance-report.md\`"
  echo "- 日志: \`output/acceptance/logs/\`"
} > "$REPORT_MD"

echo
echo "report -> $REPORT_JSON"
echo "report -> $REPORT_MD"

if [[ $api_fail -gt 0 ]]; then exit 1; fi
exit 0
