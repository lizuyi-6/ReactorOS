#!/usr/bin/env bash
# Local MQTT broker acceptance: start a mosquitto container, point the
# daemon's MQTT bridge at it, publish a task on the broker task topic, and
# confirm the daemon publishes real status and receipt payloads back to
# the broker.
#
# This script is the local stand-in for the PRD §9.3 third-party broker
# acceptance; it produces output/acceptance/mqtt-broker-report.json that
# the broader `scripts/acceptance/accept-all.sh` aggregates.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

OUT_DIR="$ROOT/output/acceptance"
mkdir -p "$OUT_DIR"
REPORT="$OUT_DIR/mqtt-broker-report.json"

LOG_DIR="$ROOT/output/local-run"
mkdir -p "$LOG_DIR"

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
    echo "found Windows daemon binary but this Bash runtime cannot execute it; run from a native shell or set XINGSHU_ACCEPTANCE_BUILD_NATIVE=1 after installing a working Linux linker" >&2
  fi
  return 1
}

DAEMON_BIN="$(resolve_daemon_bin || true)"
if [[ ! -x "$DAEMON_BIN" ]]; then
  echo "daemon binary not built; run: cargo build --bin reactor-edge-daemon" >&2
  echo "{\"status\":\"skipped\",\"reason\":\"daemon binary missing\"}" > "$REPORT"
  exit 0
fi

if ! command -v docker >/dev/null 2>&1; then
  echo "docker not available; cannot run mosquitto container" >&2
  echo "{\"status\":\"skipped\",\"reason\":\"no docker\"}" > "$REPORT"
  exit 0
fi

CONTAINER="xingshu-mqtt-mosquitto"
BROKER_PORT=1883
DOCKER_NET="bridge"
MOSQUITTO_CONF="$OUT_DIR/mosquitto.acceptance.conf"

# Pull eclipse-mosquitto and start with an explicit anonymous listener so
# the test is stable across image defaults.
cat > "$MOSQUITTO_CONF" <<'EOF'
listener 1883 0.0.0.0
allow_anonymous true
EOF
docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
docker run -d --name "$CONTAINER" -p "${BROKER_PORT}:1883" \
  -v "${MOSQUITTO_CONF}:/mosquitto/config/mosquitto.conf:ro" \
  eclipse-mosquitto:2.0 \
  >/dev/null
trap 'docker rm -f "$CONTAINER" >/dev/null 2>&1 || true' EXIT

# Wait for the broker port.
for i in $(seq 1 30); do
  if (echo > /dev/tcp/127.0.0.1/${BROKER_PORT}) 2>/dev/null; then
    break
  fi
  sleep 1
done

# Use the daemon's own simulator pipeline to push samples + a fake
# AINAS dispatch. We override integration.toml at runtime.
TMP_INTEGRATION="$(mktemp -d)/integration.toml"
cat > "$TMP_INTEGRATION" <<'EOF'
[mqtt]
enabled = true
host = "127.0.0.1"
port = 1883
use_tls = false
client_id = "xingshu-mqtt-acceptance"
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

# Start the daemon pointed at the mosquitto container.
DAEMON_LOG="$LOG_DIR/mqtt-acceptance-daemon.log"
"$DAEMON_BIN" \
  --config config/device.toml \
  --safety config/safety.toml \
  --memory config/ai_memory.toml \
  --integration "$TMP_INTEGRATION" \
  --db data/mqtt-acceptance.sqlite3 \
  --assets auto \
  --bind 127.0.0.1:18200 \
  --enable-test-reset \
  > "$DAEMON_LOG" 2>&1 &
DAEMON_PID=$!
trap 'kill $DAEMON_PID 2>/dev/null || true; docker rm -f "$CONTAINER" >/dev/null 2>&1 || true' EXIT

# Wait for the daemon health endpoint.
for i in $(seq 1 30); do
  if curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:18200/health" | grep -q 200; then
    break
  fi
  sleep 1
done

# Feed the daemon with live samples so the status/alert path has runtime
# data while MQTT is being exercised.
AINAS_LOG="$LOG_DIR/mqtt-acceptance-ainas.log"
node "$ROOT/scripts/simulate-device.js" \
  --url http://127.0.0.1:18200 \
  --profile production \
  --interval-ms 1000 \
  > "$AINAS_LOG" 2>&1 &
SIM_PID=$!
trap 'kill $SIM_PID $DAEMON_PID 2>/dev/null || true; docker rm -f "$CONTAINER" >/dev/null 2>&1 || true' EXIT
sleep 4

# Confirm the retained status frame exists on the broker, then subscribe
# to the receipt topic, publish a command onto the task topic, and validate
# the receipt payload received from mosquitto itself.
STATUS_PAYLOAD="$LOG_DIR/mqtt-status-payload.json"
RECEIPT_PAYLOAD="$LOG_DIR/mqtt-receipt-payload.json"
STATUS_SUB_OK=0
for i in $(seq 1 20); do
  if docker exec "$CONTAINER" mosquitto_sub -h 127.0.0.1 -p 1883 \
    -t "xingshu/reactor_001/status" -C 1 -W 3 > "$STATUS_PAYLOAD" 2>> "$DAEMON_LOG"; then
    STATUS_SUB_OK=1
    break
  fi
  sleep 1
done

TASK_EXTERNAL_ID="mqtt-acceptance-$(date +%s)"
RECEIPT_SUB_OK=0
TASK_PUBLISH_OK=0
docker exec "$CONTAINER" mosquitto_sub -h 127.0.0.1 -p 1883 \
  -t "xingshu/reactor_001/task_receipts" -C 1 -W 15 > "$RECEIPT_PAYLOAD" 2>> "$DAEMON_LOG" &
RECEIPT_SUB_PID=$!
sleep 1
TASK_PAYLOAD="$(python - "$TASK_EXTERNAL_ID" <<'PY'
import json
import sys

print(json.dumps({
    "external_task_id": sys.argv[1],
    "action": "set_targets",
    "target_temperature_c": 60,
    "target_stirrer_rpm": 300,
    "target_shake_speed_cpm": 0,
    "reason": "mqtt broker acceptance"
}, separators=(",", ":")))
PY
)"
if docker exec -i "$CONTAINER" mosquitto_pub -h 127.0.0.1 -p 1883 \
  -t "xingshu/reactor_001/tasks" -q 1 -m "$TASK_PAYLOAD"; then
  TASK_PUBLISH_OK=1
fi
if wait "$RECEIPT_SUB_PID"; then
  RECEIPT_SUB_OK=1
fi

MQTT_LOG_LINES=$(grep -c "MQTT bridge\|mqtt" "$DAEMON_LOG" || true)
PAYLOAD_VALIDATE_OK=0
if [[ "$STATUS_SUB_OK" -eq 1 && "$RECEIPT_SUB_OK" -eq 1 ]]; then
  if python - "$STATUS_PAYLOAD" "$RECEIPT_PAYLOAD" "$TASK_EXTERNAL_ID" <<'PY'
import json
import sys

status_path, receipt_path, external_id = sys.argv[1:4]
with open(status_path, "r", encoding="utf-8") as fh:
    status = json.load(fh)
with open(receipt_path, "r", encoding="utf-8") as fh:
    receipt = json.load(fh)

assert status.get("device_id") == "reactor_001", status
assert status.get("status") == "online", status
assert status.get("task_topic") == "xingshu/reactor_001/tasks", status
assert receipt.get("ok") is True, receipt
assert receipt.get("source") == "mqtt", receipt
assert receipt.get("external_task_id") == external_id, receipt
assert receipt.get("action") == "set_targets", receipt
assert receipt.get("status") == "executed", receipt
PY
  then
    PAYLOAD_VALIDATE_OK=1
  fi
fi

STATUS="ok"
if [[ "$STATUS_SUB_OK" -ne 1 || "$TASK_PUBLISH_OK" -ne 1 || "$RECEIPT_SUB_OK" -ne 1 || "$PAYLOAD_VALIDATE_OK" -ne 1 ]]; then
  STATUS="fail"
fi

cat > "$REPORT" <<EOF
{
  "status": "${STATUS}",
  "broker": "mosquitto:2.0 (docker)",
  "broker_port": ${BROKER_PORT},
  "daemon_bound": "127.0.0.1:18200",
  "log_lines_with_mqtt_keyword": ${MQTT_LOG_LINES},
  "status_subscribe_ok": ${STATUS_SUB_OK},
  "task_publish_ok": ${TASK_PUBLISH_OK},
  "receipt_subscribe_ok": ${RECEIPT_SUB_OK},
  "payload_validate_ok": ${PAYLOAD_VALIDATE_OK},
  "status_payload": "${STATUS_PAYLOAD}",
  "receipt_payload": "${RECEIPT_PAYLOAD}",
  "external_task_id": "${TASK_EXTERNAL_ID}"
}
EOF

echo "mqtt broker acceptance report -> $REPORT"
if [[ "$STATUS" != "ok" ]]; then
  echo "mqtt broker acceptance failed: status_subscribe=${STATUS_SUB_OK} task_publish=${TASK_PUBLISH_OK} receipt_subscribe=${RECEIPT_SUB_OK} payload_validate=${PAYLOAD_VALIDATE_OK}" >&2
  exit 1
fi
