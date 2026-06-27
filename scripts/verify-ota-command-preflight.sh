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

BASE_BIN="${TMP_ROOT}/base-bin"
mkdir -p "$BASE_BIN"
for cmd in awk basename cat chmod date df dirname env find flock grep install ln mkdir mktemp mv readlink rm rmdir sed seq sha256sum sleep sort stat sync tar tee touch tr wc; do
  if command -v "$cmd" >/dev/null 2>&1; then
    ln -s "$(command -v "$cmd")" "${BASE_BIN}/${cmd}"
  fi
done

CHECK_RC=0
CHECK_OUTPUT=""

run_with_path() {
  local path="$1"
  shift
  set +e
  CHECK_OUTPUT="$( PATH="$path" "$@" 2>&1 )"
  CHECK_RC=$?
  set -e
}

expect_pass() {
  local label="$1"
  local path="$2"
  shift 2
  run_with_path "$path" "$@"
  if [[ "$CHECK_RC" -ne 0 ]]; then
    echo "expected pass for ${label}, got rc=${CHECK_RC}" >&2
    printf '%s\n' "$CHECK_OUTPUT" >&2
    exit 1
  fi
}

expect_fail() {
  local label="$1"
  local path="$2"
  local expected="$3"
  shift 3
  run_with_path "$path" "$@"
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

without_cmd() {
  local cmd_to_remove="$1"
  local target_dir="${TMP_ROOT}/without-${cmd_to_remove}"
  mkdir -p "$target_dir"
  for entry in "${BASE_BIN}"/*; do
    local name
    name="$(basename "$entry")"
    [[ "$name" == "$cmd_to_remove" ]] && continue
    ln -s "$(readlink -f "$entry")" "${target_dir}/${name}"
  done
  printf '%s' "$target_dir"
}

expect_pass "update command preflight" "$BASE_BIN" require_ota_update_commands
expect_pass "rollback command preflight" "$BASE_BIN" require_ota_rollback_commands
expect_pass "boot-check command preflight" "$BASE_BIN" require_ota_boot_check_commands

NO_TAR_BIN="$(without_cmd tar)"
expect_fail "missing tar for update" "$NO_TAR_BIN" "missing required command(s): tar" require_ota_update_commands
expect_pass "missing tar does not block rollback" "$NO_TAR_BIN" require_ota_rollback_commands
expect_pass "missing tar does not block boot-check" "$NO_TAR_BIN" require_ota_boot_check_commands

NO_FLOCK_BIN="$(without_cmd flock)"
expect_fail "missing flock for update" "$NO_FLOCK_BIN" "missing required command(s): flock" require_ota_update_commands
expect_pass "missing flock does not block rollback" "$NO_FLOCK_BIN" require_ota_rollback_commands
expect_pass "missing flock does not block boot-check" "$NO_FLOCK_BIN" require_ota_boot_check_commands

NO_INSTALL_BIN="$(without_cmd install)"
expect_fail "missing install for rollback" "$NO_INSTALL_BIN" "missing required command(s): install" require_ota_rollback_commands
expect_fail "missing install for boot-check" "$NO_INSTALL_BIN" "missing required command(s): install" require_ota_boot_check_commands

NO_SYNC_BIN="$(without_cmd sync)"
expect_fail "missing sync for update" "$NO_SYNC_BIN" "missing required command(s): sync" require_ota_update_commands
expect_fail "missing sync for rollback" "$NO_SYNC_BIN" "missing required command(s): sync" require_ota_rollback_commands
expect_fail "missing sync for boot-check" "$NO_SYNC_BIN" "missing required command(s): sync" require_ota_boot_check_commands

echo "OTA command preflight gate passed"
