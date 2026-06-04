# ReactorOS Edge Supervisor

运行在树莓派上的反应釜边缘上位机 PoC。系统通过 ESP32 串口桥或外部数据管线采集反应釜数据，在树莓派本地完成安全限幅、批次记录、AI 推荐和 Web 上位机展示。

这个项目的定位不是替代反应釜本体保护，而是做“可审计、可解释、可离线运行”的上位机控制层：低层设备负责实际加热和搅拌执行，树莓派只下发经过安全边界校验的目标温度和目标转速。

## 功能概览

- 实时采集温度、压力、搅拌转速、摇罐速度、流量、产物浓度和 pH。
- Web 上位机提供实时仪表、温度曲线、参数控制、急停、自动控制开关、批次结果录入和历史记录。
- SQLite 本地持久化传感器样本、批次、控制事件、产物结果和 AI 推荐上下文。
- 文件型 AI 记忆通过 `config/ai_memory.toml` 管理参考批次、搜索边界、禁区和传感器阈值。
- 安全控制层强制检查急停、人工锁定、传感器超时、温度/转速上限和单次调整步长。
- ESP32 通过 ASCII 串口协议向树莓派传递采集帧，树莓派向 ESP32 下发目标参数帧。
- Playwright 覆盖桌面和移动端完整操作流程，Rust 测试覆盖 API、控制、数据库、优化器和 ESP32 协议。

## 架构

```text
Sensors / actuators
        |
      ESP32
        |
 USB-Serial / UART / RS485
        |
 reactor-edge-daemon (Raspberry Pi)
  |          |           |
 SQLite   Safety     AI memory + optimizer
        |
 HTTP API + static Web dashboard
```

`reactor-edge-daemon` 是主控 Rust 进程，可选通过 `--safety-guard` 调用独立 `reactor-safety-guard` 子进程做安全判定：

- 后台控制循环持续读取设备数据并写入 SQLite。
- Web UI 作为静态文件由同一进程托管。
- API 与控制循环共享运行状态。
- 所有真实设备写入都走安全限幅器并生成审计事件；自动控制环路可把安全决策委托给独立 safety guard 进程。

## 目录结构

```text
config/                         设备、安全和 AI 记忆配置
docs/                           ESP32 协议和项目文档
e2e/                            Playwright 前端端到端验收
firmware/esp32_reactor_bridge/  ESP32 示例固件
src/                            Rust 后端源码
static/                         单页 Web 上位机
tests/                          Rust 集成测试
deploy/                         systemd 部署文件
docker-compose.yml              本地运行与测试编排
Dockerfile                      构建、测试、运行镜像
```

## 技术选型

项目面向树莓派长期运行，优先选择运行成本低、部署简单、依赖少的方案。

- 后端：Rust + Tokio + Axum。
- 数据库：SQLite，本地文件存储。
- 前端：单个 `static/index.html`，原生 HTML/CSS/JavaScript，无前端框架和打包器。
- 通信：默认外部数据管线，硬件联调支持 ESP32 Serial、JSON 文件桥接；配置中保留 Modbus RTU 映射。
- 部署：单二进制 + `systemd`，也支持 Docker Compose。

## 快速启动

默认配置使用外部数据管线模式，不会自动生成传感器读数。没有 ESP32 或测试管线样本流入时，`/api/live` 会按约定返回 `503`，前端显示空值和错误码。

```powershell
docker compose up --build reactor-edge
```

打开：

```text
http://127.0.0.1:8000/
```

健康检查：

```powershell
Invoke-RestMethod http://127.0.0.1:8000/health
Invoke-RestMethod http://127.0.0.1:8000/api/live
Invoke-RestMethod http://127.0.0.1:8000/api/devices/status
```

## 鲁班猫 2 Debian 10 交付构建

鲁班猫 2 使用 RK3568 / ARM64 / Cortex-A55，继续采用电脑侧交叉编译，不在开发板上编译。推荐生成鲁班猫 2 专用包，包内 systemd 默认用户为 `cat`，Chromium kiosk 默认使用 `/home/cat/.Xauthority`。

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-lubancat2-debian10.ps1
```

WSL/Linux 环境可用：

```bash
./scripts/build-lubancat2-debian10.sh
```

生成结果位于 `dist/`，最新鲁班猫 2 包路径记录在 `dist/latest-lubancat2-debian10-package.txt`。开发板只需要安装 `ca-certificates`、`libudev1`、`chromium`/`chromium-browser`、`curl`、`x11-xserver-utils` 等运行依赖，然后解压包运行 `./run.sh ./config/device.json_bridge.toml`。

详细流程见 [docs/lubancat2_debian10_deploy.md](docs/lubancat2_debian10_deploy.md)。

## 通用 A55 Debian 10 交付构建

开发板性能低时，不要在板子上编译。推荐在电脑侧用 Docker 交叉编译并生成完整运行包：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-a55-debian10.ps1
```

WSL/Linux 环境可用：

```bash
./scripts/build-a55-debian10.sh
```

生成结果位于 `dist/`，最新包路径记录在 `dist/latest-a55-debian10-package.txt`。开发板只需要安装 `ca-certificates`、`libudev1`、`chromium`/`chromium-browser`、`curl`、`x11-xserver-utils` 等运行依赖，然后解压包运行 `./run.sh ./config/device.json_bridge.toml`。

详细流程见 [docs/a55_debian10_pc_build.md](docs/a55_debian10_pc_build.md)。

## 客户演示数据

如果需要给客户展示工艺管理、历史批次、告警队列和 AI 学习推荐，可以在启动时加入：

```bash
./reactor-edge-daemon \
  --config config/device.json_bridge.toml \
  --safety config/safety.toml \
  --memory config/ai_memory.toml \
  --db data/reactor.sqlite3 \
  --assets static \
  --bind 0.0.0.0:8000 \
  --seed-demo-context
```

演示种子只写入：

- 工艺定义和工艺步骤。
- 历史批次、人工录入产率和产物比例。
- AI 推荐结果。
- 非传感器类演示告警和操作事件。

它不会写入 `sensor_samples`，也不会设置 `runtime.latest_sample`。因此没有真实 `state.json`、ESP32 或外部管线数据时，`/api/live` 仍然返回 `503`，传感器区域仍显示真实错误码。前端会从 `/api/demo/context` 读取演示上下文，用来展示 AI 和工艺功能。

## 本地模拟下游设备

本地开发或客户演示时，可以启动一个显式的外部模拟设备，把数据按真实管线写入后端。这个脚本不属于生产采集逻辑，也不会让前端或后端在缺数据时自动造假；停止脚本后，传感器数据仍会按超时规则变成错误状态。

默认 `docker-compose.yml` 使用 `config/device.toml` 的 `pipeline` 模式，先启动后端：

```powershell
docker compose up --build reactor-edge
```

然后在另一个终端启动模拟设备：

```powershell
npm run simulate:device
```

只打一帧样本用于验收：

```powershell
npm run simulate:device:once
```

脚本默认每秒向 `POST /api/v1/reactor/reactor_001/samples` 上报温度、压力、搅拌转速、摇罐速度、二值倾角、流量、浓度和 pH。可选参数示例：

```powershell
node scripts\simulate-device.js --profile production --interval-ms 1000
node scripts\simulate-device.js --url http://127.0.0.1:8000 --device-id reactor_001
```

如果要模拟 `state.json/control.json` 读写分离桥接协议：

```powershell
node scripts\simulate-device.js --mode json-bridge --state data\simulator\state.json --control data\simulator\control.json
```

JSON 桥接模式会持续写入 `state.json`，并监听 ReactorOS 写入的 `control.json`，支持 `motor`、`speed`、`relay` 三类命令。开发板部署时仍应由真实下游桥接程序写入 `state.json`。

## 本地开发

如果本机已安装 Rust：

```powershell
cargo run -- --config config/device.toml --safety config/safety.toml --memory config/ai_memory.toml --integration config/integration.toml --db data/reactor.sqlite3 --assets static --bind 127.0.0.1:8000
```

常用参数：

```text
--config              设备配置，默认 config/device.toml
--safety              安全边界配置，默认 config/safety.toml
--memory              AI 记忆配置，默认 config/ai_memory.toml
--integration         第三方集成配置，默认 config/integration.toml
--db                  SQLite 数据库路径，默认 data/reactor.sqlite3
--assets              静态前端目录，默认 static
--bind                HTTP 监听地址，默认 127.0.0.1:8000
--tls-cert            HTTPS 证书 PEM 路径，必须与 --tls-key 成对提供
--tls-key             HTTPS 私钥 PEM 路径，必须与 --tls-cert 成对提供
--safety-guard        可选独立安全判定进程，通常指向 reactor-safety-guard
--enable-test-reset   启用 /api/test/reset 和 /api/test/pipeline-sample，仅用于本地验收测试
```

独立安全判定进程可单独验收：

```powershell
cargo run --bin xingshu -- --json safety check --temp 999 --rpm 9999 --shake 99 --pressure 99
```

本地 HTTPS 验证示例：

```powershell
cargo run --bin reactor-edge-daemon -- `
  --config config/device.toml `
  --safety config/safety.toml `
  --memory config/ai_memory.toml `
  --integration config/integration.toml `
  --db data/reactor-tls-test.sqlite3 `
  --assets static `
  --bind 127.0.0.1:18443 `
  --tls-cert output/tls-test/server.crt `
  --tls-key output/tls-test/server.key `
  --enable-test-reset
```

## ESP32 接入

硬件链路建议：

```text
传感器 / 执行器 -> ESP32 -> USB-Serial/UART/RS485 -> 树莓派 -> ReactorOS
```

硬件联调配置使用：

```bash
./reactor-edge-daemon \
  --config config/device.esp32.toml \
  --safety config/safety.toml \
  --memory config/ai_memory.toml \
  --db data/reactor.sqlite3 \
  --assets static \
  --bind 0.0.0.0:8000
```

默认串口参数：

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

ESP32 上行采集帧示例：

```text
RX|v=1|seq=123|ms=456789|temp=175.4|pressure=0.21|stir_speed=450|shake_speed=30|tilt_state=1|flow_rate=2.5|chk=AB
```

摇罐倾角传感器只要求 ESP32 上报二值 `tilt_state=0|1`。ReactorOS 会结合 `shake_speed` 在软件侧拟合 `tilt_angle_deg` 曲线，用于趋势图和报警；该曲线不是硬件直接采集的模拟倾角。

树莓派下行控制帧示例：

```text
TX|v=1|heat_time=300|hold_time=600|cool_time=180|target_temp=120.0|stir_speed=850|shake_speed=35|target_pressure=0.50|chk=CD
```

协议细节见 [docs/esp32_protocol.md](docs/esp32_protocol.md)，示例固件见 [firmware/esp32_reactor_bridge/esp32_reactor_bridge.ino](firmware/esp32_reactor_bridge/esp32_reactor_bridge.ino)。

## JSON 文件桥接接入

如果下游串口桥已经按文件读写分离工作，使用 `json_bridge` 模式：

- `state.json`：下游持续写入当前状态，ReactorOS 只读。
- `control.json`：ReactorOS 原子写入控制命令，下游只读并下发串口。

启动示例：

```bash
./reactor-edge-daemon \
  --config config/device.json_bridge.toml \
  --safety config/safety.toml \
  --memory config/ai_memory.toml \
  --db data/reactor.sqlite3 \
  --assets static \
  --bind 0.0.0.0:8000
```

默认路径：

```toml
[json_bridge]
state_path = "/project/state.json"
control_path = "/project/control.json"
max_state_age_ms = 6000
```

`state.json` 必须提供真实传感器字段；缺少温度、压力、转速、摇罐速度、流量、浓度或 pH 时，后端会返回错误，不会补本地假数据。摇罐倾角传感器只需要回传 `tilt = 0|1` 或 `status` bit2，ReactorOS 会在软件侧拟合 `tilt_angle_deg` 曲线。

`control.json` 每次写入都会生成新的 `request_id`，并只使用下游文档约定的 `motor`、`speed`、`relay` 等命令。详细字段见 [docs/json_bridge_protocol.md](docs/json_bridge_protocol.md)。

## 配置说明

### `config/device.toml`

设备通信配置。默认 `mode = "pipeline"`，只接受外部管线流入的数据，不在后端造数。ESP32 串口模式使用 `config/device.esp32.toml`，JSON 文件桥接模式使用 `config/device.json_bridge.toml`。

### `config/safety.toml`

安全限幅配置，关键字段包括：

- `control.sensor_timeout_ms`：传感器超时后禁止自动控制写入。
- `temperature.max_c`：上位机允许的最高目标温度。
- `temperature.max_step_c`：单次控制循环允许调整的最大温差。
- `stirrer.max_rpm`：上位机允许的最高目标转速。
- `stirrer.max_step_rpm`：单次控制循环允许调整的最大转速变化。
- `optimizer.*`：AI 推荐搜索边界，必须落在安全策略允许范围内。

### `config/ai_memory.toml`

AI 推荐的文件型记忆入口：

- `reference_batches`：真实批次不足时的参考样本。
- `recommendation.bounds`：AI 搜索空间。
- `forbidden_zones`：人工标记的禁区，推荐不会落入这些区域。
- `sensor_limits`：页面状态和报警使用的传感器阈值。

## StepFun AI 接入

系统默认使用本地轻量优化器，保证树莓派无网、无密钥时也能离线运行。需要接入阶跃 `step-3.6` 时，复制 `.env.example` 为本地 `.env`，并填入自己的 API Key：

```powershell
Copy-Item .env.example .env
```

```env
STEPFUN_AI_ENABLED=true
STEPFUN_API_KEY=你的密钥
STEPFUN_BASE_URL=https://api.stepfun.com/v1
STEPFUN_API_TYPE=chat_completions
STEPFUN_MODEL=step-3.6
STEPFUN_REASONING_EFFORT=medium
STEPFUN_TIMEOUT_SECONDS=20
```

默认使用 Chat Completions 接口：

```text
POST https://api.stepfun.com/v1/chat/completions
model: step-3.6
reasoning_effort: low | medium | high
```

如果要切到 StepFun 新的 Messages API：

```env
STEPFUN_BASE_URL=https://api.stepfun.com
STEPFUN_API_TYPE=messages
```

对应完整请求路径：

```text
POST https://api.stepfun.com/v1/messages
```

`STEPFUN_BASE_URL` 填 `https://api.stepfun.com` 或 `https://api.stepfun.com/v1` 都可以，后端会自动归一化到 `/v1`。

调用策略：

- `/api/live` 不直接请求外部模型，避免页面轮询刷爆 API。
- 录入批次结果或请求 `/api/recommendations/latest` 时会尝试调用 StepFun。
- 模型输出必须是推荐参数 JSON，后端会再次校验边界和禁区。
- StepFun 超时、无 key、返回异常或输出落入禁区时，会自动回退本地优化器。
- API Key 只从环境变量读取，不写入代码、配置模板或数据库。

树莓派 `systemd` 部署建议把密钥放到 `/etc/reactor-edge/reactor-edge.env`，并在服务文件中启用：

```ini
EnvironmentFile=-/etc/reactor-edge/reactor-edge.env
```

## Xingshu CLI

The upper-computer CLI is provided as a second binary named `xingshu`. It talks to
the running daemon through the same safety-gated REST API used by the Web HMI.
This keeps CLI writes auditable and prevents a separate command path from
bypassing interlocks.

Build or run it from the repo:

```powershell
cargo run --bin xingshu -- --help
cargo run --bin xingshu -- status
cargo run --bin xingshu -- config --local --json
```

Protected write, export, audit, and Modbus debug operations require a signed
local bearer session. Login once and pass the returned token with `--token`, or
store it in `XINGSHU_TOKEN`:

```powershell
$login = cargo run --bin xingshu -- auth login --username engineer --password engineer123
$env:XINGSHU_TOKEN = ($login | ConvertFrom-Json).data.token
cargo run --bin xingshu -- auth me
```

Default local users are `operator/operator123`, `engineer/engineer123`, and
`admin/admin123`. Override them with `XINGSHU_OPERATOR_PASSWORD`,
`XINGSHU_ENGINEER_PASSWORD`, `XINGSHU_ADMIN_PASSWORD`, and set
`XINGSHU_AUTH_SECRET` before production-style acceptance tests.

Main command groups:

```text
xingshu start                         # run reactor-edge-daemon in foreground
xingshu stop                          # stop active process or disable auto control
xingshu auth login --username engineer --password engineer123
xingshu auth me
xingshu status                        # service, device, interlock, and AI status
xingshu config                        # runtime API config summary
xingshu config --local --json         # local device/safety TOML summary
xingshu data list                     # list experiment batches
xingshu data export --out batches.csv # export batch CSV
xingshu data export-xlsx --out batches.xlsx
xingshu data report --batch-id 1      # export Markdown experiment report
xingshu control set --temp 60 --rpm 300
xingshu control start --process-id 1
xingshu control stop
xingshu control estop
xingshu control estop --reset
xingshu ai suggest
xingshu ai model
xingshu ai train                      # reports current LoRA training API gap
xingshu audit list
xingshu audit export --out audit.csv
xingshu modbus map
xingshu modbus read temperature_c
xingshu modbus write target_temperature_c 65 --reason "acceptance test"
```

Local Qwen/LoRA readiness is also visible through `/api/config/summary` as
`local_ai`. Set `XINGSHU_LOCAL_AI_BIN`, `XINGSHU_LOCAL_AI_GGUF`,
`XINGSHU_LOCAL_AI_LORA`, `XINGSHU_LOCAL_AI_TRAIN_SCRIPT`,
`XINGSHU_LOCAL_AI_CONVERT_SCRIPT`, and `XINGSHU_LOCAL_AI_RK_REPORT` to make the
upper-computer boundary report concrete assets. See
`docs/local_ai_adapter_status_addendum.md`.

`xingshu ai train` intentionally fails until a local LoRA training API is added
to the daemon. That makes the remaining PRD model self-evolution gap visible in
the operator tooling instead of pretending the feature exists.

## 主要 API

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `GET` | `/health` | 服务健康检查 |
| `GET` | `/api/live` | Web UI 实时聚合数据 |
| `GET` | `/api/devices/status` | 当前在线设备数量和设备状态 |
| `POST` | `/api/processes/:id/start` | 启动工艺流程，创建活动批次并写入安全限幅后的目标 |
| `POST` | `/api/processes/current/stop` | 停止当前工艺流程，结束活动批次并关闭自动控制 |
| `POST` | `/api/processes/:id/stop` | 停止指定工艺的活动批次，不匹配时返回 `409` |
| `POST` | `/api/batches/start` | 启动批次并写入目标参数 |
| `POST` | `/api/batches/:id/finish` | 结束批次 |
| `GET` | `/api/batches/export.csv` | 导出批次与实验结果 CSV |
| `GET` | `/api/batches/export.xlsx` | 导出批次、结果与汇总 Excel 工作簿 |
| `GET` | `/api/batches/:id/report.md` | 生成单批次 Markdown 实验报告 |
| `POST` | `/api/product-results` | 录入产率和产物比例 |
| `POST` | `/api/control/auto` | 开启或关闭自动控制 |
| `POST` | `/api/control/manual-lock` | 开启或关闭人工锁定 |
| `POST` | `/api/control/targets` | 更新目标温度和转速 |
| `POST` | `/api/control/emergency-stop` | 急停或复位急停 |
| `GET` | `/api/recommendations/latest` | 获取最新 AI 推荐 |
| `GET` | `/api/ai/experiment-plan` | 基于批次结果和安全边界生成只读实验 SOP 草案 |
| `GET` | `/api/v1/devices/status` | 文档版设备在线状态接口 |
| `POST` | `/api/v1/reactor/:device_id/control` | 文档版控制接口 |
| `POST` | `/api/v1/reactor/:device_id/samples` | 数据管线上行样本写入接口 |
| `GET` | `/api/v1/reactor/:device_id/realtime` | 文档版实时数据接口 |
| `GET` | `/api/v1/reactor/:device_id/history` | 文档版历史数据接口 |
| `WS` | `/ws/v1/reactor/:device_id/realtime` | 文档版实时 WebSocket |

Additional upper-computer endpoints used by the HMI and CLI:

```text
GET /api/audit/logs              tamper-evident audit events and chain status
GET /api/audit/export.csv        audit CSV export
POST /api/auth/login             local username/password login for bearer session
GET /api/auth/me                 inspect the current bearer session
GET /api/config/summary          device, safety, AI, permissions, integration summary
GET /api/permissions/roles       local role policy with enforced bearer sessions
GET /api/ai/experiment-plan      read-only safety-gated experiment SOP draft
POST /api/integrations/ainas/tasks
GET /api/integrations/ainas/tasks
GET /api/integrations/ainas/tasks/:id
GET /api/modbus/registers        configured Modbus map and current values
GET /api/modbus/registers/:name/read
POST /api/modbus/registers/:name/write
GET /api/batches/export.csv      batch/result CSV export
GET /api/batches/export.xlsx     Excel workbook export with batches, results, and summary sheets
GET /api/batches/:id/report.md   Markdown experiment report
```

AINAS task dispatch supports safety-gated `set_targets`, `start_process`, and
`stop_process` actions. Each authenticated task is persisted with the raw
request, execution status, and response payload:

```json
{
  "external_task_id": "ainas-001",
  "action": "set_targets",
  "target_temperature_c": 60,
  "target_stirrer_rpm": 300,
  "target_shake_speed_cpm": 24,
  "reason": "AINAS recipe handoff"
}
```

Set `XINGSHU_DB_ENCRYPTION_KEY` to enable AES-256-GCM encryption at rest for
`integration_tasks.request_json` and `integration_tasks.response_json`. The key
may be 32 raw bytes, 64 hex characters, or base64-encoded 32 bytes. Existing
plain JSON task rows remain readable for local database upgrades, and
`GET /api/config/summary` reports the current `data_security.storage_encryption`
status for HMI/acceptance checks.

`/api/modbus/registers` now returns the upper-computer Modbus map for local
debug and interface handoff: eight read registers
(`temperature_c`, `stirrer_rpm`, `pressure_mpa`, `shake_speed_cpm`,
`tilt_angle_deg`, `flow_rate_l_min`, `product_concentration_percent`, `ph`),
seven safety-gated write registers (`target_temperature_c`,
`target_stirrer_rpm`, `target_shake_speed_cpm`, `target_pressure_mpa`,
`heat_time_s`, `hold_time_s`, `cool_time_s`), plus coils and discrete inputs for
runtime state. The optional Modbus TCP server is configured through
`config/integration.toml` and disabled by default. Its PDU handler supports
function codes `01` (read coils), `02` (read discrete inputs), `03` (read holding
registers), and `06` (write single holding register). Writes reuse the same
safety-gated runtime target update and audit path as the REST debug endpoint.
Automated tests cover direct PDU handling, a real local TCP/MBAP client read
request, and a local TLS/MBAP read request.
HTTP/HTTPS is available through `--tls-cert` and `--tls-key` and has been
validated locally with a self-signed certificate. Modbus TCP TLS is configured
through `tls_cert` and `tls_key` in `config/integration.toml`; external
Modbus Poll/Slave and production certificate-chain acceptance still need to be
run in the lab network.

MQTT bridge support is configured through `config/integration.toml` and is
disabled by default for local development. When enabled, the daemon uses
`rumqttc` as an MQTT 3.1.1 client, subscribes to
`xingshu/reactor_001/tasks`, executes the same safety-gated task payloads used
by the AINAS REST API, persists them as `source=mqtt` integration tasks, and
publishes receipts to `xingshu/reactor_001/task_receipts`. It also publishes a
retained alarm snapshot to `xingshu/reactor_001/alerts` on the configured alert
interval so third-party supervisors can consume the same active alarm state used
by the Web HMI. The default template uses port `8883` with TLS enabled and
supports `ca_cert`, `client_cert`, and `client_key`; provide broker credentials
and production certificates in the config before connecting to a production
broker.

本地 E2E 使用的 `/api/test/reset` 和 `/api/test/pipeline-sample` 只有在启动参数包含 `--enable-test-reset` 时可用，生产部署不要开启。

工艺流程启停是生产控制入口：启动接口会拒绝急停、人工锁定、已有活动批次和缺少新鲜传感器数据的状态；停止接口会写入停止目标、关闭自动控制、结束当前批次并生成 `process_stopped` 审计事件。`/api/processes/:id/apply` 仍保留为兼容别名，但内部走同一套启动安全门。

设备状态接口示例：

```json
{
  "code": 0,
  "message": "success",
  "data": {
    "total_count": 1,
    "online_count": 0,
    "devices": [
      {
        "device_id": "reactor_001",
        "device_role": "reactor_bridge",
        "online": false,
        "status": "offline",
        "last_seen_at": null,
        "last_seen_age_ms": null,
        "stale_after_ms": 6000,
        "active_batch_id": null,
        "emergency_stop": false,
        "last_control_error": null
      }
    ]
  }
}
```

`status` 取值为 `offline`、`stale`、`error`、`idle`、`running`。这个接口即使在 `/api/live` 因缺少管线数据返回 `503` 时也会返回 `200`，用于前端和运维脚本判断设备在线数量。

## 测试

后端测试：

```powershell
docker compose run --rm test cargo test
```

格式检查：

```powershell
docker compose run --rm test bash -lc "/usr/local/cargo/bin/rustup component add rustfmt >/tmp/rustfmt-install.log && /usr/local/cargo/bin/cargo fmt -- --check"
```

前端端到端测试：

```powershell
npm install
npx playwright install chromium
npx playwright test
```

E2E 覆盖：

- 桌面端正常流程：应用 AI 推荐、启动批次、开启自动控制、停止批次、录入结果。
- 桌面端异常流程：非法温度/转速被后端拒绝，并显示简洁错误。
- 移动端流程：单列响应式、启动、急停、复位、停止。
- UI/UX 检查：横向溢出、文字裁切、关键文案、控制台错误、按钮圆角和阴影一致性。

## 树莓派部署

树莓派上编译需要系统依赖：

```bash
sudo apt-get update
sudo apt-get install -y libudev-dev pkg-config
cargo build --release
```

安装为 `systemd` 服务：

```bash
sudo install -m 0755 target/release/reactor-edge-daemon /usr/local/bin/reactor-edge-daemon
sudo mkdir -p /etc/reactor-edge /var/lib/reactor-edge /opt/reactor-edge
sudo cp config/*.toml /etc/reactor-edge/
sudo cp -r static /opt/reactor-edge/
sudo cp deploy/reactor-edge.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now reactor-edge
```

查看日志：

```bash
journalctl -u reactor-edge -f
```

## 安全边界

这个 PoC 不应作为唯一安全保护。真实反应釜上线前必须满足：

- 反应釜本体有独立温控保护、超温断电、急停和电气联锁。
- ESP32 固件侧也要做本地限幅和异常断开策略。
- 树莓派服务的目标温度、转速、压力等单位和量程必须按现场设备手册重新校准。
- 首次硬件联调必须空载或使用安全介质，逐步验证采集、下发、限幅和急停。
- 自动控制默认关闭，由人工确认现场状态后再开启。
