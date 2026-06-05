# 上位机需求缺口状态

本文档用于跟踪李祖祎负责的上位机范围与 PRD / 团队分工文档之间的当前实现状态。

剩余硬件、外部接口、LoRA/RK、生产安全、性能可靠性和用户验收的执行清单见 `docs/upper_computer_external_acceptance_checklist.md`。面向李祖祎汇报的短版缺口摘要见 `docs/upper_computer_current_gap_summary_for_lizuyi.md`。

## 已补齐

| 模块 | 当前证据 |
| --- | --- |
| Web HMI 本地运行 | `reactor-edge-daemon` 提供静态 HMI，当前本地监听 `127.0.0.1:8000` |
| 中英切换 | HMI 全局语言切换、动态 Modbus/MQTT/集成状态字块已做浏览器验证 |
| RBAC / 审计 | 本地 bearer session、角色权限、审计哈希链、CSV 导出；审计链状态默认校验最近 10,000 条 hash 事件并显式暴露窗口是否截断 |
| AINAS 任务下发 | `POST/GET /api/integrations/ainas/tasks`，支持 `set_targets`、`start_process`、`stop_process`，任务持久化 |
| MQTT 基础桥 | `config/integration.toml`，`rumqttc` MQTT 3.1.1 client，默认 TLS/8883，订阅 task topic，发布 receipt topic，按 `alert_interval_s` 发布 retained 报警快照，payload 复用 AINAS 安全执行路径；bridge 启动时同步刷新 `mqtt_status` 配置快照 |
| Modbus RTU/TCP 调试映射 | `/api/modbus/registers` 暴露 8 个读寄存器、7 个写寄存器、coils、discrete inputs；Modbus TCP PDU 支持 `01/02/03/06`；HTTP 调试写目标为 admin-only，并走安全夹紧和非空 reason 审计 |
| Modbus TCP TLS | `config/integration.toml` 支持 `tls_cert`/`tls_key`；本地自签证书 TLS 握手 + MBAP 读请求测试已通过 |
| AES-256 静态加密 | `XINGSHU_DB_ENCRYPTION_KEY` 启用 SQLite 集成任务请求/回执字段 AES-256-GCM 加密；`db_tests` 已验证原始列不含明文且旧明文行可兼容读取 |
| 密钥/敏感字段清单 | `docs/upper_computer_security_key_lifecycle.md` 已列出数据库加密 key、RBAC secret、角色密码、TLS/MQTT/Modbus 证书、StepFun key、CLI token 和本地 AI 资产路径 |
| 独立 safety guard | 新增 `reactor-safety-guard` 子进程和 stdin/stdout JSON 协议；`reactor-edge-daemon --safety-guard` 可委托自动控制安全判定，`xingshu safety check` 可本地验收夹紧结果；外部进程等待已改为 `wait-timeout` 超时等待并覆盖慢 guard 超时回归 |
| CLI 上位机入口 | `xingshu` CLI 覆盖启动、状态、配置、数据、控制、AI、审计、Modbus 调试 |
| 数据导出 | 批次 CSV/XLSX、单批次 Markdown 实验报告、审计 CSV；CSV/XLSX/Markdown 内容生成已集中到 `src/reports.rs`，XLSX 包装已改用 `zip` crate，导出测试会解包校验 workbook/sheet XML |
| 代码结构债务收敛 | `JsonBridgeDevice::write_component` 已拆分为直接命令、搅拌 RPM 和目标转换 helper；审计/批次 CSV、批次 XLSX 和 Markdown 报表生成已从 `src/api.rs` 拆到 `src/reports.rs`；Modbus 调试寄存器 map、HTTP 读写 payload 和 admin-only 调试写入审计已拆到 `src/modbus_registers.rs`；AINAS REST 任务和 MQTT 任务复用执行路径已拆到 `src/api_integrations.rs`；本地 bearer session 与 RBAC 权限策略已拆到 `src/api_auth.rs`；API 响应信封、错误脱敏和 JSON extractor 已拆到 `src/api_response.rs`；Recent 窗口查询统一为“先取最新 N 条窗口，再按 id 正序返回”，不再依赖 `DESC LIMIT + reverse()` |
| HTTP/HTTPS 入口 | `reactor-edge-daemon` 支持 `--tls-cert`/`--tls-key`，已用本地自签证书验证 `https://127.0.0.1:18443/health` |
| 无硬件实时监控演示 | `xingshu data sample --duration-s 180 --interval-ms 500` 通过正式 v1 样本入口保持 `/api/live` 新鲜，HMI 可显示 `SYSTEM HEALTH: NORMAL` |
| 本地性能/资源冒烟 | `xingshu perf smoke` 已生成 API/安全计算 p95 报告；`output/upper-computer-resource-snapshot.json` 记录 Windows debug 资源快照 |
| 本地交付 gate | `npm run acceptance:local-gate` 检查 `/health`、HMI shell/i18n 标记、三角色登录/RBAC、配置摘要、Modbus map、视觉 i18n 审计 JSON 和关键交付文档；报告归档到 `output/upper-computer-local-gate-20260606.json` |
| 正式交付文档 | 已新增上位机开发文档、用户手册、测试报告、第三方接口验收报告、API 验收手册、CLI 参考手册、维护手册、Modbus 映射手册、RK 部署验收指南、测试追踪矩阵、交付就绪索引和李祖祎短版缺口摘要；培训材料计划已补，PPT/视频成品仍待制作 |

## 部分完成

| 模块 | 已有能力 | 仍缺 |
| --- | --- | --- |
| Modbus TCP | 可配置 TCP/TLS server、MBAP/PDU 处理、`01/02/03/06` 功能码、寄存器/线圈/离散输入 map 已实现，已有本地真实 TCP/MBAP 与 TLS/MBAP 客户端测试，默认关闭 | 外部 Modbus Poll/Slave 联调、真实证书链验收 |
| MQTT | 配置、状态摘要、rumqttc client、任务执行、receipt 逻辑、alert topic 报警快照、CA/客户端证书配置字段和启动时状态同步已实现并测试 | 外部 broker 联调、断线重连验收、MQTT.fx 证书链验收 |
| 本地 AI | 本地传统优化器、AI 建议、云端 provider、StepFun 指数退避重试、StepFun 配置下的本地缓存推荐会标记为 `stale_local_recommendation`、CLI 显示 LoRA 缺口 | Qwen3.5-2B + LoRA 推理、训练、GGUF 转换、RK 端延迟验证 |
| 安全/非功能 | 控制范围、步长、急停、人工锁、传感器超时、RBAC、审计链、HTTP/HTTPS 入口、Modbus TCP TLS 本地回归、第三方任务载荷 AES-256-GCM 静态加密、密钥/敏感字段清单、独立 safety guard 本地验收、本地 API/安全计算性能冒烟和 Windows debug 资源快照 | MQTT/Modbus TCP 证书链外部验收、生产密钥托管/轮换演练、safety guard 生产 watchdog/权限隔离、release/RK 稳态 CPU/内存、正式安全测试报告 |

## 未完成

| 模块 | 缺口 |
| --- | --- |
| Modbus TCP 外部验收 | PRD 要求 Modbus TCP 加密；当前本地 TCP/TLS server 与 MBAP 测试可用，但 Modbus Poll/Slave、真实证书链和现场网络验收尚未完成 |
| 本地 LoRA 自进化 | 未集成 Qwen3.5-2B、PEFT/LoRA 训练、llama.cpp/GGUF 推理链路 |
| 安全交付验收 | 密钥生命周期和敏感字段清单已文档化；仍需生产密钥托管/轮换演练、safety guard 生产 watchdog/权限隔离/故障演练和正式渗透/漏洞扫描报告 |
| 长时间性能可靠性 | 本地短测已补；仍需 release/RK 稳态 CPU/内存、真实采集/执行控制延迟、7x24、MTBF、RS485 丢包率报告 |
| 兼容性验收 | 仍需用 MQTT.fx、Modbus Poll/Slave、第三方上位机系统进行实机/仿真验收 |
| 培训成品 | `docs/upper_computer_training_material_plan.md` 已给出 PPT 结构、视频脚本和培训记录模板；仍需实际 PPTX、MP4、签到和问题闭环记录 |

## 当前验证命令

```powershell
$env:CARGO_TARGET_DIR='C:\tmp\xingshu-target-bugfix'
cargo fmt --check
cargo test --all-targets -- --nocapture --test-threads=1
npm run acceptance:local-gate
```

本轮新增通过项：

- lib unit tests：6 passed，含 StepFun 指数退避和 MQTT 状态同步单测。
- `api_tests`：50 passed，含配置摘要 AES 状态断言、Modbus TCP TLS、Modbus admin-only 写入、Modbus reason 强制、MQTT TLS fail-closed、StepFun 配置下本地缓存推荐 stale 标识、流程回滚等回归。
- `cli_tests`：11 passed，含 safety guard、外部 guard 超时、AI SOP、perf help、数据样本入口等 CLI 覆盖。
- `config_tests`：6 passed。
- `control_tests`：5 passed。
- `db_tests`：8 passed，含 AES-256-GCM 加密落盘、旧明文兼容读取、审计链窗口校验和 Recent 窗口正序返回校验。
- `esp32_protocol_tests`：7 passed。
- `json_bridge_protocol_tests`：8 passed。
- `optimizer_tests`：4 passed。
- `npm run acceptance:local-gate`：7 passed，生成 `output/upper-computer-local-gate-20260606.json`。

本地补充验收：

- `xingshu perf smoke --iterations 20`：`output/upper-computer-perf-smoke.json`，API p95 最高 4ms，`safety_compute` p95=1ms。
- `npm run acceptance:local-gate`：只读检查本地服务、权限、配置、Modbus map、视觉 i18n 报告和交付文档，报告 `output/upper-computer-local-gate-20260606.json`。
- `xingshu data sample --duration-s 180 --interval-ms 500`：持续样本流下 `/api/live` 返回 200，HMI 显示实时温度/压力，截图 `output/upper-computer-hmi-live-sample-final.png`。
- Windows debug 资源快照：`output/upper-computer-resource-snapshot.json`，working set 26.977MB，CPU 5 秒采样 max 1.533%。

## 当前本地启动命令

```powershell
$env:CARGO_TARGET_DIR='C:\tmp\xingshu-target-bugfix'
$env:XINGSHU_DB_ENCRYPTION_KEY='0123456789abcdef0123456789abcdef'
cargo run --bin reactor-edge-daemon -- `
  --config config/device.toml `
  --safety config/safety.toml `
  --memory config/ai_memory.toml `
  --integration config/integration.toml `
  --db data/reactor.sqlite3 `
  --assets static `
  --bind 127.0.0.1:8000 `
  --safety-guard C:\tmp\xingshu-target-bugfix\debug\reactor-safety-guard.exe `
  --enable-test-reset
```

## 2026-06-04 Local AI/LoRA status addendum

This round turns the PRD's local Qwen3.5-2B + LoRA / GGUF / RK latency gap into an inspectable upper-computer status surface instead of a single generic `xingshu ai train` failure:

- Added `src/local_ai.rs` to check local AI asset boundaries from `XINGSHU_LOCAL_AI_ENABLED`, `XINGSHU_LOCAL_AI_BIN`, `XINGSHU_LOCAL_AI_GGUF`, `XINGSHU_LOCAL_AI_LORA`, `XINGSHU_LOCAL_AI_TRAIN_SCRIPT`, `XINGSHU_LOCAL_AI_CONVERT_SCRIPT`, and `XINGSHU_LOCAL_AI_RK_REPORT`.
- `/api/config/summary` now exposes `local_ai` with `ready_for_inference`, `ready_for_training`, `missing`, and separate inference / LoRA adapter / training / conversion / RK validation statuses.
- `xingshu ai model` prints the `local_ai` status; `xingshu ai train` still fails honestly and lists the missing local model/training assets.
- The HMI AI page, Settings page, and Modbus Integration Surface now show Local Model Boundary / Local Qwen LoRA status, with the new blocks wired into language switching.

Remaining gap: this does not claim real LoRA self-evolution is complete. Real Qwen3.5-2B weights, LoRA adapter, PEFT/LoRA training script, GGUF conversion script, and RK-side latency validation report still need to be supplied by the algorithm/hardware owners and then wired into this boundary.

HTTP TLS 验证示例：

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
