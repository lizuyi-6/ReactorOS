#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

export REACTOR_EDGE_PREFIX="${TMP_ROOT}/prefix"
export REACTOR_EDGE_DATA_DIR="${TMP_ROOT}/data"
export REACTOR_EDGE_PROJECT_DIR="${TMP_ROOT}/project"
export REACTOR_EDGE_OTA_LOCK="${TMP_ROOT}/ota.lock"

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
}

make_transformed_tar() {
  local archive="$1"
  local src="$2"
  local transform="$3"
  if tar --version 2>/dev/null | grep -qi 'GNU tar'; then
    tar -czf "$archive" --transform "$transform" -C "$src" reactor-os-test-release
  else
    tar -czf "$archive" -s "$transform" -C "$src" reactor-os-test-release
  fi
}

expect_pass() {
  local archive="$1"
  if ! validate_tar_package "$archive" >/dev/null; then
    echo "expected tar package to pass: $archive" >&2
    exit 1
  fi
}

expect_fail() {
  local archive="$1"
  local expected="$2"
  local output rc
  set +e
  output="$(validate_tar_package "$archive" 2>&1)"
  rc=$?
  set -e
  if [[ "$rc" -eq 0 ]]; then
    echo "expected tar package to fail: $archive" >&2
    exit 1
  fi
  if [[ "$output" != *"$expected"* ]]; then
    echo "expected failure to contain '$expected', got:" >&2
    printf '%s\n' "$output" >&2
    exit 1
  fi
}

src="${TMP_ROOT}/src"
mkdir -p "$src"
make_release_tree "$src"

safe="${TMP_ROOT}/safe.tar.gz"
tar -czf "$safe" -C "$src" reactor-os-test-release
expect_pass "$safe"

multi="${TMP_ROOT}/multi.tar.gz"
mkdir -p "${src}/second-root"
printf 'extra\n' >"${src}/second-root/file.txt"
tar -czf "$multi" -C "$src" reactor-os-test-release second-root
expect_fail "$multi" "exactly one top-level directory"

traversal="${TMP_ROOT}/traversal.tar.gz"
make_transformed_tar "$traversal" "$src" 's|^reactor-os-test-release|../reactor-os-test-release|'
expect_fail "$traversal" "unsafe path"

link_src="${TMP_ROOT}/link-src"
mkdir -p "$link_src"
make_release_tree "$link_src"
ln -s /etc/passwd "${link_src}/reactor-os-test-release/static/escape-link"
link_archive="${TMP_ROOT}/link.tar.gz"
tar -czf "$link_archive" -C "$link_src" reactor-os-test-release
expect_fail "$link_archive" "unsupported tar member type"

echo "OTA tar safety gate passed"
