#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

export REACTOR_EDGE_PREFIX="${TMP_ROOT}/prefix"
export REACTOR_EDGE_DATA_DIR="${TMP_ROOT}/data"
export REACTOR_EDGE_PROJECT_DIR="${TMP_ROOT}/project"
export REACTOR_EDGE_OTA_LOCK="${TMP_ROOT}/ota.lock"

FAKE_BIN="${TMP_ROOT}/bin"
mkdir -p "$FAKE_BIN"

cat >"${FAKE_BIN}/systemctl" <<'EOF'
#!/usr/bin/env bash
case "${1:-}" in
  is-active)
    if [[ "${FAKE_BACKEND_ACTIVE:-0}" == "1" ]]; then
      exit 0
    fi
    exit 3
    ;;
  *)
    exit 0
    ;;
esac
EOF

cat >"${FAKE_BIN}/curl" <<'EOF'
#!/usr/bin/env bash
case "${FAKE_CURL_MODE:-idle}" in
  fail)
    exit 7
    ;;
  invalid)
    printf 'not-json\n'
    ;;
  busy)
    printf '{"data":{"devices":[{"device_id":"R1","online":true,"status":"running","active_batch_id":"B-001","emergency_stop":false,"auto_enabled":true,"manual_lock":false,"last_control_error":null,"last_command_ok":true}]}}\n'
    ;;
  emergency)
    printf '{"data":{"devices":[{"device_id":"R1","online":false,"status":"error","active_batch_id":null,"emergency_stop":true,"auto_enabled":false,"manual_lock":false,"last_control_error":null,"last_command_ok":true}]}}\n'
    ;;
  auto)
    printf '{"data":{"devices":[{"device_id":"R1","online":true,"status":"idle","active_batch_id":null,"emergency_stop":false,"auto_enabled":true,"manual_lock":false,"last_control_error":null,"last_command_ok":true}]}}\n'
    ;;
  manual)
    printf '{"data":{"devices":[{"device_id":"R1","online":true,"status":"idle","active_batch_id":null,"emergency_stop":false,"auto_enabled":false,"manual_lock":true,"last_control_error":null,"last_command_ok":true}]}}\n'
    ;;
  control_fault)
    printf '{"data":{"devices":[{"device_id":"R1","online":true,"status":"error","active_batch_id":null,"emergency_stop":false,"auto_enabled":false,"manual_lock":false,"last_control_error":"write timeout","last_command_ok":true}]}}\n'
    ;;
  command_fault)
    printf '{"data":{"devices":[{"device_id":"R1","online":true,"status":"idle","active_batch_id":null,"emergency_stop":false,"auto_enabled":false,"manual_lock":false,"last_control_error":null,"last_command_ok":false}]}}\n'
    ;;
  stale)
    printf '{"data":{"devices":[{"device_id":"R1","online":false,"status":"stale","active_batch_id":null,"emergency_stop":false,"auto_enabled":false,"manual_lock":false,"last_control_error":null,"last_command_ok":true}]}}\n'
    ;;
  empty)
    printf '{"data":{"devices":[]}}\n'
    ;;
  idle)
    printf '{"data":{"devices":[{"device_id":"R1","online":true,"status":"idle","active_batch_id":null,"emergency_stop":false,"auto_enabled":false,"manual_lock":false,"last_control_error":null,"last_command_ok":true}]}}\n'
    ;;
  *)
    printf 'unknown fake curl mode: %s\n' "${FAKE_CURL_MODE:-}" >&2
    exit 2
    ;;
esac
EOF

chmod +x "${FAKE_BIN}/systemctl" "${FAKE_BIN}/curl"
export PATH="${FAKE_BIN}:${PATH}"

# shellcheck source=../deploy/reactor-edge-ota-lib.sh
source "${ROOT}/deploy/reactor-edge-ota-lib.sh"
ensure_runtime_dirs

CHECK_RC=0
CHECK_OUTPUT=""

run_check() {
  local force="$1"
  local backend_active="$2"
  local curl_mode="$3"
  export FAKE_BACKEND_ACTIVE="$backend_active"
  export FAKE_CURL_MODE="$curl_mode"
  set +e
  CHECK_OUTPUT="$( ( check_not_busy "$force" ) 2>&1 )"
  CHECK_RC=$?
  set -e
}

expect_pass() {
  local label="$1"
  local force="$2"
  local backend_active="$3"
  local curl_mode="$4"
  run_check "$force" "$backend_active" "$curl_mode"
  if [[ "$CHECK_RC" -ne 0 ]]; then
    echo "expected pass for ${label}, got rc=${CHECK_RC}" >&2
    printf '%s\n' "$CHECK_OUTPUT" >&2
    exit 1
  fi
}

expect_fail() {
  local label="$1"
  local force="$2"
  local backend_active="$3"
  local curl_mode="$4"
  local expected="$5"
  run_check "$force" "$backend_active" "$curl_mode"
  if [[ "$CHECK_RC" -eq 0 ]]; then
    echo "expected failure for ${label}" >&2
    exit 1
  fi
  if [[ "$CHECK_OUTPUT" != *"$expected"* ]]; then
    echo "expected failure for ${label} to contain '${expected}', got:" >&2
    printf '%s\n' "$CHECK_OUTPUT" >&2
    exit 1
  fi
}

expect_fail "backend inactive" 0 0 idle "cannot prove reactor is idle"
expect_fail "status unreadable" 0 1 fail "cannot read device status"
expect_fail "active batch" 0 1 busy "device is running an active process"
expect_fail "emergency stop" 0 1 emergency "emergency stop is active"
expect_fail "automatic control enabled" 0 1 auto "automatic control is enabled"
expect_fail "manual lock active" 0 1 manual "manual lock is active"
expect_fail "control fault" 0 1 control_fault "device control fault is uncleared"
expect_fail "downstream command fault" 0 1 command_fault "device control fault is uncleared"
expect_fail "stale status" 0 1 stale "device status is not proven idle and online"
expect_fail "empty status" 0 1 empty "device status did not report any devices"
expect_pass "idle status" 0 1 idle
expect_pass "force bypass" 1 0 busy

expect_health_pass() {
  local label="$1"
  local backend_active="$2"
  local curl_mode="$3"
  export FAKE_BACKEND_ACTIVE="$backend_active"
  export FAKE_CURL_MODE="$curl_mode"
  set +e
  CHECK_OUTPUT="$( ( health_check_loop 1 0 1 ) 2>&1 )"
  CHECK_RC=$?
  set -e
  if [[ "$CHECK_RC" -ne 0 ]]; then
    echo "expected health pass for ${label}, got rc=${CHECK_RC}" >&2
    printf '%s\n' "$CHECK_OUTPUT" >&2
    exit 1
  fi
}

expect_health_fail() {
  local label="$1"
  local backend_active="$2"
  local curl_mode="$3"
  local expected="$4"
  export FAKE_BACKEND_ACTIVE="$backend_active"
  export FAKE_CURL_MODE="$curl_mode"
  set +e
  CHECK_OUTPUT="$( ( health_check_loop 1 0 1 ) 2>&1 )"
  CHECK_RC=$?
  set -e
  if [[ "$CHECK_RC" -eq 0 ]]; then
    echo "expected health failure for ${label}" >&2
    exit 1
  fi
  if [[ "$CHECK_OUTPUT" != *"$expected"* ]]; then
    echo "expected health failure for ${label} to contain '${expected}', got:" >&2
    printf '%s\n' "$CHECK_OUTPUT" >&2
    exit 1
  fi
}

expect_health_pass "safe idle commit gate" 1 idle
expect_health_fail "auto-enabled commit gate" 1 auto "automatic control enabled"
expect_health_fail "control-fault commit gate" 1 control_fault "control fault uncleared"

echo "OTA busy-state gate passed"
