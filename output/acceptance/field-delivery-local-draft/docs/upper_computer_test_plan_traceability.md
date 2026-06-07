# 星宿智能反应釜上位机测试计划追踪矩阵

日期：2026-06-04

对象：李祖祎负责的 RK/PC 上位机软件测试交付。

对照来源：

- PRD 第八章测试计划：8.1 单元测试、8.2 集成测试、8.3 系统测试、8.4 性能测试、8.5 安全测试、8.6 第三方集成测试、8.7 工业环境测试、8.8 用户验收测试。
- 团队分工文档：李祖祎在阶段三负责软硬件全量联调、AI 模块与工艺模块融合测试、全功能压力测试、边界测试、异常测试、bug 闭环和测试报告。

## 1. 测试计划追踪

| PRD 测试项 | PRD 要求 | 当前证据 | 当前结论 | 还缺什么 |
| --- | --- | --- | --- | --- |
| 8.1 单元测试 | 所有函数与方法，`cargo test`，覆盖率 > 90% | `cargo test --all-targets -- --nocapture --test-threads=1` 已通过；结果见 `docs/upper_computer_test_report.md` | 部分完成 | 还缺正式覆盖率报告 |
| 8.2 集成测试 | Modbus、AI、安全过滤器、第三方集成接口 | REST API、AINAS、MQTT payload、Modbus TCP PDU、safety guard、AI SOP 草案已有自动化/本地证据 | 部分完成 | 还缺真实 STM32 RTU、外部 MQTT broker、Modbus Poll/Slave、AINAS 真实平台 |
| 8.3 系统测试 | 所有功能性需求与非功能性需求 | Web HMI、CLI、REST、RBAC、审计、导出、中英切换和本地样本流已验证 | 部分完成 | 非功能项、硬件闭环、真实 LoRA、用户验收尚未全量完成 |
| 8.4 性能测试 | 数据采集延迟、AI 推理延迟、安全控制响应、资源占用、RS485 丢包率 | `output/upper-computer-perf-smoke.json`、`output/upper-computer-resource-snapshot.json` | 本地冒烟完成 | 缺 STM32/RS485 <100ms、RK LoRA <3s、release/RK 稳态 CPU/内存、RS485 丢包率、7x24/30 天报告 |
| 8.5 安全测试 | 安全过滤器、数据加密、权限管理、网络安全；工具 OWASP ZAP、Nessus | RBAC、审计链、安全禁区、AES-256-GCM、HTTP/Modbus TCP 本地 TLS、密钥生命周期文档已有 | 部分完成 | 缺生产证书链、密钥轮换演练、watchdog/权限隔离、OWASP ZAP/Nessus 或等效报告 |
| 8.6 第三方集成测试 | CLI、REST API、Modbus RTU/TCP、MQTT；工具 Postman、MQTT.fx、Modbus Poll、Modbus Slave | CLI、REST、AINAS 本地通过；`docs/upper_computer_api_acceptance_manual.md` 已输出；Modbus TCP/MQTT 代码路径和本地测试通过 | 部分完成 | 缺 Postman/第三方系统、MQTT.fx/mosquitto、Modbus Poll/Slave、生产网络证据 |
| 8.7 工业环境测试 | 真实工业现场稳定性、抗干扰、通信可靠性，连续运行 30 天 | 本地短时运行、无硬件样本演示和资源快照已有 | 未完成 | 缺 30 天或等效工业环境运行、RS485 干扰、远距离传输、断电恢复报告 |
| 8.8 用户验收测试 | 客户代表按实际业务场景验收 | 本地自测、视觉验证、用户手册和 `docs/upper_computer_user_acceptance_script.md` 场景脚本已有 | 部分完成 | 缺客户代表或项目负责人按脚本执行后的签字、现场证据和问题闭环记录 |

## 2. 团队分工测试职责追踪

| 团队分工要求 | 当前证据 | 当前结论 | 还缺什么 |
| --- | --- | --- | --- |
| 软硬件全量联调 | 上位机 Modbus 映射、`docs/upper_computer_modbus_register_map.md`、RTU/TCP 调试入口已准备 | 待外部验收 | STM32 实机、寄存器最终手册、传感器/执行器闭环 |
| AI 模块与工艺模块融合测试 | 本地优化器、AI 建议、只读 SOP 草案、HMI AI 页和 CLI `ai plan` 已实现 | 部分完成 | 真实 Qwen3.5-2B + LoRA 推理/训练、自进化评估、人工审核流、执行闭环 |
| 全功能压力测试 | `xingshu perf smoke` 已覆盖本机 API 和安全计算冒烟 | 部分完成 | 100 并发 Web、1000 条/秒控制、安全过滤压力、10 个 Modbus TCP 客户端、MQTT 100 条任务 |
| 边界测试 | 范围、步长、禁区、急停、人工锁、传感器超时已有自动化覆盖 | 本地通过 | 真实硬件边界和异常动作仍需联调 |
| 异常测试 | 本地传感器超时、控制拒绝、TLS 参数、AES 兼容、LoRA 缺口显式失败已覆盖 | 部分完成 | RS485 断线、CRC 错误、网络闪断、掉电恢复、执行器异常 |
| bug 闭环 | 当前文档记录缺口和下一步验收项 | 部分完成 | 需要外部验收缺陷编号、修复提交、复测记录 |
| 输出测试报告 | `docs/upper_computer_test_report.md` 已有 | 本地完成 | 最终报告需并入硬件、第三方、RK、安全、用户验收结果 |

## 3. 证据索引

| 类型 | 当前文件 |
| --- | --- |
| 缺口矩阵 | `docs/upper_computer_requirement_gap_matrix.md` |
| 交付就绪索引 | `docs/upper_computer_delivery_readiness_index.md` |
| CLI 参考手册 | `docs/upper_computer_cli_reference.md` |
| 维护手册 | `docs/upper_computer_maintenance_manual.md` |
| 测试报告 | `docs/upper_computer_test_report.md` |
| 外部验收清单 | `docs/upper_computer_external_acceptance_checklist.md` |
| 第三方接口报告 | `docs/third_party_interface_acceptance_report.md` |
| API 验收手册 | `docs/upper_computer_api_acceptance_manual.md` |
| Modbus 映射手册 | `docs/upper_computer_modbus_register_map.md` |
| RK 部署验收指南 | `docs/upper_computer_rk_deployment_acceptance_guide.md` |
| 安全密钥生命周期 | `docs/upper_computer_security_key_lifecycle.md` |
| 视觉证据索引 | `docs/upper_computer_visual_evidence_index.md` |
| 李祖祎短版缺口摘要 | `docs/upper_computer_current_gap_summary_for_lizuyi.md` |
| 培训材料计划 | `docs/upper_computer_training_material_plan.md` |
| 培训课件源稿 | `docs/upper_computer_training_deck.md` |
| 培训 PPTX 草稿 | `docs/upper_computer_training_deck.pptx` |
| 培训视频 storyboard | `docs/upper_computer_training_video_storyboard.md` |
| 培训视频静音草稿 | `outputs/manual-20260607-training/video/upper_computer_training_video_draft.mp4` |
| 培训交付物 gate | `scripts/verify-training-deliverables.mjs`、`output/acceptance/training-deliverables-report.json` |
| 用户验收操作脚本 | `docs/upper_computer_user_acceptance_script.md` |
| 培训签到与问题闭环模板 | `docs/upper_computer_training_attendance_and_issues.md` |
| 本地性能冒烟 | `output/upper-computer-perf-smoke.json` |
| 本地资源快照 | `output/upper-computer-resource-snapshot.json` |

## 4. 当前结论

测试交付当前处于“本地自测和联调准备完成、用户验收脚本与培训材料草稿已准备、外部/生产/工业验收待补”状态。不能把本地测试报告、静音 MP4 草稿和脚本模板等同于 PRD 第八章的最终完整测试验收；最终版测试报告必须并入 STM32、AINAS、MQTT.fx/mosquitto、Modbus Poll/Slave、RK release、LoRA、自进化、安全扫描、工业 30 天、真实培训/用户验收执行记录和签字证据。
