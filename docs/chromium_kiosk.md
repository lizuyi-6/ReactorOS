# ReactorOS Chromium Kiosk

This is the recommended HMI launcher when the board Qt stack is too old to render the ReactorOS frontend correctly.

The browser engine name is Chromium. More precisely, Chromium uses the Blink rendering engine and V8 JavaScript engine. Qt WebEngine also embeds Chromium, but old board images often ship Qt builds that are too old or compiled without WebEngine support. Running system Chromium directly is usually more stable on Debian 10 boards.

```text
Chromium kiosk -> http://127.0.0.1:8000/ -> reactor-edge-daemon -> state.json/control.json
```

The kiosk launcher does not change backend behavior. Production data still comes only from the configured pipeline or JSON bridge. Missing or stale data still returns API error codes.

## Install On Debian 10

Install Chromium and basic X11 helpers:

```bash
sudo apt-get update
sudo apt-get install -y chromium x11-xserver-utils curl
```

If your vendor image uses the Ubuntu/Raspberry Pi package name:

```bash
sudo apt-get install -y chromium-browser x11-xserver-utils curl
```

Optional cursor hiding:

```bash
sudo apt-get install -y unclutter
```

If the Debian 10 mirror is unavailable, use the board vendor mirror or Debian archive:

```bash
sudo tee /etc/apt/sources.list >/dev/null <<'EOF'
deb http://archive.debian.org/debian buster main contrib non-free
deb http://archive.debian.org/debian-security buster/updates main contrib non-free
deb http://archive.debian.org/debian buster-updates main contrib non-free
EOF
sudo apt-get -o Acquire::Check-Valid-Until=false update
```

## Manual Run

Start the backend first:

```bash
./run.sh ./config/device.json_bridge.toml
```

In the graphical desktop session:

```bash
chmod +x kiosk/run-chromium-kiosk.sh
./kiosk/run-chromium-kiosk.sh
```

Debug in a normal browser window:

```bash
REACTOR_OS_WINDOWED=1 ./kiosk/run-chromium-kiosk.sh
```

Use a custom browser path:

```bash
CHROMIUM_BIN=/usr/bin/chromium ./kiosk/run-chromium-kiosk.sh
```

## Systemd Kiosk

Install to `/opt/reactor-edge`:

```bash
sudo mkdir -p /opt/reactor-edge /etc/reactor-edge /var/lib/reactor-edge
sudo cp -r bin static kiosk /opt/reactor-edge/
sudo cp config/*.toml /etc/reactor-edge/
sudo cp deploy/reactor-edge.service deploy/reactor-os-chromium.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now reactor-edge
sudo systemctl enable --now reactor-os-chromium
```

If the display user is not `pi`, edit `deploy/reactor-os-chromium.service` before installing it and change `User=`, `Group=`, and `XAUTHORITY=`.

## Environment Variables

- `REACTOR_OS_URL`: HMI URL, default `http://127.0.0.1:8000/`.
- `REACTOR_OS_HEALTH_URL`: backend readiness check URL, default `http://127.0.0.1:8000/health`.
- `REACTOR_OS_WAIT_SECONDS`: startup wait timeout, default `60`; set `0` to skip.
- `REACTOR_OS_LOW_LOAD`: default `1`; disables background Chromium networking,
  sync, extension, component update, translate/media features, and crash upload
  work that is unnecessary for a closed industrial kiosk.
- `REACTOR_OS_DISABLE_GPU`: set `1` only when the board image has broken GPU
  acceleration. Keep unset on RK3568/LubanCat 2 unless rendering is unstable.
- `CHROMIUM_BIN`: explicit browser binary path.
- `REACTOR_OS_WINDOWED`: set to any non-empty value to run in a normal window.
- `REACTOR_OS_EXTRA_CHROMIUM_FLAGS`: extra Chromium flags separated by spaces.

## Common Errors

`Chromium browser not found`: install `chromium` or `chromium-browser`, or set `CHROMIUM_BIN`.

`Could not open display` or a black screen: the desktop/X11 session is not ready. Check `DISPLAY=:0`, LightDM, and the service user.

Browser opens before backend is ready: keep `REACTOR_OS_WAIT_SECONDS` at the default or increase it.
