#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

export REACTOR_EDGE_PREFIX="${TMP_ROOT}/prefix"
export REACTOR_EDGE_DATA_DIR="${TMP_ROOT}/data"
export REACTOR_EDGE_PROJECT_DIR="${TMP_ROOT}/project"
export REACTOR_EDGE_OTA_LOCK="${TMP_ROOT}/ota.lock"

# shellcheck source=../deploy/reactor-edge-ota-lib.sh
source "${ROOT}/deploy/reactor-edge-ota-lib.sh"
ensure_runtime_dirs

mkdir -p "$(slot_path a)" "$(slot_path b)" "${TMP_ROOT}/outside-slot"

CHECK_RC=0
CHECK_OUTPUT=""

run_check() {
  set +e
  CHECK_OUTPUT="$( "$@" 2>&1 )"
  CHECK_RC=$?
  set -e
}

expect_pass() {
  local label="$1"
  shift
  run_check "$@"
  if [[ "$CHECK_RC" -ne 0 ]]; then
    echo "expected pass for ${label}, got rc=${CHECK_RC}" >&2
    printf '%s\n' "$CHECK_OUTPUT" >&2
    exit 1
  fi
}

expect_fail() {
  local label="$1"
  local expected="$2"
  shift 2
  run_check "$@"
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

ln -sfn "$(slot_path a)" "$CURRENT_LINK"
expect_pass "current slot in managed slot" require_current_slot_path

ln -sfn "${TMP_ROOT}/outside-slot" "$CURRENT_LINK"
expect_fail "current slot outside managed slots" "current slot is outside managed slots" require_current_slot_path
expect_fail "optional current slot outside managed slots" "current slot is outside managed slots" optional_current_slot_path

rm -f "$CURRENT_LINK"
expect_pass "optional current slot missing for initial update" optional_current_slot_path
expect_fail "required current slot missing" "current slot link is missing or invalid" require_current_slot_path

ln -sfn "$(slot_path b)" "$PREVIOUS_LINK"
expect_pass "previous slot in managed slot" previous_slot_path

ln -sfn "${TMP_ROOT}/outside-slot" "$PREVIOUS_LINK"
expect_fail "previous slot outside managed slots" "previous slot is outside managed slots" previous_slot_path

rm -f "$PREVIOUS_LINK"
expect_fail "previous slot missing" "previous slot link is missing or invalid" previous_slot_path

ln -sfn "$(slot_path a)" "$CURRENT_LINK"
ln -sfn "$(slot_path a)" "$PREVIOUS_LINK"
CURRENT_PATH="$(require_current_slot_path)"
PREVIOUS_PATH="$(previous_slot_path)"
if [[ "$CURRENT_PATH" != "$PREVIOUS_PATH" ]]; then
  echo "expected current and previous to resolve to the same path" >&2
  exit 1
fi

echo "OTA slot integrity gate passed"
