#!/usr/bin/env bash
set -euo pipefail

PREFIX="${REACTOR_EDGE_PREFIX:-/opt/reactor-edge}"
ETC_DIR="${REACTOR_EDGE_ETC_DIR:-/etc/reactor-edge}"
DATA_DIR="${REACTOR_EDGE_DATA_DIR:-/var/lib/reactor-edge}"
PROJECT_DIR="${REACTOR_EDGE_PROJECT_DIR:-/project}"
SLOTS_DIR="${REACTOR_EDGE_SLOTS_DIR:-${PREFIX}/slots}"
CURRENT_LINK="${REACTOR_EDGE_CURRENT_LINK:-${PREFIX}/current}"
PREVIOUS_LINK="${REACTOR_EDGE_PREVIOUS_LINK:-${PREFIX}/previous}"
STATE_DIR="${REACTOR_EDGE_OTA_STATE_DIR:-${DATA_DIR}/ota}"
STATE_FILE="${STATE_DIR}/state.json"
LOG_FILE="${STATE_DIR}/ota.log"
LOCK_FILE="${REACTOR_EDGE_OTA_LOCK:-/var/lock/reactor-edge-ota.lock}"
DB_PATH="${REACTOR_EDGE_DB:-${DATA_DIR}/reactor.sqlite3}"
HEALTH_URL="${REACTOR_OS_HEALTH_URL:-http://127.0.0.1:8000/health}"
HMI_URL="${REACTOR_OS_URL:-http://127.0.0.1:8000/}"
STATUS_URL="${REACTOR_EDGE_STATUS_URL:-http://127.0.0.1:8000/api/devices/status}"
BACKEND_SERVICE="${REACTOR_EDGE_SERVICE:-reactor-edge}"
KIOSK_SERVICE="${REACTOR_EDGE_KIOSK_SERVICE:-reactor-os-chromium}"
BACKUP_TIMER="${REACTOR_EDGE_BACKUP_TIMER:-reactor-edge-backup.timer}"
SYSTEMD_UNIT_DIR="${REACTOR_EDGE_SYSTEMD_UNIT_DIR:-/etc/systemd/system}"
OTA_SERVICE_START_ALLOWED="${REACTOR_EDGE_OTA_SERVICE_START_ALLOWED:-/run/reactor-edge/ota-service-start-allowed}"
OTA_SERVICE_START_ALLOWED_TTL="${REACTOR_EDGE_OTA_SERVICE_START_ALLOWED_TTL:-1800}"
OTA_CLEANUP_PATHS=()
OTA_CLEANUP_BASES=()
OTA_CLEANUP_RAW_DIRS=()
RELEASE_PACKAGE_VERSION=""
RELEASE_PACKAGE_GIT=""

log() {
  mkdir -p "$STATE_DIR"
  printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" | tee -a "$LOG_FILE"
}

die() {
  log "ERROR: $*"
  exit 1
}

run_registered_cleanups() {
  local rc=$?
  set +e
  if declare -F ota_exit_hook >/dev/null 2>&1; then
    ota_exit_hook "$rc" >/dev/null 2>&1 || true
  fi
  local i
  for i in "${!OTA_CLEANUP_PATHS[@]}"; do
    if [[ -n "${OTA_CLEANUP_PATHS[$i]:-}" ]]; then
      safe_remove_path "${OTA_CLEANUP_PATHS[$i]}" "${OTA_CLEANUP_BASES[$i]}" >/dev/null 2>&1 || true
    fi
  done
  for i in "${!OTA_CLEANUP_RAW_DIRS[@]}"; do
    if [[ -n "${OTA_CLEANUP_RAW_DIRS[$i]:-}" ]]; then
      rm -rf "${OTA_CLEANUP_RAW_DIRS[$i]}" >/dev/null 2>&1 || true
    fi
  done
  exit "$rc"
}

ensure_cleanup_trap() {
  trap run_registered_cleanups EXIT
}

register_safe_remove_cleanup() {
  local path="$1"
  local base="$2"
  OTA_CLEANUP_PATHS+=("$path")
  OTA_CLEANUP_BASES+=("$base")
  ensure_cleanup_trap
}

register_raw_dir_cleanup() {
  local path="$1"
  OTA_CLEANUP_RAW_DIRS+=("$path")
  ensure_cleanup_trap
}

require_root() {
  if [[ "${REACTOR_EDGE_ALLOW_NON_ROOT_FOR_TESTS:-0}" == "1" ]]; then
    return 0
  fi
  if [[ "${EUID:-$(id -u)}" -ne 0 ]]; then
    die "must run as root; use sudo"
  fi
}

require_positive_int() {
  local name="$1"
  local value="$2"
  [[ "$value" =~ ^[1-9][0-9]*$ ]] || die "${name} must be a positive integer"
}

require_commands() {
  local command_name missing=()
  for command_name in "$@"; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
      missing+=("$command_name")
    fi
  done
  if (( ${#missing[@]} > 0 )); then
    die "missing required command(s): ${missing[*]}"
  fi
}

require_ota_update_commands() {
  require_commands awk basename cat date df dirname find flock install ln mkdir mktemp mv readlink rm rmdir seq sha256sum sleep sort stat sync tar tee tr wc
}

require_ota_rollback_commands() {
  require_commands cat date dirname install ln mkdir mv readlink rm seq sleep sync tee
}

require_ota_boot_check_commands() {
  require_commands awk cat date dirname find install ln mkdir mv readlink rm seq sleep stat sync tee
}

validate_health_args() {
  local attempts="$1"
  local interval="$2"
  local required="$3"
  require_positive_int "--health-attempts" "$attempts"
  require_positive_int "--health-interval" "$interval"
  require_positive_int "--required-passes" "$required"
  (( required <= attempts )) || die "--required-passes cannot exceed --health-attempts"
}

require_confirmed_dangerous_option() {
  local option="$1"
  local confirmation="$2"
  local enabled="$3"
  local confirmed="$4"
  if [[ "$enabled" -eq 1 && "$confirmed" -ne 1 ]]; then
    die "${option} requires ${confirmation}; do not use field bypasses without a recorded maintenance decision"
  fi
}

ensure_runtime_dirs() {
  mkdir -p "$PREFIX" "$SLOTS_DIR" "$DATA_DIR" "$STATE_DIR" "$PROJECT_DIR"
  touch "$LOG_FILE"
}

flush_ota_disk() {
  local reason="${1:-OTA critical write}"
  log "syncing filesystem after ${reason}"
  if [[ "${REACTOR_EDGE_SKIP_SYNC_FOR_TESTS:-0}" == "1" ]]; then
    log "sync skipped for tests after ${reason}"
    return 0
  fi
  sync
}

mark_ota_service_start_allowed() {
  local tmp pid_start_ticks
  pid_start_ticks="$(process_start_ticks "$$")" || die "cannot create OTA service-start marker because current process identity is unavailable"
  mkdir -p "$(dirname "$OTA_SERVICE_START_ALLOWED")"
  tmp="${OTA_SERVICE_START_ALLOWED}.tmp.$$"
  {
    printf 'ota_pid=%s\n' "$$"
    printf 'ota_pid_start_ticks=%s\n' "$pid_start_ticks"
    printf 'created_at=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  } >"$tmp"
  mv -f "$tmp" "$OTA_SERVICE_START_ALLOWED"
}

clear_ota_service_start_allowed() {
  rm -f "$OTA_SERVICE_START_ALLOWED"
}

process_start_ticks() {
  local pid="$1"
  local stat_line rest
  [[ "$pid" =~ ^[0-9]+$ ]] || return 1
  [[ -r "/proc/${pid}/stat" ]] || return 1
  stat_line="$(cat "/proc/${pid}/stat")" || return 1
  rest="${stat_line##*) }"
  set -- $rest
  [[ "${20:-}" =~ ^[0-9]+$ ]] || return 1
  printf '%s' "$20"
}

ota_marker_value() {
  local key="$1"
  local file="$2"
  awk -F= -v key="$key" '$1 == key { print $2; found = 1; exit } END { if (!found) exit 1 }' "$file"
}

ota_service_start_allowed() {
  [[ -f "$OTA_SERVICE_START_ALLOWED" ]] || return 1
  local now mtime age marker_pid marker_start current_start
  now="$(date +%s)"
  mtime="$(stat -c %Y "$OTA_SERVICE_START_ALLOWED" 2>/dev/null || printf '0')"
  if [[ "$now" =~ ^[0-9]+$ && "$mtime" =~ ^[0-9]+$ ]]; then
    age=$((now - mtime))
    if (( age <= OTA_SERVICE_START_ALLOWED_TTL )); then
      marker_pid="$(ota_marker_value ota_pid "$OTA_SERVICE_START_ALLOWED" 2>/dev/null || printf '')"
      marker_start="$(ota_marker_value ota_pid_start_ticks "$OTA_SERVICE_START_ALLOWED" 2>/dev/null || printf '')"
      if [[ "$marker_pid" =~ ^[0-9]+$ && "$marker_start" =~ ^[0-9]+$ ]]; then
        current_start="$(process_start_ticks "$marker_pid" 2>/dev/null || printf '')"
        if [[ "$current_start" == "$marker_start" ]]; then
          return 0
        fi
      fi
      rm -f "$OTA_SERVICE_START_ALLOWED"
      log "removed stale OTA service-start marker because owner process is not active"
      return 1
    fi
  fi
  rm -f "$OTA_SERVICE_START_ALLOWED"
  log "removed stale OTA service-start marker"
  return 1
}

json_escape() {
  local input="${1:-}"
  input="${input//\\/\\\\}"
  input="${input//\"/\\\"}"
  input="${input//$'\n'/ }"
  input="${input//$'\r'/ }"
  printf '%s' "$input"
}

write_ota_state() {
  local status="$1"
  local slot="${2:-}"
  local reason="${3:-}"
  local package="${4:-}"
  local from_version="${5:-}"
  local to_version="${6:-}"
  local from_git="${7:-}"
  local to_git="${8:-}"
  mkdir -p "$STATE_DIR"
  cat >"$STATE_FILE.tmp" <<EOF
{
  "status": "$(json_escape "$status")",
  "slot": "$(json_escape "$slot")",
  "reason": "$(json_escape "$reason")",
  "package": "$(json_escape "$package")",
  "from_version": "$(json_escape "$from_version")",
  "to_version": "$(json_escape "$to_version")",
  "from_git": "$(json_escape "$from_git")",
  "to_git": "$(json_escape "$to_git")",
  "updated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
EOF
  mv -f "$STATE_FILE.tmp" "$STATE_FILE"
  flush_ota_disk "OTA state ${status}"
}

enter_ota_failed_state() {
  local slot="$1"
  local reason="$2"
  local package="${3:-}"
  local from_version="${4:-}"
  local to_version="${5:-}"
  local from_git="${6:-}"
  local to_git="${7:-}"

  clear_ota_service_start_allowed
  write_ota_state "failed" "$slot" "$reason" "$package" "$from_version" "$to_version" "$from_git" "$to_git"
  stop_runtime_services
}

metadata_value() {
  local dir="$1"
  local key="$2"
  local file="${dir}/BUILD-METADATA.properties"
  [[ -f "$file" ]] || {
    printf ''
    return 0
  }
  awk -F= -v key="$key" '
    $1 == key {
      value = substr($0, length($1) + 2)
      print value
      found = 1
      exit
    }
    END {
      if (!found) {
        exit 0
      }
    }
  ' "$file"
}

release_version_from_dir() {
  local dir="$1"
  local value
  value="$(metadata_value "$dir" REACTOR_EDGE_PACKAGE_NAME)"
  if [[ -n "$value" ]]; then
    printf '%s' "$value"
  elif [[ -f "${dir}/BUILD-METADATA.properties" ]]; then
    printf 'unknown'
  else
    printf ''
  fi
}

release_git_from_dir() {
  local dir="$1"
  local value
  value="$(metadata_value "$dir" REACTOR_EDGE_GIT_SHA)"
  if [[ -n "$value" ]]; then
    printf '%s' "$value"
  elif [[ -f "${dir}/BUILD-METADATA.properties" ]]; then
    printf 'unknown'
  else
    printf ''
  fi
}

validate_build_metadata() {
  local dir="$1"
  local metadata="${dir}/BUILD-METADATA.properties"
  [[ -f "$metadata" ]] || die "candidate missing BUILD-METADATA.properties"
  [[ "$(metadata_value "$dir" REACTOR_EDGE_BUILD_SCHEMA)" == "reactor-edge.build.v1" ]] || die "candidate build metadata schema is invalid"
  [[ -n "$(metadata_value "$dir" REACTOR_EDGE_PACKAGE_NAME)" ]] || die "candidate build metadata missing package name"
  [[ -n "$(metadata_value "$dir" REACTOR_EDGE_GIT_SHA)" ]] || die "candidate build metadata missing git sha"
  [[ -n "$(metadata_value "$dir" REACTOR_EDGE_BUILT_AT_UTC)" ]] || die "candidate build metadata missing build timestamp"
}

verify_sha256_for_package() {
  local package="$1"
  local sidecar="$2"
  [[ -f "$package" ]] || die "release package not found: $package"
  [[ -f "$sidecar" ]] || die "sha256 sidecar not found: $sidecar"

  local expected package_name actual
  package_name="$(basename "$package")"
  expected="$(awk -v pkg="$package_name" '
    NF >= 2 {
      name = $NF
      sub(/^\*/, "", name)
      gsub(/^.*\//, "", name)
      if (name == pkg) {
        print $1
        found = 1
        exit
      }
    }
    END {
      if (!found) {
        exit 2
      }
    }
  ' "$sidecar")" || die "sha256 sidecar does not reference package ${package_name}"
  [[ "$expected" =~ ^[0-9A-Fa-f]{64}$ ]] || die "sha256 sidecar contains invalid digest for ${package_name}"
  actual="$(sha256sum "$package" | awk '{ print $1 }')"
  [[ "${actual,,}" == "${expected,,}" ]] || die "sha256 mismatch for ${package_name}"
  log "package sha256 verified for ${package_name}"
}

acquire_ota_lock() {
  mkdir -p "$(dirname "$LOCK_FILE")"
  exec 9>"$LOCK_FILE"
  if command -v flock >/dev/null 2>&1; then
    flock -n 9 || die "another OTA operation is already running"
  else
    local lock_dir="${LOCK_FILE}.d"
    if ! mkdir "$lock_dir" 2>/dev/null; then
      die "another OTA operation is already running"
    fi
    register_raw_dir_cleanup "$lock_dir"
  fi
}

resolved_path() {
  local path="$1"
  if [[ -e "$path" || -L "$path" ]]; then
    readlink -f "$path"
  else
    printf '%s' "$path"
  fi
}

active_slot_path() {
  if [[ -L "$CURRENT_LINK" ]]; then
    resolved_path "$CURRENT_LINK"
  else
    printf ''
  fi
}

slot_path() {
  local slot="$1"
  printf '%s/%s' "$SLOTS_DIR" "$slot"
}

managed_slot_name_from_path() {
  local path="$1"
  case "$path" in
    "$(slot_path a)") printf 'a' ;;
    "$(slot_path b)") printf 'b' ;;
    *) printf '' ;;
  esac
}

slot_name_from_path() {
  managed_slot_name_from_path "$1"
}

active_slot_name() {
  slot_name_from_path "$(active_slot_path)"
}

require_current_slot_path() {
  local path
  path="$(active_slot_path)"
  [[ -n "$path" ]] || die "current slot link is missing or invalid"
  [[ -n "$(managed_slot_name_from_path "$path")" ]] || die "current slot is outside managed slots: $path"
  [[ -d "$path" ]] || die "current slot path is missing: $path"
  printf '%s' "$path"
}

optional_current_slot_path() {
  local path
  path="$(active_slot_path)"
  if [[ -z "$path" ]]; then
    printf ''
    return 0
  fi
  [[ -n "$(managed_slot_name_from_path "$path")" ]] || die "current slot is outside managed slots: $path"
  [[ -d "$path" ]] || die "current slot path is missing: $path"
  printf '%s' "$path"
}

previous_slot_path() {
  local path=""
  if [[ -L "$PREVIOUS_LINK" ]]; then
    path="$(resolved_path "$PREVIOUS_LINK")"
  fi
  [[ -n "$path" ]] || die "previous slot link is missing or invalid"
  [[ -n "$(managed_slot_name_from_path "$path")" ]] || die "previous slot is outside managed slots: $path"
  [[ -d "$path" ]] || die "previous slot path is missing: $path"
  printf '%s' "$path"
}

inactive_slot_name() {
  local active
  active="$(active_slot_name)"
  if [[ "$active" == "a" ]]; then
    printf 'b'
  else
    printf 'a'
  fi
}

opposite_slot_name() {
  local slot="$1"
  case "$slot" in
    a) printf 'b' ;;
    b) printf 'a' ;;
    *) die "invalid slot name: ${slot}" ;;
  esac
}

assert_under_dir() {
  local path="$1"
  local base="$2"
  local real_base real_parent
  mkdir -p "$base"
  real_base="$(readlink -f "$base")"
  real_parent="$(readlink -f "$(dirname "$path")")"
  case "${real_parent}/" in
    "${real_base}/"*) ;;
    *) die "refusing to operate outside ${real_base}: ${path}" ;;
  esac
}

safe_remove_path() {
  local path="$1"
  local base="$2"
  [[ -n "$path" && "$path" != "/" ]] || die "refusing unsafe remove path"
  assert_under_dir "$path" "$base"
  rm -rf "$path"
}

atomic_symlink() {
  local target="$1"
  local link="$2"
  local tmp="${link}.tmp.$$"
  ln -sfn "$target" "$tmp"
  mv -Tf "$tmp" "$link"
  flush_ota_disk "symlink ${link}"
}

replace_with_link_preserving_existing() {
  local name="$1"
  local target="$2"
  local path="${PREFIX}/${name}"
  local legacy_dir="${PREFIX}/legacy-before-slots-$(date -u +%Y%m%d-%H%M%S)"
  if [[ -L "$path" ]]; then
    rm -f "$path"
  elif [[ -e "$path" ]]; then
    mkdir -p "$legacy_dir"
    mv "$path" "${legacy_dir}/${name}"
    log "preserved legacy ${path} at ${legacy_dir}/${name}"
  fi
  ln -sfnT "$target" "$path"
}

sync_compat_links() {
  [[ -L "$CURRENT_LINK" ]] || return 0
  replace_with_link_preserving_existing "bin" "current/bin"
  replace_with_link_preserving_existing "frontend" "current/frontend"
  replace_with_link_preserving_existing "static" "current/static"
  replace_with_link_preserving_existing "kiosk" "current/kiosk"
  replace_with_link_preserving_existing "backup.sh" "current/backup.sh"
  replace_with_link_preserving_existing "health-check.sh" "current/health-check.sh"
  flush_ota_disk "compatibility links"
}

install_root_ota_tools_from_slot() {
  local slot_dir="$1"
  for script in ota-update.sh ota-rollback.sh ota-lib.sh ota-boot-check.sh; do
    if [[ -f "${slot_dir}/${script}" ]]; then
      install -m 0755 "${slot_dir}/${script}" "${PREFIX}/${script}"
    fi
  done
  flush_ota_disk "root OTA tools"
}

install_systemd_units_from_slot() {
  local slot_dir="$1"
  [[ -f "${slot_dir}/deploy/reactor-edge-ota-boot-check.service" ]] || die "slot missing deploy/reactor-edge-ota-boot-check.service"
  [[ -f "${slot_dir}/deploy/reactor-edge.service" ]] || die "slot missing deploy/reactor-edge.service"
  [[ -f "${slot_dir}/deploy/reactor-edge-backup.service" ]] || die "slot missing deploy/reactor-edge-backup.service"
  [[ -f "${slot_dir}/deploy/reactor-edge-backup.timer" ]] || die "slot missing deploy/reactor-edge-backup.timer"
  [[ -f "${slot_dir}/deploy/reactor-os-chromium.service" ]] || die "slot missing deploy/reactor-os-chromium.service"
  mkdir -p "$SYSTEMD_UNIT_DIR"
  install -m 0644 "${slot_dir}/deploy/reactor-edge-ota-boot-check.service" "${SYSTEMD_UNIT_DIR}/reactor-edge-ota-boot-check.service"
  install -m 0644 "${slot_dir}/deploy/reactor-edge.service" "${SYSTEMD_UNIT_DIR}/reactor-edge.service"
  install -m 0644 "${slot_dir}/deploy/reactor-edge-backup.service" "${SYSTEMD_UNIT_DIR}/reactor-edge-backup.service"
  install -m 0644 "${slot_dir}/deploy/reactor-edge-backup.timer" "${SYSTEMD_UNIT_DIR}/reactor-edge-backup.timer"
  install -m 0644 "${slot_dir}/deploy/reactor-os-chromium.service" "${SYSTEMD_UNIT_DIR}/reactor-os-chromium.service"
  flush_ota_disk "systemd units"
}

service_exists() {
  local service="$1"
  command -v systemctl >/dev/null 2>&1 && systemctl list-unit-files "$service" >/dev/null 2>&1
}

stop_runtime_services() {
  if command -v systemctl >/dev/null 2>&1; then
    systemctl stop "$KIOSK_SERVICE" >/dev/null 2>&1 || true
    systemctl stop "$BACKEND_SERVICE" >/dev/null 2>&1 || true
  fi
}

start_runtime_services() {
  if command -v systemctl >/dev/null 2>&1; then
    mark_ota_service_start_allowed
    systemctl daemon-reload || {
      clear_ota_service_start_allowed
      return 1
    }
    systemctl start "$BACKEND_SERVICE" || {
      clear_ota_service_start_allowed
      return 1
    }
    systemctl restart "$BACKUP_TIMER" >/dev/null 2>&1 || true
    systemctl start "$KIOSK_SERVICE" >/dev/null 2>&1 || true
    clear_ota_service_start_allowed
  fi
}

backend_is_active() {
  command -v systemctl >/dev/null 2>&1 && systemctl is-active --quiet "$BACKEND_SERVICE"
}

check_not_busy() {
  local force="$1"
  if [[ "$force" -eq 1 ]]; then
    log "busy check bypassed by confirmed maintenance-window force"
    return 0
  fi
  if ! backend_is_active; then
    die "cannot prove reactor is idle because backend service is not active; use --force --confirm-maintenance-window only during a verified maintenance window"
  fi
  if ! command -v curl >/dev/null 2>&1; then
    die "curl is unavailable; cannot prove reactor is idle"
  fi
  local status_rc
  set +e
  prove_ota_safe_idle_status
  status_rc=$?
  set -e
  if [[ "$status_rc" -ne 0 ]]; then
    explain_ota_safe_idle_status_failure "$status_rc"
  fi
  log "device status proves no active process"
}

prove_ota_safe_idle_status() {
  local tmp
  tmp="$(mktemp)"
  if ! curl -fsS --max-time 3 "$STATUS_URL" >"$tmp"; then
    rm -f "$tmp"
    return 20
  fi
  if ! command -v python3 >/dev/null 2>&1; then
    rm -f "$tmp"
    echo "python3 is unavailable; cannot parse device status safely" >&2
    return 21
  fi
  local rc
  if python3 - "$tmp" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as fh:
    payload = json.load(fh)

data = payload.get("data", payload)
devices = data.get("devices", [])
if not devices:
    print("device status did not report any devices", file=sys.stderr)
    sys.exit(16)

busy = []
emergency = []
automatic = []
manual_lock = []
control_fault = []
unsafe_status = []
for device in devices:
    device_id = str(device.get("device_id", "unknown"))
    status = str(device.get("status", "")).lower()
    if device.get("active_batch_id") is not None or status == "running":
        busy.append(device_id)
    if device.get("emergency_stop"):
        emergency.append(device_id)
    if device.get("auto_enabled") is True:
        automatic.append(device_id)
    if device.get("manual_lock") is True:
        manual_lock.append(device_id)
    last_control_error = device.get("last_control_error")
    if isinstance(last_control_error, str):
        last_control_error = last_control_error.strip()
    if last_control_error or device.get("last_command_ok") is False:
        control_fault.append(device_id)
    if device.get("online") is not True or status != "idle":
        unsafe_status.append(f"{device_id}:{status or 'unknown'}")

if busy:
    print("busy devices: " + ",".join(busy), file=sys.stderr)
    sys.exit(10)
if emergency:
    print("emergency stop active: " + ",".join(emergency), file=sys.stderr)
    sys.exit(11)
if automatic:
    print("automatic control enabled: " + ",".join(automatic), file=sys.stderr)
    sys.exit(12)
if manual_lock:
    print("manual lock active: " + ",".join(manual_lock), file=sys.stderr)
    sys.exit(15)
if control_fault:
    print("control fault uncleared: " + ",".join(control_fault), file=sys.stderr)
    sys.exit(13)
if unsafe_status:
    print("device status not proven idle and online: " + ",".join(unsafe_status), file=sys.stderr)
    sys.exit(14)
PY
  then
    rc=0
  else
    rc=$?
  fi
  rm -f "$tmp"
  return "$rc"
}

explain_ota_safe_idle_status_failure() {
  local rc="$1"
  if [[ "$rc" -ne 0 ]]; then
    if [[ "$rc" -eq 10 ]]; then
      die "device is running an active process; wait for a maintenance window or use --force --confirm-maintenance-window"
    fi
    if [[ "$rc" -eq 11 ]]; then
      die "emergency stop is active; resolve field safety before OTA or use --force --confirm-maintenance-window"
    fi
    if [[ "$rc" -eq 12 ]]; then
      die "automatic control is enabled; disable automatic control and verify idle field state before OTA"
    fi
    if [[ "$rc" -eq 13 ]]; then
      die "device control fault is uncleared; resolve and reset the fault before OTA"
    fi
    if [[ "$rc" -eq 14 ]]; then
      die "device status is not proven idle and online; use --force --confirm-maintenance-window only during a verified maintenance window"
    fi
    if [[ "$rc" -eq 15 ]]; then
      die "manual lock is active; complete field handover before OTA or use --force --confirm-maintenance-window"
    fi
    if [[ "$rc" -eq 16 ]]; then
      die "device status did not report any devices; cannot prove reactor is idle"
    fi
    if [[ "$rc" -eq 20 ]]; then
      die "cannot read device status; use --force --confirm-maintenance-window only during a verified maintenance window"
    fi
    if [[ "$rc" -eq 21 ]]; then
      die "python3 is unavailable; cannot parse device status safely"
    fi
    die "device status parse failed"
  fi
}

run_pre_update_backup() {
  local skip_backup="$1"
  if [[ "$skip_backup" -eq 1 ]]; then
    log "database backup skipped by confirmed --skip-backup"
    return 0
  fi
  if [[ ! -f "$DB_PATH" ]]; then
    log "database not present yet; skipping backup: $DB_PATH"
    return 0
  fi
  local backup_script
  backup_script="$(backup_script_path)"
  [[ -n "$backup_script" ]] || die "backup script missing; refusing update"
  log "creating pre-update SQLite snapshot"
  REACTOR_EDGE_DB="$DB_PATH" "$backup_script"
}

backup_script_path() {
  if [[ -x "${CURRENT_LINK}/backup.sh" ]]; then
    printf '%s' "${CURRENT_LINK}/backup.sh"
  elif [[ -x "${PREFIX}/backup.sh" ]]; then
    printf '%s' "${PREFIX}/backup.sh"
  else
    printf ''
  fi
}

check_pre_update_backup_available() {
  local skip_backup="$1"
  if [[ "$skip_backup" -eq 1 ]]; then
    log "database backup availability preflight skipped by confirmed --skip-backup"
    return 0
  fi
  if [[ ! -f "$DB_PATH" ]]; then
    log "database not present yet; backup availability preflight skipped: $DB_PATH"
    return 0
  fi
  local backup_script
  backup_script="$(backup_script_path)"
  [[ -n "$backup_script" ]] || die "backup script missing; refusing update"
  log "database backup availability preflight passed: $backup_script"
}

ensure_space_for_package() {
  local package="$1"
  local package_bytes available_kb available_bytes required_bytes
  package_bytes="$(stat -c %s "$package")"
  available_kb="$(df -Pk "$SLOTS_DIR" | awk 'NR == 2 { print $4 }')"
  [[ "$package_bytes" =~ ^[0-9]+$ && "$available_kb" =~ ^[0-9]+$ ]] || die "cannot determine disk space for OTA"
  available_bytes=$((available_kb * 1024))
  # Keep room for tar extraction, slot replacement, logs, and a small safety margin.
  required_bytes=$((package_bytes * 3 + 104857600))
  if (( available_bytes < required_bytes )); then
    die "not enough disk space for OTA: available=${available_bytes} required=${required_bytes}"
  fi
  log "disk space check passed: available=${available_bytes} required=${required_bytes}"
}

validate_tar_package() {
  local package="$1"
  local list_file verbose_file top_levels_file member top_level type_char top_count
  list_file="$(mktemp)"
  verbose_file="$(mktemp)"
  top_levels_file="$(mktemp)"

  if ! tar -tzf "$package" >"$list_file"; then
    rm -f "$list_file" "$verbose_file" "$top_levels_file"
    die "release package tar directory cannot be read"
  fi
  if ! tar -tvzf "$package" >"$verbose_file"; then
    rm -f "$list_file" "$verbose_file" "$top_levels_file"
    die "release package tar metadata cannot be read"
  fi

  while IFS= read -r member; do
    [[ -n "$member" ]] || die "release package contains an empty tar member name"
    case "$member" in
      /*|../*|*/../*|..|*/..)
        rm -f "$list_file" "$verbose_file" "$top_levels_file"
        die "release package contains unsafe path: $member"
        ;;
    esac
    top_level="${member%%/*}"
    [[ -n "$top_level" && "$top_level" != "." && "$top_level" != ".." ]] || {
      rm -f "$list_file" "$verbose_file" "$top_levels_file"
      die "release package contains unsafe top-level path: $member"
    }
    printf '%s\n' "$top_level" >>"$top_levels_file"
  done <"$list_file"

  top_count="$(sort -u "$top_levels_file" | wc -l | tr -d '[:space:]')"
  [[ "$top_count" == "1" ]] || {
    rm -f "$list_file" "$verbose_file" "$top_levels_file"
    die "release package must contain exactly one top-level directory, found ${top_count}"
  }

  while IFS= read -r member; do
    type_char="${member:0:1}"
    case "$type_char" in
      -|d) ;;
      *)
        rm -f "$list_file" "$verbose_file" "$top_levels_file"
        die "release package contains unsupported tar member type: $member"
        ;;
    esac
  done <"$verbose_file"

  rm -f "$list_file" "$verbose_file" "$top_levels_file"
  log "release package tar safety check passed"
}

release_candidate_dir() {
  local extract_dir="$1"
  local candidate
  if [[ -x "${extract_dir}/bin/reactor-edge-daemon" ]]; then
    printf '%s' "$extract_dir"
    return 0
  fi
  while IFS= read -r candidate; do
    if [[ -x "${candidate}/bin/reactor-edge-daemon" ]]; then
      printf '%s' "$candidate"
      return 0
    fi
  done < <(find "$extract_dir" -mindepth 1 -maxdepth 1 -type d | sort)
  die "package does not contain a ReactorOS release root"
}

extract_release_candidate_to_stage() {
  local package="$1"
  local extract_dir="$2"
  local stage_dir="$3"
  local candidate_dir
  mkdir -p "$extract_dir"
  tar -xzf "$package" -C "$extract_dir"
  candidate_dir="$(release_candidate_dir "$extract_dir")"
  validate_slot_dir "$candidate_dir"
  validate_build_metadata "$candidate_dir"
  mv "$candidate_dir" "$stage_dir"
  safe_remove_path "$extract_dir" "$SLOTS_DIR"
  validate_slot_dir "$stage_dir"
  validate_build_metadata "$stage_dir"
  flush_ota_disk "staged candidate ${stage_dir}"
}

dry_run_release_candidate_validation() {
  local package="$1"
  local target_slot="$2"
  local extract_dir="${SLOTS_DIR}/.${target_slot}.dry-run.extract.$$"
  local stage_dir="${SLOTS_DIR}/.${target_slot}.dry-run.stage.$$"
  register_safe_remove_cleanup "$extract_dir" "$SLOTS_DIR"
  register_safe_remove_cleanup "$stage_dir" "$SLOTS_DIR"
  safe_remove_path "$extract_dir" "$SLOTS_DIR" 2>/dev/null || true
  safe_remove_path "$stage_dir" "$SLOTS_DIR" 2>/dev/null || true
  extract_release_candidate_to_stage "$package" "$extract_dir" "$stage_dir"
  safe_remove_path "$stage_dir" "$SLOTS_DIR"
  log "dry-run candidate slot validation passed for ${target_slot}"
}

read_release_metadata_from_package() {
  local package="$1"
  local target_slot="$2"
  local extract_dir="${SLOTS_DIR}/.${target_slot}.metadata.$$"
  local candidate_dir
  RELEASE_PACKAGE_VERSION=""
  RELEASE_PACKAGE_GIT=""
  register_safe_remove_cleanup "$extract_dir" "$SLOTS_DIR"
  safe_remove_path "$extract_dir" "$SLOTS_DIR" 2>/dev/null || true
  mkdir -p "$extract_dir"
  tar -xzf "$package" -C "$extract_dir"
  candidate_dir="$(release_candidate_dir "$extract_dir")"
  validate_build_metadata "$candidate_dir"
  RELEASE_PACKAGE_VERSION="$(release_version_from_dir "$candidate_dir")"
  RELEASE_PACKAGE_GIT="$(release_git_from_dir "$candidate_dir")"
  safe_remove_path "$extract_dir" "$SLOTS_DIR"
  log "candidate build metadata preflight passed: version=${RELEASE_PACKAGE_VERSION} git=${RELEASE_PACKAGE_GIT}"
}

health_check_loop() {
  local attempts="${1:-12}"
  local interval="${2:-5}"
  local required="${3:-3}"
  local ok_count=0
  local i status_rc
  for i in $(seq 1 "$attempts"); do
    if backend_is_active \
      && command -v curl >/dev/null 2>&1 \
      && curl -fsS --max-time 3 "$HEALTH_URL" >/dev/null \
      && curl -fsS --max-time 5 "$HMI_URL" >/dev/null; then
      set +e
      prove_ota_safe_idle_status
      status_rc=$?
      set -e
      if [[ "$status_rc" -eq 0 ]]; then
        ok_count=$((ok_count + 1))
        log "health check ${i}/${attempts} passed with safe idle status (${ok_count}/${required})"
        if [[ "$ok_count" -ge "$required" ]]; then
          return 0
        fi
      else
        ok_count=0
        log "health check ${i}/${attempts} failed: device status is not safe idle proof (rc=${status_rc})"
      fi
    else
      ok_count=0
      log "health check ${i}/${attempts} failed"
    fi
    sleep "$interval"
  done
  return 1
}

validate_slot_dir() {
  local dir="$1"
  [[ -x "${dir}/bin/reactor-edge-daemon" ]] || die "candidate missing executable bin/reactor-edge-daemon"
  [[ -x "${dir}/bin/reactor-safety-guard" ]] || die "candidate missing executable bin/reactor-safety-guard"
  [[ -x "${dir}/bin/xingshu" ]] || die "candidate missing executable bin/xingshu"
  [[ -x "${dir}/backup.sh" ]] || die "candidate missing executable backup.sh"
  [[ -x "${dir}/health-check.sh" ]] || die "candidate missing executable health-check.sh"
  [[ -x "${dir}/ota-boot-check.sh" ]] || die "candidate missing executable ota-boot-check.sh"
  [[ -x "${dir}/ota-update.sh" ]] || die "candidate missing executable ota-update.sh"
  [[ -x "${dir}/ota-rollback.sh" ]] || die "candidate missing executable ota-rollback.sh"
  [[ -x "${dir}/ota-lib.sh" ]] || die "candidate missing executable ota-lib.sh"
  [[ -f "${dir}/deploy/reactor-edge-ota-boot-check.service" ]] || die "candidate missing deploy/reactor-edge-ota-boot-check.service"
  [[ -f "${dir}/deploy/reactor-edge.service" ]] || die "candidate missing deploy/reactor-edge.service"
  [[ -f "${dir}/deploy/reactor-edge-backup.service" ]] || die "candidate missing deploy/reactor-edge-backup.service"
  [[ -f "${dir}/deploy/reactor-edge-backup.timer" ]] || die "candidate missing deploy/reactor-edge-backup.timer"
  [[ -f "${dir}/deploy/reactor-os-chromium.service" ]] || die "candidate missing deploy/reactor-os-chromium.service"
  [[ -f "${dir}/frontend/dist/index.html" || -f "${dir}/static/index.html" ]] || die "candidate missing HMI assets"
}
