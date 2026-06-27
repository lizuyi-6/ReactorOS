# 李祖祎上位机当前缺口摘要

日期：2026-06-07

本文档面向项目内部汇报，用一句话说明当前上位机状态：上位机主体软件已经具备本地运行、演示、联调和继续验收的基础，但仍不是 PRD 意义上的全量最终交付版本。

## 1. 当前已经能说明什么

| 范围 | 当前结论 | 主要证据 |
| --- | --- | --- |
| Web HMI | PRD Vue3/Element Plus/ECharts/Pinia 生产构建已进入 release 默认资源路径；七大页面主体、本地监控、控制、AI、历史、审计、Modbus、系统配置已具备演示能力；History 页已补批次搜索、状态筛选、产物比例筛选、产物结果录入、产率/产物比例/目标参数展示和 CSV/XLSX 下载点击验证；Monitor 页已补正式样本入口触发的温度/压力越限报警流、中英报警字段映射和截图验证；AI 页已补 master-control 结果复核、SOP 结构化草案和暗色描述表可读性；legacy `static/index.html` 保留为回退；手机/平板 Chromium 视口本地自动化检查已补；Playwright 浏览器矩阵严格模式已覆盖 bundled Chromium、系统 Chrome、系统 Microsoft Edge、Firefox 与 WebKit 桌面七路由 × 中英，70 个页面/语言组合全通过，0 skipped，0 console error | `frontend/dist/index.html`、`scripts/verify-vue-release-assets.mjs`、`scripts/verify-vue-mobile.mjs`、`scripts/verify-vue-browser-matrix.mjs`、`docs/upper_computer_visual_evidence_index.md`、`output/playwright/vue-parity-verification.json`、`output/playwright/vue-mobile-verification.json`、`output/playwright/vue-browser-matrix-verification.json`、`output/playwright/vue-parity-history-zh.png`、`output/playwright/vue-parity-history-en.png`、`output/playwright/vue-parity-monitor-alarm-zh.png`、`output/playwright/vue-parity-monitor-alarm-en.png`、`output/playwright/vue-parity-ai-zh.png`、`output/playwright/vue-parity-ai-en.png` |
| 中英切换 | Vue 七路由关键字块、主要静态字块和动态状态字块已接入语言切换，SOP 草案和 AI 结果复核字块也能切换；桌面 parity、手机/平板 gate 与 Chromium/Chrome/Edge/Firefox/WebKit 浏览器矩阵均已加入 `[object Object]` 渲染占位符检查 | `output/playwright/vue-i18n-verification.json`、`output/playwright/vue-parity-verification.json`、`output/playwright/vue-mobile-verification.json`、`output/playwright/vue-browser-matrix-verification.json`、`output/playwright/vue-parity-ai-zh.png`、`output/playwright/vue-parity-ai-en.png` |
| REST / CLI | HMI、CLI 和第三方调用共用的本地 API 已具备；CLI 覆盖状态、配置、数据、控制、AI、审计、Modbus、安全和性能冒烟 | `docs/upper_computer_api_acceptance_manual.md`、`docs/upper_computer_cli_reference.md` |
| 数据与报告 | SQLite 批次/样本、CSV/XLSX 导出、Markdown 实验报告、审计 CSV 已具备 | `docs/upper_computer_user_manual.md`、`docs/upper_computer_test_report.md` |
| 安全控制 | 范围、步长、急停、人工锁、传感器超时、温度-转速禁区、独立 safety guard 已有本地自动化证据 | `config/safety.toml`、`tests/control_tests.rs`、`docs/upper_computer_test_report.md` |
| 第三方接口基础 | AINAS REST、MQTT bridge、Modbus RTU/TCP 映射和 Modbus TCP/TLS 本地路径已准备 | `docs/third_party_interface_acceptance_report.md`、`docs/upper_computer_modbus_register_map.md` |
| AI 推荐缓存语义 | `GET /api/recommendations/latest` 保持只读；StepFun 配置下的本地缓存推荐会标记为 `stale_local_recommendation`，AI 主控前需用 POST 重新生成 | `src/api.rs`、`src/ai_provider.rs`、`tests/api_tests.rs` |
| 本地 LoRA 训练边界 | `xingshu ai train --export-only` 已能从真实 SQLite 批次、产品结果、样本和审计事件导出监督训练 JSONL；配置训练入口后可编排调用本地训练脚本，并生成训练 manifest；显式 `--promote` 可在评估分数达标时备份并晋级候选 adapter | `src/bin/xingshu.rs`、`src/local_ai.rs`、`tests/cli_tests.rs`、`scripts/probe-cli-ops.ps1` |
| 本地性能冒烟 | 本地 Web/API 往返、安全计算和 Windows debug 资源快照已有短测证据 | `output/upper-computer-perf-smoke.json`、`output/upper-computer-resource-snapshot.json` |
| 交付文档 | 开发、用户、测试、API、CLI、维护、Modbus、RK、外部验收、交付索引等文档已有初版；培训课件源稿、16 页可编辑 PPTX 草稿、静音 MP4 课件轮播草稿、用户验收操作脚本、培训签到与问题闭环模板、现场交付执行包说明已补齐 | `docs/upper_computer_delivery_readiness_index.md`、`docs/upper_computer_training_deck.md`、`docs/upper_computer_training_deck.pptx`、`docs/upper_computer_training_video_storyboard.md`、`outputs/manual-20260607-training/video/upper_computer_training_video_draft.mp4`、`docs/upper_computer_user_acceptance_script.md`、`docs/upper_computer_training_attendance_and_issues.md`、`docs/upper_computer_field_delivery_execution_pack.md` |
| PRD 偏离口径 | Vue/SQLx/tokio-modbus/LoRA/安全进程/备份擦除/页面命名等偏离已单独说明，方便评审统一说法 | `docs/architecture-deviations.md` |

## 2. 当前还不能宣称完成什么

| 优先级 | 缺口 | 为什么不能本地直接宣称完成 | 需要的证据 |
| --- | --- | --- | --- |
| P0 | STM32 / 反应釜整机 Modbus RTU 联调 | 当前只有上位机默认映射和本地协议路径，没有真实 STM32 固件、寄存器最终表和执行器闭环 | STM32 最终寄存器手册、RTU 实机读写记录、传感器/执行器闭环报告、故障注入记录 |
| P0 | 本地 Qwen3.5-2B + LoRA | 当前上位机已补齐数据集导出、训练命令编排、manifest 归档和显式候选 adapter 晋级/备份边界，但不含真实权重、生产 adapter、生产 PEFT 训练脚本、GGUF 推理链路和 RK 验收 | 模型权重、生产 LoRA adapter、生产 PEFT 训练脚本、GGUF 转换脚本、daemon 推理 API、自动触发/审批策略、RK 延迟报告 |
| P0 | AINAS / MQTT / Modbus Poll 外部验收 | 当前是本地接口和协议实现，没有真实平台、真实 broker 或外部工具截图 | AINAS 任务下发回执、MQTT.fx/mosquitto 连接和断线重连记录、Modbus Poll/Slave 读写截图 |
| P1 | PRD 性能指标最终证明 | 本地冒烟只证明 HTTP/API 和安全计算，不证明真实 RS485、执行器链路和 RK release 稳态 | 真实采集/控制 <100ms、LoRA <3s、release/RK CPU/内存、7x24 或 30 天、RS485 丢包率、MTBF 论证 |
| P1 | 生产安全验收 | 本地有 AES/TLS/RBAC/审计/密钥清单，但没有生产证书链和第三方安全测试报告 | 证书链、密钥托管/轮换演练、watchdog/权限隔离、OWASP ZAP/Nessus 或等效报告 |
| P1 | 多浏览器/移动端/用户验收 | 当前已补 Chromium 桌面/手机/平板、系统 Chrome、系统 Edge、Firefox、WebKit 桌面视口自动化截图和可复跑浏览器矩阵脚本；用户验收操作脚本已补齐；仍没有 macOS Safari、真实 iOS/Android 和客户代表验收签字 | 补 macOS Safari、iOS/Android 真机截图、按 `docs/upper_computer_user_acceptance_script.md` 执行验收、问题闭环和签字记录 |
| P1 | 培训交付物 | 用户手册、培训课件 Markdown 源稿、16 页可编辑 PPTX 草稿、视频 storyboard、静音 MP4 课件轮播草稿、签到与问题闭环模板已具备；但没有按现场最终截图更新的 PPTX、真实操作录屏/旁白 MP4 和真实培训签字 | 现场最终版 PPTX、真实操作录屏文件、培训签到、问题闭环和签字记录 |

## 3. 对外汇报建议用语

建议表述：

> 李祖祎负责的上位机本地软件主体、Vue 默认 HMI 资源路径、API、CLI、数据导出、安全门控、基础第三方接口、History 产物结果录入/筛选/导出本地验证、Monitor 正式样本报警流验证、AI 结果复核/SOP 结构化展示、LoRA 数据集导出/训练编排边界、培训课件源稿、PPTX 草稿、静音 MP4 草稿、用户验收脚本和交付文档已经完成到联调准备版。当前剩余风险主要是硬件实机联调、真实 Qwen/GGUF/LoRA 模型资产与 RK 验收、外部平台验收、生产安全/性能可靠性、现场硬件报警验收、多端验收、现场最终版 PPTX/真实操作录屏视频和用户培训签字，这些需要硬件、算法、外部平台和验收环境共同补证。

避免表述：

> 上位机已经完整满足 PRD。

原因是 PRD 中的真实 LoRA 自进化、STM32 整机闭环、工业 30 天运行、安全扫描和外部平台验收还没有最终证据。

## 4. 接下来最小闭环顺序

1. 固化 STM32 最终寄存器手册，并用上位机完成 RTU 读写、控制下发、异常保护联调。
2. 用 AINAS、MQTT.fx/mosquitto、Modbus Poll/Slave 补齐第三方接口验收截图和日志。
3. 由算法侧提供 Qwen3.5-2B/GGUF/LoRA/生产训练脚本/RK 报告，再接入上位机推理入口，并用现有 `xingshu ai train` 导出/编排链路做训练验收。
4. 在 RK 或目标 PC 上生成 release 部署包、SHA256、资源采样和部署计时记录。
5. 执行 macOS Safari/iOS/Android 复验，按 `docs/upper_computer_user_acceptance_script.md` 做用户验收，用 `docs/upper_computer_training_deck.pptx` 更新现场最终版培训 PPTX，按 `docs/upper_computer_training_video_storyboard.md` 录制真实操作培训视频并完成签字归档。
6. 若团队坚持 PRD 原始技术栈完全闭环，继续单独排期现场硬件报警流、SQLx schema migration 和 Modbus TCP/tokio-modbus 评估，避免和当前 PoC 功能验收混在同一条线上。
