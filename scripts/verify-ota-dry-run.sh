#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

export REACTOR_EDGE_PREFIX="${TMP_ROOT}/prefix"
export REACTOR_EDGE_DATA_DIR="${TMP_ROOT}/data"
export REACTOR_EDGE_PROJECT_DIR="${TMP_ROOT}/project"
export REACTOR_EDGE_OTA_LOCK="${TMP_ROOT}/ota.lock"
export REACTOR_EDGE_SKIP_SYNC_FOR_TESTS=1

# shellcheck source=../deploy/reactor-edge-ota-lib.sh
source "${ROOT}/deploy/reactor-edge-ota-lib.sh"
ensure_runtime_dirs

make_release_tree() {
  local root="$1"
  local release="${root}/reactor-os-test-release"
  mkdir -p "${release}/bin" "${release}/deploy" "${release}/frontend/dist" "${release}/static"
  printf '#!/usr/bin/env bash\n' >"${release}/bin/reactor-edge-daemon"
  printf '#!/usr/bin/env bash\n' >"${release}/bin/reactor-safety-guard"
  printf '#!/usr/bin/env bash\n' >"${release}/bin/xingshu"
  printf '#!/usr/bin/env bash\n' >"${release}/backup.sh"
  printf '#!/usr/bin/env bash\n' >"${release}/health-check.sh"
  printf '#!/usr/bin/env bash\n' >"${release}/ota-update.sh"
  printf '#!/usr/bin/env bash\n' >"${release}/ota-rollback.sh"
  printf '#!/usr/bin/env bash\n' >"${release}/ota-lib.sh"
  printf '#!/usr/bin/env bash\n' >"${release}/ota-boot-check.sh"
  chmod +x \
    "${release}/bin/reactor-edge-daemon" \
    "${release}/bin/reactor-safety-guard" \
    "${release}/bin/xingshu" \
    "${release}/backup.sh" \
    "${release}/health-check.sh" \
    "${release}/ota-update.sh" \
    "${release}/ota-rollback.sh" \
    "${release}/ota-lib.sh" \
    "${release}/ota-boot-check.sh"
  printf '[Unit]\n' >"${release}/deploy/reactor-edge-ota-boot-check.service"
  printf '[Service]\n' >"${release}/deploy/reactor-edge.service"
  printf '[Service]\n' >"${release}/deploy/reactor-edge-backup.service"
  printf '[Timer]\n' >"${release}/deploy/reactor-edge-backup.timer"
  printf '[Service]\n' >"${release}/deploy/reactor-os-chromium.service"
  printf '<div id="app"></div>\n' >"${release}/frontend/dist/index.html"
  cat >"${release}/BUILD-METADATA.properties" <<'EOF'
REACTOR_EDGE_BUILD_SCHEMA=reactor-edge.build.v1
REACTOR_EDGE_PACKAGE_NAME=reactor-os-test-release
REACTOR_EDGE_GIT_SHA=testsha
REACTOR_EDGE_GIT_FULL_SHA=testsha0000000000000000000000000000000000
REACTOR_EDGE_GIT_DIRTY=false
REACTOR_EDGE_BUILT_AT_UTC=2026-06-08T00:00:00Z
REACTOR_EDGE_TARGET=aarch64-unknown-linux-gnu
REACTOR_EDGE_PROFILE=release
REACTOR_EDGE_PKG_PREFIX=reactor-os-test
REACTOR_EDGE_BOARD_NAME=test-board
REACTOR_EDGE_SERVICE_USER=pi
REACTOR_EDGE_CONFIG_NAME=device.json_bridge.toml
EOF
}

expect_fail() {
  local label="$1"
  local expected="$2"
  shift 2
  local output rc
  set +e
  output="$( "$@" 2>&1 )"
  rc=$?
  set -e
  if [[ "$rc" -eq 0 ]]; then
    echo "expected failure for ${label}" >&2
    exit 1
  fi
  if [[ "$output" != *"$expected"* ]]; then
    echo "expected failure for ${label} to contain '${expected}', got:" >&2
    printf '%s\n' "$output" >&2
    exit 1
  fi
}

mkdir -p "$(slot_path a)" "$(slot_path b)"
ln -sfn "$(slot_path a)" "$CURRENT_LINK"
ln -sfn "$(slot_path b)" "$PREVIOUS_LINK"
printf 'sqlite placeholder\n' >"$DB_PATH"

expect_fail "missing backup script" "backup script missing; refusing update" check_pre_update_backup_available 0

printf '#!/usr/bin/env bash\nprintf backup-called >>"%s"\n' "${TMP_ROOT}/backup-called" >"${CURRENT_LINK}/backup.sh"
chmod +x "${CURRENT_LINK}/backup.sh"
check_pre_update_backup_available 0
[[ ! -e "${TMP_ROOT}/backup-called" ]] || {
  echo "dry-run backup availability check executed the backup script" >&2
  exit 1
}

src="${TMP_ROOT}/src"
mkdir -p "$src"
make_release_tree "$src"
package="${TMP_ROOT}/reactor-os-test-release.tar.gz"
tar -czf "$package" -C "$src" reactor-os-test-release

current_before="$(readlink -f "$CURRENT_LINK")"
previous_before="$(readlink -f "$PREVIOUS_LINK")"
dry_run_release_candidate_validation "$package" "$(inactive_slot_name)"
read_release_metadata_from_package "$package" "$(inactive_slot_name)"
current_after="$(readlink -f "$CURRENT_LINK")"
previous_after="$(readlink -f "$PREVIOUS_LINK")"

[[ "$current_before" == "$current_after" ]] || {
  echo "dry-run changed current slot link" >&2
  exit 1
}
[[ "$previous_before" == "$previous_after" ]] || {
  echo "dry-run changed previous slot link" >&2
  exit 1
}
if find "$SLOTS_DIR" -maxdepth 1 \( -name '*.dry-run.extract.*' -o -name '*.dry-run.stage.*' \) | grep -q .; then
  echo "dry-run left temporary extract/stage directories" >&2
  exit 1
fi
[[ ! -e "$(slot_path b)/bin/reactor-edge-daemon" ]] || {
  echo "dry-run wrote candidate payload into inactive slot" >&2
  exit 1
}
[[ "$RELEASE_PACKAGE_VERSION" == "reactor-os-test-release" ]] || {
  echo "dry-run metadata version was not captured" >&2
  exit 1
}
[[ "$RELEASE_PACKAGE_GIT" == "testsha" ]] || {
  echo "dry-run metadata git sha was not captured" >&2
  exit 1
}
write_ota_state "dry_run_passed" "$(inactive_slot_name)" "" "$package" "old-release" "$RELEASE_PACKAGE_VERSION" "oldsha" "$RELEASE_PACKAGE_GIT"
python3 - "$STATE_FILE" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as fh:
    state = json.load(fh)

expected = {
    "status": "dry_run_passed",
    "from_version": "old-release",
    "to_version": "reactor-os-test-release",
    "from_git": "oldsha",
    "to_git": "testsha",
}
for key, value in expected.items():
    if state.get(key) != value:
        raise SystemExit(f"state {key} expected {value!r}, got {state.get(key)!r}")
PY

bad_src="${TMP_ROOT}/bad-src"
mkdir -p "$bad_src"
make_release_tree "$bad_src"
rm -f "${bad_src}/reactor-os-test-release/bin/reactor-safety-guard"
bad_package="${TMP_ROOT}/bad-release.tar.gz"
tar -czf "$bad_package" -C "$bad_src" reactor-os-test-release
expect_fail "bad candidate package" "candidate missing executable bin/reactor-safety-guard" \
  dry_run_release_candidate_validation "$bad_package" "$(inactive_slot_name)"

missing_metadata_src="${TMP_ROOT}/missing-metadata-src"
mkdir -p "$missing_metadata_src"
make_release_tree "$missing_metadata_src"
rm -f "${missing_metadata_src}/reactor-os-test-release/BUILD-METADATA.properties"
missing_metadata_package="${TMP_ROOT}/missing-metadata-release.tar.gz"
tar -czf "$missing_metadata_package" -C "$missing_metadata_src" reactor-os-test-release
expect_fail "missing candidate metadata" "candidate missing BUILD-METADATA.properties" \
  dry_run_release_candidate_validation "$missing_metadata_package" "$(inactive_slot_name)"

echo "OTA dry-run gate passed"
