#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

export REACTOR_EDGE_PREFIX="${TMP_ROOT}/prefix"
export REACTOR_EDGE_DATA_DIR="${TMP_ROOT}/data"
export REACTOR_EDGE_PROJECT_DIR="${TMP_ROOT}/project"
export REACTOR_EDGE_OTA_LOCK="${TMP_ROOT}/ota.lock"
export REACTOR_EDGE_ALLOW_NON_ROOT_FOR_TESTS=1
export REACTOR_EDGE_SKIP_SYNC_FOR_TESTS=1

FAKE_BIN="${TMP_ROOT}/bin"
mkdir -p "$FAKE_BIN"

cat >"${FAKE_BIN}/systemctl" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF

cat >"${FAKE_BIN}/curl" <<'EOF'
#!/usr/bin/env bash
printf '{"data":{"devices":[]}}\n'
EOF

chmod +x "${FAKE_BIN}/systemctl" "${FAKE_BIN}/curl"

export PATH="${FAKE_BIN}:${PATH}"

PACKAGE="${TMP_ROOT}/reactor-os-test.tar.gz"
printf 'not a real release\n' >"$PACKAGE"

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

expect_fail "missing checksum confirmation" "--allow-missing-checksum requires --confirm-unsafe-no-checksum" \
  bash "${ROOT}/deploy/reactor-edge-ota-update.sh" "$PACKAGE" --allow-missing-checksum --dry-run

expect_fail "missing force confirmation on update" "--force requires --confirm-maintenance-window" \
  bash "${ROOT}/deploy/reactor-edge-ota-update.sh" "$PACKAGE" --force --dry-run

expect_fail "missing skip-backup confirmation" "--skip-backup requires --confirm-skip-backup" \
  bash "${ROOT}/deploy/reactor-edge-ota-update.sh" "$PACKAGE" --skip-backup --dry-run

expect_fail "missing force confirmation on rollback" "--force requires --confirm-maintenance-window" \
  bash "${ROOT}/deploy/reactor-edge-ota-rollback.sh" --force

expect_fail "confirmed unsafe checksum reaches package validation" "release package tar directory cannot be read" \
  bash "${ROOT}/deploy/reactor-edge-ota-update.sh" "$PACKAGE" \
    --allow-missing-checksum \
    --confirm-unsafe-no-checksum \
    --force \
    --confirm-maintenance-window \
    --skip-backup \
    --confirm-skip-backup \
    --dry-run

echo "OTA dangerous-options gate passed"
