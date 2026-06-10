#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

export REACTOR_EDGE_PREFIX="${TMP_ROOT}/prefix"
export REACTOR_EDGE_DATA_DIR="${TMP_ROOT}/data"
export REACTOR_EDGE_PROJECT_DIR="${TMP_ROOT}/project"
export REACTOR_EDGE_OTA_LOCK="${TMP_ROOT}/ota.lock"
export REACTOR_EDGE_SYSTEMD_UNIT_DIR="${TMP_ROOT}/systemd"
export REACTOR_EDGE_OTA_SERVICE_START_ALLOWED="${TMP_ROOT}/run/ota-service-start-allowed"
export REACTOR_EDGE_ALLOW_NON_ROOT_FOR_TESTS=1
export REACTOR_EDGE_SKIP_SYNC_FOR_TESTS=1

# shellcheck source=../deploy/reactor-edge-ota-lib.sh
source "${ROOT}/deploy/reactor-edge-ota-lib.sh"
ensure_runtime_dirs

make_slot() {
  local dir="$1"
  local name="$2"
  mkdir -p \
    "${dir}/bin" \
    "${dir}/deploy" \
    "${dir}/frontend/dist" \
    "${dir}/static"
  for bin in reactor-edge-daemon reactor-safety-guard xingshu; do
    printf '#!/usr/bin/env bash\nexit 0\n' >"${dir}/bin/${bin}"
    chmod +x "${dir}/bin/${bin}"
  done
  for script in backup.sh health-check.sh ota-update.sh ota-rollback.sh ota-lib.sh ota-boot-check.sh; do
    printf '#!/usr/bin/env bash\nexit 0\n' >"${dir}/${script}"
    chmod +x "${dir}/${script}"
  done
  for unit in reactor-edge.service reactor-edge-backup.service reactor-edge-backup.timer reactor-os-chromium.service reactor-edge-ota-boot-check.service; do
    printf '[Unit]\nDescription=%s for %s\n' "$unit" "$name" >"${dir}/deploy/${unit}"
  done
  printf '<!doctype html>\n' >"${dir}/frontend/dist/index.html"
  cat >"${dir}/BUILD-METADATA.properties" <<EOF
REACTOR_EDGE_BUILD_SCHEMA=reactor-edge.build.v1
REACTOR_EDGE_PACKAGE_NAME=${name}
REACTOR_EDGE_GIT_SHA=${name}-sha
REACTOR_EDGE_BUILT_AT_UTC=2026-06-08T00:00:00Z
EOF
}

make_slot "$(slot_path a)" "old-slot"
make_slot "$(slot_path b)" "new-slot"
atomic_symlink "$(slot_path a)" "$CURRENT_LINK"
atomic_symlink "$(slot_path a)" "$PREVIOUS_LINK"
sync_compat_links
write_ota_state "staged" "b" "" "" "old-slot" "new-slot" "oldsha" "newsha"

bash "${ROOT}/deploy/reactor-edge-ota-boot-check.sh"

current_after="$(readlink -f "$CURRENT_LINK")"
expected_previous="$(readlink -f "$(slot_path a)")"
[[ "$current_after" == "$expected_previous" ]] || {
  echo "boot-check changed current slot for interrupted pre-switch state: current=${current_after} expected=${expected_previous}" >&2
  exit 1
}

grep -Fq '"status": "interrupted_before_switch"' "$STATE_FILE" || {
  echo "boot-check did not record interrupted_before_switch" >&2
  cat "$STATE_FILE" >&2
  exit 1
}

write_ota_state "switching" "b" "" "" "old-slot" "new-slot" "oldsha" "newsha"
atomic_symlink "$(slot_path a)" "$CURRENT_LINK"
atomic_symlink "$(slot_path a)" "$PREVIOUS_LINK"
bash "${ROOT}/deploy/reactor-edge-ota-boot-check.sh"
[[ "$(readlink -f "$CURRENT_LINK")" == "$expected_previous" ]] || {
  echo "boot-check changed current slot when switching was interrupted before current link moved" >&2
  exit 1
}
grep -Fq '"status": "rolled_back_on_boot"' "$STATE_FILE" || {
  echo "boot-check did not close interrupted switching-before-current as rolled_back_on_boot" >&2
  cat "$STATE_FILE" >&2
  exit 1
}
grep -Fq 'before current switch completed' "$STATE_FILE" || {
  echo "boot-check did not record the switching-before-current interruption reason" >&2
  cat "$STATE_FILE" >&2
  exit 1
}

write_ota_state "health_checking" "b" "" "" "old-slot" "new-slot" "oldsha" "newsha"
atomic_symlink "$(slot_path b)" "$CURRENT_LINK"
bash "${ROOT}/deploy/reactor-edge-ota-boot-check.sh"

current_after="$(readlink -f "$CURRENT_LINK")"
expected_previous="$(readlink -f "$(slot_path a)")"
[[ "$current_after" == "$expected_previous" ]] || {
  echo "boot-check did not restore previous slot after interrupted health check: current=${current_after} previous=${expected_previous}" >&2
  exit 1
}

grep -Fq '"status": "rolled_back_on_boot"' "$STATE_FILE" || {
  echo "boot-check did not record rolled_back_on_boot" >&2
  cat "$STATE_FILE" >&2
  exit 1
}
grep -Fq '"slot": "a"' "$STATE_FILE" || {
  echo "boot-check did not record previous slot name" >&2
  cat "$STATE_FILE" >&2
  exit 1
}

write_ota_state "rolling_back" "a" "manual rollback" "" "new-slot" "old-slot" "newsha" "oldsha"
atomic_symlink "$(slot_path b)" "$CURRENT_LINK"
atomic_symlink "$(slot_path b)" "$PREVIOUS_LINK"
bash "${ROOT}/deploy/reactor-edge-ota-boot-check.sh"
[[ "$(readlink -f "$CURRENT_LINK")" == "$expected_previous" ]] || {
  echo "boot-check did not restore manual rollback target slot after previous link was torn" >&2
  exit 1
}
grep -Fq '"status": "rolled_back_on_boot"' "$STATE_FILE" || {
  echo "boot-check did not close interrupted manual rollback as rolled_back_on_boot" >&2
  cat "$STATE_FILE" >&2
  exit 1
}
grep -Fq '"slot": "a"' "$STATE_FILE" || {
  echo "boot-check did not record manual rollback target slot" >&2
  cat "$STATE_FILE" >&2
  exit 1
}

for unit in reactor-edge.service reactor-edge-backup.service reactor-edge-backup.timer reactor-os-chromium.service reactor-edge-ota-boot-check.service; do
  [[ -f "${REACTOR_EDGE_SYSTEMD_UNIT_DIR}/${unit}" ]] || {
    echo "boot-check did not reinstall ${unit}" >&2
    exit 1
  }
done

write_ota_state "committed" "a" "" "" "old-slot" "old-slot" "oldsha" "oldsha"
atomic_symlink "$(slot_path a)" "$CURRENT_LINK"
bash "${ROOT}/deploy/reactor-edge-ota-boot-check.sh"
[[ "$(readlink -f "$CURRENT_LINK")" == "$expected_previous" ]] || {
  echo "boot-check changed current slot for committed state" >&2
  exit 1
}

cat >"$STATE_FILE" <<'EOF'
{
  "slot": "b",
  "reason": "simulated torn OTA state file"
}
EOF
set +e
missing_status_output="$(bash "${ROOT}/deploy/reactor-edge-ota-boot-check.sh" 2>&1)"
missing_status_rc=$?
set -e
if [[ "$missing_status_rc" -eq 0 ]]; then
  echo "boot-check allowed OTA state file with missing status" >&2
  exit 1
fi
if [[ "$missing_status_output" != *"missing status"* ]]; then
  echo "boot-check missing-status output did not explain fail-closed state:" >&2
  printf '%s\n' "$missing_status_output" >&2
  exit 1
fi
grep -Fq '"status": "failed"' "$STATE_FILE" || {
  echo "boot-check did not rewrite missing-status OTA state as failed" >&2
  cat "$STATE_FILE" >&2
  exit 1
}
grep -Fq 'OTA state file missing status on boot' "$STATE_FILE" || {
  echo "boot-check did not record missing-status failure reason" >&2
  cat "$STATE_FILE" >&2
  exit 1
}

write_ota_state "health_checking" "b" "" "" "old-slot" "new-slot" "oldsha" "newsha"
atomic_symlink "$(slot_path a)" "$PREVIOUS_LINK"
mark_ota_service_start_allowed
atomic_symlink "$(slot_path b)" "$CURRENT_LINK"
bash "${ROOT}/deploy/reactor-edge-ota-boot-check.sh"
[[ "$(readlink -f "$CURRENT_LINK")" == "$(readlink -f "$(slot_path b)")" ]] || {
  echo "boot-check ignored active OTA service-start marker" >&2
  exit 1
}

write_ota_state "health_checking" "b" "" "" "old-slot" "new-slot" "oldsha" "newsha"
atomic_symlink "$(slot_path a)" "$PREVIOUS_LINK"
atomic_symlink "$(slot_path b)" "$CURRENT_LINK"
mkdir -p "$(dirname "$OTA_SERVICE_START_ALLOWED")"
cat >"$OTA_SERVICE_START_ALLOWED" <<'EOF'
ota_pid=999999
ota_pid_start_ticks=1
created_at=2026-06-08T00:00:00Z
EOF
bash "${ROOT}/deploy/reactor-edge-ota-boot-check.sh"
[[ "$(readlink -f "$CURRENT_LINK")" == "$expected_previous" ]] || {
  echo "boot-check accepted stale OTA service-start marker" >&2
  exit 1
}
grep -Fq '"status": "rolled_back_on_boot"' "$STATE_FILE" || {
  echo "boot-check did not roll back after stale service-start marker" >&2
  cat "$STATE_FILE" >&2
  exit 1
}
[[ ! -e "$OTA_SERVICE_START_ALLOWED" ]] || {
  echo "boot-check did not remove stale OTA service-start marker" >&2
  exit 1
}

write_ota_state "failed" "b" "rollback health check failed" "" "old-slot" "new-slot" "oldsha" "newsha"
mark_ota_service_start_allowed
atomic_symlink "$(slot_path b)" "$CURRENT_LINK"
set +e
failed_output="$(bash "${ROOT}/deploy/reactor-edge-ota-boot-check.sh" 2>&1)"
failed_rc=$?
set -e
if [[ "$failed_rc" -eq 0 ]]; then
  echo "boot-check allowed failed OTA state to start production" >&2
  exit 1
fi
if [[ "$failed_output" != *"keep device in maintenance"* ]]; then
  echo "boot-check failed state output did not mention maintenance:" >&2
  printf '%s\n' "$failed_output" >&2
  exit 1
fi
[[ ! -e "$OTA_SERVICE_START_ALLOWED" ]] || {
  echo "boot-check failed state did not clear OTA service-start marker" >&2
  exit 1
}
[[ "$(readlink -f "$CURRENT_LINK")" == "$(readlink -f "$(slot_path b)")" ]] || {
  echo "boot-check changed slot for failed state instead of requiring maintenance" >&2
  exit 1
}

echo "OTA boot-check gate passed"
