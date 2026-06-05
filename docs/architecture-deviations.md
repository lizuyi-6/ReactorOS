# 星宿智能反应釜上位机 PRD 偏离说明

日期：2026-06-05

适用范围：李祖祎负责的 RK/PC 上位机软件，包括 Web HMI、Rust daemon、REST API、CLI、Modbus、MQTT、AINAS、SQLite、审计和安全控制。

本文档只记录“PRD v2.2 写法”和“当前工程实现”之间的偏离、边界和补偿措施。它不是缺陷清单，也不替代 `docs/upper_computer_requirement_gap_matrix.md`；它用于团队评审、客户验收和后续排期时统一解释口径。

## 1. 总体结论

当前上位机主体软件已经达到本地运行、演示、联调准备和继续验收状态。PRD 中多数“上位机基础功能”已有代码、测试或文档证据，但以下几类不能直接宣称完全满足 PRD：

- 本地 Qwen3.5-2B + LoRA 推理、训练、自进化、GGUF 转换和 RK 端延迟验收。
- PRD 指定的 Vue3、Element Plus、ECharts、Pinia、SQLx、tokio-modbus 技术栈。
- 生产级安全与运维能力，包括 watchdog/权限隔离、自动备份、介质安全擦除、生产密钥托管和安全扫描。
- 真实硬件、真实第三方平台、release/RK 稳态性能和长期运行验收。

## 2. 偏离汇总

| 编号 | PRD 表述 | 当前实现 | 状态 | 影响 | 补偿或下一步 |
| --- | --- | --- | --- | --- | --- |
| D1 | 前端采用 Vue 3.4+、Vite、Element Plus、ECharts、Pinia | `static/index.html` 是单文件原生 HTML/CSS/JS HMI，`frontend/src/main.ts` 仅作为组件化迁移目标 | 已接受的工程偏离 | 功能可运行，但与 PRD 技术栈和招聘/维护预期不一致 | 保留现有 HMI 作为 PoC/联调版本；若客户或团队坚持 PRD 技术栈，单独排期 Vue 迁移 |
| D2 | 后端数据库采用 SQLx ORM | 当前采用 `rusqlite`、SQLite WAL、手写 row mapping | 已接受的工程偏离 | 功能等效，但编译期 SQL 校验和连接池能力与 SQLx 不同 | 在开发文档中明确低依赖、本地部署优先；如后续切 PostgreSQL/多连接，再评估 SQLx |
| D3 | Modbus 后端库采用 `tokio-modbus` | 当前采用 `serialport` 手写 RTU 帧，Modbus TCP server 也为自实现 | 已接受的工程偏离 | 可控性高，但需要更严格互操作测试 | 保留手写实现；用 Modbus Poll/Slave、STM32 实机和 TLS 工具补足验收证据 |
| D4 | 本地 LoRA 推理、自训练、自进化、GGUF 转换 | `local_ai.rs` 只探测模型、adapter、脚本和资产路径；daemon 未执行真实推理/训练 | P0 未交付 | PRD P0 卖点未完成，不能宣称 M2/M3 完成 | 算法侧提供模型/adapter/训练脚本/RK 报告；上位机接入 llama.cpp HTTP 或等效推理服务 |
| D5 | 独立安全过滤器/安全进程 | `reactor-safety-guard` 已调用共享安全判断，不是空壳；外部进程等待已使用 `wait-timeout` 超时等待并在超时后 kill 子进程；但默认未启用，生产 watchdog 和权限隔离未完成 | 部分完成 | 本地安全逻辑成立，生产隔离和故障演练证据不足 | 部署时强制启用 `--safety-guard`，补 watchdog、低权限用户、故障注入验收 |
| D6 | 自动定期备份数据库、数据彻底擦除 | 当前没有应用层定期备份调度；测试清理路径只是 DELETE | 未交付 | 生产数据保护和退役销毁不满足 PRD | 增加 `xingshu backup`/计划任务示例；安全擦除按目标介质制定运维 SOP |
| D7 | PRD 2.2 非功能指标持续证明 | 已有本地 perf smoke 和 Windows debug 资源快照，但无 release/RK 长稳态 CI 断言 | 部分完成 | 可支持 PoC 说明，不能替代正式性能验收 | 在 RK/release 环境补 CPU、内存、采集延迟、RS485 丢包率、7x24 或 30 天报告 |
| D8 | 七大页面命名 | HMI 实际有 9 个 tab：monitor、recipes、program、ai、materials、alarms、audit、modbus、settings | 功能覆盖但命名偏离 | 功能比 PRD 细分更细，但截图和 PRD 页面名需要映射 | 在验收材料中按 PRD 七大页面映射，不把 9 tab 解释成新增范围 |
| D9 | 双模型融合 AI 决策 | 当前是云端 StepFun 优先，本地优化器 fallback/补充；不是两个模型同时融合 | 部分完成 | AI 决策能力可演示，但不满足“融合模型”表述 | 明确当前策略为 cloud-first + local fallback；双模型融合另行排期 |
| D10 | 防火墙/VPN、STM32 物理急停 | 当前应用层提供 TLS/RBAC/软件急停和状态字段；网络边界和物理急停属于部署/硬件 | 外部边界 | 上位机不能单独完成这些 PRD 项 | 部署文档补 iptables/VPN 建议；硬件侧提供急停信号上报链路和联调记录 |

## 3. 关键偏离说明

### 3.1 前端技术栈

PRD v2.2 指定 Vue 3.4+、Vite、Element Plus、ECharts 和 Pinia。当前真实 HMI 是 `static/index.html` 单文件原生实现，目标是降低 RK/边缘设备部署成本、减少构建链风险、便于离线单二进制托管。

当前 HMI 已实现实时监控、参数/工艺控制、AI、历史批次、物料/产品结果、报警、审计、Modbus、系统配置、中英切换和本地视觉验证。但它不满足 PRD 的前端技术栈要求。对外应表述为“功能版 HMI 已交付，Vue/Element Plus/ECharts 为后续组件化迁移项”，不能表述为“已按 PRD 前端栈交付”。

### 3.2 数据库和 Modbus 技术栈

PRD 写明 SQLx 和 tokio-modbus。当前实现选择 `rusqlite`、`serialport` 和自实现 Modbus TCP，主要原因是本地单机部署、SQLite 文件库、低内存和可审计协议控制更符合 PoC 阶段目标。

该偏离不直接导致功能缺失，但会影响维护方式、静态 SQL 校验和第三方互操作信心。补偿措施不是马上重写，而是补齐外部工具验收、STM32 实机验收、寄存器映射确认和协议错误注入测试。

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
| P2 | 前端组件化迁移 | Vue/Element Plus/ECharts/Pinia 迁移计划和页面验收 |

## 6. 推荐对外说法

建议说：

> 上位机主体软件、HMI、API、CLI、数据导出、安全门控、审计和基础第三方接口已完成到本地 PoC/联调准备版。当前与 PRD 的主要差异是技术栈选型、真实本地 LoRA、自进化、生产安全运维和正式硬件/外部平台验收，这些已拆成后续交付项。

避免说：

> 上位机已经完整满足 PRD v2.2 的所有技术栈和 P0 AI 自进化要求。

原因是本地 LoRA 和自进化仍未真实交付，Vue/SQLx/tokio-modbus 等 PRD 技术栈也没有按原文实现。
