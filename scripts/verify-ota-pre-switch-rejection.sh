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
export REACTOR_EDGE_ALLOW_NON_ROOT_FOR_TESTS=1
export REACTOR_EDGE_SKIP_SYNC_FOR_TESTS=1
export REACTOR_EDGE_SERVICE=reactor-edge-test
export REACTOR_EDGE_KIOSK_SERVICE=reactor-os-chromium-test

FAKE_BIN="${TMP_ROOT}/fake-bin"
SYSTEMCTL_LOG="${TMP_ROOT}/systemctl.log"
mkdir -p "$FAKE_BIN"
cat >"${FAKE_BIN}/systemctl" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >>"${SYSTEMCTL_LOG}"
case "\$*" in
  "is-active --quiet ${REACTOR_EDGE_SERVICE}") exit 0 ;;
  *) exit 0 ;;
esac
EOF
chmod +x "${FAKE_BIN}/systemctl"
cat >"${FAKE_BIN}/curl" <<'EOF'
#!/usr/bin/env bash
for arg in "$@"; do
  case "$arg" in
    */api/devices/status)
      printf '{"data":{"devices":[{"device_id":"reactor-1","online":true,"status":"idle","active_batch_id":null,"emergency_stop":false,"auto_enabled":false,"manual_lock":false,"last_control_error":null,"last_command_ok":true}]}}\n'
      exit 0
      ;;
  esac
done
printf 'ok\n'
EOF
chmod +x "${FAKE_BIN}/curl"
export PATH="${FAKE_BIN}:${PATH}"

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

make_candidate_tree() {
  local dir="$1"
  make_slot "$dir" "candidate-slot"
}

make_slot "$(slot_path a)" "old-slot"
make_slot "$(slot_path b)" "spare-slot"
atomic_symlink "$(slot_path a)" "$CURRENT_LINK"
atomic_symlink "$(slot_path a)" "$PREVIOUS_LINK"
sync_compat_links

candidate_root="${TMP_ROOT}/candidate-release"
make_candidate_tree "$candidate_root"
package="${TMP_ROOT}/candidate.tar.gz"
tar -C "$TMP_ROOT" -czf "$package" "$(basename "$candidate_root")"

bad_sidecar="${TMP_ROOT}/candidate.tar.gz.sha256"
printf '%064d  %s\n' 0 "$(basename "$package")" >"$bad_sidecar"

set +e
bad_checksum_output="$(bash "${ROOT}/deploy/reactor-edge-ota-update.sh" "$package" --sha256 "$bad_sidecar" --dry-run 2>&1)"
bad_checksum_rc=$?
set -e
if [[ "$bad_checksum_rc" -eq 0 ]]; then
  echo "OTA update unexpectedly passed with bad checksum" >&2
  exit 1
fi
if [[ "$bad_checksum_output" != *"sha256 mismatch"* ]]; then
  echo "bad checksum failure did not report sha256 mismatch:" >&2
  printf '%s\n' "$bad_checksum_output" >&2
  exit 1
fi
grep -Fq '"status": "rejected_before_switch"' "$STATE_FILE" || {
  echo "bad checksum did not record rejected_before_switch" >&2
  cat "$STATE_FILE" >&2
  exit 1
}
grep -Fq 'current slot remains active' "$STATE_FILE" || {
  echo "bad checksum rejection did not record current slot safety reason" >&2
  cat "$STATE_FILE" >&2
  exit 1
}
[[ "$(readlink -f "$CURRENT_LINK")" == "$(readlink -f "$(slot_path a)")" ]] || {
  echo "bad checksum changed current slot" >&2
  exit 1
}

unsafe_root="${TMP_ROOT}/unsafe-release"
mkdir -p "${unsafe_root}/one" "${unsafe_root}/two"
printf 'a\n' >"${unsafe_root}/one/a.txt"
printf 'b\n' >"${unsafe_root}/two/b.txt"
unsafe_package="${TMP_ROOT}/unsafe.tar.gz"
tar -C "$unsafe_root" -czf "$unsafe_package" one two
(cd "$TMP_ROOT" && sha256sum "$(basename "$unsafe_package")" >"${unsafe_package}.sha256")

set +e
unsafe_output="$(bash "${ROOT}/deploy/reactor-edge-ota-update.sh" "$unsafe_package" --sha256 "${unsafe_package}.sha256" --dry-run 2>&1)"
unsafe_rc=$?
set -e
if [[ "$unsafe_rc" -eq 0 ]]; then
  echo "OTA update unexpectedly passed with unsafe tar" >&2
  exit 1
fi
if [[ "$unsafe_output" != *"exactly one top-level directory"* ]]; then
  echo "unsafe tar failure did not report top-level directory problem:" >&2
  printf '%s\n' "$unsafe_output" >&2
  exit 1
fi
grep -Fq '"status": "rejected_before_switch"' "$STATE_FILE" || {
  echo "unsafe tar did not record rejected_before_switch" >&2
  cat "$STATE_FILE" >&2
  exit 1
}
[[ "$(readlink -f "$CURRENT_LINK")" == "$(readlink -f "$(slot_path a)")" ]] || {
  echo "unsafe tar changed current slot" >&2
  exit 1
}

bash "${ROOT}/deploy/reactor-edge-ota-boot-check.sh"
[[ "$(readlink -f "$CURRENT_LINK")" == "$(readlink -f "$(slot_path a)")" ]] || {
  echo "boot-check changed current slot for rejected_before_switch" >&2
  exit 1
}

echo "OTA pre-switch rejection gate passed"
