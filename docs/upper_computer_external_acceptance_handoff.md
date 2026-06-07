# 上位机外部联调前置条件与跟踪表

本文档是李祖祎（上位机负责人）把"上位机侧已就绪"的内容汇总成的对外
handoff。每项 PENDING-EXTERNAL 任务都明确：上位机能提供什么、对方
需要做什么、验收命令。

## 1. 跟踪表

| # | 项目 | 上位机侧已就绪 | 对接方 | 上位机可验收命令 | 状态 |
|---|---|---|---|---|---|
| 1 | STM32 RTU 寄存器联调 | `/api/modbus/registers`、`/api/modbus/registers/:name/{read,write}`、`xingshu modbus {map,read,write}` 已实现 | 王天宇 | `xingshu modbus map` + `xingshu modbus read --register target_temperature_c` | PENDING-EXTERNAL (需 STM32 实机 + Modbus Poll) |
| 2 | Modbus TCP over TLS | `verify-modbus-tls.ps1` 自检脚本 + `Config::modbus_tcp_status` 已暴露 `tls_status` 字段 | 王天宇 | `pwsh scripts/verify-modbus-tls.ps1 -Host <ip> -Port 502 -Cert <pem>` | PENDING-EXTERNAL (需 STM32 / Modbus Slave 工具) |
| 3 | Modbus 故障注入 | `scripts/modbus-fault-proxy.mjs` 透明 TCP 代理，支持 loss/delay/bit-flip 注入 | 王天宇 | `node scripts/modbus-fault-proxy.mjs --listen 0.0.0.0:5502 --upstream <ip>:502 --loss-pct 5 --delay-ms 50` | READY (上位机交付，需对方跑联调) |
| 4 | AINAS 远程任务回执 | `POST /api/integrations/ainas/tasks` + AES-256-GCM 信封 + `mqtt.rs` 发布 receipt 主题 | 闫露 / 邓琪曦 | `node scripts/verify-ainas-mqtt.mjs` | PENDING-EXTERNAL (需 AINAS 真实回执回调) |
| 5 | MQTT TLS 接 mosquitto | `mqtt.rs` 已支持 `use_tls=true` + 客户端/CA 证书 + `ca_cert_configured` 状态 | 王天宇 | `mosquitto_sub -h <ip> -p 8883 --cafile ca.pem --cert client.pem --key client.key -t 'xingshu/reactor_001/tasks'` | PENDING-EXTERNAL (需 mosquitto broker) |
| 6 | 本地 Qwen3.5-2B LoRA 接入 | `local_ai.rs` 已区分基础模型入口、LoRA 推理、训练和 PRD LoRA/RK 证据；HMI AI/Settings/Modbus 均显示 readiness | 闫露 | `xingshu ai model` + `curl -H "Authorization: Bearer $T" -X POST http://127.0.0.1:8000/api/recommendations/latest` | PENDING-EXTERNAL (需 Qwen3.5-2B GGUF + LoRA adapter + 推理服务 + RK 报告) |
| 7 | 自进化增量训练 | 已具备 `xingshu ai train --export-only` 数据集导出、训练入口编排、manifest、显式候选 adapter 晋级和备份边界 | 闫露 | `xingshu ai train --export-only --dataset lora.jsonl`；生产脚本到位后跑 `xingshu ai train --dataset lora.jsonl --manifest train.manifest.json --promote --min-eval-score 0.8` | PENDING-EXTERNAL (需生产 PEFT 训练脚本、评估集、自动触发/审批策略和 RK 验收) |
| 8 | 防爆认证 | 上位机不直接处理；硬件侧需取得防爆合格证 | 王天宇 | N/A | PENDING-EXTERNAL (需第三方测试机构) |
| 9 | 物理急停信号链路 | 上位机通过 Modbus / JSON bridge 接收 `emergency_stop=true` 样本；`safety.toml` 已配 `sensor_timeout_ms=6000` | 王天宇 | `xingshu control estop` (软件) + 万用表测硬件 (物理) | PENDING-EXTERNAL (需安全继电器 / 安全 PLC 接入) |
| 10 | iptables / VPN 部署 | `upper_computer_production_operations.md` 已给配置模板 | SRE / 邓琪曦对接 | `iptables -L -n -v` | PENDING-EXTERNAL (需部署环境) |
| 11 | 7x24 长稳态 | `xingshu perf smoke` 已能给 p50/p95 | 团队 | `xingshu perf smoke --iterations 1000` 跑 30 天 | PENDING-EXTERNAL (需长期跑) |
| 12 | 生产密钥轮换 | `xingshu key generate` + `xingshu key rekey-integration-tasks` 已实现；rekey 事务式迁移 `integration_tasks.request_json/response_json`，并把历史明文行一并加密 | 团队 | `xingshu key rekey-integration-tasks --db /opt/xingshu/data/reactor.sqlite3 --old-key-file old.env --new-key-file reactor.key --dry-run` 后改 `--yes` | PARTIAL (本地工具和回归测试已交付；仍需生产 key 托管、停机演练、恢复验证和签字) |
| 13 | 备份/恢复 | `xingshu ops backup` 已使用 SQLite `VACUUM INTO` 在线快照；release 包含 `reactor-edge-backup.service/.timer` 和 `/opt/reactor-edge/backup.sh`，每日生成时间戳快照、sha256 和 `latest.snapshot`；restore 校验 SQLite magic header 和 integrity_check | SRE | `systemctl start reactor-edge-backup.service` + `xingshu ops restore ... --yes` | PARTIAL (自动备份发布路径已交付；仍需现场恢复演练、保留策略和异地归档验收) |
| 14 | RBAC 真实登录 | `Permission::ApplyIntegrationTask` 已新增并仅 engineer/admin 拥有；`create_ainas_task` 立即 require_permission(ApplyIntegrationTask) 后再做 action-specific 校验；operator 调 `/api/integrations/ainas/tasks` 稳定返回 403。`verify-load-and-rbac.ps1` 严格通过条件已收紧。`login` + `auth/me` 已暴露 | 团队 | `pwsh scripts/verify-load-and-rbac.ps1`（期望 operator→ainas = 401/403，否则脚本 exit 1） | PARTIAL (operator/engineer/admin 三角色基本矩阵 + AINAS 集成路径 403 已实装；modbus TCP / 系统配置 / 删除数据等细粒度权限矩阵仍待补齐) |
| 15 | TLS 1.3 证书链 | `axum-server` + `rustls` 已用 | 团队 / CA | `openssl s_client -connect <ip>:8443 -tls1_3` | PENDING-EXTERNAL (需正式 CA 签发) |
| 16 | STM32 Modbus 寄存器映射确认 | `docs/upper_computer_modbus_register_map.md` 草案已写 | 王天宇 | N/A | PENDING-EXTERNAL (需 STM32 手册最终版) |
| 17 | 工艺探索 / 7 大页面 / 中英切换 | Vue + 静态双版本已交付；`verify-vue-parity.mjs`（七页中英 + 横向溢出 + 缺文案必 fail）、`verify-vue-process-lifecycle.mjs`（工艺生命周期）和 `verify-vue-history-xlsx.mjs`（History CSV/XLSX 下载）通过；`verify-load-and-rbac.ps1` 仍受 RBAC PARTIAL 项影响（见 #14） | 团队 | `node scripts/verify-vue-parity.mjs` + `node scripts/verify-vue-process-lifecycle.mjs` + `node scripts/verify-vue-history-xlsx.mjs` | PARTIAL (Vue 7 页面 + 中英 + 工艺 + Modbus 调试 + 审计 + History CSV/XLSX 下载已交付；设置多卡视觉 / 物料页未到 PRD 完成度) |
| 18 | 报告生成 | `xingshu data export / report` + Vue `/#/history` 已交付 | 团队 | `xingshu data export --out /tmp/batches.csv` | READY (上位机交付) |

## 2. 联调前置清单（上位机能直接提供什么）

### 2.1 接口面

- REST（200+ 端点）：`/api/v1/*` 走 v1 协议（设备/sample/control），`/api/*` 走应用协议（auth/audit/config/process/batches/ainas/modbus/recommendations/ai/permissions/control/emergency-stop）。
- WebSocket：`/ws/v1/reactor/:device_id/realtime`（需 bearer token）。
- Modbus RTU 主站：`tokio-modbus + tokio-serial`，在 `config/device.toml` 配 `serial.port` / `unit_id`。
- Modbus TCP server：自实现 MBAP/PDU，支持 01/02/03/06，绑定 0.0.0.0:502；可启用 TLS。
- MQTT bridge：`rumqttc` 客户端，发布 alerts / status / task_receipts，订阅 task；支持 TLS。
- AINAS 远程任务：`POST /api/integrations/ainas/tasks` 接受 `set_targets` / `start_process` / `stop_process`，写入 AES-256-GCM 信封。

### 2.2 CLI

- `xingshu status` / `xingshu config` / `xingshu start|stop`
- `xingshu data {list,export,export-xlsx,report,delete,sample}`
- `xingshu control {set,start,stop,estop}`
- `xingshu ai {suggest,plan,model,train}`
- `xingshu audit {list,export}`
- `xingshu modbus {map,read,write}`
- `xingshu safety ...`
- `xingshu perf smoke`
- `xingshu ops {backup,restore,wipe}`
- `xingshu key generate` / `xingshu key rekey-integration-tasks`

### 2.3 验证脚本

- `scripts/verify-vue-process-lifecycle.mjs` — Vue 工艺生命周期 + 中英切换
- `scripts/verify-vue-parity.mjs` — Vue 7 页面中英切换 + 横向溢出检查
- `scripts/verify-load-and-rbac.ps1` — 20 并发控制写入 + 禁区 + RBAC 矩阵
- `scripts/verify-ainas-mqtt.mjs` — AINAS / MQTT 自检
- `scripts/verify-modbus-tls.ps1` — Modbus TCP TLS 握手自检
- `scripts/modbus-fault-proxy.mjs` — Modbus TCP 故障注入代理
- `scripts/start-vue-hmi.ps1` — 启动 daemon + Vue HMI（auto 模式）

### 2.4 文档

- `docs/upper_computer_development_doc.md` — 开发文档
- `docs/upper_computer_user_manual.md` — 使用手册
- `docs/upper_computer_maintenance_manual.md` — 维护手册
- `docs/upper_computer_cli_reference.md` — CLI 参考
- `docs/upper_computer_api_acceptance_manual.md` — API 验收手册
- `docs/upper_computer_security_key_lifecycle.md` — 密钥生命周期
- `docs/upper_computer_static_cutover.md` — 静态资源切换预案
- `docs/upper_computer_production_operations.md` — 生产部署与运维
- `docs/upper_computer_external_acceptance_handoff.md` — 本文档
- `docs/upper_computer_external_acceptance_checklist.md` — 外部验收清单
- `docs/architecture-deviations.md` — 偏离说明

## 3. 上位机侧已完成的工业级切片（避免重复实现）

| 切片 | 提交 | 验收 | 完成度 |
|---|---|---|---|
| Vue 工艺生命周期 | `4707cf13` | `scripts/verify-vue-process-lifecycle.mjs`（已严格通过条件） | READY |
| Vue AI/历史/设置/Monitor parity | `c9586ead` | `scripts/verify-vue-parity.mjs`（已严格通过条件） | READY |
| Vue 字段读取返工（A/B） | `031ec56a` | `npm run frontend:build` + `cargo test` | READY |
| 生产静态资源切换 | `e22052c3` | `cargo check` + `daemon --help` 显示 `--assets auto` | READY |
| systemd unit + TLS 终端 | `031ec56a` | `deploy/reactor-edge-daemon.service` + `cargo check` | READY |
| `xingshu ops backup/restore/wipe` CLI + 自动备份 timer | `33c969ed` + `031ec56a` + `c7709402` + 当前工作树 | `scripts/probe-cli-ops.ps1` 端到端跑通：real SQLite → VACUUM INTO backup 含 sha256 → restore 拒非 SQLite → restore 接受真 snapshot → wipe 拒无 `--yes` → wipe 真删除；`scripts/verify-production-backup-schedule.mjs` 和 `scripts/verify-production-backup-script.ps1` 覆盖 release timer 和 backup.sh 时间戳快照 | PARTIAL（自动备份发布路径已交付；恢复演练、保留策略和异地归档仍待现场验收） |
| `xingshu key generate` / `rekey-integration-tasks` CLI | `031ec56a` + `c7709402` + 当前工作树 | `scripts/probe-cli-ops.ps1` 覆盖 key generate；`cargo test --test cli_tests xingshu_key_rekey_integration_tasks_migrates_existing_payloads` 覆盖 dry-run、正式 rekey、新 key 可读、旧 key 不可读和密钥不打印 | PARTIAL（本地迁移工具已交付；生产密钥托管、停机窗口和恢复演练仍需现场签字） |
| Modbus 故障代理 / TLS / AINAS-MQTT 自检 | `eb25fd15` | `scripts/modbus-fault-proxy.mjs` + `verify-modbus-tls.ps1` + `verify-ainas-mqtt.mjs` | READY |
| 压测 / 禁区 / RBAC 验证 | `6aeaef00` + `031ec56a` | `scripts/verify-load-and-rbac.ps1`（已严格 RBAC 判定） | PARTIAL（operator→ainas 已稳定 403；modbus 写仍受 #14 限制） |
| AINAS RBAC 真实修复（ApplyIntegrationTask） | 本提交 | `cargo test --test api_tests ainas` 2/2 通过 | READY |
| SQLx schema migration | `13f47502` | `cargo test` 27 db + 50 api 通过 | PARTIAL（schema migration 字符串仍走 `rusqlite`） |
| 部署与运维文档 | `8750d0f9` | `docs/upper_computer_production_operations.md` 审阅 | READY |
| 外部联调前置清单 + 跟踪表 | `1b25c70b` | 本文件 + 跟踪表 | READY |

## 4. 下一阶段跟踪动作

### 王天宇

1. 提供 STM32 固件 + 寄存器手册终版 → `xingshu modbus read` 全表验证。
2. 接物理急停信号 → 急停信号链路演练。
3. 部署 Modbus TCP TLS 证书链 → `verify-modbus-tls.ps1` 应返回 `Verify return code: 0`。

### 闫露

1. 提供 Qwen3.5-2B GGUF + LoRA adapter + 推理 HTTP 服务 → `xingshu ai suggest` 和 `apply_ai_suggestion` 端到端。
2. 提供 PEFT 增量训练脚本 + 评估集 + 阈值 → `local_ai` readiness 升级。
3. 与 AINAS 平台对接真实任务回执 → `verify-ainas-mqtt.mjs` 全 ok。

### 邓琪曦

1. 协助发布正式产品手册 / 用户手册。
2. 商业物料与生产部署宣传对接。

### SRE

1. RK3568/RK3588 部署验收 → `upper_computer_rk_deployment_acceptance_guide.md`。
2. iptables / VPN 落地。
3. 7x24 长稳态 + 监控告警接入。

### 团队

1. 启动 release 性能压测（`xingshu perf smoke` 跑 7 天）。
2. 真实 Modbus Poll / Modbus Slave 工具验收。
3. 真实 STM32 实机验收。

## 5. 验收结论

李祖祎这一侧的上位机切片按工业级"完成度"分类如下。**所有"全部完成 / 工业级"的措辞必须按本节口径**，不要扩展到 PARTIAL / SCRIPT-ONLY 项。

- 七大页面 Vue 迁移 + 中英视觉验证：`verify-vue-parity.mjs` 通过（每页中英 100% 必检短语 + 横向溢出 0）
- 工艺生命周期（创建 / 步骤 / 启动 / 停止）：`verify-vue-process-lifecycle.mjs` 通过
- AI 主控 dry-run / execute + SOP 草案：AIView 已接入 `apply_ai_suggestion` 权限 + `/api/ai/control` 端到端
- 历史 / 设置 / 报警 视图 Vue 完整 parity：见 `verify-vue-parity.mjs` 三页结果
- 审计哈希链：120+ events, valid=True in load test
- AINAS 集成 dispatch 路径 RBAC：operator 稳定 403（已加 `Permission::ApplyIntegrationTask`）
- Modbus 故障注入 + TLS 自检 + AINAS/MQTT 联调脚本：`scripts/modbus-fault-proxy.mjs` / `verify-modbus-tls.ps1` / `verify-ainas-mqtt.mjs`
- 真实并发压测 + 禁区边界：`verify-load-and-rbac.ps1` 已能跑（operator→ainas 现在 403 必过）
- systemd unit + 低权限用户 + 防火墙 + 物理急停路径文档：`docs/upper_computer_production_operations.md` + `deploy/reactor-edge-daemon.service`
- 外部联调前置清单 + 跟踪表：本文件

**仍未到工业级（PARTIAL / SCRIPT-ONLY）**：

- 备份/恢复/擦除 CLI（`xingshu ops`）：PARTIAL — backup 已使用 SQLite `VACUUM INTO` 在线快照并随 release timer 自动执行；restore 校验 magic header 和 integrity_check，恢复仍必须停 daemon；wipe 已覆盖主文件、WAL/SHM/JOURNAL、key 和同级 backups 匹配快照，但物理介质擦除、恢复演练、保留策略和异地归档仍待现场验收；详见 #13
- 密钥生成与 rekey CLI（`xingshu key generate` / `xingshu key rekey-integration-tasks`）：PARTIAL — 本地工具已能迁移 integration task 旧密文和历史明文行，密钥材料不打印；生产仍需真实 key 托管、停机演练、恢复验证和签字；详见 #12
- SQLx schema 完整迁移：PARTIAL — 主流运行路径（audit、process、batch、sensor、recommendation、integration task、AINAS）已走 SQLx；schema migration（`SCHEMA_SQL` 字符串）和内存测试库仍走 `rusqlite`；编译期 SQL 约束未全面接入
- RBAC 矩阵：PARTIAL — 当前覆盖三角色基本矩阵 + AINAS 集成；modbus TCP / 系统配置 / 删除数据等细粒度权限矩阵仍待补齐；详见 #14
- 历史 XLSX 报告：READY — `xingshu data export-xlsx` 写文件后包结构由测试解包校验，Vue History 已提供 `Export XLSX`，`scripts/verify-vue-history-xlsx.mjs` 已验证 CSV/XLSX 浏览器下载事件和截图。
- Vue 完整 parity：PARTIAL — 七大页面已交付；设置多卡视觉 / 物料页 Vue 端未达 PRD 完整度
- 工艺探索 / 7 大页面：PARTIAL — 同上

**剩余的 PENDING-EXTERNAL 项需要外部团队/硬件/平台配合验收**；本
文档跟踪表持续更新到所有项 close。
