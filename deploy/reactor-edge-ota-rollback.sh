#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [[ -f "${ROOT}/ota-lib.sh" ]]; then
  # shellcheck source=/dev/null
  source "${ROOT}/ota-lib.sh"
elif [[ -f "${ROOT}/reactor-edge-ota-lib.sh" ]]; then
  # shellcheck source=/dev/null
  source "${ROOT}/reactor-edge-ota-lib.sh"
elif [[ -f "/opt/reactor-edge/ota-lib.sh" ]]; then
  # shellcheck source=/dev/null
  source "/opt/reactor-edge/ota-lib.sh"
else
  echo "missing ota-lib.sh" >&2
  exit 1
fi

usage() {
  cat <<'EOF'
Roll ReactorOS back to the previous application slot.

Usage:
  sudo ota-rollback.sh [options]

Options:
  --force                      Bypass production busy/emergency checks.
  --confirm-maintenance-window Required with --force.
  --health-attempts <n>        Health attempts after switching, default 12.
  --health-interval <seconds>  Delay between attempts, default 5.
  --required-passes <n>        Consecutive successful health checks, default 3.
  -h, --help                   Show this help.

This script only switches /opt/reactor-edge/current back to /opt/reactor-edge/previous.
It does not roll back SQLite data.
EOF
}

FORCE=0
CONFIRM_MAINTENANCE_WINDOW=0
HEALTH_ATTEMPTS=12
HEALTH_INTERVAL=5
REQUIRED_PASSES=3

while [[ $# -gt 0 ]]; do
  case "$1" in
    --force)
      FORCE=1
      shift
      ;;
    --confirm-maintenance-window)
      CONFIRM_MAINTENANCE_WINDOW=1
      shift
      ;;
    --health-attempts)
      HEALTH_ATTEMPTS="${2:-}"
      shift 2
      ;;
    --health-interval)
      HEALTH_INTERVAL="${2:-}"
      shift 2
      ;;
    --required-passes)
      REQUIRED_PASSES="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unexpected argument: $1"
      ;;
    esac
done

validate_health_args "$HEALTH_ATTEMPTS" "$HEALTH_INTERVAL" "$REQUIRED_PASSES"
require_confirmed_dangerous_option "--force" "--confirm-maintenance-window" "$FORCE" "$CONFIRM_MAINTENANCE_WINDOW"

require_root
ensure_runtime_dirs
require_ota_rollback_commands
acquire_ota_lock
check_not_busy "$FORCE"

CURRENT_PATH="$(require_current_slot_path)"
PREVIOUS_PATH="$(previous_slot_path)"

[[ "$CURRENT_PATH" != "$PREVIOUS_PATH" ]] || die "current and previous slots are identical"
validate_slot_dir "$PREVIOUS_PATH"

TARGET_SLOT="$(slot_name_from_path "$PREVIOUS_PATH")"
FROM_VERSION="$(release_version_from_dir "$CURRENT_PATH")"
FROM_GIT="$(release_git_from_dir "$CURRENT_PATH")"
TARGET_VERSION="$(release_version_from_dir "$PREVIOUS_PATH")"
TARGET_GIT="$(release_git_from_dir "$PREVIOUS_PATH")"

write_ota_state "rolling_back" "$TARGET_SLOT" "manual rollback" "" "$FROM_VERSION" "$TARGET_VERSION" "$FROM_GIT" "$TARGET_GIT"
log "manual rollback from ${CURRENT_PATH:-unknown} to ${PREVIOUS_PATH}"

stop_runtime_services
install_systemd_units_from_slot "$PREVIOUS_PATH"
atomic_symlink "$PREVIOUS_PATH" "$CURRENT_LINK"
if [[ -n "$CURRENT_PATH" && -d "$CURRENT_PATH" ]]; then
  atomic_symlink "$CURRENT_PATH" "$PREVIOUS_LINK"
fi
sync_compat_links
install_root_ota_tools_from_slot "$PREVIOUS_PATH"

if start_runtime_services && health_check_loop "$HEALTH_ATTEMPTS" "$HEALTH_INTERVAL" "$REQUIRED_PASSES"; then
  write_ota_state "rolled_back" "$TARGET_SLOT" "manual rollback" "" "$FROM_VERSION" "$TARGET_VERSION" "$FROM_GIT" "$TARGET_GIT"
  log "manual rollback committed on slot ${TARGET_SLOT}"
  exit 0
fi

enter_ota_failed_state "$TARGET_SLOT" "manual rollback health check failed" "" "$FROM_VERSION" "$TARGET_VERSION" "$FROM_GIT" "$TARGET_GIT"
die "manual rollback health check failed; keep device in maintenance"
