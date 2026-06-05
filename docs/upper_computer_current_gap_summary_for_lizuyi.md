# 李祖祎上位机当前缺口摘要

日期：2026-06-05

本文档面向项目内部汇报，用一句话说明当前上位机状态：上位机主体软件已经具备本地运行、演示、联调和继续验收的基础，但仍不是 PRD 意义上的全量最终交付版本。

## 1. 当前已经能说明什么

| 范围 | 当前结论 | 主要证据 |
| --- | --- | --- |
| Web HMI | 七大页面主体、本地监控、控制、AI、历史、审计、Modbus、系统配置已具备演示能力 | `static/index.html`、`docs/upper_computer_visual_evidence_index.md`、`output/upper-computer-hmi-live-sample-final.png` |
| 中英切换 | 主要静态字块和动态状态字块已接入语言切换，SOP 草案也能切换 | `static/index.html`、`output/upper-computer-sop-zh.png`、`output/upper-computer-sop-en.png` |
| REST / CLI | HMI、CLI 和第三方调用共用的本地 API 已具备；CLI 覆盖状态、配置、数据、控制、AI、审计、Modbus、安全和性能冒烟 | `docs/upper_computer_api_acceptance_manual.md`、`docs/upper_computer_cli_reference.md` |
| 数据与报告 | SQLite 批次/样本、CSV/XLSX 导出、Markdown 实验报告、审计 CSV 已具备 | `docs/upper_computer_user_manual.md`、`docs/upper_computer_test_report.md` |
| 安全控制 | 范围、步长、急停、人工锁、传感器超时、温度-转速禁区、独立 safety guard 已有本地自动化证据 | `config/safety.toml`、`tests/control_tests.rs`、`docs/upper_computer_test_report.md` |
| 第三方接口基础 | AINAS REST、MQTT bridge、Modbus RTU/TCP 映射和 Modbus TCP/TLS 本地路径已准备 | `docs/third_party_interface_acceptance_report.md`、`docs/upper_computer_modbus_register_map.md` |
| AI 推荐缓存语义 | `GET /api/recommendations/latest` 保持只读；StepFun 配置下的本地缓存推荐会标记为 `stale_local_recommendation`，AI 主控前需用 POST 重新生成 | `src/api.rs`、`src/ai_provider.rs`、`tests/api_tests.rs` |
| 本地性能冒烟 | 本地 Web/API 往返、安全计算和 Windows debug 资源快照已有短测证据 | `output/upper-computer-perf-smoke.json`、`output/upper-computer-resource-snapshot.json` |
| 交付文档 | 开发、用户、测试、API、CLI、维护、Modbus、RK、外部验收、交付索引等文档已有初版 | `docs/upper_computer_delivery_readiness_index.md` |
| PRD 偏离口径 | Vue/SQLx/tokio-modbus/LoRA/安全进程/备份擦除/页面命名等偏离已单独说明，方便评审统一说法 | `docs/architecture-deviations.md` |

## 2. 当前还不能宣称完成什么

| 优先级 | 缺口 | 为什么不能本地直接宣称完成 | 需要的证据 |
| --- | --- | --- | --- |
| P0 | STM32 / 反应釜整机 Modbus RTU 联调 | 当前只有上位机默认映射和本地协议路径，没有真实 STM32 固件、寄存器最终表和执行器闭环 | STM32 最终寄存器手册、RTU 实机读写记录、传感器/执行器闭环报告、故障注入记录 |
| P0 | 本地 Qwen3.5-2B + LoRA | 当前只暴露模型资产缺口和本地优化器，不含真实权重、adapter、训练脚本、GGUF 推理链路 | 模型权重、LoRA adapter、PEFT 训练脚本、GGUF 转换脚本、daemon 推理/训练 API、RK 延迟报告 |
| P0 | AINAS / MQTT / Modbus Poll 外部验收 | 当前是本地接口和协议实现，没有真实平台、真实 broker 或外部工具截图 | AINAS 任务下发回执、MQTT.fx/mosquitto 连接和断线重连记录、Modbus Poll/Slave 读写截图 |
| P1 | PRD 性能指标最终证明 | 本地冒烟只证明 HTTP/API 和安全计算，不证明真实 RS485、执行器链路和 RK release 稳态 | 真实采集/控制 <100ms、LoRA <3s、release/RK CPU/内存、7x24 或 30 天、RS485 丢包率、MTBF 论证 |
| P1 | 生产安全验收 | 本地有 AES/TLS/RBAC/审计/密钥清单，但没有生产证书链和第三方安全测试报告 | 证书链、密钥托管/轮换演练、watchdog/权限隔离、OWASP ZAP/Nessus 或等效报告 |
| P1 | 多浏览器/移动端/用户验收 | 当前只做了本地浏览器视觉验证，未形成客户代表验收签字 | Chrome/Firefox/Safari、iOS/Android 截图，用户验收脚本、问题闭环和签字记录 |
| P1 | 培训交付物 | 用户手册已有，但没有独立培训 PPT 和视频 | PPT、视频脚本、录制文件、培训签到和问题记录 |

## 3. 对外汇报建议用语

建议表述：

> 李祖祎负责的上位机本地软件主体、HMI、API、CLI、数据导出、安全门控、基础第三方接口和交付文档已经完成到联调准备版。当前剩余风险主要是硬件实机联调、真实 Qwen/LoRA、外部平台验收、生产安全/性能可靠性和用户培训签字，这些需要硬件、算法、外部平台和验收环境共同补证。

避免表述：

> 上位机已经完整满足 PRD。

原因是 PRD 中的真实 LoRA 自进化、STM32 整机闭环、工业 30 天运行、安全扫描和外部平台验收还没有最终证据。

## 4. 接下来最小闭环顺序

1. 固化 STM32 最终寄存器手册，并用上位机完成 RTU 读写、控制下发、异常保护联调。
2. 用 AINAS、MQTT.fx/mosquitto、Modbus Poll/Slave 补齐第三方接口验收截图和日志。
3. 由算法侧提供 Qwen3.5-2B/GGUF/LoRA/训练脚本/RK 报告，再接入上位机推理与训练入口。
4. 在 RK 或目标 PC 上生成 release 部署包、SHA256、资源采样和部署计时记录。
5. 执行多浏览器/移动端、用户验收、培训 PPT/视频和签字归档。
6. 若团队坚持 PRD 原始技术栈，单独排期 Vue/Element Plus/ECharts、SQLx 和 tokio-modbus 迁移，避免和当前 PoC 功能验收混在同一条线上。
