# 星宿智能反应釜上位机 PRD 偏离说明

日期：2026-06-07

适用范围：李祖祎负责的 RK/PC 上位机软件，包括 Web HMI、Rust daemon、REST API、CLI、Modbus、MQTT、AINAS、SQLite、审计和安全控制。

本文档只记录“PRD v2.2 写法”和“当前工程实现”之间的偏离、边界和补偿措施。它不是缺陷清单，也不替代 `docs/upper_computer_requirement_gap_matrix.md`；它用于团队评审、客户验收和后续排期时统一解释口径。

## 1. 总体结论

当前上位机主体软件已经达到本地运行、演示、联调准备和继续验收状态。PRD 中多数“上位机基础功能”已有代码、测试或文档证据，但以下几类不能直接宣称完全满足 PRD：

- 本地 Qwen3.5-2B + LoRA 真实推理、自进化、GGUF 转换、生产训练脚本和 RK 端延迟验收。
- PRD 指定的 SQLx 技术栈仍在迁移中；`Db::open` 文件库已建立 SQLx SQLite pool，审计 total/list/chain/export 读取、审计事件写入、工艺流程/步骤读写与应用标记、批次/产物结果 history 读取、批次详情/报告读取、demo alarm 读取、批次创建/结束写入、AI 推荐输入和推荐缓存读取、AI 推荐写入、产物结果写入、实时曲线/v1 history/批次报告样本读取、实时采集/外部 pipeline 样本写入、以及 AINAS/MQTT 集成任务查询/创建/更新已走 SQLx，schema migration 和部分保留兼容路径仍使用 `rusqlite`。
- Vue3、Element Plus、ECharts、Pinia 迁移已在 `codex/prd-tech-stack-migration` 分支落到 release 路径：daemon/package/systemd 使用 `--assets auto`，有 `frontend/dist/index.html` 时默认托管 Vue 生产构建，legacy `static/index.html` 保留为回退；Modbus RTU 主站路径已迁到 `tokio-modbus`，Modbus TCP server 仍为自实现。
- 生产级安全与运维能力，包括现场 watchdog/权限隔离演练、介质安全擦除、生产密钥托管、安全扫描、长期备份保留策略和恢复演练。
- 真实硬件、真实第三方平台、release/RK 稳态性能和长期运行验收。

## 2. 偏离汇总

| 编号 | PRD 表述 | 当前实现 | 状态 | 影响 | 补偿或下一步 |
| --- | --- | --- | --- | --- | --- |
| D1 | 前端采用 Vue 3.4+、Vite、Element Plus、ECharts、Pinia | `frontend/` 已接入 Vue 3、Vite、Element Plus、ECharts、Pinia、Vue Router，包含 PRD 七大页面、Pinia 后端数据 store、ECharts 实时曲线、持久化中英切换、控制目标写入、自动控制、人工锁定、急停/复位、审计链指标/事件过滤/分页/CSV 导出、Modbus 寄存器读回和 admin-only 调试写入；Vue 工艺管理切片（`/api/processes` 列表/详情/创建/添加步骤/启动/停止 + `/api/batches` 摘要 + 当前 active batch 与最近 batches）已并入 `ControlView.vue`；History 页已补批次搜索、状态筛选、产物比例筛选、产物结果录入、产率/产物比例/目标参数展示和 CSV/XLSX 下载点击验证；Monitor 页已补正式样本入口触发的温度/压力越限报警流、中英报警字段映射和截图验证；AI 页已补 master-control 决策摘要、动作复核、安全门控、推荐目标、SOP 结构化草案和已知枚举/固定文案中英本地化；`npm run frontend:build`、七路由中英 DOM/截图验证、控制写入、审计导出、Modbus 写入、工艺生命周期、History CSV/XLSX 浏览器验证、Monitor 真实样本报警流和 AI 结果复核字块验证已通过；release/systemd/package 已用 `--assets auto` 默认优先托管 `frontend/dist/index.html`，legacy `static/index.html` 保留为回退 | 迁移中 | PRD 前端栈已进入生产资源路径，并具备可验证的七页面迁移版；工艺管理、历史数据录入/筛选/导出、本地真实样本报警流和 AI 结果复核切片已并入 Vue HMI，正式多端/RK/用户验收尚未完成 | 用 release/RK、多浏览器、真机移动端和现场硬件报警验收固化 |
| D2 | 后端数据库采用 SQLx ORM | 文件数据库已接入 SQLx SQLite pool，审计日志 total/list/chain/export、审计事件写入、工艺流程/步骤读写与应用标记、批次创建/结束、批次/产物结果 history、批次详情/报告读取、demo alarm 读取、AI 推荐输入/缓存/写入、产物结果写入、实时曲线/v1 history/批次报告样本、实时采集/外部 pipeline 样本写入、AINAS/MQTT 集成任务查询/创建/更新已由真实 API/MQTT 路径调用 SQLx；schema/migration 和部分保留兼容路径仍使用 `rusqlite` | 迁移中 | 已开始对齐 PRD 技术栈，但当前仍是混合数据库层；尚未获得全面 SQLx migration 封装和更广泛编译期 SQL 约束 | 继续把 schema migration 迁到 SQLx，随后收缩 `rusqlite` 到兼容/诊断工具路径 |
| D3 | Modbus 后端库采用 `tokio-modbus` | `DeviceMode::Modbus` 的 RTU 主站读写已迁到 `tokio-modbus` + `tokio-serial`；Modbus TCP server 仍为自实现 MBAP/PDU 处理 | 部分迁移 | RTU 主站技术栈已对齐 PRD；TCP server 还需评估是否改用 `tokio-modbus` server feature 或保留现有 TLS/审计集成实现 | 补 STM32 实机 RTU 验收；评估 Modbus TCP server 是否迁到 `tokio-modbus` server feature；继续用 Modbus Poll/Slave、故障注入和 TLS 工具补足互操作证据 |
| D4 | 本地 LoRA 推理、自训练、自进化、GGUF 转换 | 上位机已实现 local_ai readiness、HTTP/命令式推理入口、训练数据集导出、训练入口编排、manifest 和显式候选 adapter 晋级/备份；真实模型资产、生产训练脚本、自动触发、自主评估和 RK 延迟报告未交付 | P0 部分完成 | PRD P0 卖点仍不能宣称完成，不能宣称 M2/M3 完成 | 算法侧提供模型/adapter/生产训练脚本/RK 报告；上位机继续接入真实 llama.cpp HTTP 或等效推理服务、生产审批流和 HMI 证据 |
| D5 | 独立安全过滤器/安全进程 | `reactor-safety-guard` 已调用共享安全判断，不是空壳；外部进程等待已使用 `wait-timeout` 超时等待并在超时后 kill 子进程；release package 会携带 `reactor-safety-guard`，`run.sh` 和 systemd service 默认以 `--safety-guard` 启动；`scripts/verify-production-safety-guard.mjs` 已加入一键验收 | 部分完成 | 生产发布路径已启用独立 guard，但现场 watchdog 演练、低权限账号确认和故障注入证据仍不足 | 在 RK/现场补 systemd restart/watchdog 演练、最小权限运行账号确认、guard 异常/超时故障注入验收 |
| D6 | 自动定期备份数据库、数据彻底擦除 | `install.sh` 会在停服务前校验 release 包完整性，缺二进制、OTA/备份脚本、unit、配置、build metadata 或 HMI 资源时先失败；`xingshu ops backup` 已使用 SQLite `VACUUM INTO` 生成在线快照；release package 已包含 `reactor-edge-backup.service/.timer` 和 `/opt/reactor-edge/current/backup.sh`，默认每日先用非阻塞锁串行化 timer/OTA 前备份，再写临时快照，确认非空、sha256 sidecar 校验通过且具备 SQLite header 后才发布为时间戳快照并更新 `latest.snapshot`；应用发布路径已改为 `/opt/reactor-edge/slots/{a,b}` + `current/previous`，`ota-update.sh` 覆盖板端命令预检、checksum sidecar 包名绑定校验、健康检查参数校验、tar 成员安全校验、managed slot 链接校验、状态不可证实时 fail-closed、忙碌/急停拦截、磁盘空间预检、`--dry-run` 不切槽预检、切槽前拒绝坏包/不安全现场时记录 `rejected_before_switch`、`BUILD-METADATA.properties` 构建追溯、更新前备份、失败 staging 清理、关键状态/slot/链接写入后 `sync` 落盘、开机 boot-check 对切槽前中断保留 current、切槽后中断恢复 previous、backend 每次启动前重跑 boot-check、backend/kiosk 启动限流、健康失败自动回滚和 OTA 状态日志，进入 failed 时清除健康检查临时放行并停止生产服务；`scripts/verify-install-board-preflight.sh`、`scripts/verify-production-backup-schedule.mjs`、`scripts/verify-production-backup-script.ps1`、`scripts/verify-ota-ab-release-path.mjs`、`scripts/verify-ota-systemd-boot-gate.mjs`、`scripts/verify-ota-tar-safety.sh`、`scripts/verify-ota-busy-state.sh`、`scripts/verify-ota-input-guards.sh`、`scripts/verify-ota-pre-switch-rejection.sh`、`scripts/verify-ota-cleanup.sh`、`scripts/verify-ota-slot-integrity.sh`、`scripts/verify-ota-command-preflight.sh`、`scripts/verify-ota-durability-sync.sh`、`scripts/verify-ota-boot-check.sh`、`scripts/verify-ota-failed-state.sh` 与 `scripts/verify-ota-dry-run.sh` 已纳入一键验收。`ops wipe` 已覆盖 SQLite 主文件、WAL/SHM/JOURNAL、`<db>.key` 和同目录 `backups/` 匹配快照，但 SSD/NVMe 物理擦除仍需运维 SOP | 部分完成 | 自动备份和应用级 A/B OTA 发布路径已补；仍缺现场恢复演练、备份保留/异地归档策略验收、真实 OTA 演练和退役介质物理擦除证据 | 在 RK/现场执行 timer、恢复演练、A/B 更新/失败回滚演练、NFS/离线归档和 blkdiscard/hdparm 等介质退役流程 |
| D7 | PRD 2.2 非功能指标持续证明 | 已有本地 perf smoke 和 Windows debug 资源快照；已针对 LubanCat 2 / RK3568 做 release 运行时调优（`[profile.release]` fat LTO + `codegen-units=1` + `panic=abort` + `strip`，tokio worker 限 2 线程，SQLite `synchronous=NORMAL`/`wal_autocheckpoint`/`temp_store=MEMORY`/`cache_size` 收敛），ARM64 交叉编译产出 stripped 二进制；但无 release/RK 长稳态 CI 断言，运行时内存/CPU 仍未在 ARM64 实测 | 部分完成 | 可支持 PoC 说明和"已为目标板调优"表述，不能替代正式性能验收 | 在 RK/QEMU 实测优化后二进制的内存/CPU/采集延迟，补 RS485 丢包率、7x24 或 30 天报告 |
| D8 | 七大页面命名 | Vue 生产 HMI 当前按 PRD 七大页面提供 `monitor/control/ai/history/audit/modbus/settings` 七个 hash 路由；legacy `static/index.html` 历史证据中曾拆成 9 个 tab | 当前已对齐；legacy 证据需注明来源 | 当前 Vue HMI 不再因页面数量产生偏离；旧截图归档时需要说明是 legacy fallback 历史证据 | 正式验收以 Vue 七路由截图和用例为主，legacy 9-tab 截图仅作为回退能力证据 |
| D9 | 双模型融合 AI 决策 | 当前是云端 StepFun 优先，本地优化器 fallback/补充；不是两个模型同时融合 | 部分完成 | AI 决策能力可演示，但不满足“融合模型”表述 | 明确当前策略为 cloud-first + local fallback；双模型融合另行排期 |
| D10 | 防火墙/VPN、STM32 物理急停 | 当前应用层提供 TLS/RBAC/软件急停和状态字段；网络边界和物理急停属于部署/硬件 | 外部边界 | 上位机不能单独完成这些 PRD 项 | 部署文档补 iptables/VPN 建议；硬件侧提供急停信号上报链路和联调记录 |

## 3. 关键偏离说明

### 3.1 前端技术栈

PRD v2.2 指定 Vue 3.4+、Vite、Element Plus、ECharts 和 Pinia。`codex/prd-tech-stack-migration` 分支已经把 `frontend/` 从占位 TypeScript 页面推进为真实 Vue 应用：`App.vue` 承载工业 HMI shell，`router.ts` 映射 PRD 七大页面，`stores/plant.ts` 集中访问 `/health`、`/api/config/summary`、`/api/audit/logs`、`/api/audit/export.csv`、`/api/modbus/registers` 和 `/api/recommendations/latest`，`MonitorView.vue` 使用 ECharts 绘制实时曲线，七个 Vue 页面已接入 Pinia 持久化中英切换，`ControlView.vue` 已接入 `/api/control/targets`、`/api/control/auto`、`/api/control/manual-lock`、`/api/control/emergency-stop` 和 reset 路径，`AuditView.vue` 已接入审计链指标、事件类型过滤、分页查询和 bearer 授权 CSV 导出，`ModbusView.vue` 已接入 `/api/modbus/registers/:name/read` 和 admin-only `/write` 路径，并要求非空审计原因。当前已用 `npm run frontend:build` 验证单文件构建，并用 Playwright/Chromium 对 `/#/monitor`、`/#/control`、`/#/ai`、`/#/history`、`/#/audit`、`/#/modbus`、`/#/settings` 做中文/英文 DOM 字块检查；控制写入验证通过 engineer 登录、目标写入、后端 200 返回和页面回显；审计导出验证通过 engineer 登录、事件过滤、CSV 下载和中英页面截图；Modbus 写入验证通过 admin 登录、写入 `target_temperature_c=67`、后端返回 raw `670` 和页面读回。截图证据位于 `output/playwright/vue-i18n-monitor-zh.png`、`output/playwright/vue-i18n-monitor-en.png`、`output/playwright/vue-i18n-modbus-zh.png`、`output/playwright/vue-i18n-modbus-en.png`、`output/playwright/vue-control-write-en.png`、`output/playwright/vue-audit-export-en.png` 和 `output/playwright/vue-modbus-write-en.png`。

当前 release 路径已从“只托管 `static/index.html`”推进为 `--assets auto`：有 `frontend/dist/index.html` 时 daemon 默认托管 Vue 生产构建，缺少 Vue 构建时自动回退到 legacy `static/index.html`。对外应表述为“PRD 前端技术栈已进入生产资源默认路径，并有构建、DOM、截图、控制写入、审计导出、Modbus 写入、工艺生命周期、History 筛选与 CSV/XLSX 下载点击证据；完整功能 parity、多浏览器/移动端、release/RK 和用户验收仍需补齐”，不能表述为“已按 PRD 前端栈完成最终生产验收”。

### 3.2 数据库和 Modbus 技术栈

PRD 写明 SQLx 和 tokio-modbus。当前 `codex/prd-tech-stack-migration` 分支已把 `DeviceMode::Modbus` 的 RTU 主站从 `serialport` 手写 RTU 帧迁到 `tokio-modbus` + `tokio-serial`，保留 ESP32 串口桥的 `serialport`。数据库迁移已启动：文件数据库打开时同时建立 SQLx SQLite pool，`GET /api/audit/logs` 的 total/list/chain 查询通过 `Db::audit_event_count_sqlx`、`Db::audit_events_sqlx` 和 `Db::audit_chain_status_sqlx` 由 SQLx 查询，`GET /api/audit/export.csv` 的导出读取也复用 SQLx 路径；真实 API、AINAS/MQTT 执行路径、Modbus debug 写入和 daemon 控制循环通过 `Db::insert_control_event_sqlx` 走 SQLx 写入审计事件，并用异步写锁串行化 previous/event hash 计算，避免连接池并发写入破坏审计链；工艺流程列表/详情/创建/更新、步骤新增/更新和应用标记通过 `Db::list_processes_sqlx`、`Db::process_detail_sqlx`、`Db::create_process_sqlx`、`Db::update_process_sqlx`、`Db::add_process_step_sqlx`、`Db::update_process_step_sqlx` 和 `Db::mark_process_applied_sqlx` 走 SQLx，步骤写入用异步写锁保持 step_index 顺序；`POST /api/batches/start`、流程启动创建批次、流程停止/失败回滚/批次完成通过 `Db::create_batch_for_process_sqlx` 和 `Db::finish_batch_sqlx` 走 SQLx；批次详情、单批次报告事件、最近审计事件和 demo alarm 读取通过 `Db::batch_by_id_sqlx`、`Db::batch_outcome_by_id_sqlx`、`Db::control_events_for_batch_sqlx`、`Db::recent_control_events_sqlx` 和 `Db::recent_demo_alarms_sqlx` 走 SQLx；`live`、demo context、批次列表、批次 CSV/XLSX 导出和 AI 实验计划中的 Recent 批次/产物结果读取通过 `Db::recent_batches_sqlx` 和 `Db::recent_batch_outcomes_sqlx` 走 SQLx；AI 推荐输入的全量产物结果读取通过 `Db::batch_outcomes_sqlx` 走 SQLx；HMI/demo/AI master-control/latest recommendation 的推荐缓存读取通过 `Db::latest_recommendation_sqlx` 走 SQLx；`POST /api/recommendations/latest`、AI master-control 懒生成推荐和 AI 实验方案推荐落库通过 `Db::insert_recommendation_sqlx` 走 SQLx；`POST /api/product-results` 的产物结果落库通过 `Db::insert_product_result_sqlx` 走 SQLx；`live` 实时曲线、`GET /api/v1/reactor/:device_id/history`、批次详情和批次 Markdown 报告的样本读取通过 `Db::recent_sample_records_sqlx`、`Db::samples_between_sqlx` 和 `Db::sample_records_for_batch_sqlx` 走 SQLx；daemon 实时采集循环和 `POST /api/v1/reactor/:device_id/samples` / test pipeline 样本写入通过 `Db::insert_sample_sqlx` 走 SQLx；AINAS/MQTT 集成任务列表、详情、创建和状态更新通过 `Db::integration_tasks_sqlx`、`Db::integration_task_sqlx`、`Db::create_integration_task_sqlx` 和 `Db::update_integration_task_sqlx` 走 SQLx，并保留 AES-256-GCM 信封加解密和旧明文行兼容。内存测试库、schema migration 和部分保留兼容路径仍使用 `rusqlite`，以避免在未完成迁移前打断现有 schema/migration。Modbus TCP server 仍为自实现 MBAP/PDU 处理，主要原因是现有 TCP 路径已绑定 TLS 状态、审计、安全写入和测试工具验收。

Vue 工艺生命周期切片（`/#/control` 工艺/批次面板、`src/stores/plant.ts` 的 `loadProcesses`/`loadProcessDetail`/`createProcess`/`addProcessStep`/`startProcess`/`stopCurrentProcess`、`refreshProtected` 收敛 `/api/processes` + `/api/batches`、`mergeRuntimeFallback` 过滤 `undefined`、`styles.css` 新增 `.process-panel` 等 class）已经并入，并在 `npm run frontend:build`、中英 DOM 校验、Playwright 工艺生命周期截图 + verification.json 三个守门下通过。History 页已补搜索/状态/产物比例筛选、产物结果录入、产率/产物比例/目标参数展示和 CSV/XLSX 下载点击验证；Monitor 页已通过正式 `/api/v1/reactor/:device_id/samples` 入口注入温度/压力越限样本，验证 `/api/live` 生成 `temperature_limit`/`pressure_limit` 并在中英页面渲染报警字段；AI 页已补 master-control 结构化结果复核和 SOP 结构化草案；release package、systemd 和 QEMU smoke 已改为优先使用 `frontend/dist`，`static/index.html` 作为回退和 legacy parity 基线；最终验收前还需完成 release 性能、STM32 寄存器联调、现场硬件报警、真机移动端/多浏览器验收和生产运维演练。

该偏离不直接导致功能缺失，但会影响维护方式、静态 SQL 校验和第三方互操作信心。下一步应把 schema migration 迁到 SQLx，并收缩 `rusqlite` 到兼容/诊断工具路径，再决定 Modbus TCP 是否迁到 `tokio-modbus` server feature；同时补齐外部工具验收、STM32 实机验收、寄存器映射确认和协议错误注入测试。

### 3.3 本地 LoRA 与自进化边界

当前上位机已经具备：

- 云端 StepFun 推荐接入边界。
- 本地 `local-ga-sa-pid` 优化器。
- 批次结果、产品结果、推荐上下文和只读 SOP 草案。
- `local_ai` readiness 摘要，用于暴露模型资产是否就位。
- `xingshu ai train --export-only` 监督训练 JSONL 导出。
- `xingshu ai train` 训练入口编排和训练 manifest 归档。
- `xingshu ai train --promote --min-eval-score ...` 显式候选 adapter 晋级和旧 adapter 备份。

当前上位机尚未具备：

- 项目认可的 Qwen3.5-2B/GGUF 权重与生产 LoRA adapter。
- 真实 LoRA adapter 推理效果验收。
- 生产 PEFT/Transformers/Datasets 训练脚本和最终数据集契约评审。
- 10 批次/空闲时间/手动触发的增量训练调度。
- 生产评估/审批流、20% 留出集、准确率阈值验收。

因此本地 LoRA 仍是 P0 真缺口，不应被“训练边界已实现”覆盖。建议由算法侧先交付可调用的 llama.cpp HTTP 服务或等效本地推理服务、生产训练脚本和 RK 报告，上位机再接入 API、审计、权限、HMI 状态展示和自动触发/审批流。

### 3.4 独立安全进程

产研反馈中提到 `reactor-safety-guard.rs` 是空壳，这个说法需要修正。当前 `src/bin/reactor-safety-guard.rs` 会读取 stdin JSON，解析 `SafetyGuardRequest`，并调用 `evaluate_safety_request` 输出 `SafetyGuardResponse`。它复用了 `src/control.rs` 的真实安全判断，不是空 stub。

真正缺口已经从“未随生产启动”收敛为“现场安全运维证据不足”：

- ARM64 release package 已携带 `reactor-safety-guard`，`run.sh` 和 `deploy/reactor-edge.service` 默认传入 `--safety-guard`。
- systemd unit 已有 `NoNewPrivileges=true`、`PrivateTmp=true`、`ProtectSystem=full`、`ProtectHome=true` 和服务用户/组配置。
- 仍缺现场 RK/生产环境的 watchdog 重启演练、低权限账号确认、guard 异常/超时故障注入记录和安全扫描报告。
- 独立进程 JSON 往返 p95 在本地 debug 快照中约 315ms，只适合作为诊断路径，不应被当作控制计算本身耗时。

对外应表述为“安全逻辑已实现，独立进程可运行；生产级隔离和 watchdog 验收待补”。

### 3.5 页面数量映射

PRD 和团队分工写的是七大页面。当前 Vue 生产 HMI 已按 PRD 七大页面提供七个 hash 路由：

| PRD 七大页面 | 当前 Vue 路由 | 说明 |
| --- | --- | --- |
| 实时监控 | `/#/monitor` | 实时指标、曲线、报警队列和设备状态 |
| 参数配置 | `/#/control` | 目标写入、自动控制、人工锁定、急停/复位、工艺和批次生命周期 |
| AI 智能决策 | `/#/ai` | 推荐、AI master-control 结构化复核、只读 SOP 结构化草案 |
| 历史数据 | `/#/history` | 批次历史、产物结果录入、产物比例筛选、产率/目标参数展示和导出 |
| 审计日志 | `/#/audit` | 审计链、事件列表、过滤、分页和 CSV 导出 |
| Modbus 调试 | `/#/modbus` | 寄存器映射、读写、安全门控、集成状态 |
| 系统配置 | `/#/settings` | 权限、端点、设备、组件、安全和集成摘要 |

旧版 `static/index.html` fallback 曾把实时监控、历史和报警拆成 9 个 tab；这部分截图可以作为历史/回退证据保留，但正式验收应以 Vue 七路由截图和用例为准。

## 4. 非功能指标证据边界

当前已有本地证据：

- `output/upper-computer-perf-smoke.json`：本地只读 API p95 最高 4ms，`safety_compute` p95=1ms。
- `output/upper-computer-resource-snapshot.json`：Windows debug 演示进程 working set 26.977MB、private memory 6.102MB、CPU 5 秒采样 max 1.533%。

这些数据可以说明本地 PoC 没有明显资源异常，但不能替代 PRD 正式验收：

- 不是 release 构建。
- 不是 RK3568/RK3588 目标板。
- 不是 7x24 或 30 天持续采样。
- 不覆盖真实 RS485、STM32、执行器链路和本地 LoRA 模型权重。

因此资源指标当前应标记为“本地快照通过 / 待正式稳态验收”，不能直接标记为“PRD 完全通过”。

### 4.1 LubanCat 2 / RK3568 运行时调优（已做 / 待实测）

已针对目标板 LubanCat 2（RK3568 / 4×Cortex-A55 / ARM64）做了一轮 release 运行时调优，目标是这类“长跑、低负载边缘进程”的二进制体积与常驻内存：

- `Cargo.toml` 新增 `[profile.release]`：`lto = "fat"`、`codegen-units = 1`（跨 crate 内联、去死码）、`panic = "abort"`（去 unwind 表；代码无 `catch_unwind` 依赖，安全）、`strip = "symbols"`；保留 `overflow-checks = true` 不牺牲安全。
- tokio 运行时限 2 worker 线程（默认每核一 worker，A55 上 4 个对低负载是浪费）。
- SQLite 两条连接路径（rusqlite + SQLx pool）统一加 `synchronous=NORMAL`（WAL 下安全，减 fsync 与 eMMC 写放大）、`wal_autocheckpoint=400`、`temp_store=MEMORY`、`cache_size=-4096`（约 4 MiB）。

已实测的客观产物（ARM64 交叉编译包 `dist/reactor-os-lubancat2-...`）：

- 三个二进制 daemon / safety-guard / xingshu 均为 ARM aarch64、已 strip。
- tar.gz 与 sha256 sidecar 校验通过，`run.sh` 默认带 `--safety-guard`。

仍未取得的证据（本机无 ARM64 binfmt/QEMU，无法模拟执行优化后二进制）：

- 优化后二进制在 RK3568/QEMU 上的实测内存（PRD <30MB）、单核 CPU 稳态（PRD <3%）。
- 采集/控制延迟、RS485 丢包率、长稳态采样。

因此对外只能说“已为 LubanCat 2 目标板做 release 运行时调优并产出 stripped ARM64 交付包”，**不能**说“已实测满足 PRD 内存/CPU 指标”——后者必须在真机或 ARM64 QEMU 上跑优化后二进制取数后才成立。

## 5. 优先级排期建议

| 优先级 | 工作 | 交付物 |
| --- | --- | --- |
| P0 | 本地 LoRA 真接入 | 模型/adapter/推理服务、生产训练脚本、RK 延迟报告、上位机 API/HMI 接入和自动触发/审批流 |
| P0 | STM32/Modbus 实机联调 | 最终寄存器表、RTU 读写记录、控制闭环、故障注入报告 |
| P0 | 外部接口验收 | AINAS 任务回执、MQTT broker TLS 验收、Modbus Poll/Slave 截图 |
| P1 | 生产安全 | safety guard 发布路径已默认启用；继续补 watchdog/低权限账号现场演练、密钥轮换和安全扫描 |
| P1 | 备份与擦除 | 自动备份 timer 和脚本已随 release 发布；继续补现场恢复演练、备份保留/异地归档和物理擦除 SOP |
| P1 | 性能可靠性 | release/RK 资源采样、采集/控制延迟、RS485 丢包率、7x24 或 30 天报告 |
| P1 | 前端组件化迁移 | Vue/Element Plus/ECharts/Pinia 已进入 release 默认资源路径，已有七页面迁移版、中英切换视觉证据、控制写入、审计导出、Modbus 写入、工艺生命周期、History 产物结果录入/筛选/CSV/XLSX 下载、Monitor 正式样本报警流、AI 结果复核/SOP 结构化展示和 release asset gate；继续补齐多端/RK 和现场硬件报警验收 |

## 6. 推荐对外说法

建议说：

> 上位机主体软件、HMI、API、CLI、数据导出、安全门控、审计、基础第三方接口和 LoRA 训练边界已完成到本地 PoC/联调准备版。当前与 PRD 的主要差异是技术栈选型、真实本地 LoRA 模型资产/推理、自进化自动触发、生产安全运维和正式硬件/外部平台验收，这些已拆成后续交付项。

避免说：

> 上位机已经完整满足 PRD v2.2 的所有技术栈和 P0 AI 自进化要求。

原因是真实本地 LoRA 模型资产/推理和自动自进化仍未真实交付，SQLx 已覆盖主要运行时读路径、批次生命周期、实时/外部样本写入、AI 推荐/产物结果写入和 AINAS/MQTT 集成任务创建/更新，但 schema migration 和部分兼容路径仍未完全迁移，Modbus TCP server 尚未迁到 `tokio-modbus`，Vue 前端栈虽已进入 release 默认资源路径，仍缺完整 parity、多端/RK 和用户验收。
