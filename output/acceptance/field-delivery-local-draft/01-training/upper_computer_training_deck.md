# 星宿智能反应釜上位机系统操作培训课件源稿

日期：2026-06-07

对象：李祖祎负责的 RK/PC 上位机培训交付物。

状态：本文档是可直接转成 PPTX 的 Markdown 课件源稿，覆盖 PRD 第十章要求的“系统操作培训 PPT”内容范围。它可以用于现场培训和验收讲解，但不等同于已经录制完成的 MP4 视频。

## 0. 使用说明

建议培训时长：45 到 60 分钟。

建议演示环境：

| 项目 | 建议值 |
| --- | --- |
| 上位机地址 | `http://127.0.0.1:8000/` 或 RK/现场部署地址 |
| 浏览器 | Chrome、Microsoft Edge 或已验收通过的目标浏览器 |
| 账号角色 | operator、engineer、admin 各准备一个 |
| 演示数据 | 使用本地样本流、真实批次数据或现场 STM32 数据 |
| 录屏分辨率 | 1920x1080，浏览器缩放 100% |

培训引用文档：

| 文档 | 绝对路径 |
| --- | --- |
| PRD v2.2 | `C:\Users\Abraham\Downloads\星宿智能反应釜体系 (Xingshu Intelligent Reactor System) 产品需求文档 (PRD) v2.2.md` |
| 团队分工与里程碑 | `C:\Users\Abraham\Downloads\星宿智能反应釜项目-团队分工&开发里程碑&DDL规划方案.docx` |
| 用户手册 | `X:\tianhks\docs\upper_computer_user_manual.md` |
| 开发文档 | `X:\tianhks\docs\upper_computer_development_doc.md` |
| 测试报告 | `X:\tianhks\docs\upper_computer_test_report.md` |
| 交付就绪索引 | `X:\tianhks\docs\upper_computer_delivery_readiness_index.md` |
| 外部验收清单 | `X:\tianhks\docs\upper_computer_external_acceptance_checklist.md` |
| 视觉证据索引 | `X:\tianhks\docs\upper_computer_visual_evidence_index.md` |
| API 验收手册 | `X:\tianhks\docs\upper_computer_api_acceptance_manual.md` |
| CLI 参考 | `X:\tianhks\docs\upper_computer_cli_reference.md` |
| Modbus 映射手册 | `X:\tianhks\docs\upper_computer_modbus_register_map.md` |
| RK 部署验收指南 | `X:\tianhks\docs\upper_computer_rk_deployment_acceptance_guide.md` |
| 密钥生命周期 | `X:\tianhks\docs\upper_computer_security_key_lifecycle.md` |
| 当前缺口摘要 | `X:\tianhks\docs\upper_computer_current_gap_summary_for_lizuyi.md` |

---

## Slide 1. 上位机系统定位

培训目标：

让参训人员理解上位机负责什么、不负责什么，以及它在智能反应釜系统里的边界。

讲解要点：

| 内容 | 说明 |
| --- | --- |
| 上位机定位 | RK/PC 侧边缘中枢，提供 Web HMI、REST API、CLI、数据持久化、审计、安全门控、第三方接口和 AI 推荐入口 |
| 对下连接 | 通过 Modbus RTU/TCP、JSON bridge 或配置化设备通道连接 STM32/反应釜控制器 |
| 对上连接 | 提供 HMI、CLI、AINAS REST、MQTT、第三方 REST 和导出文件 |
| 安全边界 | 所有控制入口都必须经过范围、步长、急停、人工锁、传感器超时和禁区校验 |
| 当前口径 | 本地软件主体已具备联调准备版能力，最终 PRD 全量完成仍依赖真实硬件、模型、外部平台、安全和用户验收 |

演示动作：

1. 打开上位机 HMI。
2. 展示七个主页面：监控、控制、AI、历史、审计、Modbus、系统配置。
3. 打开交付就绪索引，说明哪些是本地完成，哪些需要外部补证。

证据路径：

| 证据 | 绝对路径 |
| --- | --- |
| 交付就绪索引 | `X:\tianhks\docs\upper_computer_delivery_readiness_index.md` |
| 当前缺口摘要 | `X:\tianhks\docs\upper_computer_current_gap_summary_for_lizuyi.md` |
| Vue 浏览器矩阵报告 | `X:\tianhks\output\playwright\vue-browser-matrix-verification.json` |

---

## Slide 2. 系统架构和数据流

培训目标：

让参训人员知道数据从反应釜进入上位机、再进入页面、审计和第三方系统的路径。

讲解要点：

| 链路 | 说明 |
| --- | --- |
| 采集链路 | STM32/模拟样本 -> daemon -> SQLite -> HMI 实时监控和历史曲线 |
| 控制链路 | HMI/CLI/AI/AINAS/MQTT/Modbus debug -> RBAC -> safety guard -> 设备写入 -> 审计 |
| AI 链路 | 历史批次和产品结果 -> 推荐/优化器/云端或本地模型入口 -> 人工复核 -> 安全门控 |
| 审计链路 | 控制、登录、导出、AI、第三方任务都写入 control_events，并形成 hash chain |
| 第三方链路 | REST、MQTT、AINAS、Modbus TCP 用于任务下发、数据提取和报警同步 |

演示动作：

1. 展示 `GET /health` 或 HMI 顶部系统状态。
2. 展示监控页实时样本。
3. 展示审计页最近事件。
4. 展示设置页集成状态和安全状态。

证据路径：

| 证据 | 绝对路径 |
| --- | --- |
| 开发文档 | `X:\tianhks\docs\upper_computer_development_doc.md` |
| API 验收手册 | `X:\tianhks\docs\upper_computer_api_acceptance_manual.md` |
| 维护手册 | `X:\tianhks\docs\upper_computer_maintenance_manual.md` |

---

## Slide 3. 登录、角色和权限

培训目标：

让操作员知道不同角色能做什么，避免用高权限账号进行日常操作。

讲解要点：

| 角色 | 典型权限 | 培训重点 |
| --- | --- | --- |
| operator | 查看监控、查看历史、发起安全范围内的基础操作 | 日常操作账号，不应执行调试写寄存器 |
| engineer | 调试、工艺验证、更多诊断入口 | 适合联调，不应作为长期生产值守账号 |
| admin | 高危配置、Modbus 调试写入、用户/密钥/系统维护 | 只用于维护和验收，操作必须有审计 reason |

演示动作：

1. 以 operator 访问控制页，尝试受限操作。
2. 以 engineer 访问 Modbus 页面，说明调试边界。
3. 以 admin 展示高权限入口，并强调审计记录。

验收画面：

| 画面 | 预期 |
| --- | --- |
| 无权限操作 | 页面或 API 拒绝，返回权限错误 |
| 有权限操作 | 操作可继续，但仍受 safety guard 限制 |
| 审计记录 | 角色、动作、原因和结果可追溯 |

证据路径：

| 证据 | 绝对路径 |
| --- | --- |
| 用户手册 | `X:\tianhks\docs\upper_computer_user_manual.md` |
| RBAC/load 报告 | `X:\tianhks\output\load-and-rbac-report.json` |
| 测试报告 | `X:\tianhks\docs\upper_computer_test_report.md` |

---

## Slide 4. 实时监控页面

培训目标：

让操作员掌握实时状态、趋势、报警和系统健康的读取方式。

讲解要点：

| 区块 | 说明 |
| --- | --- |
| 实时指标 | 温度、压力、搅拌、流量、pH、浓度等当前值 |
| 趋势图 | 观察运行过程是否平稳，有无突变 |
| 系统健康 | normal、warning、alarm 或无新鲜样本等状态 |
| 报警列表 | 显示报警级别、类型、当前值、限值和建议 |
| 中英切换 | 页面静态字块和动态报警字段都应同步切换 |

演示动作：

1. 打开监控页。
2. 注入或等待新鲜样本，确认系统健康正常。
3. 注入越限样本，确认温度/压力报警出现。
4. 切换中文和英文，确认报警字段切换完整。

验收画面：

| 画面 | 预期 |
| --- | --- |
| 正常样本 | 系统健康显示 normal，曲线刷新 |
| 越限样本 | 显示 temperature_limit / pressure_limit 或对应中文说明 |
| 无新鲜样本 | 页面或 API 明确提示数据过期，不继续误导用户 |

证据路径：

| 证据 | 绝对路径 |
| --- | --- |
| 监控中文截图 | `X:\tianhks\output\playwright\vue-parity-monitor-alarm-zh.png` |
| 监控英文截图 | `X:\tianhks\output\playwright\vue-parity-monitor-alarm-en.png` |
| 视觉证据索引 | `X:\tianhks\docs\upper_computer_visual_evidence_index.md` |

---

## Slide 5. 手动控制和安全门控

培训目标：

让操作员知道怎样安全设置目标值，以及系统为什么会拒绝某些目标。

讲解要点：

| 安全机制 | 说明 |
| --- | --- |
| 范围限制 | 温度、压力、转速、流量等目标必须在配置范围内 |
| 单次步长 | 防止一次性跳变过大 |
| 急停 | 急停后禁止继续下发危险控制 |
| 人工锁 | 现场人工锁定时，上位机不得覆盖现场状态 |
| 传感器超时 | 没有新鲜数据时，不允许基于旧数据继续控制 |
| 禁区组合 | 例如温度和转速组合进入安全禁区时拒绝 |

演示动作：

1. 输入安全范围内目标，观察通过结果。
2. 输入超限目标，观察拒绝结果。
3. 输入温度-转速禁区组合，观察拒绝结果。
4. 打开审计页确认控制请求被记录。

讲师提示：

不要把“页面能输入”理解为“设备一定执行”。最终写入必须同时通过权限、审计 reason、安全门控和设备层校验。

证据路径：

| 证据 | 绝对路径 |
| --- | --- |
| 安全配置 | `X:\tianhks\config\safety.toml` |
| 控制写入验证 | `X:\tianhks\output\playwright\vue-control-write-verification.json` |
| 控制页截图 | `X:\tianhks\output\playwright\vue-control-write-en.png` |

---

## Slide 6. AI 建议、AI 主控和 SOP 草案

培训目标：

让参训人员理解 AI 只是建议和受控执行入口，不是绕过安全链路的自动驾驶。

讲解要点：

| 模块 | 当前能力 | 边界 |
| --- | --- | --- |
| AI 参数建议 | 可基于历史批次和本地优化器给出目标建议 | 真实 StepFun/Qwen/LoRA 资产需外部配置和验收 |
| AI 主控 dry-run | 可预览决策、动作、安全门控和推荐目标 | dry-run 不写设备 |
| AI 主控 execute | 需要权限、审计 reason 和安全校验 | 仍需真实硬件闭环验收 |
| SOP 草案 | 展示摘要、步骤、验收指标、安全说明和下一步 | 当前是只读草案，不代表自动执行完整实验方案 |
| 本地 LoRA | 已有数据集导出、训练编排、manifest 和候选 adapter 晋级边界 | 缺真实权重、生产训练脚本、GGUF/RK 延迟报告 |

演示动作：

1. 打开 AI 页面。
2. 查看推荐缓存状态。
3. 展示 dry-run 结果复核。
4. 展示 SOP 草案中英文切换。
5. 说明 local_ai readiness 的未就绪原因。

证据路径：

| 证据 | 绝对路径 |
| --- | --- |
| AI 中文截图 | `X:\tianhks\output\playwright\vue-parity-ai-zh.png` |
| AI 英文截图 | `X:\tianhks\output\playwright\vue-parity-ai-en.png` |
| SOP 中文截图 | `X:\tianhks\output\upper-computer-sop-zh.png` |
| SOP 英文截图 | `X:\tianhks\output\upper-computer-sop-en.png` |
| 本地 AI 边界说明 | `X:\tianhks\docs\local_ai_adapter_status_addendum.md` |

---

## Slide 7. 工艺探索与批次生命周期

培训目标：

让操作员掌握批次创建、运行、暂停/恢复、停止、记录结果和形成报告的流程。

讲解要点：

| 阶段 | 操作要点 |
| --- | --- |
| 准备 | 确认配置、安全边界、传感器新鲜度和权限 |
| 开始 | 创建批次或选择工艺，记录目标条件 |
| 运行 | 监控样本、报警和控制动作 |
| 暂停/恢复 | 由现场状态和上位机状态共同确认 |
| 停止 | 停止过程并记录结束原因 |
| 结果 | 录入产物结果、产率、产物比例和备注 |
| 报告 | 导出 CSV/XLSX/Markdown 或审计 CSV |

演示动作：

1. 展示工艺生命周期入口。
2. 展示批次状态变化。
3. 在 History 页面录入产品结果。
4. 导出报告。

证据路径：

| 证据 | 绝对路径 |
| --- | --- |
| 工艺生命周期验证 | `X:\tianhks\output\playwright\vue-process-lifecycle-verification.json` |
| 中文生命周期截图 | `X:\tianhks\output\playwright\vue-process-lifecycle-zh.png` |
| 英文生命周期截图 | `X:\tianhks\output\playwright\vue-process-lifecycle-en.png` |

---

## Slide 8. 历史数据、筛选和导出

培训目标：

让用户掌握历史查询、产品结果录入、筛选联动和导出。

讲解要点：

| 功能 | 说明 |
| --- | --- |
| 批次搜索 | 按批次 ID、名称或关键字段查找 |
| 状态筛选 | 按 running、completed、failed 等状态过滤 |
| 产物比例筛选 | 结合产品结果查找目标批次 |
| 结果录入 | 录入产率、产物比例、备注等 |
| 曲线和目标 | 对照样本曲线、目标温度等关键参数 |
| 导出 | CSV/XLSX/Markdown 报告，供复盘和模型训练 |

演示动作：

1. 打开 History 页面。
2. 输入批次搜索条件。
3. 调整状态和产物比例筛选。
4. 录入产品结果并保存。
5. 下载 CSV 或报告。

证据路径：

| 证据 | 绝对路径 |
| --- | --- |
| History 中文截图 | `X:\tianhks\output\playwright\vue-parity-history-zh.png` |
| History 英文截图 | `X:\tianhks\output\playwright\vue-parity-history-en.png` |
| parity 验证报告 | `X:\tianhks\output\playwright\vue-parity-verification.json` |

---

## Slide 9. 审计日志和追溯

培训目标：

让运维和项目负责人知道如何查谁在什么时候做了什么，以及如何判断审计链是否完整。

讲解要点：

| 内容 | 说明 |
| --- | --- |
| 审计对象 | 控制目标、AI 主控、Modbus 写入、第三方任务、导出、系统事件 |
| hash chain | 每条事件和上一条事件关联，支持完整性检查 |
| 导出 | 可导出审计 CSV 供归档和复核 |
| 问题定位 | 通过角色、动作、reason、结果和时间定位问题 |
| 生产边界 | 仍需生产备份、归档、防删和恢复演练 |

演示动作：

1. 打开审计页。
2. 搜索或查看最近控制事件。
3. 导出审计 CSV。
4. 说明如何在问题单里引用审计 event id。

证据路径：

| 证据 | 绝对路径 |
| --- | --- |
| 审计中文截图 | `X:\tianhks\output\playwright\vue-browser-matrix-chromium-audit-zh.png` |
| 审计英文截图 | `X:\tianhks\output\playwright\vue-browser-matrix-chromium-audit-en.png` |
| 审计导出验证 | `X:\tianhks\output\playwright\vue-audit-export-verification.json` |

---

## Slide 10. Modbus 调试

培训目标：

让工程师理解寄存器映射、调试入口和写入边界，避免把调试入口当成生产控制通道。

讲解要点：

| 内容 | 说明 |
| --- | --- |
| 寄存器 map | 温度、压力、转速、流量、浓度、pH、目标值等点位 |
| 读入口 | 用于联调和核对设备状态 |
| 写入口 | 只用于受控调试，必须走权限、审计 reason 和安全校验 |
| 外部工具 | Modbus Poll/Slave 或同类工具需要补现场验收截图 |
| 真实硬件 | STM32 最终地址、单位、缩放系数必须由硬件侧确认 |

演示动作：

1. 打开 Modbus 页面。
2. 展示寄存器 map。
3. 做只读查询。
4. 说明 admin 写入测试必须填写 reason 并经过安全链路。

证据路径：

| 证据 | 绝对路径 |
| --- | --- |
| Modbus 映射手册 | `X:\tianhks\docs\upper_computer_modbus_register_map.md` |
| Modbus 中文截图 | `X:\tianhks\output\playwright\vue-i18n-modbus-zh.png` |
| Modbus 英文截图 | `X:\tianhks\output\playwright\vue-i18n-modbus-en.png` |
| Modbus 写入验证 | `X:\tianhks\output\playwright\vue-modbus-write-verification.json` |

---

## Slide 11. AINAS、MQTT 和 REST 对接

培训目标：

让第三方对接人员知道上位机如何接收任务、回执和报警，以及哪些证据还要现场补齐。

讲解要点：

| 接口 | 当前能力 | 现场还缺 |
| --- | --- | --- |
| REST API | HMI、CLI 和第三方系统共用 API；支持状态、配置、控制、历史、AI、审计等 | Postman 或第三方系统验收记录 |
| AINAS | 支持任务创建、查询、执行回执和 AES 静态加密 | 真实平台任务下发和回执截图 |
| MQTT | 支持任务 topic、receipt、alert 快照和 TLS 配置边界 | 外部 broker、账号、证书链、断线重连记录 |
| Modbus TCP | 本地 PDU/TLS 路径具备 | 外部工具和生产网络验收 |

演示动作：

1. 打开设置或集成状态。
2. 展示 AINAS/MQTT/Modbus 状态。
3. 打开 API 验收手册说明对接步骤。

证据路径：

| 证据 | 绝对路径 |
| --- | --- |
| API 验收手册 | `X:\tianhks\docs\upper_computer_api_acceptance_manual.md` |
| 第三方接口报告 | `X:\tianhks\docs\third_party_interface_acceptance_report.md` |
| AINAS/MQTT 验证脚本 | `X:\tianhks\scripts\verify-ainas-mqtt.mjs` |
| 一键验收报告 | `X:\tianhks\output\acceptance\acceptance-report.json` |

---

## Slide 12. 系统配置和安全配置

培训目标：

让运维知道哪些配置可以改、哪些必须走变更审批，以及敏感信息不能暴露在哪里。

讲解要点：

| 配置 | 说明 |
| --- | --- |
| `device.toml` | 设备、采集、串口或桥接配置 |
| `safety.toml` | 安全范围、步长、禁区、传感器超时 |
| `integration.toml` | AINAS、MQTT、Modbus TCP、TLS 等集成配置 |
| `ai_memory.toml` | AI 记忆、训练数据、adapter 路径和 readiness |
| 环境变量 | 数据库加密 key、auth secret、外部 provider key |
| 证书 | HTTP/MQTT/Modbus TCP 证书和私钥必须按生产策略托管 |

演示动作：

1. 打开 Settings 页面。
2. 查看安全、集成、本地 AI 和加密状态。
3. 说明哪些配置变更后需要重启或复验。

证据路径：

| 证据 | 绝对路径 |
| --- | --- |
| 设备配置 | `X:\tianhks\config\device.toml` |
| 安全配置 | `X:\tianhks\config\safety.toml` |
| 集成配置 | `X:\tianhks\config\integration.toml` |
| AI 配置 | `X:\tianhks\config\ai_memory.toml` |
| 密钥生命周期 | `X:\tianhks\docs\upper_computer_security_key_lifecycle.md` |

---

## Slide 13. 异常处理和应急流程

培训目标：

让值守人员知道发生异常时先做什么、看哪里、怎么记录。

讲解要点：

| 异常 | 处理原则 |
| --- | --- |
| 传感器超时 | 停止基于旧数据继续控制，检查设备通讯和样本入口 |
| 温度/压力越限 | 优先现场安全，确认急停、冷却、泄压或人工处置流程 |
| 控制被拒绝 | 查看拒绝原因，检查范围、步长、禁区、人工锁和权限 |
| AI 不可用 | 不阻断基础监控控制，按本地优化器或人工流程处理 |
| MQTT/AINAS 断连 | 核对网络、证书、账号和 broker/platform 状态 |
| 数据库或导出失败 | 保护原始数据库和日志，转交维护人员处理 |

演示动作：

1. 展示一次控制拒绝或报警。
2. 打开审计记录定位原因。
3. 打开维护手册说明如何收集证据。

证据路径：

| 证据 | 绝对路径 |
| --- | --- |
| 维护手册 | `X:\tianhks\docs\upper_computer_maintenance_manual.md` |
| 外部验收清单 | `X:\tianhks\docs\upper_computer_external_acceptance_checklist.md` |
| 生产运维文档 | `X:\tianhks\docs\upper_computer_production_operations.md` |

---

## Slide 14. 部署、备份和维护

培训目标：

让运维知道如何启动、检查、备份、恢复和升级。

讲解要点：

| 操作 | 说明 |
| --- | --- |
| 本地启动 | 使用 release 或 debug daemon 启动 HMI/API |
| RK 部署 | 按 RK 部署验收指南配置 systemd、路径和权限 |
| 健康检查 | `/health`、HMI 系统状态、日志和进程状态 |
| 备份 | SQLite、配置、证书、审计导出和训练资产要分级备份 |
| 恢复 | 需要本地恢复演练和现场/RK 恢复演练 |
| 升级回滚 | 记录版本、SHA256、配置变更、回滚命令和复测结果 |

演示动作：

1. 展示 `/health`。
2. 展示备份/恢复演练报告位置。
3. 展示 RK 部署验收指南。

证据路径：

| 证据 | 绝对路径 |
| --- | --- |
| RK 部署验收指南 | `X:\tianhks\docs\upper_computer_rk_deployment_acceptance_guide.md` |
| 维护手册 | `X:\tianhks\docs\upper_computer_maintenance_manual.md` |
| 恢复演练输出目录 | `X:\tianhks\output\acceptance\restore-drill` |
| 一键验收报告 | `X:\tianhks\output\acceptance\acceptance-report.md` |

---

## Slide 15. 用户验收范围

培训目标：

让项目负责人明确验收时要看哪些页面、哪些接口、哪些证据，以及哪些不能由上位机单方完成。

验收范围：

| 范围 | 本地可验收 | 仍需外部证据 |
| --- | --- | --- |
| 七大 HMI 页面 | 页面加载、中英切换、基础交互、无占位符、无控制台错误 | 客户签字、真实部署地址、多端真机 |
| 控制安全 | 范围、步长、禁区、急停、人工锁、传感器超时 | 真实执行器动作和故障注入 |
| 历史与导出 | 查询、筛选、产品结果、CSV/XLSX/Markdown | 真实实验数据样本 |
| 审计 | hash chain、CSV 导出、控制事件追溯 | 生产归档、防删和恢复 |
| AI | dry-run、SOP 草案、本地训练边界 | 真实 Qwen/GGUF/LoRA/RK 延迟 |
| 第三方接口 | REST、AINAS/MQTT/Modbus 本地路径 | AINAS、broker、Modbus Poll/Slave 外部截图 |

演示动作：

1. 打开用户验收脚本。
2. 说明逐项填结果、证据路径、问题编号和签字。
3. 说明失败项必须复测闭环。

证据路径：

| 证据 | 绝对路径 |
| --- | --- |
| 用户验收操作脚本 | `X:\tianhks\docs\upper_computer_user_acceptance_script.md` |
| 培训签到与问题记录 | `X:\tianhks\docs\upper_computer_training_attendance_and_issues.md` |
| 外部验收清单 | `X:\tianhks\docs\upper_computer_external_acceptance_checklist.md` |

---

## Slide 16. 常见问题

培训目标：

让用户在常见异常下能先自查，并知道什么时候需要找上位机、硬件、算法或平台负责人。

| 问题 | 优先检查 | 责任边界 |
| --- | --- | --- |
| 页面打不开 | 服务是否启动、端口、浏览器、证书、网络 | 上位机/运维 |
| 实时数据为空 | 传感器是否有新鲜样本、STM32/RS485、样本入口 | 硬件/上位机 |
| 控制写入失败 | 角色权限、reason、安全范围、禁区、急停、人工锁 | 上位机/现场 |
| AI 显示不可用 | StepFun key、本地模型权重、adapter、训练脚本、RK 推理服务 | 算法/上位机 |
| MQTT/AINAS 任务失败 | broker/platform、账号、证书、topic、任务格式 | 第三方平台/上位机 |
| 导出失败 | 目录权限、数据库、磁盘空间、文件占用 | 上位机/运维 |
| 审计链异常 | 数据库完整性、备份恢复、手动篡改风险 | 上位机/安全 |

结课动作：

1. 参训人员完成签到。
2. 现场问题写入问题闭环表。
3. 项目负责人确认是否进入用户验收脚本执行阶段。

证据路径：

| 证据 | 绝对路径 |
| --- | --- |
| 培训签到与问题记录 | `X:\tianhks\docs\upper_computer_training_attendance_and_issues.md` |
| 用户手册 | `X:\tianhks\docs\upper_computer_user_manual.md` |
| 维护手册 | `X:\tianhks\docs\upper_computer_maintenance_manual.md` |
