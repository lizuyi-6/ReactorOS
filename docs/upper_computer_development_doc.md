# 星宿智能反应釜上位机开发文档

适用范围：李祖祎负责的 RK 上位机软件，包括 Web HMI、REST API、CLI、Modbus 调试、AINAS/MQTT 第三方任务接入、本地数据存储、安全限幅和审计链。

参考文档：

- `C:/Users/Abraham/Downloads/星宿智能反应釜体系 (Xingshu Intelligent Reactor System) 产品需求文档 (PRD) v2.2.md`
- `C:/Users/Abraham/Downloads/星宿智能反应釜项目-团队分工&开发里程碑&DDL规划方案.docx`

## 1. 责任范围对照

团队分工文档中李祖祎负责：

- 七大页面：实时监控、参数配置、AI 智能决策、历史数据、审计日志、Modbus 调试、系统配置。
- 核心功能：工艺探索、手动/AI 自动双模式、Modbus 寄存器读写、曲线渲染、数据存储导出、日志审计。
- 通信对接：对接下位机 Modbus 协议，完成寄存器解析、指令下发、异常处理。
- 第三方对接：AINAS 远程对接、CLI 控制台、API/MQTT 协议适配。
- 自测交付：功能测试、边界测试、异常测试、测试报告、用户手册和开发文档。

当前实现已经覆盖上位机主体软件与接口框架，并已支持 HTTP/HTTPS 入口、Modbus TCP TLS 本地回归和第三方任务载荷 AES-256-GCM 静态加密；尚未完成的内容主要集中在真实硬件联调、MQTT/Modbus TCP 外部 TLS 验收、外部 MQTT/Modbus 工具验收、本地 Qwen3.5-2B + LoRA 链路。PRD 指定技术栈与当前工程实现的偏离口径见 `docs/architecture-deviations.md`。

## 2. 系统结构

```text
Web HMI / CLI / AINAS / MQTT / Modbus TCP
        |
        v
Axum REST API + static assets
        |
        v
Safety Guard + RBAC + audit chain
        |
        v
Runtime state + SQLite + control loop
        |
        v
ESP32 serial / JSON bridge / Modbus RTU map / external data pipeline
```

核心进程是 `reactor-edge-daemon`，可选通过 `--safety-guard` 调用独立 `reactor-safety-guard` 子进程做自动控制安全判定。主进程负责：

- 托管静态 Web HMI。
- 暴露 REST API 与 WebSocket。
- 维护运行状态、批次、产品结果、AI 推荐上下文和审计事件。
- 执行安全限幅、急停、人工锁定、传感器超时保护；自动控制环路可把判定委托给独立 safety guard 进程。
- 执行温度-转速组合禁区拦截；`config/safety.toml` 的 `forbidden_control_zones` 对自动控制、手动目标、AI 执行、AINAS、Modbus 调试写入、工艺步骤和 v1 控制入口均生效。
- 通过配置选择 ESP32 串口、JSON bridge、Modbus RTU 或测试管线数据源。
- 通过 `config/integration.toml` 管理 MQTT 与 Modbus TCP 第三方入口。

## 3. 源码模块

| 文件 | 作用 |
| --- | --- |
| `src/main.rs` | daemon 启动参数、配置加载、控制循环、HTTP 服务、MQTT/Modbus TCP 启动 |
| `src/api.rs` | REST API 路由挂载、批次、审计、配置摘要 |
| `src/api_integrations.rs` | AINAS REST 任务、MQTT 任务复用执行路径和第三方任务持久化回执 |
| `src/api_response.rs` | 统一 API 成功/错误响应信封、JSON 请求解析拒绝处理和内部错误脱敏 |
| `src/api_auth.rs` | 本地 bearer session、默认角色登录、RBAC 权限策略和权限 guard |
| `src/db.rs` | SQLite schema、批次/样本/审计/集成任务持久化、集成任务请求/回执 AES-256-GCM 加密；文件库已接入 SQLx SQLite pool，审计日志 total/list/chain、审计 CSV 导出读取、批次/产物结果 history、AI 推荐输入/缓存、实时曲线/v1 history/批次报告样本读取已开始走 SQLx，主体写入和多数业务读写仍在迁移中 |
| `src/config.rs` | 设备、寄存器、数据桥和硬件通信配置 |
| `src/control.rs` | 安全限幅、目标参数更新、控制循环逻辑、安全守护进程 JSON 协议 |
| `src/device.rs` | ESP32、JSON bridge、Modbus RTU 和管线设备适配；Modbus RTU 主站读写使用 `tokio-modbus` + `tokio-serial` |
| `src/modbus_registers.rs` | Modbus 调试寄存器 map、HTTP 读写 payload、admin-only 调试写入审计和安全校验复用 |
| `src/reports.rs` | 审计/批次 CSV、批次 XLSX 包和单批次 Markdown 实验报告生成；XLSX 包装使用 `zip` crate，不再维护手写 ZIP central directory / CRC32 |
| `src/mqtt.rs` | MQTT 3.1.1 bridge、任务订阅、receipt 发布、状态摘要 |
| `src/modbus_tcp.rs` | 自实现 Modbus TCP MBAP/PDU 处理、`01/02/03/06` 功能码、安全写入复用；是否切到 `tokio-modbus` server feature 仍待评估 |
| `src/optimizer.rs` | 本地 `local-ga-sa-pid` 参数寻优，结合 GA 交叉/变异、SA 接受/降温搜索和精英趋势校正 |
| `src/bin/xingshu.rs` | 上位机 CLI，复用 REST API 和 `src/api_auth.rs` 签发的 bearer token |
| `src/bin/reactor-safety-guard.rs` | 独立安全判定进程，stdin/stdout JSON 协议 |
| `static/index.html` | 单页 Web HMI、七大页面、中英切换、浏览器端交互 |
| `frontend/` | PRD 前端技术栈迁移工程，已接入 Vue 3、Vite、Element Plus、ECharts、Pinia 和 Vue Router；生产替换前仍需 parity 和视觉验收 |
| `tests/*.rs` | Rust 集成测试，覆盖 API、CLI、DB、配置、控制和协议 |

DB Recent/History 查询约定：实时样本、报警、批次、产物结果和审计事件类 Recent 接口先用 `ORDER BY id DESC LIMIT N` 限定“最新窗口”，再在外层按 `id ASC` 返回给 HMI/报告使用，保证用户看到的是窗口内从旧到新的时间线。PRD SQLx 技术栈迁移采用分段方式推进：当前文件数据库通过 SQLx SQLite pool 支撑 `/api/audit/logs` 的审计 total/list/chain 查询、`/api/audit/export.csv` 的导出读取，`live`、demo context、批次列表、批次 CSV/XLSX 导出和 AI 实验计划中的 Recent 批次/产物结果读取，AI 推荐输入的全量 batch outcomes 和推荐缓存读取，以及 `live` 实时曲线、v1 history、批次详情和批次 Markdown 报告的样本读取；内存测试库和多数持久化路径继续使用 `rusqlite` 作为兼容层，后续逐步扩大 SQLx 覆盖面。

## 4. 配置文件

| 文件 | 说明 |
| --- | --- |
| `config/device.toml` | 默认设备配置，含 Modbus 寄存器映射 |
| `config/device.esp32.toml` | ESP32 串口桥接配置 |
| `config/device.json_bridge.toml` | JSON 文件桥接配置 |
| `config/safety.toml` | 温度、转速、压力、步长和安全禁区 |
| `config/ai_memory.toml` | AI 记忆、参考批次、搜索边界和阈值 |
| `config/integration.toml` | MQTT、Modbus TCP、AINAS/第三方接口配置 |
| `docs/upper_computer_delivery_readiness_index.md` | PRD 第十章交付物和当前上位机证据的就绪索引 |
| `docs/upper_computer_cli_reference.md` | `xingshu` CLI 命令参考手册 |
| `docs/upper_computer_maintenance_manual.md` | 上位机维护手册，覆盖备份、恢复、日志、密钥、证书、升级回滚和常见故障 |
| `docs/upper_computer_api_acceptance_manual.md` | REST API、WebSocket、AINAS 和验收步骤手册 |
| `docs/upper_computer_modbus_register_map.md` | 上位机当前默认 Modbus 寄存器映射手册，正式交付前需与 STM32 最终手册对齐 |
| `docs/upper_computer_rk_deployment_acceptance_guide.md` | RK3568/RK3588 平台部署和验收取证指南 |
| `docs/upper_computer_external_acceptance_checklist.md` | 硬件、外部接口、LoRA/RK、生产安全、性能可靠性和用户验收执行清单 |
| `docs/upper_computer_test_plan_traceability.md` | PRD 第八章测试计划和团队分工测试职责追踪矩阵 |
| `docs/upper_computer_security_key_lifecycle.md` | 生产密钥生命周期、证书、token、敏感字段和轮换验收清单 |
| `docs/architecture-deviations.md` | PRD v2.2 技术栈、LoRA、安全进程、备份擦除、非功能验收和页面命名偏离说明 |

## 5. Web HMI 功能

当前生产 Web HMI 由 `static/index.html` 提供，支持中英切换。动态字块已经覆盖 Modbus/MQTT/集成状态等接口返回字段。`codex/prd-tech-stack-migration` 分支已启动 PRD 前端技术栈切换，`frontend/` 可构建 Vue 3 / Element Plus / ECharts / Pinia 首版迁移壳，但尚未替换生产静态资源。

| 页面 | 当前能力 |
| --- | --- |
| 实时监控 | 实时数值、曲线、设备状态、急停/锁定状态、当前目标 |
| 参数控制 | 目标温度、搅拌转速、自动控制、人工锁定、急停 |
| AI 智能决策 | 本地优化建议、云端 provider 状态、推荐上下文展示 |
| 历史数据 | 批次、产品结果、CSV/XLSX/Markdown 报告导出；XLSX 包结构由自动化测试解包校验 |
| 审计日志 | 审计链状态、事件列表、CSV 导出 |
| Modbus 调试 | 寄存器映射、读值、写入测试、集成接口状态 |
| 系统配置 | 设备、安全、AI、权限和集成摘要 |

## 6. REST API

主要接口见 `README.md` 的“主要 API”。上位机新增/关键接口如下：

```text
POST /api/auth/login
GET  /api/auth/me
GET  /api/config/summary
GET  /api/permissions/roles
GET  /api/ai/experiment-plan

GET  /api/live
GET  /api/v1/devices/status
POST /api/v1/reactor/:device_id/samples
GET  /api/v1/reactor/:device_id/realtime        # requires bearer token with monitor permission
GET  /api/v1/reactor/:device_id/history
POST /api/v1/reactor/:device_id/control
WS   /ws/v1/reactor/:device_id/realtime         # requires bearer token with monitor permission

POST /api/control/targets
POST /api/control/auto
POST /api/control/manual-lock
POST /api/control/emergency-stop
POST /api/processes/:id/start
POST /api/processes/:id/stop

GET  /api/audit/logs
GET  /api/audit/export.csv
GET  /api/recommendations/latest                # read cached latest recommendation only
POST /api/recommendations/latest                # generate and persist latest recommendation

POST /api/integrations/ainas/tasks
GET  /api/integrations/ainas/tasks
GET  /api/integrations/ainas/tasks/:id

GET  /api/modbus/registers
GET  /api/modbus/registers/:name/read
POST /api/modbus/registers/:name/write
```

写操作通过 RBAC bearer token 控制；控制类写入还会经过安全限幅和审计。

`GET /api/ai/experiment-plan` 是只读 AI 实验方案/SOP 草案接口。它复用当前缓存推荐、批次结果、当前 safety/optimizer 边界和本地 LoRA readiness 状态，输出三段式 heat/hold/cool 草案、验收指标、安全说明和模型边界说明。`GET /api/recommendations/latest` 只读取缓存；需要触发模型调用和推荐落库时使用 `POST /api/recommendations/latest`。当 StepFun provider 已配置但缓存推荐来自本地优化器时，GET 会返回 `provider.mode = "stale_local_recommendation"`，表示 AI 主控前必须重新生成 StepFun 推荐，而不是 StepFun 请求失败 fallback。该接口不会启动工艺、不会写目标、不会替代操作员复核；真实执行仍必须通过 AI master-control dry-run、RBAC、安全限幅和审计链。

本地推荐器 provider model 标识为 `local-ga-sa-pid`。当存在至少三条真实或参考批次结果时，推荐器会在安全 optimizer 边界内执行：

- GA 风格候选生成：精英批次种群、交叉、变异。
- SA 风格候选接受：按温度衰减接受更优或概率接受邻域候选。
- 精英趋势校正：向最佳批次、精英均值和历史参数变化方向做小步修正；`local-ga-sa-pid` 是兼容保留的 provider model 标识，不表示存在闭环 PID 控制器。

推荐输出仍会避开 `ai_memory.toml` 的 `forbidden_zones`，真实控制写入还会继续经过 `config/safety.toml` 的硬性 `forbidden_control_zones`。

## 7. AINAS 任务接口

AINAS REST 入口支持三类动作：

- `set_targets`
- `start_process`
- `stop_process`

示例：

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

执行路径：

```text
REST/MQTT payload -> integration task persisted -> action validation -> safety guard -> runtime write -> audit event -> task receipt
```

如设置环境变量 `XINGSHU_DB_ENCRYPTION_KEY`，上位机会把 `integration_tasks.request_json` 和 `integration_tasks.response_json` 以 AES-256-GCM 信封格式写入 SQLite。密钥支持 32 字节原文、64 位 hex 或 base64 编码。`GET /api/config/summary` 的 `data_security.storage_encryption` 会返回是否启用、算法和当前覆盖字段。旧版本明文任务行仍可读取，以便升级已有本地数据库。

## 8. MQTT Bridge

`config/integration.toml` 默认关闭 MQTT，生产/联调时启用：

```toml
[mqtt]
enabled = false
broker = "mqtts://broker.example.com:8883"
client_id = "xingshu-reactor-001"
task_topic = "xingshu/reactor_001/tasks"
receipt_topic = "xingshu/reactor_001/task_receipts"
alert_topic = "xingshu/reactor_001/alerts"
tls = true
```

当前已实现：

- MQTT 3.1.1 client。
- task topic 订阅。
- task payload 复用 AINAS 安全执行路径。
- receipt topic 发布执行结果。
- alert topic 按 `alert_interval_s` 发布 retained 报警快照。
- `/api/config/summary` 暴露状态摘要。
- `use_tls = true` 时必须配置非空 `ca_cert`，缺失时启动 MQTT TLS 连接会 fail-closed，不会隐式信任系统根证书。

仍需外部 broker 验收、断线重连验收和 MQTT.fx/生产证书链测试。

## 9. Modbus 映射

`/api/modbus/registers` 暴露当前上位机调试映射：

- 8 个读寄存器：`temperature_c`、`stirrer_rpm`、`pressure_mpa`、`shake_speed_cpm`、`tilt_angle_deg`、`flow_rate_l_min`、`product_concentration_percent`、`ph`。
- 7 个写寄存器：`target_temperature_c`、`target_stirrer_rpm`、`target_shake_speed_cpm`、`target_pressure_mpa`、`heat_time_s`、`hold_time_s`、`cool_time_s`。
- coils/discrete inputs 表达运行状态、自动控制、急停、人工锁定、传感器新鲜度等布尔点位。

Modbus TCP PDU 当前支持：

| 功能码 | 功能 |
| --- | --- |
| `01` | Read Coils |
| `02` | Read Discrete Inputs |
| `03` | Read Holding Registers |
| `06` | Write Single Holding Register |

默认配置要求 TLS，`config/integration.toml` 可配置 `tls_cert` 和 `tls_key`。实验室可信网络可以将 `require_tls=false` 并改用非特权端口联调。

自动化测试已覆盖函数级 PDU 处理、本地真实 TCP/MBAP 客户端读请求、本地 TLS 握手 + MBAP 读请求；外部 Modbus Poll/Slave 验收仍需在联调环境中执行。

## 10. CLI

上位机 CLI 二进制名为 `xingshu`，复用 REST API：

```powershell
cargo run --bin xingshu -- --help
cargo run --bin xingshu -- status
cargo run --bin xingshu -- config --local --json
cargo run --bin xingshu -- modbus map
cargo run --bin xingshu -- data sample --duration-s 180 --interval-ms 500
cargo run --bin xingshu -- data delete --yes
cargo run --bin xingshu -- ai train
cargo run --bin xingshu -- perf smoke --iterations 20 --json
```

`xingshu ai train` 当前明确返回 LoRA 训练接口尚未暴露，用于把 PRD 的本地自进化缺口显式暴露给验收人员。

`xingshu perf smoke` 会测量本机只读 API 往返和安全计算耗时，并输出 p50/p95/max。该命令默认不写控制目标、不启动工艺；`safety_guard_process_spawn` 仅作为独立进程启动/JSON 往返诊断项，不用于替代真实硬件控制延迟验收。

`xingshu data sample` 通过正式 `/api/v1/reactor/:device_id/samples` 外部样本入口注入演示数据，不写控制目标、不绕过 safety。无硬件本地演示时建议使用 `--duration-s 180 --interval-ms 500` 保持样本新鲜；当前 `sensor_timeout_ms=6000`，单条样本超过 6 秒后 `/api/live` 会按预期返回 503。

## 11. 本地运行

```powershell
$env:CARGO_TARGET_DIR='C:\tmp\xingshu-target-bugfix'
cargo run --bin reactor-edge-daemon -- `
  --config config/device.toml `
  --safety config/safety.toml `
  --memory config/ai_memory.toml `
  --integration config/integration.toml `
  --db data/reactor.sqlite3 `
  --assets static `
  --bind 127.0.0.1:8000 `
  --enable-test-reset
```

打开：

```text
http://127.0.0.1:8000/
```

无硬件实时监控演示：

```powershell
cargo run --bin xingshu -- data sample --duration-s 180 --interval-ms 500
```

该命令需要在打开 HMI 前或同时运行；停止样本流后，实时监控会在 `sensor_timeout_ms` 后恢复为 pipeline stale/503，这是安全新鲜度检查的预期行为。

HTTP TLS/HTTPS 本地验证可同时传入证书和私钥：

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

`--tls-cert` 和 `--tls-key` 必须成对提供。已用本地自签证书验证 `https://127.0.0.1:18443/health`。

## 12. 测试入口

```powershell
cargo fmt --check
cargo test --all-targets -- --nocapture --test-threads=1
npm run acceptance:local-gate
```

性能冒烟：

```powershell
cargo run --bin xingshu -- --json perf smoke `
  --iterations 20 `
  --api-threshold-ms 100 `
  --safety-threshold-ms 100
```

最近一次全量 Rust 回归结果：

- lib unit tests: 6 passed。
- `api_tests`: 50 passed。
- `cli_tests`: 11 passed。
- `config_tests`: 6 passed。
- `control_tests`: 5 passed。
- `db_tests`: 8 passed。
- `esp32_protocol_tests`: 7 passed。
- `json_bridge_protocol_tests`: 8 passed。
- `optimizer_tests`: 4 passed。
- 本地交付 gate: 7 passed，报告 `output/upper-computer-local-gate-20260606.json`。

## 13. 已知缺口

| 缺口 | 说明 |
| --- | --- |
| 本地 LoRA | 尚未集成 Qwen3.5-2B、PEFT/LoRA 训练、GGUF 转换和 RK 端延迟验证 |
| TLS/证书 | HTTP/HTTPS 入口、Modbus TCP over TLS 已本地验证；MQTT 证书链和外部工具 TLS 验收未完成 |
| AES-256 / 密钥 | 集成任务请求/回执字段已支持 AES-256-GCM 静态加密并完成本地测试；密钥生命周期和敏感字段清单见 `docs/upper_computer_security_key_lifecycle.md`；生产密钥托管、轮换演练和签字验收仍未完成 |
| 独立安全进程 | `reactor-safety-guard` 已支持独立进程 JSON 判定，daemon 可通过 `--safety-guard` 委托自动控制安全决策；外部进程等待使用 `wait-timeout` 超时等待并在超时后 kill 子进程；生产部署 watchdog、权限隔离和故障演练仍需验收 |
| 外部工具验收 | MQTT.fx、Modbus Poll/Slave、第三方上位机系统联调未完成 |
| 真实硬件联调 | 需要等待 STM32/硬件侧寄存器和实机状态稳定后做整机验收 |
