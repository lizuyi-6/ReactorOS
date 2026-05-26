#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

if [[ -n "${QMAKE:-}" ]]; then
  qmake_bin="$QMAKE"
elif [[ -x /usr/lib/qt5/bin/qmake ]]; then
  qmake_bin=/usr/lib/qt5/bin/qmake
elif command -v qmake >/dev/null 2>&1; then
  qmake_bin=qmake
else
  echo "qmake not found. Install Qt first:" >&2
  echo "sudo apt-get install -y build-essential qt5-qmake qt5-qmake-bin qtbase5-dev qtwebengine5-dev libqt5webenginewidgets5" >&2
  exit 1
fi

backend="${REACTOR_OS_QT_BACKEND:-auto}"
if [[ "$backend" == "auto" ]]; then
  if pkg-config --exists Qt5WebEngineWidgets 2>/dev/null || [[ -d /usr/include/aarch64-linux-gnu/qt5/QtWebEngineWidgets ]] || [[ -d /usr/include/x86_64-linux-gnu/qt5/QtWebEngineWidgets ]]; then
    backend=webengine
  elif pkg-config --exists Qt5WebKitWidgets 2>/dev/null || [[ -d /usr/include/aarch64-linux-gnu/qt5/QtWebKitWidgets ]] || [[ -d /usr/include/x86_64-linux-gnu/qt5/QtWebKitWidgets ]]; then
    backend=webkit
  else
    echo "No Qt web view module found." >&2
    echo "Install one of:" >&2
    echo "  sudo apt-get install -y qtwebengine5-dev libqt5webenginewidgets5" >&2
    echo "  sudo apt-get install -y libqt5webkit5-dev" >&2
    exit 1
  fi
fi

mkdir -p build
cd build
REACTOR_OS_QT_BACKEND="$backend" "$qmake_bin" ../reactor-os-qt.pro
make -j"$(nproc)"

echo "Built: $(pwd)/reactor-os-qt"
echo "Qt web backend: $backend"
