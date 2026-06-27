#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f "${ROOT}/ota-lib.sh" ]]; then
  # Packaged release path.
  # shellcheck source=/dev/null
  source "${ROOT}/ota-lib.sh"
elif [[ -f "${ROOT}/reactor-edge-ota-lib.sh" ]]; then
  # Repository deploy/ path.
  # shellcheck source=/dev/null
  source "${ROOT}/reactor-edge-ota-lib.sh"
elif [[ -f "/opt/reactor-edge/ota-lib.sh" ]]; then
  # Installed root path.
  # shellcheck source=/dev/null
  source "/opt/reactor-edge/ota-lib.sh"
else
  echo "missing ota-lib.sh" >&2
  exit 1
fi

json_state_value() {
  local key="$1"
  local file="$2"
  awk -v key="\"${key}\"" '
    index($0, key) {
      line = $0
      sub(/^[^:]*:[[:space:]]*"/, "", line)
      sub(/",[[:space:]]*$/, "", line)
      sub(/"$/, "", line)
      gsub(/\\"/, "\"", line)
      gsub(/\\\\/, "\\", line)
      print line
      exit
    }
  ' "$file"
}

restore_previous_slot_after_interrupted_ota() {
  local interrupted_status="$1"
  local interrupted_slot="$2"
  local reason="$3"
  local from_version="$4"
  local to_version="$5"
  local from_git="$6"
  local to_git="$7"
  local state_reason="${8:-}"
  local current_path previous_path previous_slot

  current_path="$(require_current_slot_path)"
  if [[ "$interrupted_status" == "rolling_back" && "$state_reason" == "manual rollback" && -n "$interrupted_slot" ]]; then
    previous_path="$(slot_path "$interrupted_slot")"
    [[ -n "$(managed_slot_name_from_path "$previous_path")" ]] || die "manual rollback target slot is outside managed slots: $previous_path"
    [[ -d "$previous_path" ]] || die "manual rollback target slot path is missing: $previous_path"
  else
    previous_path="$(previous_slot_path)"
  fi
  validate_slot_dir "$previous_path"
  previous_slot="$(slot_name_from_path "$previous_path")"
  [[ -n "$previous_slot" ]] || die "previous slot is not managed during interrupted OTA recovery"

  if [[ "$current_path" == "$previous_path" ]]; then
    log "interrupted OTA status=${interrupted_status} slot=${interrupted_slot:-unknown}; current already points at previous slot ${previous_slot}"
    install_systemd_units_from_slot "$previous_path"
    sync_compat_links
    install_root_ota_tools_from_slot "$previous_path"
    write_ota_state "rolled_back_on_boot" "$previous_slot" "$reason before current switch completed" "" "$to_version" "$from_version" "$to_git" "$from_git"
    return 0
  fi

  log "interrupted OTA status=${interrupted_status} slot=${interrupted_slot:-unknown}; restoring previous slot ${previous_slot}"
  install_systemd_units_from_slot "$previous_path"
  atomic_symlink "$previous_path" "$CURRENT_LINK"
  sync_compat_links
  install_root_ota_tools_from_slot "$previous_path"
  write_ota_state "rolled_back_on_boot" "$previous_slot" "$reason" "" "$to_version" "$from_version" "$to_git" "$from_git"
}

record_interrupted_before_switch() {
  local interrupted_status="$1"
  local interrupted_slot="$2"
  local from_version="$3"
  local to_version="$4"
  local from_git="$5"
  local to_git="$6"
  local current_path current_slot

  current_path="$(require_current_slot_path)"
  validate_slot_dir "$current_path"
  current_slot="$(slot_name_from_path "$current_path")"
  [[ -n "$current_slot" ]] || die "current slot is not managed during interrupted OTA pre-switch recovery"

  log "interrupted OTA status=${interrupted_status} before current switch; keeping current slot ${current_slot}"
  write_ota_state \
    "interrupted_before_switch" \
    "$current_slot" \
    "interrupted OTA ${interrupted_status} before current switch" \
    "" \
    "$from_version" \
    "$to_version" \
    "$from_git" \
    "$to_git"
}

require_root
ensure_runtime_dirs
require_ota_boot_check_commands

if [[ ! -f "$STATE_FILE" ]]; then
  log "OTA boot check passed: no OTA state file"
  exit 0
fi

STATUS="$(json_state_value status "$STATE_FILE")"
SLOT="$(json_state_value slot "$STATE_FILE")"
REASON="$(json_state_value reason "$STATE_FILE")"
FROM_VERSION="$(json_state_value from_version "$STATE_FILE")"
TO_VERSION="$(json_state_value to_version "$STATE_FILE")"
FROM_GIT="$(json_state_value from_git "$STATE_FILE")"
TO_GIT="$(json_state_value to_git "$STATE_FILE")"

case "$STATUS" in
  committed|rolled_back|rolled_back_on_boot|interrupted_before_switch|rejected_before_switch|dry_run_passed)
    log "OTA boot check passed: status=${STATUS:-none}"
    exit 0
    ;;
  "")
    write_ota_state "failed" "$SLOT" "OTA state file missing status on boot" "" "$FROM_VERSION" "$TO_VERSION" "$FROM_GIT" "$TO_GIT"
    die "OTA state file missing status on boot; use recovery or manual rollback"
    ;;
  failed)
    clear_ota_service_start_allowed
    stop_runtime_services
    die "OTA state is failed on boot; keep device in maintenance and use recovery or manual rollback"
    ;;
  downloading|verifying|verified|backup_done|staged)
    record_interrupted_before_switch \
      "$STATUS" \
      "$SLOT" \
      "$FROM_VERSION" \
      "$TO_VERSION" \
      "$FROM_GIT" \
      "$TO_GIT"
    exit 0
    ;;
  switching|health_checking|rolling_back)
    if ota_service_start_allowed; then
      log "OTA boot check bypassed for active OTA health-check start: status=${STATUS}"
      exit 0
    fi
    restore_previous_slot_after_interrupted_ota \
      "$STATUS" \
      "$SLOT" \
      "interrupted OTA ${STATUS} detected on boot" \
      "$FROM_VERSION" \
      "$TO_VERSION" \
      "$FROM_GIT" \
      "$TO_GIT" \
      "$REASON"
    exit 0
    ;;
  *)
    write_ota_state "failed" "$SLOT" "unexpected OTA state on boot: ${STATUS}" "" "$FROM_VERSION" "$TO_VERSION" "$FROM_GIT" "$TO_GIT"
    die "unexpected OTA state on boot: ${STATUS}; use recovery or manual rollback"
    ;;
esac
