# ESP32 Serial Protocol

本文件是给硬件工程师联调 ESP32 与树莓派的落地协议说明，字段和单位按 `docs/03_技术1文档.docx`、`docs/04_上位机接口文档.docx` 对齐。

## Link

- 物理链路：ESP32 USB-Serial、UART 或 UART 转 RS485 均可，树莓派侧表现为 `/dev/ttyUSB0` 或 `/dev/ttyAMA0`。
- 默认串口：`115200 8N1`。
- 帧格式：一行 ASCII 文本，字段用 `|` 分隔，行尾必须是 `\n`。
- 校验：`chk` 为 XOR 校验，计算范围是不包含 `|chk=` 的整行文本。
- 采样频率：1 Hz。

## RX: ESP32 -> Raspberry Pi

ESP32 每秒上报一帧传感器数据：

```text
RX|v=1|seq=123|ms=456789|temp=175.4|pressure=0.21|stir_speed=450|shake_speed=30|tilt_state=1|flow_rate=2.5|chk=AB
```

必填字段：

| 字段 | 类型 | 单位 | 精度目标 | 说明 |
| --- | --- | --- | --- | --- |
| `v` | Int | - | - | 协议版本，固定为 `1` |
| `seq` | UInt32 | - | - | ESP32 自增序号 |
| `ms` | UInt32 | ms | - | ESP32 启动后的毫秒数 |
| `temp` | Float | degC | +/- 0.1 degC | 反应釜温度 |
| `pressure` | Float | MPa | +/- 0.01 MPa | 釜内压力，不是 kPa |
| `stir_speed` | Int | RPM | +/- 1 RPM | 搅拌速度 |
| `shake_speed` | Int | 次/分 | +/- 1 次/分 | 摇罐速度 |
| `tilt_state` | Int | - | 0 或 1 | 摇罐倾角开关量。硬件只回传二值状态，树莓派软件侧会按 `tilt_state + shake_speed` 拟合展示用倾角曲线 |
| `flow_rate` | Float | L/min | +/- 0.1 L/min | 流量 |
| `chk` | Hex | - | - | 2 位大写或小写十六进制 XOR |

可选扩展字段：

| 字段 | 类型 | 单位 | 说明 |
| --- | --- | --- | --- |
| `product_concentration` | Float | % | 如接入在线浓度传感器，可上报产物浓度 |
| `ph` | Float | pH | 如接入 pH 传感器，可上报 pH |

兼容字段：后端仍能读取旧样机字段 `rpm`、`shake`、`tilt`、`flow`、`conc`，但新固件应使用上表字段。

注意：`tilt_state` 是唯一硬件输入，取值只能是 `0` 或 `1`。API 中返回的 `tilt_angle` / `tilt_angle_deg` 是软件拟合值，用于趋势图和报警判断，不代表倾角传感器直接测得的模拟量。

## TX: Raspberry Pi -> ESP32

树莓派只在通过安全校验后下发目标参数。ESP32 收到后仍必须做本地限幅和硬件联锁，不允许把树莓派作为唯一安全保护。

```text
TX|v=1|heat_time=300|hold_time=600|cool_time=180|target_temp=120.0|stir_speed=850|shake_speed=35|target_pressure=0.50|chk=CD
```

字段：

| 字段 | 类型 | 单位 | 文档范围 | 说明 |
| --- | --- | --- | --- | --- |
| `heat_time` | Float | s | 0-3600 | 升温时间 |
| `hold_time` | Float | s | 0-7200 | 保温时间 |
| `cool_time` | Float | s | 0-3600 | 降温时间 |
| `target_temp` | Float | degC | 0-500 | 目标温度，实际还受 `config/safety.toml` 设备上限限制 |
| `stir_speed` | Int | RPM | 0-2000 | 搅拌速度，实际还受 `config/safety.toml` 设备上限限制 |
| `shake_speed` | Int | 次/分 | 0-60 | 摇罐速度 |
| `target_pressure` | Float | MPa | 0-10 | 目标压力 |

兼容字段：示例固件仍能识别旧下行字段 `target_rpm`、`target_shake`，但新协议应使用 `stir_speed`、`shake_speed`。

## Raspberry Pi Config

硬件联调时优先使用 `config/device.esp32.toml`。如需直接修改默认配置，则把 `config/device.toml` 切换为：

```toml
mode = "esp32_serial"

[serial]
port = "/dev/ttyUSB0"
baudrate = 115200
parity = "N"
stopbits = 1
bytesize = 8
timeout_ms = 1000
```

如果使用 RS485 转接器，串口名可能是 `/dev/ttyUSB0`；如果直接使用树莓派 UART，串口名可能是 `/dev/ttyAMA0` 或 `/dev/serial0`。

## Bench Test

1. 先烧录 `firmware/esp32_reactor_bridge/esp32_reactor_bridge.ino`。
2. ESP32 接树莓派后确认串口名：

```bash
ls -l /dev/serial/by-id/
```

3. 启动服务：

```bash
./reactor-edge-daemon --config config/device.esp32.toml --safety config/safety.toml --db data/reactor.sqlite3 --assets static --bind 0.0.0.0:8000
```

4. 实时数据检查：

```bash
curl http://127.0.0.1:8000/api/v1/reactor/reactor_001/realtime
```

返回的 `data.current_temp`、`data.current_pressure`、`data.stir_speed`、`data.shake_speed`、`data.tilt_state`、`data.tilt_angle`、`data.flow_rate` 应随 ESP32 数据刷新。其中 `data.tilt_angle_source` 固定为 `software_fit_from_binary_sensor`。

设备在线状态检查：

```bash
curl http://127.0.0.1:8000/api/devices/status
```

没有新鲜上行数据时，`/api/live` 和实时数据接口会返回 HTTP 503；v1 realtime WebSocket 会发送同样的错误信封并断开，不继续推送旧样本值或伪造当前时间戳。设备状态接口仍返回 HTTP 200，并在 `data.online_count` 与 `data.devices[].status` 中表达 `offline`、`stale` 或 `error`。

5. 下发控制检查：

```bash
curl -X POST http://127.0.0.1:8000/api/v1/reactor/reactor_001/control \
  -H 'Content-Type: application/json' \
  -d '{"command_id":"cmd_bench_001","params":{"heat_time":300,"hold_time":600,"cool_time":180,"stir_speed":850,"shake_speed":35,"target_temp":120.0,"target_pressure":0.5},"priority":"normal","auto_start":false}'
```

ESP32 串口应收到一行 `TX|...` 命令；如果 `target_temp` 或 `stir_speed` 超过安全配置，树莓派会返回 HTTP 400，不会下发设备写入。
