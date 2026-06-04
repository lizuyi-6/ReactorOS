# 星宿智能反应釜上位机需求缺口矩阵

日期：2026-06-04

对象：李祖祎负责的 RK/PC 上位机软件。  
对照来源：

- `星宿智能反应釜体系 (Xingshu Intelligent Reactor System) 产品需求文档 (PRD) v2.2.md`
- `星宿智能反应釜项目-团队分工&开发里程碑&DDL规划方案.docx`
- 当前仓库实现、自动化测试、HMI 视觉验证与本地运行状态

## 1. 总结

当前上位机已经达到“本地 PoC/联调验收”状态：Web HMI、七大页面主体、REST API、CLI、RBAC、审计链、批次数据、导出、AINAS REST、基础 MQTT、Modbus RTU/TCP 调试映射、安全限幅、HTTP/HTTPS、本地 AES 加密、独立 safety guard 均已有实现或本地测试证据。

不能宣称已完全满足 PRD 的内容主要集中在四类：

1. 本地 Qwen3.5-2B + LoRA 推理、训练、自进化、GGUF 转换和 RK 端延迟验收。
2. 真实 STM32/反应釜整机 Modbus RTU 联调、硬件寄存器最终地址/缩放系数固化。
3. MQTT、Modbus TCP、第三方上位机、AINAS 真实外部工具/生产网络验收。
4. 非功能与生产安全验收：本地 Web/API 响应、安全计算、资源快照、密钥生命周期和敏感字段清单已有证据；7x24、MTBF、正式证书链、生产密钥轮换演练、watchdog/权限隔离、漏洞扫描仍未完成。

## 2. 李祖祎团队分工对照

| 团队文档要求 | 当前状态 | 当前证据 | 还缺什么 | 依赖 |
| --- | --- | --- | --- | --- |
| 实时监控、参数配置、AI 智能决策、历史数据、审计日志、Modbus 调试、系统配置七大页面完整开发 | 本地通过 | `static/index.html`；`docs/upper_computer_development_doc.md`；`docs/upper_computer_test_report.md`；本地 `http://127.0.0.1:8000/health` 正常 | 需要正式用户验收记录和多浏览器/移动端完整验收记录 | 上位机、验收环境 |
| 中英切换与所有主要字块切换 | 本地通过 | `static/index.html` i18n map；`docs/upper_computer_test_report.md`；`docs/upper_computer_visual_evidence_index.md`；`output/upper-computer-sop-zh.png`；`output/upper-computer-sop-en.png` | 仍需多浏览器、移动端真机和正式用户验收记录 | 上位机 / 验收环境 |
| 工艺探索模块落地 | 部分完成 | 流程/批次生命周期、AI 建议、历史批次、曲线、报告和只读实验 SOP 草案已实现 | 本地 LoRA 深度参与探索、真实模型自主生成/评审闭环尚未完成 | 上位机 + 算法 |
| 手动/AI 自动双模式切换 | 本地通过 | `/api/control/auto`、`/api/control/targets`、自动控制安全门、HMI Control/AI 页面 | 需要真实硬件执行闭环验证，证明 AI/自动目标能安全下发到 STM32 | 硬件 |
| Modbus 寄存器读写 | 部分完成 | `/api/modbus/registers`；`xingshu modbus map/read/write`；Modbus TCP PDU `01/02/03/06` 测试；`docs/upper_computer_modbus_register_map.md` | STM32 RTU 实机联调、最终寄存器地址/单位/缩放系数确认、Modbus Poll/Slave 外部验收 | 硬件、现场网络 |
| 数据曲线渲染、数据存储与导出 | 本地通过 | SQLite 批次/样本；CSV/XLSX/Markdown 报告；`docs/upper_computer_test_report.md` | 需要真实实验数据持续采集后的完整报告样本 | 硬件/实验 |
| 日志审计功能 | 本地通过 | 审计哈希链、审计 CSV、RBAC 操作事件 | 生产环境需补防篡改运维策略、日志备份/归档制度 | 运维/安全 |
| 通信异常处理 | 部分完成 | 传感器超时、急停、人工锁、控制拒绝、MQTT/Modbus 状态字段 | 真实 RS485 断线、CRC 错误、设备异常、网络闪断的整机故障注入报告 | 硬件、现场网络 |
| 功能测试、边界测试、异常测试、bug 闭环 | 部分完成 | `api_tests`、`cli_tests`、`db_tests`、`control_tests`、`config_tests` 已通过 | PRD 要求的性能、安全、工业环境、用户验收测试还未完整执行 | 测试环境、验收方 |
| AINAS 远程对接 | 本地通过 / 外部待验收 | `/api/integrations/ainas/tasks` 支持 `set_targets`、`start_process`、`stop_process`；任务持久化和 AES 加密 | AINAS 真实平台任务下发、数据提取、回执确认记录 | AINAS 平台 |
| CLI 控制台 | 本地通过 | `xingshu` 覆盖 status/config/data/control/ai/audit/modbus/safety/perf，`xingshu ai plan` 可查看 SOP 草案，`xingshu perf smoke` 可输出本地性能冒烟报告 | `xingshu ai train` 仍按预期暴露 LoRA 训练缺口 | 算法 |
| API/MQTT 协议上位机适配 | 部分完成 | REST API 完成；`docs/upper_computer_api_acceptance_manual.md` 已输出；MQTT bridge 框架、任务、receipt、alert 已实现 | Postman/第三方系统验收记录、MQTT.fx/mosquitto 外部 broker 验收、断线重连/backoff、生产 broker 证书链 | 第三方 broker |
| 输出上位机开发文档、用户手册、测试报告 | 本地完成 | `docs/upper_computer_development_doc.md`、`docs/upper_computer_user_manual.md`、`docs/upper_computer_test_report.md`、`docs/upper_computer_delivery_readiness_index.md`、`docs/upper_computer_cli_reference.md`、`docs/upper_computer_maintenance_manual.md`、`docs/upper_computer_api_acceptance_manual.md`、`docs/upper_computer_modbus_register_map.md`、`docs/upper_computer_rk_deployment_acceptance_guide.md` | 需要最终版随真实联调结果更新；培训 PPT/视频仍待补 | 全团队 |

## 3. PRD 功能需求对照

| PRD 模块 | 需求 | 当前状态 | 当前证据 | 还缺什么 |
| --- | --- | --- | --- | --- |
| 数据采集与控制 | 温度、压力、转速、摇速、流量、浓度、pH 秒级采集 | 部分完成 | Modbus 映射已覆盖 7 类指标；pipeline/SQLite/HMI 可展示；`xingshu data sample --duration-s 180 --interval-ms 500` 可通过正式 v1 样本入口驱动无硬件实时监控演示 | 真实 STM32/传感器秒级采集验证，采集延迟和数据一致性报告 |
| 数据采集与控制 | 温度、转速、流量等自动/手动控制 | 部分完成 | 控制 API、自动控制、手动目标、safety guard | 真实执行器闭环验证；当前流量控制更多是映射/显示，真实控制需硬件确认 |
| 数据采集与控制 | 急停、暂停、恢复 | 部分完成 | 急停 API、停止流程、恢复/重置联锁入口 | “暂停/恢复”作为完整生产语义需和硬件状态机联调确认 |
| AI 智能决策 | 云端大模型参数建议 | 部分完成 | AI provider、历史批次推荐、StepFun/provider 配置边界、只读 SOP 草案接口 | 真实 StepFun 账号、提示词、A/B 测试和云端 SOP 生成验证 |
| AI 智能决策 | 本地 LoRA 参数建议 | 未完成 | `/api/config/summary.local_ai.ready_for_inference=false`；`xingshu ai train` 显示缺口 | Qwen3.5-2B/GGUF、LoRA adapter、llama.cpp 推理接口 |
| AI 智能决策 | 本地 GA/SA/PID 参数寻优 | 本地通过 | `src/optimizer.rs` 的 `local-ga-sa-pid` 策略包含 GA 风格交叉/变异、SA 接受/降温搜索和 PID 风格误差校正；`tests/optimizer_tests.rs` 覆盖边界、禁区和策略 rationale；`cargo test --test optimizer_tests -- --nocapture` 通过 | 仍需真实批次长期指标验证和与本地 LoRA 自进化的联动验收 |
| AI 智能决策 | 本地模型自动增量微调/自进化 | 未完成 | `docs/local_ai_adapter_status_addendum.md` 明确为 readiness boundary | PEFT/LoRA 训练脚本、数据集契约、自动触发、评估回滚、RK 验收 |
| AI 智能决策 | AI 实验方案/SOP 自动生成 | 部分完成 | `GET /api/ai/experiment-plan` 和 `xingshu ai plan` 可基于批次推荐与安全边界生成只读三段式 SOP 草案；HMI AI 页可展示 | 仍缺云端/本地 LoRA 模型自主生成完整实验方案、人工审核流和真实执行闭环 |
| 安全控制 | 独立安全过滤器、非法参数拦截 | 本地通过 | `reactor-safety-guard`、`xingshu safety check`、`control_tests` | 生产 watchdog、权限隔离、故障演练 |
| 安全控制 | 单次步长限制 | 本地通过 | `config/safety.toml`、控制测试 | 真实硬件执行时的步进行为验证 |
| 安全控制 | 传感器掉线/数据超时保护 | 本地通过 | `sensor_timeout_ms`、控制路径拒绝旧数据 | RS485 断线/CRC 错误/噪声场景实测 |
| 安全控制 | 温度-转速安全禁区 | 本地通过 | `config/safety.toml` 的 `forbidden_control_zones`；`src/control.rs` 自动控制阻断；`src/api.rs` 手动/AI/AINAS/Modbus/工艺写入口拒绝禁区组合；`control_tests::control_blocks_forbidden_temperature_stirrer_zone`；`api_tests::operator_target_update_rejects_forbidden_temperature_stirrer_zone` | 仍需真实硬件异常工况和现场故障注入验收 |
| 数据审计与管理 | 秒级本地持久化 | 部分完成 | SQLite 样本/批次存储 | 真实硬件秒级连续采样与 7x24 数据丢失率验证 |
| 数据审计与管理 | 不可篡改审计日志 | 本地通过 | 审计 hash chain、审计导出 | 生产备份、归档、防删策略 |
| 数据审计与管理 | 历史查询、导出、可视化 | 本地通过 | HMI History、CSV/XLSX、曲线 | 真实数据样本验收 |
| 数据审计与管理 | 实验报告自动生成 | 本地通过 | 单批次 Markdown 报告 | 报告模板需随真实实验字段完善 |
| 系统交互 | PC Web 控制台 | 本地通过 | `http://127.0.0.1:8000/`；`output/upper-computer-hmi-live-sample-final.png` 显示持续样本流下 HMI 为 `SYSTEM HEALTH: NORMAL` | 正式部署域名/证书 |
| 系统交互 | 移动端响应式 UI | 部分完成 | CSS responsive 规则和局部视觉验证 | iOS/Android 真实设备或模拟器完整验收 |
| 系统交互 | 多用户权限管理 | 本地通过 | RBAC bearer session、角色权限 | 生产用户管理、密码策略、审计策略 |
| 系统交互 | 完整 CLI | 本地通过 / AI 训练除外 | `xingshu` 命令集 | 本地 LoRA 训练接入后补齐 `ai train` |
| 第三方集成 | Modbus RTU/TCP 数据上报与指令接收 | 部分完成 | RTU 映射、TCP server、PDU/TLS 本地测试 | RTU 实机、TCP 外部工具、生产证书链 |
| 第三方集成 | REST API | 本地通过 | HMI/CLI/第三方共用 API；`docs/upper_computer_api_acceptance_manual.md` | Postman/第三方系统验收记录可补 |
| 第三方集成 | RS485/RJ45 数据提取与任务下发 | 部分完成 | AINAS REST、MQTT、Modbus TCP/RTU 配置 | 真实 RS485/RJ45 工业网络验收 |
| 硬件兼容 | STM32 Modbus RTU 控制器 | 待外部验收 | 配置和寄存器 map 已准备 | STM32 固件、寄存器手册、实机联调 |
| 硬件兼容 | JSON 文件桥接 | 部分完成 | `docs/json_bridge_protocol.md`、配置字段 | 旧系统实际对接验收 |
| 硬件兼容 | 标准 Modbus TCP/RTU 设备 | 部分完成 | TCP/RTU 基础能力 | 第三方标准设备兼容性矩阵 |

## 4. PRD 非功能需求对照

| 指标 | PRD 要求 | 当前状态 | 还缺什么 |
| --- | --- | --- | --- |
| 数据采集延迟 | < 100ms | 未证明 | 当前控制循环配置为秒级；`xingshu perf smoke` 只证明本地 HTTP 往返，不证明 STM32/RS485 真实采集延迟 |
| 本地 LoRA 推理延迟 | < 3s | 未完成 | 需真实模型、RK3568/RK3588 跑分和报告 |
| 安全控制响应 | < 100ms | 部分完成 | `output/upper-computer-perf-smoke.json` 显示本机 `safety_compute` p95=1ms；仍需真实执行器链路、1000 条/秒压力测试和独立进程生产 watchdog 验收 |
| 内存占用 | < 30MB，不含模型 | 本地快照通过 / 待正式稳态 | `output/upper-computer-resource-snapshot.json` 显示 Windows debug 本地演示进程 working set 26.977MB、private memory 6.102MB；仍需 release/RK 稳态采样 |
| 单核 CPU | < 3% 稳态 | 本地快照通过 / 待正式稳态 | `output/upper-computer-resource-snapshot.json` 5 秒采样 max 1.533%；仍需 release/RK 长时间稳态采样 |
| MTBF | > 10000 小时 | 未证明 | 需要长期运行或等效可靠性论证 |
| 数据丢失率 | 0 | 部分完成 | SQLite 持久化已有；需断电/重启/长时采样恢复测试 |
| 7x24 运行 | 支持 | 未证明 | 需要 72 小时或 30 天运行报告 |
| RS485 丢包率 | < 0.01% | 未证明 | 需要真实 RS485 工业环境测试 |
| 传输和存储加密 | 全部加密 | 部分完成 | HTTP/Modbus TCP TLS、本地 AES 有证据；`docs/upper_computer_security_key_lifecycle.md` 已列出密钥生命周期和敏感字段清单；仍需 MQTT 外部证书链、生产密钥托管/轮换演练 |
| 离线运行 | 断网不影响核心功能 | 部分完成 | 本地服务/SQLite/优化器支持离线；需断网场景测试记录 |
| 部署时间 | < 30 分钟 | 部分完成 | 文档和命令已有；需全新机器部署演练计时 |
| Web 响应 | < 1s | 本地通过 | `xingshu perf smoke --iterations 20` 显示 `/health` p95=4ms、`/api/config/summary` p95=1ms、`/api/devices/status` p95=0ms；`/api/live` 在无外部样本时返回 503 但本地往返 p95=0ms |
| RK3568/RK3588 ARM64 | 支持 | 部分完成 | 有部署文档和 target 目录；需最终设备构建/运行验收 |
| Windows/Linux/macOS Web 访问 | 部分完成 | 当前 Windows 本地可运行 | Linux/macOS 浏览器验收记录待补 |
| iOS/Android 移动端访问 | 部分完成 | 响应式 CSS 已有 | 真机或模拟器验收待补 |
| 标准 Modbus RTU/TCP 设备 | 部分完成 | 协议实现和映射已具备 | 多品牌设备兼容性验收 |

## 5. 当前明确未完成项清单

按交付风险排序：

1. 本地 Qwen3.5-2B + LoRA：缺模型权重、LoRA adapter、PEFT 训练入口、GGUF 转换脚本、RK 延迟报告、真实推理/训练 API。
2. AI 自进化：缺自动触发、训练数据集生成、评估、回滚和“随实验准确率提升”的验证。
3. AI 实验方案/SOP 生成：只读安全门控草案已实现；仍缺真实云端/本地 LoRA 模型自主生成、审核流和执行闭环。
4. STM32/硬件联调：缺真实 RTU 地址/缩放系数固化、传感器/执行器闭环、异常工况硬件保护联调。
5. MQTT 外部验收：缺 MQTT.fx/mosquitto broker、断线重连、生产证书链和账号验收。
6. Modbus TCP/RTU 外部验收：缺 Modbus Poll/Slave、外部 TLS 工具、现场网络验收。
7. 生产安全：密钥生命周期和敏感字段清单已文档化；仍缺生产密钥托管/轮换演练、正式渗透/漏洞扫描、safety guard watchdog/权限隔离/故障演练。
8. 性能与可靠性：已有本地 Web/API 响应、安全计算冒烟和 Windows debug 资源快照；仍缺真实采集/执行控制 <100ms、<3s LoRA、release/RK 稳态 CPU/内存、7x24、MTBF、RS485 丢包率等报告。
9. 移动端/多浏览器：已有响应式实现，但缺 iOS/Android/Chrome/Firefox/Safari 的完整验收记录。
10. 最终交付包：开发文档、用户手册、测试报告、API 手册、CLI 参考、维护手册、Modbus 映射手册、RK 部署验收指南和交付就绪索引已有初版；培训 PPT/视频仍待补，全部文档需随真实硬件、外部接口和算法验收结果更新为最终版。

## 6. 下一步建议

详细外部验收用例、证据字段和归档模板见 `docs/upper_computer_external_acceptance_checklist.md`。下表只保留执行优先级摘要。

PRD 第八章测试计划和团队分工测试职责的逐项追踪见 `docs/upper_computer_test_plan_traceability.md`。

PRD 第十章交付物清单和当前证据对照见 `docs/upper_computer_delivery_readiness_index.md`。面向李祖祎汇报的短版缺口摘要见 `docs/upper_computer_current_gap_summary_for_lizuyi.md`。培训 PPT/视频的制作计划见 `docs/upper_computer_training_material_plan.md`。

建议把剩余工作拆成三条并行线：

| 优先级 | 工作 | 输出物 |
| --- | --- | --- |
| P0 | STM32/Modbus 实机联调 | 上位机默认映射手册已输出；继续补 STM32 最终寄存器确认、RTU/TCP 联调记录、异常工况测试报告 |
| P0 | 第三方接口外部验收 | MQTT.fx/mosquitto、Modbus Poll/Slave、AINAS 真实任务验收记录 |
| P0 | 本地 LoRA 边界补齐 | Qwen/GGUF/LoRA/训练脚本/RK 报告，随后接入 daemon 推理与训练 API |
| P1 | 性能与安全报告 | 本地 API/安全计算冒烟、Windows debug 资源快照、密钥生命周期/敏感字段清单已补；继续补采集/执行控制延迟、release/RK 稳态 CPU/内存、生产证书链/密钥轮换演练、安全扫描报告 |
| P1 | 用户验收与部署 | RK 部署验收指南和培训材料计划已输出；继续补移动端、多浏览器、全新设备部署计时、培训 PPT/视频、用户验收签字记录 |

## 7. 本地事实快照

2026-06-04 当前本地服务：

- `GET http://127.0.0.1:8000/health` 返回 `{"ok": true, "service": "reactor-edge-daemon"}`。
- `xingshu data sample --duration-s 180 --interval-ms 500` 启动持续样本流后，`GET /api/live` 返回 200，HMI 显示 `SYSTEM HEALTH: NORMAL`、实时温度/压力数值；停止样本流超过 `sensor_timeout_ms=6000` 后返回 503 是预期安全行为。
- `xingshu perf smoke --iterations 20` 生成 `output/upper-computer-perf-smoke.json`：
  - 只读 API p95 最高 4ms，满足本地 <100ms 冒烟阈值。
  - `safety_compute` p95=1ms，满足本地 <100ms 安全计算冒烟阈值。
  - 独立 `reactor-safety-guard` 进程启动/JSON 往返 p95=315ms，作为诊断项保留，不等价于控制计算耗时。
- `output/upper-computer-resource-snapshot.json` 记录当前 Windows debug 演示进程资源快照：working set 26.977MB、private memory 6.102MB、CPU 5 秒采样 max 1.533%。该快照不替代 release/RK 长时间稳态验收。
- `docs/upper_computer_security_key_lifecycle.md` 已列出 `XINGSHU_DB_ENCRYPTION_KEY`、`XINGSHU_AUTH_SECRET`、角色密码、TLS/MQTT/Modbus 证书、StepFun key、CLI token 和本地 AI 资产路径的生命周期边界；仍需生产密钥托管和轮换演练。
- `GET /api/config/summary` 显示：
  - `data_security.storage_encryption.enabled = true`，算法为 `AES-256-GCM`。
  - `local_ai.ready_for_inference = false`，`local_ai.ready_for_training = false`。
  - `integrations.mqtt_status.enabled = false`，证书字段已配置但未连接外部 broker。
  - `integrations.modbus_tcp_status.enabled = false`，TLS 状态为 configured，但当前未开启监听。
  - `permissions.authentication = bearer_session_enforced`。

因此当前结论应表述为：上位机主体软件已具备本地运行、演示、联调和继续验收的基础，但还不是 PRD 意义上的全量最终交付版本。
