# LubanCat 2 Debian 10 Deployment

This is the preferred deployment profile for the LubanCat 2 RK3568 board.
The board is ARM64 and uses Cortex-A55 cores, so the existing Debian 10 ARM64
cross-compile path is still valid. The LubanCat-specific package mainly changes
packaging metadata and systemd display user defaults.

## Board Profile And Low-Load Budget

LubanCat 2 uses Rockchip RK3568: quad-core ARM Cortex-A55 up to 2GHz, Mali-G52
GPU, HDMI/MIPI display output, USB, dual gigabit ethernet, 40-pin expansion,
and memory variants from small 1GB/2GB boards to larger 4GB/8GB boards. Treat
1GB/2GB as the production baseline unless the exact customer board is known.

Recommended runtime posture:

- Build on the PC and copy the generated ARM64 package to the board; do not run
  Rust/npm build toolchains on the LubanCat 2.
- Keep one foreground app: Chromium kiosk loading `http://127.0.0.1:8000/`.
- Use the bundled low-load kiosk defaults. Disable them only for debugging with
  `REACTOR_OS_LOW_LOAD=0`.
- Keep sensor history windows short on the HMI. Production live values still
  come only from `state.json`/pipeline; lower history depth only reduces browser
  memory and canvas work.
- Prefer `schedutil` or `ondemand` CPU governor for normal operation. Use
  `performance` only for customer demos where frame pacing matters more than
  thermals.
- Avoid Docker on the board for production kiosk deployment. The tarball +
  systemd path uses less disk and RAM.
- Keep Chromium GPU enabled by default. If a vendor image has broken GPU
  acceleration, set `REACTOR_OS_DISABLE_GPU=1` in the kiosk service override.

## Build On The PC

Windows PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-lubancat2-debian10.ps1
```

WSL/Linux/macOS with Docker:

```bash
./scripts/build-lubancat2-debian10.sh
```

Latest package pointer:

```text
dist/latest-lubancat2-debian10-package.txt
```

The generated package name starts with:

```text
reactor-os-lubancat2-rk3568-debian10-chromium-kiosk-
```

## PC-Side QEMU Emulation

Before copying the package to the board, you can run the ARM64 package through
QEMU user-mode on the PC:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\run-lubancat2-qemu.ps1 --smoke
```

This validates the ARM64 binary, Debian 10 runtime baseline, HTTP API, JSON
bridge state/control files, device discovery, and component control path. It is
not full RK3568 board emulation. See `docs/lubancat2_qemu_emulation.md`.

## Board Runtime Dependencies

Install only runtime packages on the LubanCat 2:

```bash
sudo apt-get update
sudo apt-get install -y ca-certificates libudev1 curl x11-xserver-utils
sudo apt-get install -y chromium || sudo apt-get install -y chromium-browser
```

Optional cursor hiding:

```bash
sudo apt-get install -y unclutter
```

## Manual Run

Copy the generated tarball to the board:

```bash
scp dist/reactor-os-lubancat2-rk3568-debian10-chromium-kiosk-*.tar.gz cat@BOARD_IP:/home/cat/
```

Run it on the board:

```bash
tar -xzf reactor-os-lubancat2-rk3568-debian10-chromium-kiosk-*.tar.gz
cd reactor-os-lubancat2-rk3568-debian10-chromium-kiosk-*
./run.sh ./config/device.json_bridge.toml
```

Open the HMI:

```text
http://127.0.0.1:8000/
```

Chromium kiosk:

```bash
./kiosk/run-chromium-kiosk.sh
```

## One-Command Autostart Install

The generated package includes `install.sh`. On the LubanCat 2, extract the
package and run:

```bash
tar -xzf reactor-os-lubancat2-rk3568-debian10-chromium-kiosk-*.tar.gz
cd reactor-os-lubancat2-rk3568-debian10-chromium-kiosk-*
sudo ./install.sh
```

This installs:

- `/opt/reactor-edge/bin/reactor-edge-daemon`
- `/opt/reactor-edge/bin/reactor-safety-guard`
- `/opt/reactor-edge/bin/xingshu`
- `/opt/reactor-edge/backup.sh`
- `/opt/reactor-edge/frontend`
- `/opt/reactor-edge/static`
- `/opt/reactor-edge/kiosk`
- `/var/lib/reactor-edge/backups`
- `/etc/reactor-edge/*.toml`
- `/etc/systemd/system/reactor-edge.service`
- `/etc/systemd/system/reactor-edge-backup.service`
- `/etc/systemd/system/reactor-edge-backup.timer`
- `/etc/systemd/system/reactor-os-chromium.service`

It also creates `/project` for `state.json/control.json`, enables the backend
service, the daily backup timer, and the kiosk service, and starts them
immediately. The backend service launches `reactor-safety-guard` through
`--safety-guard` by default. The backup timer calls `/opt/reactor-edge/backup.sh`
to generate SQLite online snapshots in `/var/lib/reactor-edge/backups`.

Install board runtime dependencies at the same time:

```bash
sudo ./install.sh --install-deps
```

Backend only, no screen kiosk:

```bash
sudo ./install.sh --no-kiosk
```

Customer demo context:

```bash
sudo ./install.sh --seed-demo-context
```

Status and logs:

```bash
systemctl status reactor-edge
systemctl status reactor-edge-backup.timer
systemctl list-timers reactor-edge-backup.timer
systemctl status reactor-os-chromium
journalctl -u reactor-edge -f
journalctl -u reactor-edge-backup.service --no-pager -n 50
journalctl -u reactor-os-chromium -f
```

Low-load health check:

```bash
sudo /opt/reactor-edge/health-check.sh
```

The health check reports uptime, load, CPU governor/frequency, memory, thermal
zones, disk usage, systemd service state, backend `/health`, and JSON bridge
file freshness.

## Customer Demo Context

Demo context seeds processes, historical outcomes, AI recommendations, and
non-sensor demo alarms:

```bash
REACTOR_OS_EXTRA_ARGS=--seed-demo-context ./run.sh ./config/device.json_bridge.toml
```

Production sensor rule remains unchanged: demo context does not write
`sensor_samples` and does not set `runtime.latest_sample`. Without a real
`state.json`, ESP32 frame, or external pipeline sample, `/api/live` returns 503.

## JSON Bridge Paths

Default JSON bridge paths:

```text
/project/state.json
/project/control.json
```

Create the directory if the downstream bridge runs as a normal user:

```bash
sudo mkdir -p /project
sudo chown cat:cat /project
```

The downstream bridge writes real device status into `state.json`; ReactorOS
writes control requests to `control.json`.

## Systemd Install

Prefer `sudo ./install.sh` from the package. Manual install remains available
for custom images:

The LubanCat 2 package generates unit files for default display user `cat`:

```bash
sudo mkdir -p /opt/reactor-edge /etc/reactor-edge /var/lib/reactor-edge/backups /project
sudo chown cat:cat /project

sudo cp -r bin static frontend kiosk /opt/reactor-edge/
sudo cp backup.sh /opt/reactor-edge/
sudo cp config/*.toml /etc/reactor-edge/
sudo cp deploy/reactor-edge.service deploy/reactor-edge-backup.service deploy/reactor-edge-backup.timer deploy/reactor-os-chromium.service /etc/systemd/system/

sudo systemctl daemon-reload
sudo systemctl enable --now reactor-edge
sudo systemctl enable --now reactor-edge-backup.timer
sudo systemctl enable --now reactor-os-chromium
```

If your image uses a different desktop user, rebuild with:

```bash
SERVICE_USER=your_user SERVICE_GROUP=your_group SERVICE_HOME=/home/your_user ./scripts/build-lubancat2-debian10.sh
```

or edit these fields before installing the units:

- `User=`
- `Group=`
- `Environment=XAUTHORITY=`

## Validation

```bash
curl http://127.0.0.1:8000/health
curl http://127.0.0.1:8000/api/devices/status
```

If `/api/live` returns 503, first check whether the downstream bridge is writing
a fresh `/project/state.json` with all required sensor fields.
