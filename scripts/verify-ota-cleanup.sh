#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

run_child() {
  local mode="$1"
  local child="${TMP_ROOT}/child-${mode}.sh"
  cat >"$child" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
ROOT="$1"
MODE="$2"
TMP_ROOT="$3"
export REACTOR_EDGE_PREFIX="${TMP_ROOT}/prefix"
export REACTOR_EDGE_DATA_DIR="${TMP_ROOT}/data"
export REACTOR_EDGE_PROJECT_DIR="${TMP_ROOT}/project"
export REACTOR_EDGE_OTA_LOCK="${TMP_ROOT}/ota.lock"
if [[ "$MODE" == "no-flock" ]]; then
  FAKE_BIN="${TMP_ROOT}/fake-bin"
  mkdir -p "$FAKE_BIN"
  for cmd in awk basename cat chmod date dirname env grep ln mkdir mktemp mv printf readlink rm rmdir sed seq sha256sum sleep sort stat tar tee touch tr wc; do
    if command -v "$cmd" >/dev/null 2>&1; then
      ln -s "$(command -v "$cmd")" "${FAKE_BIN}/${cmd}"
    fi
  done
  export PATH="${FAKE_BIN}"
fi
# shellcheck source=../deploy/reactor-edge-ota-lib.sh
source "${ROOT}/deploy/reactor-edge-ota-lib.sh"
ensure_runtime_dirs
extract="${SLOTS_DIR}/.a.extract.cleanup-test"
stage="${SLOTS_DIR}/a.tmp.cleanup-test"
mkdir -p "$extract" "$stage"
printf 'leftover\n' >"${extract}/file.txt"
printf 'leftover\n' >"${stage}/file.txt"
register_safe_remove_cleanup "$extract" "$SLOTS_DIR"
register_safe_remove_cleanup "$stage" "$SLOTS_DIR"
if [[ "$MODE" == "no-flock" ]]; then
  acquire_ota_lock
fi
exit 17
EOF
  chmod +x "$child"
  set +e
  "$child" "$ROOT" "$mode" "$TMP_ROOT" >/dev/null 2>&1
  local rc=$?
  set -e
  [[ "$rc" -eq 17 ]] || {
    echo "expected child ${mode} to exit 17, got ${rc}" >&2
    exit 1
  }
}

run_child standard
[[ ! -e "${TMP_ROOT}/prefix/slots/.a.extract.cleanup-test" ]] || {
  echo "extract cleanup path remained after failure" >&2
  exit 1
}
[[ ! -e "${TMP_ROOT}/prefix/slots/a.tmp.cleanup-test" ]] || {
  echo "stage cleanup path remained after failure" >&2
  exit 1
}

run_child no-flock
[[ ! -e "${TMP_ROOT}/prefix/slots/.a.extract.cleanup-test" ]] || {
  echo "extract cleanup path remained after no-flock failure" >&2
  exit 1
}
[[ ! -e "${TMP_ROOT}/prefix/slots/a.tmp.cleanup-test" ]] || {
  echo "stage cleanup path remained after no-flock failure" >&2
  exit 1
}
[[ ! -e "${TMP_ROOT}/ota.lock.d" ]] || {
  echo "directory lock remained after no-flock failure" >&2
  exit 1
}

echo "OTA cleanup gate passed"
