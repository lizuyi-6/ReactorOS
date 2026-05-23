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

`reactor-edge-daemon` 是一个 Rust 单进程：

- 后台控制循环持续读取设备数据并写入 SQLite。
- Web UI 作为静态文件由同一进程托管。
- API 与控制循环共享运行状态。
- 所有真实设备写入都走安全限幅器并生成审计事件。

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
- 通信：默认外部数据管线，硬件联调使用 ESP32 Serial；配置中保留 Modbus RTU 映射。
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
```

## 本地开发

如果本机已安装 Rust：

```powershell
cargo run -- --config config/device.toml --safety config/safety.toml --memory config/ai_memory.toml --db data/reactor.sqlite3 --assets static --bind 127.0.0.1:8000
```

常用参数：

```text
--config              设备配置，默认 config/device.toml
--safety              安全边界配置，默认 config/safety.toml
--memory              AI 记忆配置，默认 config/ai_memory.toml
--db                  SQLite 数据库路径，默认 data/reactor.sqlite3
--assets              静态前端目录，默认 static
--bind                HTTP 监听地址，默认 127.0.0.1:8000
--enable-test-reset   启用 /api/test/reset 和 /api/test/pipeline-sample，仅用于本地验收测试
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
RX|v=1|seq=123|ms=456789|temp=175.4|pressure=0.21|stir_speed=450|shake_speed=30|flow_rate=2.5|chk=AB
```

树莓派下行控制帧示例：

```text
TX|v=1|heat_time=300|hold_time=600|cool_time=180|target_temp=120.0|stir_speed=850|shake_speed=35|target_pressure=0.50|chk=CD
```

协议细节见 [docs/esp32_protocol.md](docs/esp32_protocol.md)，示例固件见 [firmware/esp32_reactor_bridge/esp32_reactor_bridge.ino](firmware/esp32_reactor_bridge/esp32_reactor_bridge.ino)。

## 配置说明

### `config/device.toml`

设备通信配置。默认 `mode = "pipeline"`，只接受外部管线流入的数据，不在后端造数。硬件模式使用 `config/device.esp32.toml`。

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
STEPFUN_MODEL=step-3.6
STEPFUN_REASONING_EFFORT=medium
STEPFUN_TIMEOUT_SECONDS=20
```

实现使用 Chat Completions 接口：

```text
POST https://api.stepfun.com/v1/chat/completions
model: step-3.6
reasoning_effort: low | medium | high
```

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

## 主要 API

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| `GET` | `/health` | 服务健康检查 |
| `GET` | `/api/live` | Web UI 实时聚合数据 |
| `POST` | `/api/batches/start` | 启动批次并写入目标参数 |
| `POST` | `/api/batches/:id/finish` | 结束批次 |
| `POST` | `/api/product-results` | 录入产率和产物比例 |
| `POST` | `/api/control/auto` | 开启或关闭自动控制 |
| `POST` | `/api/control/manual-lock` | 开启或关闭人工锁定 |
| `POST` | `/api/control/targets` | 更新目标温度和转速 |
| `POST` | `/api/control/emergency-stop` | 急停或复位急停 |
| `GET` | `/api/recommendations/latest` | 获取最新 AI 推荐 |
| `POST` | `/api/v1/reactor/:device_id/control` | 文档版控制接口 |
| `POST` | `/api/v1/reactor/:device_id/samples` | 数据管线上行样本写入接口 |
| `GET` | `/api/v1/reactor/:device_id/realtime` | 文档版实时数据接口 |
| `GET` | `/api/v1/reactor/:device_id/history` | 文档版历史数据接口 |
| `WS` | `/ws/v1/reactor/:device_id/realtime` | 文档版实时 WebSocket |

本地 E2E 使用的 `/api/test/reset` 和 `/api/test/pipeline-sample` 只有在启动参数包含 `--enable-test-reset` 时可用，生产部署不要开启。

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
