# LubanCat 2 / Cortex-A55 QEMU Emulation

This project uses PC-side cross compilation for the LubanCat 2 RK3568 board.
For deployment validation without touching the board, use QEMU user-mode to run
the ARM64 ReactorOS package on the PC.

This is not full board emulation. It does not emulate RK3568 peripherals, GPU,
touch hardware, UART electrical timing, or the vendor desktop image. It does
validate the ARM64 Linux binary, Debian 10 glibc baseline, HTTP API, static HMI,
JSON bridge state/control files, device discovery, and component control flow.

## Install WSL Dependencies

Inside Ubuntu/WSL:

```bash
sudo apt-get update
sudo apt-get install -y qemu-user gcc-aarch64-linux-gnu libc6-arm64-cross curl python3
```

This is much smaller than building another Docker image. The script fails with a
clear message if `qemu-aarch64` or the ARM64 sysroot is missing.

## Build The LubanCat 2 Package

From Windows PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-lubancat2-debian10.ps1
```

The latest package path is recorded in:

```text
dist/latest-lubancat2-debian10-package.txt
```

## Run The Emulation

From Windows PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\run-lubancat2-qemu.ps1
```

Or from WSL/Linux:

```bash
./scripts/run-lubancat2-qemu.sh
```

Open:

```text
http://127.0.0.1:8000/
```

The script creates a runtime area under:

```text
data/lubancat2-qemu/
```

It copies `device.json_bridge.toml` and rewrites only these paths:

```text
state_path   = data/lubancat2-qemu/state.json
control_path = data/lubancat2-qemu/control.json
```

The packaged board config is not modified.

## Smoke Test

Run and stop automatically after probing the main APIs:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\run-lubancat2-qemu.ps1 --smoke
```

The smoke test calls:

```text
GET /health
GET /api/live
GET /api/devices/status
```

With the local JSON bridge simulator enabled, `/api/live` should return real
values from `state.json` and `/api/devices/status` should list sensors plus
controllable components such as:

- `shake_stepper`
- `heater_relay`
- `stirrer_motor`
- optionally `temperature_controller` when relay temperature control is enabled

## JSON Bridge Simulator

By default the script starts a small Python JSON bridge simulator. It is an
explicit emulation tool, not production fallback logic.

The simulator writes:

```text
data/lubancat2-qemu/state.json
```

ReactorOS writes commands to:

```text
data/lubancat2-qemu/control.json
```

Supported control commands:

- `motor`: start/stop the shake vessel stepper.
- `speed`: step shake speed up/down.
- `relay`: toggle heater relay.
- `stir_speed`: set stirrer motor RPM.

Disable the simulator when validating error handling:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\run-lubancat2-qemu.ps1 --no-simulator
```

In that mode, no sensor values are fabricated. The backend should return the
configured JSON error state until a real `state.json` producer is attached.

## Useful Options

```bash
./scripts/run-lubancat2-qemu.sh --bind 127.0.0.1:18000
./scripts/run-lubancat2-qemu.sh --package dist/reactor-os-lubancat2-...tar.gz
./scripts/run-lubancat2-qemu.sh --sysroot /usr/aarch64-linux-gnu
./scripts/run-lubancat2-qemu.sh --no-demo-context
```

## Boundary

Use this emulation before copying a package to the LubanCat 2. Still validate on
the real board for display stack, Chromium kiosk, serial bridge, GPIO timing,
and touch behavior.
