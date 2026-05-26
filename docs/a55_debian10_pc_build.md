# A55 Debian 10 PC-Side Build

This is the recommended deployment flow for low-performance ARM A55 boards:
compile and package on the PC, then copy the finished runtime package to the
board. The board does not need Rust, Cargo, Qt build tools, Node.js, or source
code.

## Build On The PC

Windows PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-a55-debian10.ps1
```

WSL/Linux/macOS with Docker:

```bash
./scripts/build-a55-debian10.sh
```

The scripts build inside a Debian 10 Docker image and produce an ARM64 package
under `dist/`.

Latest package pointer:

```text
dist/latest-a55-debian10-package.txt
```

The package contains:

- `bin/reactor-edge-daemon`: ARM64 backend binary.
- `static/`: production HTML/CSS/JavaScript HMI.
- `config/`: device, safety, and AI memory config files.
- `kiosk/`: Chromium kiosk launcher.
- `deploy/`: systemd unit templates.
- `install.sh`: one-command board installer and boot autostart setup.
- `docs/`: JSON bridge and Chromium kiosk notes.

## Board Runtime Dependencies

Install only runtime packages on the board:

```bash
sudo apt-get update
sudo apt-get install -y ca-certificates libudev1 curl x11-xserver-utils
sudo apt-get install -y chromium || sudo apt-get install -y chromium-browser
```

Optional cursor hiding:

```bash
sudo apt-get install -y unclutter
```

If the Debian 10 mirror is unavailable, use the board vendor mirror or Debian
archive as described in `docs/chromium_kiosk.md`.

## Run On The Board

Copy the generated tarball to the board, then:

```bash
tar -xzf reactor-os-a55-arm64-debian10-chromium-kiosk-*.tar.gz
cd reactor-os-a55-arm64-debian10-chromium-kiosk-*
./run.sh ./config/device.json_bridge.toml
```

Start the local HMI:

```bash
./kiosk/run-chromium-kiosk.sh
```

## One-Command Autostart Install

For the production board, prefer installing the package as systemd services:

```bash
tar -xzf reactor-os-a55-arm64-debian10-chromium-kiosk-*.tar.gz
cd reactor-os-a55-arm64-debian10-chromium-kiosk-*
sudo ./install.sh
```

This enables boot autostart for:

- `reactor-edge`: backend, API, database, safety control loop, static HMI.
- `reactor-os-chromium`: Chromium kiosk opening `http://127.0.0.1:8000/`.

Install runtime apt dependencies at the same time:

```bash
sudo ./install.sh --install-deps
```

Backend only:

```bash
sudo ./install.sh --no-kiosk
```

Customer demo context:

```bash
sudo ./install.sh --seed-demo-context
```

Check status:

```bash
systemctl status reactor-edge
systemctl status reactor-os-chromium
curl http://127.0.0.1:8000/health
```

## Customer Demo Context

For customer presentations, demo context can seed process definitions, process
steps, historical batch outcomes, AI recommendations, and non-sensor demo
alarms:

```bash
REACTOR_OS_EXTRA_ARGS=--seed-demo-context ./run.sh ./config/device.json_bridge.toml
```

Production rule: demo context never fabricates runtime sensor values. It does
not write `sensor_samples` and does not set the live sample cache. Without a
real `state.json`, ESP32, or external pipeline sample, `/api/live` still returns
`503`.

## Validation

The package script validates the produced binary with `file` and `readelf`.
For Debian 10 compatibility, the maximum required glibc symbol version must be
`GLIBC_2.28` or lower.

Validation output is written into:

```text
BUILD-VALIDATION.txt
```
