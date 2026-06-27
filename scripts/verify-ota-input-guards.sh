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

package="${TMP_ROOT}/reactor-os-test.tar.gz"
other_package="${TMP_ROOT}/other-release.tar.gz"
printf 'release payload\n' >"$package"
printf 'other payload\n' >"$other_package"

good_sidecar="${TMP_ROOT}/good.sha256"
(cd "$TMP_ROOT" && sha256sum "$(basename "$package")" >"$good_sidecar")
expect_pass "matching sidecar" verify_sha256_for_package "$package" "$good_sidecar"

wrong_name_sidecar="${TMP_ROOT}/wrong-name.sha256"
(cd "$TMP_ROOT" && sha256sum "$(basename "$other_package")" >"$wrong_name_sidecar")
expect_fail "wrong package name sidecar" "does not reference package" verify_sha256_for_package "$package" "$wrong_name_sidecar"

bad_digest_sidecar="${TMP_ROOT}/bad-digest.sha256"
printf 'not-a-sha256  %s\n' "$(basename "$package")" >"$bad_digest_sidecar"
expect_fail "invalid digest sidecar" "invalid digest" verify_sha256_for_package "$package" "$bad_digest_sidecar"

mismatch_sidecar="${TMP_ROOT}/mismatch.sha256"
printf '%064d  %s\n' 0 "$(basename "$package")" >"$mismatch_sidecar"
expect_fail "digest mismatch" "sha256 mismatch" verify_sha256_for_package "$package" "$mismatch_sidecar"

expect_pass "valid health args" validate_health_args 12 5 3
expect_fail "zero health attempts" "must be a positive integer" validate_health_args 0 5 1
expect_fail "non-numeric health interval" "must be a positive integer" validate_health_args 12 abc 1
expect_fail "required passes exceed attempts" "cannot exceed" validate_health_args 2 1 3

echo "OTA input guard gate passed"
