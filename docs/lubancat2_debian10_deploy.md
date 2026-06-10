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

The installer validates the extracted package before stopping existing
services. Missing binaries, OTA scripts, backup/health helpers, systemd units,
configuration files, build metadata, or HMI assets fail immediately, so a
damaged package does not take a running field unit offline before the problem is
reported.

This installs:

- `/opt/reactor-edge/slots/a` as the initial application slot
- `/opt/reactor-edge/current` pointing at the active slot
- `/opt/reactor-edge/previous` reserved for rollback
- compatibility links such as `/opt/reactor-edge/bin`, `/opt/reactor-edge/frontend`,
  `/opt/reactor-edge/static`, `/opt/reactor-edge/kiosk`, `/opt/reactor-edge/backup.sh`,
  and `/opt/reactor-edge/health-check.sh`
- `/opt/reactor-edge/ota-update.sh`
- `/opt/reactor-edge/ota-rollback.sh`
- `/opt/reactor-edge/ota-lib.sh`
- `/var/lib/reactor-edge/backups`
- `/var/lib/reactor-edge/ota`
- `/etc/reactor-edge/*.toml`
- `/etc/systemd/system/reactor-edge.service`
- `/etc/systemd/system/reactor-edge-backup.service`
- `/etc/systemd/system/reactor-edge-backup.timer`
- `/etc/systemd/system/reactor-os-chromium.service`

It also creates `/project` for `state.json/control.json`, enables the backend
service, the daily backup timer, and the kiosk service, and starts them
immediately. The backend service launches `reactor-safety-guard` through
`--safety-guard` by default. systemd follows `/opt/reactor-edge/current`, so
application updates can stage the inactive slot first and only switch after the
package has been verified. The backend restarts on failure but is rate-limited
to avoid crash loops; repeated failures require maintenance intervention instead
of unbounded restart churn. The backup timer calls
`/opt/reactor-edge/current/backup.sh` to generate SQLite online snapshots in
`/var/lib/reactor-edge/backups`; the backup helper writes a temporary snapshot,
uses a non-blocking lock so timer and OTA pre-update backups cannot publish at
the same time, verifies the sha256 sidecar and SQLite header, then publishes the
timestamped snapshot and `latest.snapshot` links.

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

For production handover checks, run:

```bash
sudo /opt/reactor-edge/health-check.sh --production
```

`--production` fails unless `/api/devices/status` proves the device is online,
idle, not in emergency stop, automatic control is disabled, manual lock is
cleared, no control fault is latched, and the downstream controller is not
reporting `last_command_ok=false`. With the production default
`require_device_status_for_control=true`, a fresh pipeline sample alone does
not make `/api/devices/status` report `online=true/status=idle`; the downstream
status proof must be present.

## Application A/B OTA Update

The release package also supports an application-level A/B update path. It is
not a full rootfs A/B scheme: `/etc/reactor-edge`, `/var/lib/reactor-edge`, and
`/project` remain shared so configuration, SQLite data, and the device bridge
are not overwritten by application updates.

On the board, copy the new tarball and its generated `.sha256` sidecar. First
run a dry-run preflight, then run the real update:

```bash
sudo /opt/reactor-edge/ota-update.sh \
  reactor-os-lubancat2-rk3568-debian10-chromium-kiosk-*.tar.gz \
  --sha256 reactor-os-lubancat2-rk3568-debian10-chromium-kiosk-*.tar.gz.sha256 \
  --dry-run

sudo /opt/reactor-edge/ota-update.sh \
  reactor-os-lubancat2-rk3568-debian10-chromium-kiosk-*.tar.gz \
  --sha256 reactor-os-lubancat2-rk3568-debian10-chromium-kiosk-*.tar.gz.sha256
```

The sidecar must reference the same tarball basename that is passed to
`ota-update.sh`; a valid hash for a different package is rejected.

The updater:

- rejects concurrent OTA runs with a lock file
- checks required board commands before staging so missing runtime tools fail
  early with an OTA log entry
- verifies the checksum is bound to the package filename before extracting
- rejects invalid health-check arguments; `--health-attempts`,
  `--health-interval`, and `--required-passes` must be positive integers, and
  required passes cannot exceed attempts
- validates tar members before extracting, rejecting absolute paths, `..`
  traversal, multiple top-level roots, links, and device/special entries
- fails closed unless the backend and `/api/devices/status` prove the device is
  explicitly online and idle, with no active process batch, no emergency stop,
  automatic control disabled, manual lock cleared, no uncleared
  `last_control_error`, and no downstream `last_command_ok=false` fault;
  missing downstream status is offline in production mode, even when sensor
  samples are fresh;
  `--force` is only for a confirmed maintenance window and must be paired with
  `--confirm-maintenance-window`
- checks available disk space before staging
- requires the candidate package to include `BUILD-METADATA.properties`, then
  records `from_version`, `to_version`, `from_git`, and `to_git` in OTA state
  for field rollback and incident traceability
- supports `--dry-run`, which performs package, checksum, busy-state, disk,
  backup availability, managed-slot, and candidate-content checks without
  switching `current`/`previous`, installing systemd units, or creating a
  database snapshot
- records checksum, tar-safety, metadata, busy-state, and dry-run validation
  failures before the slot switch as `rejected_before_switch`, so field logs
  distinguish a deliberate rejection from a power-loss interruption while the
  existing `current` slot remains active
- requires OTA commit health checks to prove `/health`, the HMI, and
  `/api/devices/status` safe idle state on consecutive passes; a booted
  candidate that reports emergency stop, automatic control, manual lock,
  uncleared control fault, downstream command failure, or non-idle/offline
  device status is rolled back instead of committed
- creates a pre-update SQLite snapshot unless `--skip-backup` is explicitly used
  with `--confirm-skip-backup`
- extracts into the inactive slot under `/opt/reactor-edge/slots`
- flushes critical OTA writes with `sync` after state-file updates, staged
  candidate creation, inactive-slot replacement, systemd unit/tool installs,
  and `current`/`previous` link switches to reduce power-loss ambiguity
- enables `reactor-edge-ota-boot-check.service`, which runs before the backend
  after boot; pre-switch interruptions keep the existing `current` slot running
  and post-switch interruptions in `switching`, `health_checking`, or
  `rolling_back` restore `previous`
- runs `/opt/reactor-edge/ota-boot-check.sh` as the backend `ExecStartPre`, so
  manual restarts and automatic systemd restarts also re-check OTA state before
  production control starts
- rate-limits repeated backend/kiosk crashes with systemd `StartLimit*` so a
  bad release does not loop indefinitely and wear logs/storage
- removes temporary extract/stage directories on failed runs so repeated failed
  updates do not silently consume slot storage
- refuses to use `current` or `previous` links that point outside
  `/opt/reactor-edge/slots/{a,b}`
- switches `/opt/reactor-edge/current` only after staging succeeds
- requires repeated `/health` and HMI checks after restart
- automatically switches back to the previous slot if the new slot fails health
  checks

Manual rollback uses the previous slot and does not roll back SQLite data:

```bash
sudo /opt/reactor-edge/ota-rollback.sh
```

If the backend/status endpoint is already unavailable, rollback also fails
closed. Confirm the reactor is stopped at the field panel first, then use
`sudo /opt/reactor-edge/ota-rollback.sh --force --confirm-maintenance-window`
during the maintenance window.

OTA state and logs are kept in:

```text
/var/lib/reactor-edge/ota/state.json
/var/lib/reactor-edge/ota/ota.log
```

`state.json` includes the active OTA phase plus `from_version`, `to_version`,
`from_git`, and `to_git`. A package without build metadata is rejected before it
can replace the inactive slot.

During a normal OTA health-check restart, the updater creates a short-lived
marker under `/run/reactor-edge/` so the boot check does not roll back the
candidate while it is being tested. The marker records the OTA updater PID and
process start identity; if that process is no longer alive, boot-check removes
the marker and fails closed instead of trusting a stale bypass. `/run` is cleared
by reboot; if power is lost before the updater switches `current`, the next boot records
`interrupted_before_switch` and keeps the existing current slot running. If power
is lost after `current` has been switched but before the update reaches
`committed`, the next boot treats the candidate as untrusted and restores
`previous` before starting production control.
If OTA state is already `failed`, the boot check exits non-zero and keeps the
backend stopped so the device stays in maintenance until recovery or manual
rollback is performed. When an update or manual rollback enters `failed`, the
OTA scripts also clear the temporary health-check bypass marker and stop the
backend/kiosk services immediately.

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
