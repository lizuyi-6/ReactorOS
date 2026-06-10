#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

export REACTOR_EDGE_PREFIX="${TMP_ROOT}/prefix"
export REACTOR_EDGE_DATA_DIR="${TMP_ROOT}/data"
export REACTOR_EDGE_PROJECT_DIR="${TMP_ROOT}/project"
export REACTOR_EDGE_OTA_LOCK="${TMP_ROOT}/ota.lock"
export REACTOR_EDGE_OTA_SERVICE_START_ALLOWED="${TMP_ROOT}/run/ota-service-start-allowed"
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
exit 0
EOF
chmod +x "${FAKE_BIN}/systemctl"
export PATH="${FAKE_BIN}:${PATH}"

# shellcheck source=../deploy/reactor-edge-ota-lib.sh
source "${ROOT}/deploy/reactor-edge-ota-lib.sh"
ensure_runtime_dirs

mark_ota_service_start_allowed
grep -Fq 'ota_pid=' "$OTA_SERVICE_START_ALLOWED" || {
  echo "service-start marker did not record owner PID" >&2
  cat "$OTA_SERVICE_START_ALLOWED" >&2
  exit 1
}
grep -Fq 'ota_pid_start_ticks=' "$OTA_SERVICE_START_ALLOWED" || {
  echo "service-start marker did not record owner process start ticks" >&2
  cat "$OTA_SERVICE_START_ALLOWED" >&2
  exit 1
}
enter_ota_failed_state "b" "rollback health check failed" "/tmp/release.tar.gz" "old" "new" "oldsha" "newsha"

[[ ! -e "$OTA_SERVICE_START_ALLOWED" ]] || {
  echo "failed state did not clear active OTA service-start marker" >&2
  exit 1
}

grep -Fq '"status": "failed"' "$STATE_FILE" || {
  echo "failed state was not written" >&2
  cat "$STATE_FILE" >&2
  exit 1
}
grep -Fq '"reason": "rollback health check failed"' "$STATE_FILE" || {
  echo "failed reason was not recorded" >&2
  cat "$STATE_FILE" >&2
  exit 1
}
grep -Fq '"from_version": "old"' "$STATE_FILE" || {
  echo "failed state lost from_version traceability" >&2
  cat "$STATE_FILE" >&2
  exit 1
}
grep -Fq '"to_version": "new"' "$STATE_FILE" || {
  echo "failed state lost to_version traceability" >&2
  cat "$STATE_FILE" >&2
  exit 1
}
grep -Fq "stop reactor-os-chromium-test" "$SYSTEMCTL_LOG" || {
  echo "failed state did not stop kiosk service" >&2
  cat "$SYSTEMCTL_LOG" >&2
  exit 1
}
grep -Fq "stop reactor-edge-test" "$SYSTEMCTL_LOG" || {
  echo "failed state did not stop backend service" >&2
  cat "$SYSTEMCTL_LOG" >&2
  exit 1
}

echo "OTA failed-state gate passed"
