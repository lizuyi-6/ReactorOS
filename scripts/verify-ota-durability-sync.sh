#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

require_text() {
  local file="$1"
  local needle="$2"
  local label="$3"
  if ! grep -Fq "$needle" "${ROOT}/${file}"; then
    echo "${label}: ${file} must contain ${needle}" >&2
    exit 1
  fi
}

require_text "deploy/reactor-edge-ota-lib.sh" "flush_ota_disk()" "OTA shared library exposes a durability flush helper"
require_text "deploy/reactor-edge-ota-lib.sh" "sync" "OTA shared library uses filesystem sync after critical writes"
require_text "deploy/reactor-edge-ota-lib.sh" "REACTOR_EDGE_SKIP_SYNC_FOR_TESTS" "OTA sync skip is explicitly test-only"
require_text "deploy/reactor-edge-ota-lib.sh" 'flush_ota_disk "OTA state ${status}"' "OTA state writes are flushed"
require_text "deploy/reactor-edge-ota-lib.sh" 'flush_ota_disk "symlink ${link}"' "current/previous symlink switches are flushed"
require_text "deploy/reactor-edge-ota-lib.sh" 'flush_ota_disk "compatibility links"' "compatibility link rewrites are flushed"
require_text "deploy/reactor-edge-ota-lib.sh" 'flush_ota_disk "root OTA tools"' "root OTA tool replacement is flushed"
require_text "deploy/reactor-edge-ota-lib.sh" 'flush_ota_disk "systemd units"' "systemd unit replacement is flushed"
require_text "deploy/reactor-edge-ota-lib.sh" 'flush_ota_disk "staged candidate ${stage_dir}"' "staged candidate writes are flushed"
require_text "deploy/reactor-edge-ota-update.sh" 'flush_ota_disk "slot ${TARGET_SLOT} replacement"' "inactive slot replacement is flushed"
require_text "deploy/reactor-edge-ota-lib.sh" "require_ota_update_commands() {" "OTA update command preflight exists"
require_text "deploy/reactor-edge-ota-lib.sh" "require_ota_rollback_commands() {" "OTA rollback command preflight exists"
require_text "deploy/reactor-edge-ota-lib.sh" "require_ota_boot_check_commands() {" "OTA boot-check command preflight exists"
require_text "deploy/reactor-edge-ota-lib.sh" " sleep sort stat sync tar " "update preflight requires sync"
require_text "deploy/reactor-edge-ota-lib.sh" " sleep sync tee" "rollback preflight requires sync"
require_text "deploy/reactor-edge-ota-lib.sh" " sleep stat sync tee" "boot-check preflight requires sync"
require_text "scripts/verify-ota-command-preflight.sh" 'expect_fail "missing sync for update"' "command preflight tests missing sync for update"
require_text "scripts/verify-ota-command-preflight.sh" 'expect_fail "missing sync for rollback"' "command preflight tests missing sync for rollback"
require_text "scripts/verify-ota-command-preflight.sh" 'expect_fail "missing sync for boot-check"' "command preflight tests missing sync for boot-check"

echo "OTA durability sync gate passed"
