#!/usr/bin/env bash
set -euo pipefail

PACKAGE="${1:-}"
if [[ -z "$PACKAGE" || ! -f "$PACKAGE" ]]; then
  echo "usage: $0 <reactor-os-lubancat2-*.tar.gz>" >&2
  exit 2
fi

TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT
tar -xzf "$PACKAGE" -C "$TMP_ROOT"
PACKAGE_DIR="$(find "$TMP_ROOT" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
[[ -n "$PACKAGE_DIR" ]]

FAKE_BIN="${TMP_ROOT}/fake-bin"
INSTALL_ROOT="${TMP_ROOT}/install-root"
SYSTEMCTL_LOG="${TMP_ROOT}/systemctl.log"
mkdir -p "$FAKE_BIN" "${INSTALL_ROOT}/etc/reactor-edge"
cat >"${FAKE_BIN}/systemctl" <<EOF
#!/usr/bin/env bash
printf '%s\n' "\$*" >>"${SYSTEMCTL_LOG}"
exit 0
EOF
chmod +x "${FAKE_BIN}/systemctl"
export PATH="${FAKE_BIN}:${PATH}"

printf 'STEPFUN_AI_ENABLED=true\nSTEPFUN_API_KEY=existing-production-key\n' \
  >"${INSTALL_ROOT}/etc/reactor-edge/reactor-edge.env"
provider_env_sha="$(sha256sum "${INSTALL_ROOT}/etc/reactor-edge/reactor-edge.env" | awk '{print $1}')"

(
  cd "$PACKAGE_DIR"
  REACTOR_EDGE_INSTALL_ROOT="$INSTALL_ROOT" ./install.sh --no-start --seed-demo-context
)

installed_service="${INSTALL_ROOT}/etc/systemd/system/reactor-edge.service"
installed_kiosk="${INSTALL_ROOT}/etc/systemd/system/reactor-os-chromium.service"
auth_env="${INSTALL_ROOT}/etc/reactor-edge/reactor-edge.auth.env"
demo_dropin="${INSTALL_ROOT}/etc/systemd/system/reactor-edge.service.d/10-demo-context.conf"

grep -Fq 'User=cat' "$installed_service"
grep -Fq 'ExecStartPre=+/opt/reactor-edge/ota-boot-check.sh' "$installed_service"
grep -Fq 'EnvironmentFile=-/etc/reactor-edge/reactor-edge.auth.env' "$installed_service"
grep -Fq 'After=display-manager.service reactor-edge-ota-boot-check.service reactor-edge.service' "$installed_kiosk"
if grep -Eq '^(After|Wants)=.*graphical\.target' "$installed_kiosk"; then
  echo "packaged kiosk still blocks on graphical.target" >&2
  exit 1
fi
grep -Fq 'Environment=XINGSHU_SEED_DEMO_CONTEXT=true' "$demo_dropin"

if [[ "$(sha256sum "${INSTALL_ROOT}/etc/reactor-edge/reactor-edge.env" | awk '{print $1}')" != "$provider_env_sha" ]]; then
  echo "packaged installer replaced the provider environment file" >&2
  exit 1
fi
secret="$(awk -F= '$1 == "XINGSHU_AUTH_SECRET" {print $2}' "$auth_env")"
[[ "$secret" =~ ^[0-9a-f]{64}$ ]]
[[ "$(stat -c '%a' "$auth_env")" == "600" ]]
auth_sha="$(sha256sum "$auth_env" | awk '{print $1}')"

(
  cd "$PACKAGE_DIR"
  REACTOR_EDGE_INSTALL_ROOT="$INSTALL_ROOT" ./install.sh --no-start --seed-demo-context
)
if [[ "$(sha256sum "$auth_env" | awk '{print $1}')" != "$auth_sha" ]]; then
  echo "packaged repeat install rotated the authentication secret" >&2
  exit 1
fi

echo "packaged LubanCat 2 deployment fixes gate passed"
