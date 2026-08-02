#!/usr/bin/env bash
set -euo pipefail

URL="${REACTOR_OS_URL:-http://127.0.0.1:8000/}"
HEALTH_URL="${REACTOR_OS_HEALTH_URL:-http://127.0.0.1:8000/health}"
WAIT_SECONDS="${REACTOR_OS_WAIT_SECONDS:-60}"
USER_DATA_DIR="${REACTOR_OS_CHROMIUM_USER_DATA_DIR:-${XDG_RUNTIME_DIR:-/tmp}/reactor-os-chromium}"
CACHE_DIR="${REACTOR_OS_CHROMIUM_CACHE_DIR:-${XDG_RUNTIME_DIR:-/tmp}/reactor-os-chromium-cache}"
LOW_LOAD="${REACTOR_OS_LOW_LOAD:-1}"

export DISPLAY="${DISPLAY:-:0}"

# Reuse the display user's real session bus when it exists. Vendor images often
# leave a stale/unsupported DBUS_SESSION_BUS_ADDRESS in the system service
# environment, which makes Chromium log an error on every probe. Do not invent
# a bus address when the socket is absent; Chromium remains functional without
# session D-Bus in kiosk mode.
runtime_dir="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
if [[ -S "${runtime_dir}/bus" ]]; then
  export DBUS_SESSION_BUS_ADDRESS="unix:path=${runtime_dir}/bus"
else
  unset DBUS_SESSION_BUS_ADDRESS || true
fi

find_chromium() {
  if [[ -n "${CHROMIUM_BIN:-}" ]]; then
    printf '%s\n' "$CHROMIUM_BIN"
    return 0
  fi

  for candidate in chromium chromium-browser google-chrome google-chrome-stable; do
    if command -v "$candidate" >/dev/null 2>&1; then
      command -v "$candidate"
      return 0
    fi
  done

  return 1
}

wait_for_backend() {
  if [[ "$WAIT_SECONDS" == "0" ]]; then
    return 0
  fi

  if command -v curl >/dev/null 2>&1; then
    for ((i = 0; i < WAIT_SECONDS; i++)); do
      if curl -fsS "$HEALTH_URL" >/dev/null 2>&1; then
        return 0
      fi
      sleep 1
    done
    return 1
  fi

  if command -v wget >/dev/null 2>&1; then
    for ((i = 0; i < WAIT_SECONDS; i++)); do
      if wget -q -O /dev/null "$HEALTH_URL" >/dev/null 2>&1; then
        return 0
      fi
      sleep 1
    done
    return 1
  fi

  echo "curl/wget not found; skipping backend health wait." >&2
  return 0
}

chromium_bin="$(find_chromium)" || {
  echo "Chromium browser not found. Install chromium or chromium-browser." >&2
  exit 1
}

mkdir -p "$USER_DATA_DIR"
mkdir -p "$CACHE_DIR"

if ! wait_for_backend; then
  echo "ReactorOS backend did not become healthy at $HEALTH_URL within ${WAIT_SECONDS}s." >&2
  exit 1
fi

if command -v xset >/dev/null 2>&1; then
  xset s off -dpms s noblank >/dev/null 2>&1 || true
fi

if command -v unclutter >/dev/null 2>&1; then
  unclutter -idle 0.5 -root >/dev/null 2>&1 &
fi

flags=(
  --no-first-run
  --no-default-browser-check
  --disable-infobars
  --disable-session-crashed-bubble
  --disable-translate
  --disable-pinch
  --overscroll-history-navigation=0
  --autoplay-policy=no-user-gesture-required
  --disable-dev-shm-usage
  --disk-cache-dir="$CACHE_DIR"
  --user-data-dir="$USER_DATA_DIR"
)

if [[ "$LOW_LOAD" != "0" ]]; then
  flags+=(
    --enable-low-end-device-mode
    --renderer-process-limit=2
    --process-per-site
    --disk-cache-size=16777216
    --media-cache-size=1048576
    --disable-background-networking
    --disable-sync
    --disable-component-update
    --disable-domain-reliability
    --disable-extensions
    --disable-breakpad
    --disable-hang-monitor
    --disable-notifications
    --disable-print-preview
    --disable-speech-api
    --metrics-recording-only
    --safebrowsing-disable-auto-update
    --disable-features=TranslateUI,MediaRouter,OptimizationHints,AutofillServerCommunication,InterestFeedContentSuggestions
  )
else
  flags+=(--disable-features=TranslateUI)
fi

if [[ "${REACTOR_OS_DISABLE_GPU:-0}" == "1" ]]; then
  flags+=(--disable-gpu)
fi

if [[ "${EUID:-$(id -u)}" == "0" ]]; then
  flags+=(--no-sandbox)
fi

if [[ -n "${REACTOR_OS_WINDOWED:-}" ]]; then
  flags+=(--new-window "$URL")
else
  flags+=(--kiosk "$URL")
fi

if [[ -n "${REACTOR_OS_EXTRA_CHROMIUM_FLAGS:-}" ]]; then
  read -r -a extra_flags <<< "$REACTOR_OS_EXTRA_CHROMIUM_FLAGS"
  flags+=("${extra_flags[@]}")
fi

exec "$chromium_bin" "${flags[@]}"
