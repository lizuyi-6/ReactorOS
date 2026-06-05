# 星宿智能反应釜上位机 PRD 偏离说明

日期：2026-06-05

适用范围：李祖祎负责的 RK/PC 上位机软件，包括 Web HMI、Rust daemon、REST API、CLI、Modbus、MQTT、AINAS、SQLite、审计和安全控制。

本文档只记录“PRD v2.2 写法”和“当前工程实现”之间的偏离、边界和补偿措施。它不是缺陷清单，也不替代 `docs/upper_computer_requirement_gap_matrix.md`；它用于团队评审、客户验收和后续排期时统一解释口径。

## 1. 总体结论

当前上位机主体软件已经达到本地运行、演示、联调准备和继续验收状态。PRD 中多数“上位机基础功能”已有代码、测试或文档证据，但以下几类不能直接宣称完全满足 PRD：

- 本地 Qwen3.5-2B + LoRA 推理、训练、自进化、GGUF 转换和 RK 端延迟验收。
- PRD 指定的 SQLx 技术栈仍在迁移中；`Db::open` 文件库已建立 SQLx SQLite pool，审计 total/list/chain/export 读取、批次/产物结果 history 读取、AI 推荐输入和推荐缓存读取、以及实时曲线/v1 history/批次报告的样本读取已走 SQLx，主体写入、schema migration 和部分业务读写仍保留 `rusqlite`。Vue3、Element Plus、ECharts、Pinia 迁移已在 `codex/prd-tech-stack-migration` 分支启动但尚未替换生产 HMI，Modbus RTU 主站路径已迁到 `tokio-modbus`，Modbus TCP server 仍为自实现。
- 生产级安全与运维能力，包括 watchdog/权限隔离、自动备份、介质安全擦除、生产密钥托管和安全扫描。
- 真实硬件、真实第三方平台、release/RK 稳态性能和长期运行验收。

## 2. 偏离汇总

| 编号 | PRD 表述 | 当前实现 | 状态 | 影响 | 补偿或下一步 |
| --- | --- | --- | --- | --- | --- |
| D1 | 前端采用 Vue 3.4+、Vite、Element Plus、ECharts、Pinia | `frontend/` 已接入 Vue 3、Vite、Element Plus、ECharts、Pinia、Vue Router，包含 PRD 七大页面迁移壳、Pinia 后端数据 store 和 ECharts 实时曲线；生产服务仍托管 `static/index.html` | 迁移中 | PRD 前端栈已开始落地，但功能 parity、视觉验收和生产替换未完成 | 继续把 `static/index.html` 的控制、审计、Modbus 写入、中英切换和视觉验收迁入 Vue；通过后再把 daemon 静态资源切到 `frontend/dist/index.html` |
| D2 | 后端数据库采用 SQLx ORM | 文件数据库已接入 SQLx SQLite pool，审计日志 total/list/chain/export、批次/产物结果 history、AI 推荐输入/缓存、实时曲线/v1 history/批次报告的样本读取已由真实 API 路径调用 SQLx；主体 schema/migration、写入和部分 row mapping 仍使用 `rusqlite` | 迁移中 | 已开始对齐 PRD 技术栈，但当前仍是混合数据库层；尚未获得全面 SQLx 查询封装、连接池读写一致性和更广泛编译期 SQL 约束 | 继续把集成任务和 migration 迁到 SQLx；迁完后移除或收缩 `rusqlite` 到兼容工具路径 |
| D3 | Modbus 后端库采用 `tokio-modbus` | `DeviceMode::Modbus` 的 RTU 主站读写已迁到 `tokio-modbus` + `tokio-serial`；Modbus TCP server 仍为自实现 MBAP/PDU 处理 | 部分迁移 | RTU 主站技术栈已对齐 PRD；TCP server 还需评估是否改用 `tokio-modbus` server feature 或保留现有 TLS/审计集成实现 | 补 STM32 实机 RTU 验收；评估 Modbus TCP server 是否迁到 `tokio-modbus` server feature；继续用 Modbus Poll/Slave、故障注入和 TLS 工具补足互操作证据 |
| D4 | 本地 LoRA 推理、自训练、自进化、GGUF 转换 | `local_ai.rs` 只探测模型、adapter、脚本和资产路径；daemon 未执行真实推理/训练 | P0 未交付 | PRD P0 卖点未完成，不能宣称 M2/M3 完成 | 算法侧提供模型/adapter/训练脚本/RK 报告；上位机接入 llama.cpp HTTP 或等效推理服务 |
| D5 | 独立安全过滤器/安全进程 | `reactor-safety-guard` 已调用共享安全判断，不是空壳；外部进程等待已使用 `wait-timeout` 超时等待并在超时后 kill 子进程；但默认未启用，生产 watchdog 和权限隔离未完成 | 部分完成 | 本地安全逻辑成立，生产隔离和故障演练证据不足 | 部署时强制启用 `--safety-guard`，补 watchdog、低权限用户、故障注入验收 |
| D6 | 自动定期备份数据库、数据彻底擦除 | 当前没有应用层定期备份调度；测试清理路径只是 DELETE | 未交付 | 生产数据保护和退役销毁不满足 PRD | 增加 `xingshu backup`/计划任务示例；安全擦除按目标介质制定运维 SOP |
| D7 | PRD 2.2 非功能指标持续证明 | 已有本地 perf smoke 和 Windows debug 资源快照，但无 release/RK 长稳态 CI 断言 | 部分完成 | 可支持 PoC 说明，不能替代正式性能验收 | 在 RK/release 环境补 CPU、内存、采集延迟、RS485 丢包率、7x24 或 30 天报告 |
| D8 | 七大页面命名 | HMI 实际有 9 个 tab：monitor、recipes、program、ai、materials、alarms、audit、modbus、settings | 功能覆盖但命名偏离 | 功能比 PRD 细分更细，但截图和 PRD 页面名需要映射 | 在验收材料中按 PRD 七大页面映射，不把 9 tab 解释成新增范围 |
| D9 | 双模型融合 AI 决策 | 当前是云端 StepFun 优先，本地优化器 fallback/补充；不是两个模型同时融合 | 部分完成 | AI 决策能力可演示，但不满足“融合模型”表述 | 明确当前策略为 cloud-first + local fallback；双模型融合另行排期 |
| D10 | 防火墙/VPN、STM32 物理急停 | 当前应用层提供 TLS/RBAC/软件急停和状态字段；网络边界和物理急停属于部署/硬件 | 外部边界 | 上位机不能单独完成这些 PRD 项 | 部署文档补 iptables/VPN 建议；硬件侧提供急停信号上报链路和联调记录 |

## 3. 关键偏离说明

### 3.1 前端技术栈

PRD v2.2 指定 Vue 3.4+、Vite、Element Plus、ECharts 和 Pinia。`codex/prd-tech-stack-migration` 分支已经把 `frontend/` 从占位 TypeScript 页面推进为真实 Vue 应用：`App.vue` 承载工业 HMI shell，`router.ts` 映射 PRD 七大页面，`stores/plant.ts` 集中访问 `/health`、`/api/config/summary`、`/api/audit/logs`、`/api/modbus/registers` 和 `/api/recommendations/latest`，`MonitorView.vue` 使用 ECharts 绘制实时曲线。

当前生产 HMI 仍由 `static/index.html` 单文件原生实现提供，已覆盖实时监控、参数/工艺控制、AI、历史批次、物料/产品结果、报警、审计、Modbus、系统配置、中英切换和本地视觉验证。对外应表述为“PRD 前端技术栈迁移已启动，Vue 版本具备可构建的七页面迁移壳；生产 HMI 替换和 parity 验收待完成”，不能表述为“已按 PRD 前端栈完成生产交付”。

### 3.2 数据库和 Modbus 技术栈

PRD 写明 SQLx 和 tokio-modbus。当前 `codex/prd-tech-stack-migration` 分支已把 `DeviceMode::Modbus` 的 RTU 主站从 `serialport` 手写 RTU 帧迁到 `tokio-modbus` + `tokio-serial`，保留 ESP32 串口桥的 `serialport`。数据库迁移已启动：文件数据库打开时同时建立 SQLx SQLite pool，`GET /api/audit/logs` 的 total/list/chain 查询通过 `Db::audit_event_count_sqlx`、`Db::audit_events_sqlx` 和 `Db::audit_chain_status_sqlx` 由 SQLx 查询，`GET /api/audit/export.csv` 的导出读取也复用 SQLx 路径；`live`、demo context、批次列表、批次 CSV/XLSX 导出和 AI 实验计划中的 Recent 批次/产物结果读取通过 `Db::recent_batches_sqlx` 和 `Db::recent_batch_outcomes_sqlx` 走 SQLx；AI 推荐输入的全量产物结果读取通过 `Db::batch_outcomes_sqlx` 走 SQLx；HMI/demo/AI master-control/latest recommendation 的推荐缓存读取通过 `Db::latest_recommendation_sqlx` 走 SQLx；`live` 实时曲线、`GET /api/v1/reactor/:device_id/history`、批次详情和批次 Markdown 报告的样本读取通过 `Db::recent_sample_records_sqlx`、`Db::samples_between_sqlx` 和 `Db::sample_records_for_batch_sqlx` 走 SQLx。内存测试库和主体写入、migration、集成任务等逻辑仍使用 `rusqlite`，以避免在未完成迁移前打断现有 schema/migration 和加密任务路径。Modbus TCP server 仍为自实现 MBAP/PDU 处理，主要原因是现有 TCP 路径已绑定 TLS 状态、审计、安全写入和测试工具验收。

该偏离不直接导致功能缺失，但会影响维护方式、静态 SQL 校验和第三方互操作信心。下一步应扩大 SQLx-backed adapter 覆盖面，把集成任务和 migration 迁到 SQLx，再决定 Modbus TCP 是否迁到 `tokio-modbus` server feature；同时补齐外部工具验收、STM32 实机验收、寄存器映射确认和协议错误注入测试。

### 3.3 本地 LoRA 与自进化边界

当前上位机已经具备：

- 云端 StepFun 推荐接入边界。
- 本地 `local-ga-sa-pid` 优化器。
- 批次结果、产品结果、推荐上下文和只读 SOP 草案。
- `local_ai` readiness 摘要，用于暴露模型资产是否就位。

当前上位机尚未具备：

- Qwen3.5-2B/GGUF 权重加载。
- LoRA adapter 推理。
- PEFT/Transformers/Datasets 训练入口。
- 10 批次/空闲时间/手动触发的增量训练调度。
- 评估、回滚、20% 留出集、准确率阈值验收。

因此本地 LoRA 是 P0 真缺口，不应被“路径探测已实现”覆盖。建议由算法侧先交付可调用的 llama.cpp HTTP 服务或等效本地推理服务，上位机再接入 API、审计、权限和 HMI 状态展示。

### 3.4 独立安全进程

产研反馈中提到 `reactor-safety-guard.rs` 是空壳，这个说法需要修正。当前 `src/bin/reactor-safety-guard.rs` 会读取 stdin JSON，解析 `SafetyGuardRequest`，并调用 `evaluate_safety_request` 输出 `SafetyGuardResponse`。它复用了 `src/control.rs` 的真实安全判断，不是空 stub。

真正缺口是生产部署口径：

- 默认启动路径未强制启用 `--safety-guard`。
- 缺少 systemd watchdog、低权限用户、进程隔离和故障演练记录。
- 独立进程 JSON 往返 p95 在本地 debug 快照中约 315ms，只适合作为诊断路径，不应被当作控制计算本身耗时。

对外应表述为“安全逻辑已实现，独立进程可运行；生产级隔离和 watchdog 验收待补”。

### 3.5 页面数量映射

PRD 和团队分工写的是七大页面。当前 HMI 为 9 个 tab，是把部分页面拆得更细：

| PRD 七大页面 | 当前 HMI tab | 说明 |
| --- | --- | --- |
| 实时监控 | `monitor`、`alarms` | 实时指标、曲线、报警队列和报警中心 |
| 参数配置 | `program`、`settings` | 工艺/手动控制、系统配置、设备通道 |
| AI 智能决策 | `ai`、`monitor` AI 卡片 | 推荐、AI master-control 预览、只读 SOP 草案 |
| 历史数据 | `recipes`、`materials` | 批次历史、产品结果、学习概览和趋势 |
| 审计日志 | `audit` | 审计链、事件列表、CSV 导出 |
| Modbus 调试 | `modbus` | 寄存器映射、读写、安全门控、集成状态 |
| 系统配置 | `settings` | 权限、端点、设备、组件和集成摘要 |

验收时应按 PRD 七大页面归档截图和用例，避免因为 tab 数量不同被误判为“缺页”或“多做了未授权范围”。

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

## 5. 优先级排期建议

| 优先级 | 工作 | 交付物 |
| --- | --- | --- |
| P0 | 本地 LoRA 真接入 | 模型/adapter/推理服务、训练脚本、RK 延迟报告、上位机 API/HMI 接入 |
| P0 | STM32/Modbus 实机联调 | 最终寄存器表、RTU 读写记录、控制闭环、故障注入报告 |
| P0 | 外部接口验收 | AINAS 任务回执、MQTT broker TLS 验收、Modbus Poll/Slave 截图 |
| P1 | 生产安全 | `--safety-guard` 强制启用方案、watchdog、低权限用户、密钥轮换、安全扫描 |
| P1 | 备份与擦除 | 数据库备份命令/计划任务、恢复演练、安全擦除 SOP |
| P1 | 性能可靠性 | release/RK 资源采样、采集/控制延迟、RS485 丢包率、7x24 或 30 天报告 |
| P1 | 前端组件化迁移 | Vue/Element Plus/ECharts/Pinia 已有首版迁移壳；继续补齐功能 parity、i18n 视觉审计和生产静态资源切换 |

## 6. 推荐对外说法

建议说：

> 上位机主体软件、HMI、API、CLI、数据导出、安全门控、审计和基础第三方接口已完成到本地 PoC/联调准备版。当前与 PRD 的主要差异是技术栈选型、真实本地 LoRA、自进化、生产安全运维和正式硬件/外部平台验收，这些已拆成后续交付项。

避免说：

> 上位机已经完整满足 PRD v2.2 的所有技术栈和 P0 AI 自进化要求。

原因是本地 LoRA 和自进化仍未真实交付，SQLx 只覆盖了审计、批次/产物、样本/history 和推荐缓存的读取路径，写入、migration 和集成任务仍未完全迁移，Modbus TCP server 尚未迁到 `tokio-modbus`，Vue 前端栈也仍处于迁移壳和生产替换前状态。
