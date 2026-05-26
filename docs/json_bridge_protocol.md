# JSON Serial Bridge Integration

ReactorOS can run against the downstream JSON serial bridge by switching the device mode to `json_bridge`.

The bridge uses read/write separation:

- `state.json`: read-only state from the downstream bridge.
- `control.json`: write-only command request from ReactorOS to the downstream bridge.

ReactorOS never fabricates production sensor values in this mode. If a required field is missing, stale, disconnected, or fails frame validation, `/api/live` returns `503` through the existing error envelope and `/api/devices/status` exposes the device as offline, stale, or error.

## Required State Fields

The bridge state file must be a UTF-8 JSON object. ReactorOS validates these fields before accepting a sample:

```json
{
  "connected": true,
  "last_seen_ms": 1779552000000,
  "last_frame_ok": true,
  "temperature_c": 64.25,
  "pressure_mpa": 0.50,
  "stirrer_rpm": 125.18,
  "shake_speed_cpm": 30.00,
  "tilt": 1,
  "flow_rate_l_min": 1.20,
  "product_concentration_percent": 50.01,
  "ph": 6.15
}
```

The downstream bridge may also provide these raw/device fields:

```json
{
  "adc": 2048,
  "status": 7,
  "relay": 1,
  "motor": 1,
  "speed_delay_us": 10000,
  "last_command_request_id": "reactor-os-1779552000001",
  "last_command_ok": true,
  "last_command_error": null,
  "port": "/dev/ttyUSB0",
  "baudrate": 115200
}
```

If `tilt` is absent, ReactorOS can read `status` bit 2. The tilt sensor is binary (`0`/`1`); ReactorOS fits `tilt_angle_deg` in software from `tilt` + `shake_speed_cpm` + timestamp for trend visualization.

If one analog value is only available as `adc`, configure `[json_bridge.adc]` in `config/device.json_bridge.toml` to map it to exactly one sensor.

## Control File

Every write to `control.json` is an atomic temp-file + rename write and includes a new `request_id`.

Supported command shapes match the downstream bridge:

```json
{ "request_id": "reactor-os-1779552000001", "command": "motor", "value": 1 }
{ "request_id": "reactor-os-1779552000002", "command": "motor", "value": 0 }
{ "request_id": "reactor-os-1779552000003", "command": "speed", "value": "up" }
{ "request_id": "reactor-os-1779552000004", "command": "speed", "value": "down" }
{ "request_id": "reactor-os-1779552000005", "command": "relay", "value": 1 }
{ "request_id": "reactor-os-1779552000006", "command": "stir_speed", "value": 480.25, "name": "stirrer_motor" }
```

The shake vessel downstream protocol supports discrete commands, not exact
setpoint writes. ReactorOS therefore translates safe shake targets into:

- shake speed target `> 0`: motor on if needed.
- shake speed target `<= 0`: motor off if needed.
- shake target above current speed: speed up.
- shake target below current speed: speed down.
- stirrer RPM exact setpoint: `command = "stir_speed"` with numeric `value`.
- optional relay temperature control if `relay_temperature_control = true` and temperature is present.

## Device Capability Discovery

ReactorOS exposes connected device status and component capabilities through:

```text
GET /api/devices/status
GET /api/devices/capabilities
```

The status response includes overall device counts and a `components` list. For
the JSON bridge mode, ReactorOS exposes these controllable components:

- `shake_stepper`: stepper motor for the shake vessel.
- `heater_relay`: relay-style temperature/heater actuator.
- `stirrer_motor`: independently controlled stirrer RPM setpoint.
- `temperature_controller`: target temperature control, only when
  `relay_temperature_control = true`.

Example capability item:

```json
{
  "component_id": "shake_stepper",
  "component_type": "stepper_motor",
  "label": "Shake Vessel Stepper",
  "controllable": true,
  "status": "running",
  "state": {
    "motor": 1,
    "tilt": 1,
    "speed_delay_us": 10000,
    "target_shake_speed_cpm": 30.0,
    "current_shake_speed_cpm": 30.0
  },
  "actions": [
    { "action": "start", "value_type": "none" },
    { "action": "stop", "value_type": "none" },
    { "action": "speed_up", "value_type": "none" },
    { "action": "speed_down", "value_type": "none" },
    { "action": "set_speed", "value_type": "number", "min": 0, "max": 60, "unit": "CPM" }
  ]
}
```

## Single Component Control

Use the component-control endpoint when the operator needs to control one
actuator independently:

```text
POST /api/devices/reactor_001/components/{component_id}/control
POST /api/v1/devices/reactor_001/components/{component_id}/control
```

Examples:

```bash
curl -X POST http://127.0.0.1:8000/api/devices/reactor_001/components/shake_stepper/control \
  -H 'content-type: application/json' \
  -d '{"action":"stop","reason":"operator stopped shake vessel"}'

curl -X POST http://127.0.0.1:8000/api/devices/reactor_001/components/shake_stepper/control \
  -H 'content-type: application/json' \
  -d '{"action":"set_speed","value":24.5,"reason":"manual shake speed adjustment"}'

curl -X POST http://127.0.0.1:8000/api/devices/reactor_001/components/heater_relay/control \
  -H 'content-type: application/json' \
  -d '{"action":"off","reason":"manual heater relay off"}'

curl -X POST http://127.0.0.1:8000/api/devices/reactor_001/components/stirrer_motor/control \
  -H 'content-type: application/json' \
  -d '{"action":"set_rpm","value":480.25,"reason":"manual stirrer RPM adjustment"}'
```

Safety behavior:

- Unknown devices/components/actions return JSON error codes.
- Emergency stop and manual lock block component control.
- Invalid or stale `state.json` blocks JSON bridge writes.
- Successful writes generate `control_events` audit records.
- Sensor values are still never fabricated by component control.

## Run

```bash
./reactor-edge-daemon \
  --config config/device.json_bridge.toml \
  --safety config/safety.toml \
  --memory config/ai_memory.toml \
  --db data/reactor.sqlite3 \
  --assets static \
  --bind 0.0.0.0:8000
```

Default bridge paths in `config/device.json_bridge.toml`:

```toml
[json_bridge]
state_path = "/project/state.json"
control_path = "/project/control.json"
max_state_age_ms = 6000
```

Use absolute paths that match the deployed bridge service.
