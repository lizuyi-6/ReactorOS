#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

export QTWEBENGINE_DISABLE_SANDBOX=1
export QT_QPA_PLATFORM="${QT_QPA_PLATFORM:-xcb}"

URL="${REACTOR_OS_URL:-http://127.0.0.1:8000/}"
BACKEND_CMD="${REACTOR_OS_BACKEND_CMD:-}"

args=(--url "$URL")

if [[ -n "${REACTOR_OS_WINDOWED:-}" ]]; then
  args+=(--windowed)
fi

if [[ -n "$BACKEND_CMD" ]]; then
  args+=(--backend "$BACKEND_CMD")
fi

exec ./build/reactor-os-qt "${args[@]}"
