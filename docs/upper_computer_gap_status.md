# 上位机需求缺口状态

本文档用于跟踪李祖祎负责的上位机范围与 PRD / 团队分工文档之间的当前实现状态。

剩余硬件、外部接口、LoRA/RK、生产安全、性能可靠性和用户验收的执行清单见 `docs/upper_computer_external_acceptance_checklist.md`。面向李祖祎汇报的短版缺口摘要见 `docs/upper_computer_current_gap_summary_for_lizuyi.md`。

## 已补齐

| 模块 | 当前证据 |
| --- | --- |
| Web HMI 本地运行 | `reactor-edge-daemon --assets auto` 默认优先提供 `frontend/dist` Vue HMI，缺少 Vue 构建时回退到 legacy `static`，当前本地监听 `127.0.0.1:8000` |
| 中英切换 | HMI 全局语言切换、动态 Modbus/MQTT/集成状态字块已做浏览器验证；手机和平板 Chromium 视口已覆盖七路由中英导航、标题、滚动和横向溢出 |
| RBAC / 审计 | 本地 bearer session、角色权限、审计哈希链、CSV 导出；审计链状态默认校验最近 10,000 条 hash 事件并显式暴露窗口是否截断 |
| AINAS 任务下发 | `POST/GET /api/integrations/ainas/tasks`，支持 `set_targets`、`start_process`、`stop_process`，任务持久化 |
| MQTT 基础桥 | `config/integration.toml`，`rumqttc` MQTT 3.1.1 client，默认 TLS/8883，订阅 task topic，发布 receipt topic，按 `alert_interval_s` 发布 retained 报警快照，payload 复用 AINAS 安全执行路径；bridge 启动时同步刷新 `mqtt_status` 配置快照 |
| Modbus RTU/TCP 调试映射 | `/api/modbus/registers` 暴露 8 个读寄存器、7 个写寄存器、coils、discrete inputs；Modbus TCP PDU 支持 `01/02/03/06`；HTTP 调试写目标为 admin-only，并走安全夹紧和非空 reason 审计 |
| Modbus TCP TLS | `config/integration.toml` 支持 `tls_cert`/`tls_key`；本地自签证书 TLS 握手 + MBAP 读请求测试已通过 |
| AES-256 静态加密 | `XINGSHU_DB_ENCRYPTION_KEY` 启用 SQLite 集成任务请求/回执字段 AES-256-GCM 加密；`db_tests` 已验证原始列不含明文且旧明文行可兼容读取 |
| 密钥/敏感字段清单 | `docs/upper_computer_security_key_lifecycle.md` 已列出数据库加密 key、RBAC secret、角色密码、TLS/MQTT/Modbus 证书、StepFun key、CLI token 和本地 AI 资产路径 |
| 生产预检 gate | `xingshu ops preflight --production` 已实现，检查本地配置解析、默认口令/session secret、`XINGSHU_DB_ENCRYPTION_KEY`、MQTT/Modbus TLS 文件和备份 service/timer/script；默认口令或缺 key 会非 0 失败，已纳入一键验收 |
| 独立 safety guard | 新增 `reactor-safety-guard` 子进程和 stdin/stdout JSON 协议；`reactor-edge-daemon --safety-guard` 可委托自动控制安全判定，`xingshu safety check` 可本地验收夹紧结果；外部进程等待已改为 `wait-timeout` 超时等待并覆盖慢 guard 超时回归；ARM64 package、`run.sh` 和 systemd service 已默认携带并启用 guard，`scripts/verify-production-safety-guard.mjs` 已纳入一键验收 |
| 自动备份和 A/B 发布路径 | `install.sh` 会在停服务前校验 release 包完整性，缺二进制、OTA/备份脚本、unit、配置、build metadata 或 HMI 资源时先失败；`xingshu ops backup` 使用 SQLite `VACUUM INTO` 在线快照；release package 已包含 `reactor-edge-backup.service`、`reactor-edge-backup.timer` 和 `/opt/reactor-edge/current/backup.sh`，默认每日先用非阻塞锁串行化 timer/OTA 前备份，再写临时快照，确认非空、sha256 sidecar 校验通过且具备 SQLite header 后才发布为时间戳快照并更新 `latest.snapshot`；应用发布已改为 `/opt/reactor-edge/slots/{a,b}` + `current/previous`，`ota-update.sh` 覆盖板端命令预检、checksum sidecar 包名绑定校验、健康检查参数校验、tar 成员安全校验、managed slot 链接校验、状态不可证实时 fail-closed、忙碌/急停拦截、磁盘空间预检、`--dry-run` 不切槽预检、切槽前拒绝坏包/不安全现场时记录 `rejected_before_switch`、`BUILD-METADATA.properties` 构建追溯、更新前备份、失败 staging 清理、关键状态/slot/链接写入后 `sync` 落盘、开机 boot-check 对切槽前中断保留 current、切槽后中断恢复 previous、backend 每次启动前重跑 boot-check、backend/kiosk 启动限流、健康失败自动回滚和 OTA 状态日志，进入 failed 时清除健康检查临时放行并停止生产服务；`scripts/verify-install-board-preflight.sh`、`scripts/verify-production-backup-schedule.mjs`、`scripts/verify-production-backup-script.ps1`、`scripts/verify-ota-ab-release-path.mjs`、`scripts/verify-ota-systemd-boot-gate.mjs`、`scripts/verify-ota-tar-safety.sh`、`scripts/verify-ota-busy-state.sh`、`scripts/verify-ota-input-guards.sh`、`scripts/verify-ota-pre-switch-rejection.sh`、`scripts/verify-ota-cleanup.sh`、`scripts/verify-ota-slot-integrity.sh`、`scripts/verify-ota-command-preflight.sh`、`scripts/verify-ota-durability-sync.sh`、`scripts/verify-ota-boot-check.sh`、`scripts/verify-ota-failed-state.sh` 和 `scripts/verify-ota-dry-run.sh` 已纳入一键验收 |
| 本地备份恢复演练 | `scripts/verify-backup-restore-drill.ps1` 启动临时 daemon 写入真实批次/产品结果/审计事件，执行 `xingshu ops backup`，停 daemon 后恢复到新库，再用恢复库重启 daemon 校验 `/health`、批次详情、产品结果和审计链窗口；已纳入一键验收 |
| CLI 上位机入口 | `xingshu` CLI 覆盖启动、状态、配置、数据、控制、AI、审计、Modbus 调试 |
| 数据导出 | 批次 CSV/XLSX、单批次 Markdown 实验报告、审计 CSV；CSV/XLSX/Markdown 内容生成已集中到 `src/reports.rs`，XLSX 包装已改用 `zip` crate，导出测试会解包校验 workbook/sheet XML |
| 代码结构债务收敛 | `JsonBridgeDevice::write_component` 已拆分为直接命令、搅拌 RPM 和目标转换 helper；审计/批次 CSV、批次 XLSX 和 Markdown 报表生成已从 `src/api.rs` 拆到 `src/reports.rs`；Modbus 调试寄存器 map、HTTP 读写 payload 和 admin-only 调试写入审计已拆到 `src/modbus_registers.rs`；AINAS REST 任务和 MQTT 任务复用执行路径已拆到 `src/api_integrations.rs`；本地 bearer session 与 RBAC 权限策略已拆到 `src/api_auth.rs`；API 响应信封、错误脱敏和 JSON extractor 已拆到 `src/api_response.rs`；Recent 窗口查询统一为“先取最新 N 条窗口，再按 id 正序返回”，不再依赖 `DESC LIMIT + reverse()` |
| HTTP/HTTPS 入口 | `reactor-edge-daemon` 支持 `--tls-cert`/`--tls-key`，已用本地自签证书验证 `https://127.0.0.1:18443/health` |
| 无硬件实时监控演示 | `xingshu --token <engineer-token> data sample --duration-s 180 --interval-ms 500` 通过正式 v1 样本入口保持 `/api/live` 新鲜；token 需具备 `ingest_sensor_sample` 权限；生产严格模式下若没有下位机状态证明，HMI 应显示设备 offline/高危报警，不能只靠样本流显示 `SYSTEM HEALTH: NORMAL` |
| 本地性能/资源冒烟 | `xingshu perf smoke` 已生成 API/安全计算 p95 报告；`output/upper-computer-resource-snapshot.json` 记录 Windows debug 资源快照 |
| 本地交付 gate | `npm run acceptance:local-gate` 检查 `/health`、HMI shell/i18n 标记、三角色登录/RBAC、配置摘要、Modbus map、视觉 i18n 审计 JSON 和关键交付文档；2026-06-07 聚焦复核已将 Vue release shell 的 `Integration Surface`、`Base inference`、`PRD LoRA/RK` 标记和 `local_ai.ready_for_base_inference`、`ready_for_lora_inference`、`ready_for_training`、`ready_for_prd_lora` 纳入检查；`scripts/verify-vue-release-assets.mjs` 检查 release/package/systemd/QEMU 默认优先 Vue dist 且保留 legacy fallback；`scripts/verify-production-safety-guard.mjs` 检查生产 safety guard；`scripts/verify-production-backup-schedule.mjs` 和 `scripts/verify-production-backup-script.ps1` 检查自动备份发布与脚本行为；`scripts/package-upper-computer-delivery.mjs` 生成本地草稿交付包；`scripts/verify-training-deliverables.mjs` 检查培训课件源稿、16 页 PPTX、图片资产、UAT 脚本、签到模板、现场交付执行包、video storyboard、静音 MP4 草稿、本地交付包 manifest 和 16 张预览图；`scripts/verify-vue-mobile.mjs` 检查手机/平板视口 HMI 中英导航、布局和 Modbus 集成/LoRA readiness 字段；`xingshu ops preflight --production` 检查生产密钥/口令/TLS 路径/备份 timer 文件；报告归档到 `output/local-run/upper-computer-local-gate-20260607.json`、`output/upper-computer-local-gate-20260606.json`、`output/acceptance/training-deliverables-report.json`、`output/acceptance/field-delivery-local-draft/` 和 `output/acceptance/` |
| 正式交付文档 | 已新增上位机开发文档、用户手册、测试报告、第三方接口验收报告、API 验收手册、CLI 参考手册、维护手册、Modbus 映射手册、RK 部署验收指南、测试追踪矩阵、交付就绪索引和李祖祎短版缺口摘要；培训材料计划、培训课件源稿、16 页可编辑 PPTX 草稿、静音 MP4 课件轮播草稿、本地草稿交付包、现场证据签收清单、用户验收操作脚本、培训签到与问题闭环模板已补，现场最终版 PPTX、真实操作录屏 MP4 和真实签字仍待制作/执行 |

## 部分完成

| 模块 | 已有能力 | 仍缺 |
| --- | --- | --- |
| Modbus TCP | 可配置 TCP/TLS server、MBAP/PDU 处理、`01/02/03/06` 功能码、寄存器/线圈/离散输入 map 已实现，已有本地真实 TCP/MBAP 与 TLS/MBAP 客户端测试，默认关闭 | 外部 Modbus Poll/Slave 联调、真实证书链验收 |
| MQTT | 配置、状态摘要、rumqttc client、任务执行、receipt 逻辑、alert topic 报警快照、CA/客户端证书配置字段和启动时状态同步已实现并测试 | 外部 broker 联调、断线重连验收、MQTT.fx 证书链验收 |
| 本地 AI | 本地传统优化器、AI 建议、云端 provider、StepFun 指数退避重试、StepFun 配置下的本地缓存推荐会标记为 `stale_local_recommendation`、local_ai readiness、HTTP/命令式推理入口、`xingshu ai train` 数据集导出/训练编排/manifest/显式候选 adapter 晋级备份 | Qwen3.5-2B + LoRA 真实模型资产、生产训练脚本、GGUF 转换验收、RK 端延迟验证、自动触发/审批流 |
| 安全/非功能 | 控制范围、步长、急停、人工锁、传感器超时、RBAC、审计链、HTTP/HTTPS 入口、Modbus TCP TLS 本地回归、第三方任务载荷 AES-256-GCM 静态加密、密钥/敏感字段清单、生产预检 gate、独立 safety guard 本地验收和生产发布默认启用、自动备份 timer 发布路径、本地 API/安全计算性能冒烟和 Windows debug 资源快照 | MQTT/Modbus TCP 证书链外部验收、生产密钥托管/轮换演练、safety guard 现场 watchdog/低权限账号/故障注入演练、备份恢复/异地归档演练、release/RK 稳态 CPU/内存、正式安全测试报告 |

## 未完成

| 模块 | 缺口 |
| --- | --- |
| Modbus TCP 外部验收 | PRD 要求 Modbus TCP 加密；当前本地 TCP/TLS server 与 MBAP 测试可用，但 Modbus Poll/Slave、真实证书链和现场网络验收尚未完成 |
| 本地 LoRA 自进化 | 上位机已具备训练数据集导出、训练入口编排、manifest 和显式候选 adapter 晋级/备份；仍缺真实 Qwen3.5-2B/GGUF、生产 PEFT/LoRA 训练脚本、llama.cpp/GGUF 推理验收、自动触发/审批流和 RK 延迟报告 |
| 安全交付验收 | 密钥生命周期和敏感字段清单已文档化，`xingshu ops preflight --production` 已能阻断默认口令/缺 key/缺 TLS 文件等本地上线风险；safety guard 生产发布路径已默认启用；仍需生产密钥托管/轮换演练、safety guard 现场 watchdog/低权限账号/故障演练和正式渗透/漏洞扫描报告 |
| 备份恢复验收 | `VACUUM INTO` 在线备份、release timer、backup script、sha256 sidecar、latest link 和本地 daemon 重启恢复演练已实现；仍需现场/RK 恢复演练、备份保留策略和异地归档验收 |
| 长时间性能可靠性 | 本地短测已补；仍需 release/RK 稳态 CPU/内存、真实采集/执行控制延迟、7x24、MTBF、RS485 丢包率报告 |
| 兼容性验收 | 仍需用 MQTT.fx、Modbus Poll/Slave、第三方上位机系统进行实机/仿真验收 |
| 培训成品 | `docs/upper_computer_training_material_plan.md` 已给出制作计划；`docs/upper_computer_training_deck.md` 已给出 16 页课件源稿；`docs/upper_computer_training_deck.pptx` 已生成 16 页可编辑 PPTX 草稿；`docs/upper_computer_training_video_storyboard.md` 已给出视频分镜；`outputs/manual-20260607-training/video/upper_computer_training_video_draft.mp4` 已生成静音课件轮播草稿；`docs/upper_computer_user_acceptance_script.md` 已给出 16 项 UAT 操作脚本；`docs/upper_computer_training_attendance_and_issues.md` 已给出签到和问题闭环模板；仍需现场最终版 PPTX、真实操作录屏 MP4、培训签到、验收执行、问题闭环和签字记录 |

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
- `npm run acceptance:local-gate`：7 passed，历史报告 `output/upper-computer-local-gate-20260606.json`。

2026-06-07 LoRA/readiness 聚焦复核：

- `cargo fmt --check`：通过。
- `node --check scripts/upper-computer-local-gate.mjs`：通过。
- `npm run frontend:build`：通过。
- `cargo test local_ai --lib -- --nocapture`（`CARGO_TARGET_DIR=C:\tmp\xingshu-target-local-ai`）：7 passed。
- `cargo test --test api_tests upper_computer_supports_audit_config_and_modbus_debug_pages -- --nocapture`（`CARGO_TARGET_DIR=C:\tmp\xingshu-target-local-ai`）：通过。
- `node scripts/upper-computer-local-gate.mjs --url http://127.0.0.1:18098 --out-dir output/local-run`：7 passed，生成 `output/local-run/upper-computer-local-gate-20260607.json`。

本地补充验收：

- `xingshu perf smoke --iterations 20`：`output/upper-computer-perf-smoke.json`，API p95 最高 4ms，`safety_compute` p95=1ms。
- `npm run acceptance:local-gate`：只读检查本地服务、权限、配置、Modbus map、视觉 i18n 报告、交付文档和 local_ai readiness 边界，最新报告 `output/local-run/upper-computer-local-gate-20260607.json`。
- `xingshu --token <engineer-token> data sample --duration-s 180 --interval-ms 500`：持续样本流下 `/api/live` 返回 200，HMI 可显示实时温度/压力；生产严格模式若没有下位机状态证明，应同时显示设备 offline 和 `device_status_unavailable` 高危报警，截图 `output/upper-computer-hmi-live-sample-final.png` 只代表样本流展示链路。
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
  --assets auto `
  --bind 127.0.0.1:8000 `
  --safety-guard C:\tmp\xingshu-target-bugfix\debug\reactor-safety-guard.exe `
  --enable-test-reset
```

## 2026-06-06 Local AI/LoRA status addendum

This round turns the PRD's local Qwen3.5-2B + LoRA / GGUF / RK latency gap into an inspectable upper-computer status surface instead of a single generic `xingshu ai train` failure:

- Added `src/local_ai.rs` to check local AI asset boundaries from `XINGSHU_LOCAL_AI_ENABLED`, `XINGSHU_LOCAL_AI_BIN`, `XINGSHU_LOCAL_AI_GGUF`, `XINGSHU_LOCAL_AI_LORA`, `XINGSHU_LOCAL_AI_TRAIN_SCRIPT`, `XINGSHU_LOCAL_AI_CONVERT_SCRIPT`, and `XINGSHU_LOCAL_AI_RK_REPORT`.
- `/api/config/summary` now exposes `local_ai` with `ready_for_base_inference`, `ready_for_lora_inference`, compatibility `ready_for_inference`, `ready_for_training`, `ready_for_prd_lora`, `missing`, and separate inference / LoRA adapter / training / conversion / RK validation statuses.
- `xingshu ai model` prints the `local_ai` status.
- `xingshu ai train --export-only` writes supervised JSONL from real SQLite batches, product results, samples, and audit events.
- `xingshu ai train --manifest ...` records training stdout metadata, parsed metrics, candidate adapter path, and promotion decision.
- `xingshu ai train --promote --min-eval-score ...` refuses unsafe promotion and only copies a candidate adapter into `XINGSHU_LOCAL_AI_LORA` after an explicit request, a readable candidate file, and a passing score; it backs up the previous adapter for rollback.
- The HMI AI page, Settings page, and Modbus Integration Surface now show Local Model Boundary / Local Qwen LoRA status, with the new blocks wired into language switching.

Remaining gap: this does not claim real LoRA self-evolution is complete. Real Qwen3.5-2B weights, production LoRA adapter, production PEFT/LoRA training script, GGUF conversion validation, RK-side latency validation report, automatic trigger, and production approval/evaluation policy still need to be supplied by the algorithm/hardware owners and then wired into this boundary.

HTTP TLS 验证示例：

```powershell
cargo run --bin reactor-edge-daemon -- `
  --config config/device.toml `
  --safety config/safety.toml `
  --memory config/ai_memory.toml `
  --integration config/integration.toml `
  --db data/reactor-tls-test.sqlite3 `
  --assets auto `
  --bind 127.0.0.1:18443 `
  --tls-cert output/tls-test/server.crt `
  --tls-key output/tls-test/server.key `
  --enable-test-reset
```
