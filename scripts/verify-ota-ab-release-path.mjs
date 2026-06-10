#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const checks = [
  {
    file: "deploy/reactor-edge.service",
    mustContain: [
      "After=reactor-edge-ota-boot-check.service network-online.target",
      "Requires=reactor-edge-ota-boot-check.service",
      "WorkingDirectory=/opt/reactor-edge/current",
      "ExecStartPre=/opt/reactor-edge/ota-boot-check.sh",
      "ExecStart=/opt/reactor-edge/current/bin/reactor-edge-daemon",
      "--safety-guard /opt/reactor-edge/current/bin/reactor-safety-guard",
      "--assets auto",
      "Restart=on-failure",
      "StartLimitIntervalSec=600",
      "StartLimitBurst=5",
    ],
    label: "backend service follows the active application slot",
  },
  {
    file: "deploy/reactor-edge-backup.service",
    mustContain: [
      "WorkingDirectory=/opt/reactor-edge/current",
      "ExecStart=/opt/reactor-edge/current/backup.sh",
    ],
    label: "backup service follows the active application slot",
  },
  {
    file: "deploy/reactor-os-chromium.service",
    mustContain: [
      "After=graphical.target reactor-edge-ota-boot-check.service reactor-edge.service",
      "Requires=reactor-edge-ota-boot-check.service",
      "WorkingDirectory=/opt/reactor-edge/current",
      "ExecStart=/opt/reactor-edge/current/kiosk/run-chromium-kiosk.sh",
      "StartLimitIntervalSec=600",
      "StartLimitBurst=5",
    ],
    label: "kiosk service follows the active application slot",
  },
  {
    file: "deploy/install-board.sh",
    mustContain: [
      'SLOTS_DIR="${PREFIX}/slots"',
      'INSTALL_ROOT="${REACTOR_EDGE_INSTALL_ROOT:-}"',
      "validate_package_before_stopping_services",
      "Missing required executable package file",
      "Missing HMI assets",
      'INITIAL_SLOT="${REACTOR_EDGE_INITIAL_SLOT:-a}"',
      'systemctl stop reactor-edge',
      'copy_tree "${ROOT}/bin" "${SLOT_DIR}/bin"',
      'link_or_preserve_existing "${PREFIX}/current" "$SLOT_DIR"',
      'link_or_preserve_existing "${PREFIX}/previous" "$PREVIOUS_SLOT_DIR"',
      'link_or_preserve_existing "${PREFIX}/bin" "current/bin"',
      'install -m 0644 "${ROOT}/BUILD-METADATA.properties" "${SLOT_DIR}/BUILD-METADATA.properties"',
      'install -m 0755 "${SLOT_DIR}/ota-update.sh" "${PREFIX}/ota-update.sh"',
      'install -m 0755 "${SLOT_DIR}/ota-boot-check.sh" "${PREFIX}/ota-boot-check.sh"',
      "reactor-edge-ota-boot-check.service",
      "systemctl enable reactor-edge-ota-boot-check",
    ],
    label: "board installer initializes slots and compatibility links",
  },
  {
    file: "deploy/reactor-edge-ota-boot-check.service",
    mustContain: [
      "Description=Reactor Edge OTA Boot Recovery Check",
      "Type=oneshot",
      "Before=reactor-edge.service",
      "ExecStart=/opt/reactor-edge/ota-boot-check.sh",
      "ReadWritePaths=-/opt/reactor-edge",
      "ReadWritePaths=-/var/lib/reactor-edge",
    ],
    label: "OTA boot-check service runs before backend startup",
  },
  {
    file: "deploy/reactor-edge-ota-boot-check.sh",
    mustContain: [
      "require_ota_boot_check_commands",
      "ota_service_start_allowed",
      "restore_previous_slot_after_interrupted_ota",
      "record_interrupted_before_switch",
      "interrupted_before_switch",
      "rolled_back_on_boot",
      "OTA state is failed on boot",
      "keep device in maintenance",
      "current already points at previous slot",
      "before current switch completed",
      "interrupted OTA ${STATUS} detected on boot",
      "interrupted OTA ${interrupted_status} before current switch",
      "unexpected OTA state on boot",
      "use recovery or manual rollback",
    ],
    label: "OTA boot-check script handles interrupted slot switches",
  },
  {
    file: "deploy/reactor-edge-ota-update.sh",
    mustContain: [
      "acquire_ota_lock",
      "require_ota_update_commands",
      "validate_health_args",
      "require_confirmed_dangerous_option",
      "verify_sha256_for_package",
      "check_not_busy",
      "ensure_space_for_package",
      "validate_tar_package",
      "--dry-run",
      "--confirm-unsafe-no-checksum",
      "--confirm-maintenance-window",
      "--confirm-skip-backup",
      "check_pre_update_backup_available",
      "dry_run_release_candidate_validation",
      "read_release_metadata_from_package",
      'TARGET_VERSION="$RELEASE_PACKAGE_VERSION"',
      "write_ota_state \"dry_run_passed\"",
      "no slot switch performed",
      "run_pre_update_backup",
      "write_ota_state \"backup_done\"",
      "write_ota_state \"staged\"",
      'PREVIOUS_PATH="$(optional_current_slot_path)"',
      'register_safe_remove_cleanup "$EXTRACT_DIR" "$SLOTS_DIR"',
      'register_safe_remove_cleanup "$STAGE_DIR" "$SLOTS_DIR"',
      "extract_release_candidate_to_stage \"$PACKAGE\" \"$EXTRACT_DIR\" \"$STAGE_DIR\"",
      "write_ota_state \"switching\"",
      "write_ota_state \"health_checking\"",
      "install_systemd_units_from_slot \"$TARGET_DIR\"",
      "atomic_symlink \"$TARGET_DIR\" \"$CURRENT_LINK\"",
      "start_runtime_services && health_check_loop",
      "health_check_loop",
      "new slot ${TARGET_SLOT} failed health checks; attempting rollback",
      "install_systemd_units_from_slot \"$PREVIOUS_PATH\"",
      "atomic_symlink \"$PREVIOUS_PATH\" \"$CURRENT_LINK\"",
      "enter_ota_failed_state",
      "rollback health check failed; keep device in manual maintenance",
    ],
    label: "OTA update script verifies, switches, and rolls back safely",
  },
  {
    file: "deploy/reactor-edge-ota-rollback.sh",
    mustContain: [
      "acquire_ota_lock",
      "require_ota_rollback_commands",
      "require_confirmed_dangerous_option",
      "--confirm-maintenance-window",
      "check_not_busy",
      'CURRENT_PATH="$(require_current_slot_path)"',
      'PREVIOUS_PATH="$(previous_slot_path)"',
      'FROM_VERSION="$(release_version_from_dir "$CURRENT_PATH")"',
      'TARGET_VERSION="$(release_version_from_dir "$PREVIOUS_PATH")"',
      "install_systemd_units_from_slot \"$PREVIOUS_PATH\"",
      "atomic_symlink \"$PREVIOUS_PATH\" \"$CURRENT_LINK\"",
      "write_ota_state \"rolled_back\"",
      "enter_ota_failed_state",
      "manual rollback health check failed; keep device in maintenance",
    ],
    label: "manual rollback script protects active production state",
  },
  {
    file: "deploy/reactor-edge-ota-lib.sh",
    mustContain: [
      "STATE_FILE=",
      "LOCK_FILE=",
      "flock -n 9",
      "run_registered_cleanups",
      "register_safe_remove_cleanup",
      "require_commands",
      "require_ota_update_commands",
      "require_ota_rollback_commands",
      "require_ota_boot_check_commands",
      "SYSTEMD_UNIT_DIR=",
      "OTA_SERVICE_START_ALLOWED=",
      "missing required command(s)",
      "mark_ota_service_start_allowed",
      "clear_ota_service_start_allowed",
      "ota_service_start_allowed",
      "process_start_ticks",
      "ota_marker_value",
      "owner process is not active",
      "require_current_slot_path",
      "previous_slot_path",
      "current slot is outside managed slots",
      "previous slot is outside managed slots",
      "previous slot link is missing or invalid",
      "require_positive_int",
      "require_confirmed_dangerous_option",
      "do not use field bypasses without a recorded maintenance decision",
      "sha256 sidecar does not reference package",
      "device is running an active process",
      "emergency stop is active",
      "cannot prove reactor is idle because backend service is not active",
      "not enough disk space for OTA",
      "release package contains unsafe path",
      "unsupported tar member type",
      "backup script missing; refusing update",
      "from_version",
      "to_version",
      "from_git",
      "to_git",
      "validate_build_metadata",
      "candidate missing BUILD-METADATA.properties",
      "candidate build metadata schema is invalid",
      "flush_ota_disk",
      'flush_ota_disk "OTA state ${status}"',
      'flush_ota_disk "symlink ${link}"',
      'flush_ota_disk "compatibility links"',
      'flush_ota_disk "root OTA tools"',
      'flush_ota_disk "systemd units"',
      'flush_ota_disk "staged candidate ${stage_dir}"',
      "read_release_metadata_from_package",
      "candidate build metadata preflight passed",
      "check_pre_update_backup_available",
      "database backup availability preflight passed",
      "extract_release_candidate_to_stage",
      "dry_run_release_candidate_validation",
      "dry-run candidate slot validation passed",
      "install_systemd_units_from_slot",
      "reactor-edge-ota-boot-check.service",
      "candidate missing executable ota-boot-check.sh",
      "candidate missing deploy/reactor-edge-ota-boot-check.service",
      "mark_ota_service_start_allowed",
      'systemctl start "$BACKEND_SERVICE" || {',
      "clear_ota_service_start_allowed",
      "enter_ota_failed_state",
      "write_ota_state \"failed\"",
      "stop_runtime_services",
      "ota_pid_start_ticks",
      "candidate missing executable bin/reactor-edge-daemon",
      "candidate missing deploy/reactor-edge.service",
    ],
    label: "OTA shared library covers industrial failure modes",
  },
  {
    file: "deploy/reactor-edge-ota-update.sh",
    mustContain: [
      'flush_ota_disk "slot ${TARGET_SLOT} replacement"',
    ],
    label: "OTA update script flushes inactive slot replacement before switching",
  },
  {
    file: "scripts/package-a55-debian10.sh",
    mustContain: [
      'cp deploy/reactor-edge-ota-update.sh "${PACKAGE_DIR}/ota-update.sh"',
      'cp deploy/reactor-edge-ota-rollback.sh "${PACKAGE_DIR}/ota-rollback.sh"',
      'cp deploy/reactor-edge-ota-lib.sh "${PACKAGE_DIR}/ota-lib.sh"',
      'cp deploy/reactor-edge-ota-boot-check.sh "${PACKAGE_DIR}/ota-boot-check.sh"',
      "cp deploy/reactor-edge-ota-boot-check.service",
      "BUILD-METADATA.properties",
      "REACTOR_EDGE_BUILD_SCHEMA=reactor-edge.build.v1",
      "REACTOR_EDGE_PACKAGE_NAME=${PACKAGE_NAME}",
      "REACTOR_EDGE_GIT_SHA=${GIT_SHA}",
      "REACTOR_EDGE_BUILT_AT_UTC=${BUILT_AT_UTC}",
      'sha256sum "${PACKAGE_NAME}.tar.gz" >"${PACKAGE_NAME}.tar.gz.sha256"',
    ],
    label: "ARM64 release package carries OTA tools, checksum, and build metadata",
  },
  {
    file: "scripts/verify-ota-dry-run.sh",
    mustContain: [
      "check_pre_update_backup_available",
      "dry_run_release_candidate_validation",
      "read_release_metadata_from_package",
      "dry-run changed current slot link",
      "dry-run wrote candidate payload into inactive slot",
      "dry-run metadata version was not captured",
      "candidate missing BUILD-METADATA.properties",
      "candidate missing executable bin/reactor-safety-guard",
    ],
    label: "OTA dry-run gate validates packages without switching slots",
  },
  {
    file: "scripts/verify-ota-dangerous-options.sh",
    mustContain: [
      "--allow-missing-checksum requires --confirm-unsafe-no-checksum",
      "--force requires --confirm-maintenance-window",
      "--skip-backup requires --confirm-skip-backup",
      "confirmed unsafe checksum reaches package validation",
    ],
    label: "OTA dangerous-options gate requires recorded operator confirmation",
  },
  {
    file: "scripts/verify-ota-failed-state.sh",
    mustContain: [
      "enter_ota_failed_state",
      "service-start marker did not record owner PID",
      "service-start marker did not record owner process start ticks",
      "failed state did not clear active OTA service-start marker",
      "failed state did not stop kiosk service",
      "failed state did not stop backend service",
    ],
    label: "OTA failed-state gate stops services immediately",
  },
];

const failures = [];
for (const check of checks) {
  const fullPath = path.join(root, check.file);
  let text;
  try {
    text = await readFile(fullPath, "utf8");
  } catch (error) {
    failures.push(`${check.label}: cannot read ${check.file}: ${error.message}`);
    continue;
  }
  for (const needle of check.mustContain) {
    if (!text.includes(needle)) {
      failures.push(`${check.label}: ${check.file} must contain ${JSON.stringify(needle)}`);
    }
  }
}

const updateScript = await readFile(path.join(root, "deploy/reactor-edge-ota-update.sh"), "utf8");
const rollbackScript = await readFile(path.join(root, "deploy/reactor-edge-ota-rollback.sh"), "utf8");
const bootCheckService = await readFile(path.join(root, "deploy/reactor-edge-ota-boot-check.service"), "utf8");
if (bootCheckService.includes("RemainAfterExit=yes")) {
  failures.push("OTA boot-check service must not use RemainAfterExit=yes; it must rerun for every backend start transaction");
}

const previousIndex = updateScript.indexOf('atomic_symlink "$PREVIOUS_PATH" "$PREVIOUS_LINK"');
const switchingIndex = updateScript.indexOf('write_ota_state "switching"', previousIndex);
const rootToolsIndex = updateScript.indexOf('install_root_ota_tools_from_slot "$TARGET_DIR"', switchingIndex);
const systemdIndex = updateScript.indexOf('install_systemd_units_from_slot "$TARGET_DIR"', switchingIndex);
const currentIndex = updateScript.indexOf('atomic_symlink "$TARGET_DIR" "$CURRENT_LINK"', switchingIndex);
const healthCheckingIndex = updateScript.indexOf('write_ota_state "health_checking"', switchingIndex);
if (!(previousIndex >= 0 && switchingIndex > previousIndex && rootToolsIndex > switchingIndex && systemdIndex > rootToolsIndex && currentIndex > systemdIndex && healthCheckingIndex > currentIndex)) {
  failures.push("OTA update script must set previous before switching state, then install root OTA tools, install systemd units, switch current, and mark health_checking");
}

const rollbackStateIndex = rollbackScript.indexOf('write_ota_state "rolling_back"');
const rollbackSystemdIndex = rollbackScript.indexOf('install_systemd_units_from_slot "$PREVIOUS_PATH"', rollbackStateIndex);
const rollbackCurrentIndex = rollbackScript.indexOf('atomic_symlink "$PREVIOUS_PATH" "$CURRENT_LINK"', rollbackSystemdIndex);
const rollbackPreviousIndex = rollbackScript.indexOf('atomic_symlink "$CURRENT_PATH" "$PREVIOUS_LINK"', rollbackCurrentIndex);
if (!(rollbackStateIndex >= 0 && rollbackSystemdIndex > rollbackStateIndex && rollbackCurrentIndex > rollbackSystemdIndex && rollbackPreviousIndex > rollbackCurrentIndex)) {
  failures.push("manual rollback must install target units, switch current to the rollback target, then repoint previous to the old current slot");
}

if (failures.length) {
  console.error(failures.join("\n"));
  process.exit(1);
}

console.log("OTA A/B release path gate passed");
