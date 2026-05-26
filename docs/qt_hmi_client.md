# ReactorOS Qt HMI Client

The Qt client removes the need to manually open Chromium on the board. It is a native Qt kiosk shell that loads the existing ReactorOS web UI from the local backend:

```text
Qt HMI shell -> http://127.0.0.1:8000/ -> reactor-edge-daemon -> state.json/control.json
```

This keeps the current frontend/backend/data-pipeline behavior unchanged while giving the A55 board a direct desktop application entrypoint.

## Debian 10 Dependencies

On the board:

```bash
sudo apt-get update
sudo apt-get install -y \
  build-essential \
  pkg-config \
  qt5-qmake \
  qt5-qmake-bin \
  qtchooser \
  qtbase5-dev \
  qtwebengine5-dev \
  libqt5webenginewidgets5 \
  libqt5webenginecore5 \
  libqt5webchannel5
```

If your board image says Qt was built without WebEngine, install QtWebKit and force the fallback backend:

```bash
sudo apt-get install -y libqt5webkit5-dev
REACTOR_OS_QT_BACKEND=webkit ./scripts/build-debian10.sh
```

The build script defaults to `REACTOR_OS_QT_BACKEND=auto`: it uses WebEngine when available and falls back to WebKit when WebEngine is missing. Use `REACTOR_OS_QT_BACKEND=webengine` or `REACTOR_OS_QT_BACKEND=webkit` to force one path.

If the board has no desktop session, install and configure X11/LightDM or another window manager first. Qt WebEngine and QtWebKit both need a display server.

On archived Debian 10 images, the default mirror may be gone. Use your board vendor mirror when available, or temporarily point APT to `archive.debian.org`:

```bash
sudo tee /etc/apt/sources.list >/dev/null <<'EOF'
deb http://archive.debian.org/debian buster main contrib non-free
deb http://archive.debian.org/debian-security buster/updates main contrib non-free
deb http://archive.debian.org/debian buster-updates main contrib non-free
EOF
sudo apt-get -o Acquire::Check-Valid-Until=false update
```

## Build On Board

Copy the repository or just `qt-client/` onto the board, then run:

```bash
cd qt-client
chmod +x scripts/build-debian10.sh scripts/run-kiosk.sh
./scripts/build-debian10.sh
```

The script prefers `/usr/lib/qt5/bin/qmake`, which avoids `qtchooser` failures seen on some Debian 10 images. It also prints the selected web backend at the end of the build.

Run fullscreen kiosk:

```bash
./scripts/run-kiosk.sh
```

Run in a window for debugging:

```bash
REACTOR_OS_WINDOWED=1 ./scripts/run-kiosk.sh
```

Load a different URL:

```bash
REACTOR_OS_URL=http://192.168.1.20:8000/ ./scripts/run-kiosk.sh
```

Start the Rust backend from the Qt client:

```bash
REACTOR_OS_BACKEND_CMD='../bin/reactor-edge-daemon --config ../config/device.json_bridge.toml --safety ../config/safety.toml --memory ../config/ai_memory.toml --db ../data/reactor.sqlite3 --assets ../static --bind 127.0.0.1:8000' \
  ./scripts/run-kiosk.sh
```

For production, prefer running `reactor-edge.service` and `reactor-os-qt.service` separately under systemd.

## Systemd Kiosk

Install the Qt client to `/opt/reactor-edge/qt-client`, then:

```bash
sudo cp deploy/reactor-os-qt.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now reactor-os-qt
```

The unit expects the backend service to be available at `http://127.0.0.1:8000/`.

If your display user is not `pi`, edit `deploy/reactor-os-qt.service` before installing it and change `User=`, `Group=`, and `XAUTHORITY=`.

## Debian 10 Build Check

The source has been validated in a Debian 10 container with:

```bash
./scripts/build-debian10.sh
```

That check confirms the code builds against Debian 10 Qt 5 WebEngine and QtWebKit headers. The final A55 binary should still be compiled on the target board or inside an aarch64 Debian 10 sysroot that matches the board image.

## Common Build Errors

`Project ERROR: Unknown module(s) in QT: webenginewidgets` means the board Qt package does not include WebEngine. Use:

```bash
sudo apt-get install -y libqt5webkit5-dev
REACTOR_OS_QT_BACKEND=webkit ./scripts/build-debian10.sh
```

`Could not connect to any X display` means the program compiled but no graphical session is available. Start the desktop session, check `DISPLAY=:0`, or run it through the provided systemd unit after graphical login is available.

## Controls

- `F5`: reload HMI
- `F11`: toggle fullscreen
- `Ctrl+Q`: quit
