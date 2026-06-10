#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

FAKE_BIN="${TMP_ROOT}/fake-bin"
mkdir -p "$FAKE_BIN"

cat >"${FAKE_BIN}/curl" <<'EOF'
#!/usr/bin/env bash
url="${*: -1}"
case "$url" in
  */health)
    printf '{"ok":true,"service":"reactor-edge-daemon"}\n'
    ;;
  */api/devices/status)
    case "${FAKE_STATUS_MODE:-idle}" in
      idle)
        printf '{"data":{"devices":[{"device_id":"R1","online":true,"status":"idle","active_batch_id":null,"emergency_stop":false,"auto_enabled":false,"manual_lock":false,"last_control_error":null,"last_command_ok":true}]}}\n'
        ;;
      auto)
        printf '{"data":{"devices":[{"device_id":"R1","online":true,"status":"idle","active_batch_id":null,"emergency_stop":false,"auto_enabled":true,"manual_lock":false,"last_control_error":null,"last_command_ok":true}]}}\n'
        ;;
      fault)
        printf '{"data":{"devices":[{"device_id":"R1","online":true,"status":"error","active_batch_id":null,"emergency_stop":false,"auto_enabled":false,"manual_lock":false,"last_control_error":"write timeout","last_command_ok":true}]}}\n'
        ;;
      offline)
        printf '{"data":{"devices":[{"device_id":"R1","online":false,"status":"offline","active_batch_id":null,"emergency_stop":false,"auto_enabled":false,"manual_lock":false,"last_control_error":null,"last_command_ok":true}]}}\n'
        ;;
      *)
        printf 'unknown FAKE_STATUS_MODE=%s\n' "${FAKE_STATUS_MODE:-}" >&2
        exit 2
        ;;
    esac
    ;;
  *)
    printf 'unexpected curl url: %s\n' "$url" >&2
    exit 2
    ;;
esac
EOF
chmod +x "${FAKE_BIN}/curl"
export PATH="${FAKE_BIN}:${PATH}"
export REACTOR_OS_STATE_JSON="${TMP_ROOT}/missing-state.json"
export REACTOR_OS_CONTROL_JSON="${TMP_ROOT}/missing-control.json"

run_health() {
  local mode="$1"
  export FAKE_STATUS_MODE="$mode"
  set +e
  HEALTH_OUTPUT="$(bash "${ROOT}/deploy/board-health.sh" --production 2>&1)"
  HEALTH_RC=$?
  set -e
}

expect_pass() {
  local mode="$1"
  run_health "$mode"
  if [[ "$HEALTH_RC" -ne 0 ]]; then
    echo "expected board health production pass for ${mode}, got rc=${HEALTH_RC}" >&2
    printf '%s\n' "$HEALTH_OUTPUT" >&2
    exit 1
  fi
  if [[ "$HEALTH_OUTPUT" != *"production_state=safe_idle"* ]]; then
    echo "production pass for ${mode} did not print safe idle proof" >&2
    printf '%s\n' "$HEALTH_OUTPUT" >&2
    exit 1
  fi
}

expect_fail() {
  local mode="$1"
  local expected="$2"
  run_health "$mode"
  if [[ "$HEALTH_RC" -eq 0 ]]; then
    echo "expected board health production failure for ${mode}" >&2
    exit 1
  fi
  if [[ "$HEALTH_OUTPUT" != *"$expected"* ]]; then
    echo "expected failure for ${mode} to contain ${expected}, got:" >&2
    printf '%s\n' "$HEALTH_OUTPUT" >&2
    exit 1
  fi
}

expect_pass idle
expect_fail auto "auto-enabled"
expect_fail fault "control-fault"
expect_fail offline "not-safe-idle"

echo "board-health production gate passed"
