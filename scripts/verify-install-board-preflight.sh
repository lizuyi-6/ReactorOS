#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

FAKE_BIN="${TMP_ROOT}/fake-bin"
SYSTEMCTL_LOG="${TMP_ROOT}/systemctl.log"
INSTALL_ROOT="${TMP_ROOT}/install-root"
mkdir -p "$FAKE_BIN" "$INSTALL_ROOT"
cat >"${FAKE_BIN}/systemctl" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >>"${SYSTEMCTL_LOG}"
exit 0
EOF
chmod +x "${FAKE_BIN}/systemctl"
export PATH="${FAKE_BIN}:${PATH}"

grep -Fq 'ExecStartPre=+/opt/reactor-edge/ota-boot-check.sh' "${ROOT}/deploy/reactor-edge.service" || {
  echo "backend service boot-check is not elevated independently from the daemon" >&2
  exit 1
}
grep -Fq 'EnvironmentFile=-/etc/reactor-edge/reactor-edge.auth.env' "${ROOT}/deploy/reactor-edge.service" || {
  echo "backend service does not read the generated authentication secret" >&2
  exit 1
}
grep -Fq 'After=display-manager.service reactor-edge-ota-boot-check.service reactor-edge.service' "${ROOT}/deploy/reactor-os-chromium.service" || {
  echo "kiosk service does not wait for the display manager" >&2
  exit 1
}
if grep -Eq '^(After|Wants)=.*graphical\.target' "${ROOT}/deploy/reactor-os-chromium.service"; then
  echo "kiosk service still pulls the graphical/time-sync transaction" >&2
  exit 1
fi
grep -Fq 'Environment=XINGSHU_SEED_DEMO_CONTEXT=true' "${ROOT}/deploy/install-board.sh" || {
  echo "installer demo drop-in does not use the daemon-supported environment flag" >&2
  exit 1
}
if grep -Fq 'Environment=REACTOR_OS_EXTRA_ARGS=--seed-demo-context' "${ROOT}/deploy/install-board.sh"; then
  echo "installer still writes the ignored REACTOR_OS_EXTRA_ARGS demo flag" >&2
  exit 1
fi

make_package() {
  local dir="$1"
  rm -rf "$dir"
  mkdir -p \
    "${dir}/bin" \
    "${dir}/config" \
    "${dir}/deploy" \
    "${dir}/frontend/dist" \
    "${dir}/kiosk" \
    "${dir}/static"
  for bin in reactor-edge-daemon reactor-safety-guard xingshu; do
    printf '#!/usr/bin/env bash\nexit 0\n' >"${dir}/bin/${bin}"
    chmod +x "${dir}/bin/${bin}"
  done
  for script in backup.sh health-check.sh ota-update.sh ota-rollback.sh ota-lib.sh ota-boot-check.sh; do
    printf '#!/usr/bin/env bash\nexit 0\n' >"${dir}/${script}"
    chmod +x "${dir}/${script}"
  done
  printf '#!/usr/bin/env bash\nexit 0\n' >"${dir}/kiosk/run-chromium-kiosk.sh"
  chmod +x "${dir}/kiosk/run-chromium-kiosk.sh"
  printf 'REACTOR_EDGE_BUILD_SCHEMA=reactor-edge.build.v1\nREACTOR_EDGE_PACKAGE_NAME=test\nREACTOR_EDGE_GIT_SHA=test\nREACTOR_EDGE_BUILT_AT_UTC=2026-06-09T00:00:00Z\n' >"${dir}/BUILD-METADATA.properties"
  printf 'REACTOR_EDGE_BIND=127.0.0.1:8000\n' >"${dir}/config/reactor-edge.env"
  printf '[device]\nmode = "pipeline"\n' >"${dir}/config/device.toml"
  printf '[limits]\n' >"${dir}/config/safety.toml"
  printf '[optimizer]\n' >"${dir}/config/ai_memory.toml"
  printf '[mqtt]\nenabled = false\n[modbus_tcp]\nenabled = false\n' >"${dir}/config/integration.toml"
  printf '<!doctype html>\n' >"${dir}/frontend/dist/index.html"
  printf '<!doctype html>\n' >"${dir}/static/index.html"
  for unit in reactor-edge-ota-boot-check.service reactor-edge-backup.service reactor-edge-backup.timer reactor-os-chromium.service; do
    printf '[Unit]\nDescription=%s\n' "$unit" >"${dir}/deploy/${unit}"
  done
  cat >"${dir}/deploy/reactor-edge.service" <<'EOF'
[Unit]
Description=Reactor Edge

[Service]
User=pi
Group=pi
ExecStartPre=+/opt/reactor-edge/ota-boot-check.sh
ExecStart=/opt/reactor-edge/current/bin/reactor-edge-daemon
EnvironmentFile=-/etc/reactor-edge/reactor-edge.env
EnvironmentFile=-/etc/reactor-edge/reactor-edge.auth.env
EOF
  cp "${ROOT}/deploy/install-board.sh" "${dir}/install.sh"
  chmod +x "${dir}/install.sh"
}

bad_package="${TMP_ROOT}/bad-package"
make_package "$bad_package"
rm -f "${bad_package}/ota-update.sh"

set +e
(
  cd "$bad_package"
  REACTOR_EDGE_INSTALL_ROOT="$INSTALL_ROOT" ./install.sh --no-start
) >"${TMP_ROOT}/bad-install.log" 2>&1
bad_rc=$?
set -e
if [[ "$bad_rc" -eq 0 ]]; then
  echo "install unexpectedly passed with missing ota-update.sh" >&2
  exit 1
fi
if ! grep -Fq "Missing required executable package file" "${TMP_ROOT}/bad-install.log"; then
  echo "missing package failure did not report required executable:" >&2
  cat "${TMP_ROOT}/bad-install.log" >&2
  exit 1
fi
if [[ -s "$SYSTEMCTL_LOG" ]]; then
  echo "installer called systemctl before package preflight failed:" >&2
  cat "$SYSTEMCTL_LOG" >&2
  exit 1
fi

good_package="${TMP_ROOT}/good-package"
make_package "$good_package"
: >"$SYSTEMCTL_LOG"
mkdir -p "${INSTALL_ROOT}/etc/reactor-edge"
printf 'STEPFUN_AI_ENABLED=true\nSTEPFUN_API_KEY=existing-production-key\n' \
  >"${INSTALL_ROOT}/etc/reactor-edge/reactor-edge.env"
existing_env_sha="$(sha256sum "${INSTALL_ROOT}/etc/reactor-edge/reactor-edge.env" | awk '{print $1}')"
(
  cd "$good_package"
  REACTOR_EDGE_INSTALL_ROOT="$INSTALL_ROOT" ./install.sh --no-start --seed-demo-context
) >"${TMP_ROOT}/good-install.log" 2>&1

if [[ ! -L "${INSTALL_ROOT}/opt/reactor-edge/current" ]]; then
  echo "installer did not create current slot link under install root" >&2
  cat "${TMP_ROOT}/good-install.log" >&2
  exit 1
fi
if [[ ! -x "${INSTALL_ROOT}/opt/reactor-edge/ota-update.sh" ]]; then
  echo "installer did not install root ota-update.sh" >&2
  exit 1
fi
if [[ ! -f "${INSTALL_ROOT}/etc/systemd/system/reactor-edge.service" ]]; then
  echo "installer did not install backend systemd unit under install root" >&2
  exit 1
fi
installed_env_sha="$(sha256sum "${INSTALL_ROOT}/etc/reactor-edge/reactor-edge.env" | awk '{print $1}')"
if [[ "$installed_env_sha" != "$existing_env_sha" ]]; then
  echo "installer overwrote the existing production environment file" >&2
  exit 1
fi
if ! grep -Fq 'STEPFUN_API_KEY=existing-production-key' "${INSTALL_ROOT}/etc/reactor-edge/reactor-edge.env"; then
  echo "installer did not preserve the existing StepFun API key" >&2
  exit 1
fi
auth_env="${INSTALL_ROOT}/etc/reactor-edge/reactor-edge.auth.env"
if [[ ! -f "$auth_env" ]]; then
  echo "installer did not generate the authentication environment file" >&2
  exit 1
fi
generated_secret="$(awk -F= '$1 == "XINGSHU_AUTH_SECRET" { print $2 }' "$auth_env")"
if [[ ! "$generated_secret" =~ ^[0-9a-f]{64}$ ]]; then
  echo "installer generated an invalid authentication secret" >&2
  exit 1
fi
if [[ "$(stat -c '%a' "$auth_env")" != "600" ]]; then
  echo "generated authentication environment file is not mode 0600" >&2
  exit 1
fi
auth_env_sha="$(sha256sum "$auth_env" | awk '{print $1}')"
if [[ "$(cat "${INSTALL_ROOT}/etc/reactor-edge/device.toml")" != $'[device]\nmode = "pipeline"' ]]; then
  echo "installer did not continue updating non-secret TOML configuration" >&2
  exit 1
fi
demo_dropin="${INSTALL_ROOT}/etc/systemd/system/reactor-edge.service.d/10-demo-context.conf"
if ! grep -Fq 'Environment=XINGSHU_SEED_DEMO_CONTEXT=true' "$demo_dropin"; then
  echo "installer did not persist the supported demo seed environment flag" >&2
  exit 1
fi
if ! grep -Fq "daemon-reload" "$SYSTEMCTL_LOG"; then
  echo "installer did not reload systemd in good package path" >&2
  cat "$SYSTEMCTL_LOG" >&2
  exit 1
fi
if grep -Eq 'restart reactor-edge|restart reactor-os-chromium' "$SYSTEMCTL_LOG"; then
  echo "installer restarted runtime services despite --no-start:" >&2
  cat "$SYSTEMCTL_LOG" >&2
  exit 1
fi

: >"$SYSTEMCTL_LOG"
(
  cd "$good_package"
  REACTOR_EDGE_INSTALL_ROOT="$INSTALL_ROOT" ./install.sh --no-start --seed-demo-context
) >"${TMP_ROOT}/repeat-install.log" 2>&1
if [[ "$(sha256sum "$auth_env" | awk '{print $1}')" != "$auth_env_sha" ]]; then
  echo "repeat install rotated an existing strong authentication secret" >&2
  exit 1
fi

echo "install-board preflight gate passed"
