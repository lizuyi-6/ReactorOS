# 星宿智能反应釜上位机现场交付执行包说明

日期：2026-06-07

对象：李祖祎负责的 RK/PC 上位机现场交付、培训、用户验收和证据归档。

状态：本文档是执行入口。它不替代真实验收记录，也不把本地联调准备版包装成 PRD 最终完成版。

## 1. 交付包目标

现场交付时要完成四件事：

| 目标 | 输出 |
| --- | --- |
| 让用户会操作 | 培训 PPTX、培训签到、问题闭环 |
| 让项目负责人能验收 | 用户验收操作脚本、逐项结果、证据路径、签字页 |
| 让外部依赖能补证 | STM32、Qwen/LoRA、AINAS、MQTT、Modbus Poll/Slave、RK、安全扫描、长期运行证据 |
| 让最终交付可追溯 | 软件版本、配置、日志、截图、导出文件、问题单、复测记录 |

## 2. 必读输入

| 类型 | 绝对路径 |
| --- | --- |
| PRD | `C:\Users\Abraham\Downloads\星宿智能反应釜体系 (Xingshu Intelligent Reactor System) 产品需求文档 (PRD) v2.2.md` |
| 团队分工与里程碑 | `C:\Users\Abraham\Downloads\星宿智能反应釜项目-团队分工&开发里程碑&DDL规划方案.docx` |
| 当前缺口摘要 | `X:\tianhks\docs\upper_computer_current_gap_summary_for_lizuyi.md` |
| 需求缺口矩阵 | `X:\tianhks\docs\upper_computer_requirement_gap_matrix.md` |
| 交付就绪索引 | `X:\tianhks\docs\upper_computer_delivery_readiness_index.md` |
| 外部验收清单 | `X:\tianhks\docs\upper_computer_external_acceptance_checklist.md` |
| 现场证据签收清单 | `X:\tianhks\docs\upper_computer_field_evidence_checklist.md` |
| 现场证据签收清单 JSON | `X:\tianhks\docs\upper_computer_field_evidence_checklist.json` |

## 3. 培训交付物

| 文件 | 用途 | 当前状态 |
| --- | --- | --- |
| `X:\tianhks\docs\upper_computer_training_deck.md` | 培训课件源稿，含逐页讲解要点、演示动作和证据路径 | 已完成 |
| `X:\tianhks\docs\upper_computer_training_deck.pptx` | 16 页可编辑培训 PPTX 草稿，已嵌入 AI 生成视觉资产 | 已生成 |
| `X:\tianhks\docs\upper_computer_training_video_storyboard.md` | 培训视频分镜和现场录屏要求 | 已完成 |
| `X:\tianhks\outputs\manual-20260607-training\video\upper_computer_training_video_draft.mp4` | 静音课件轮播 MP4 草稿，只作培训素材预览 | 已生成 |
| `X:\tianhks\docs\assets\upper-computer-training\README.md` | 培训 PPT 图片资产说明，明确图片不是真实验收证据 | 已完成 |
| `X:\tianhks\scripts\generate-upper-computer-training-deck.mjs` | 从脚本复生成 PPTX 和预览图 | 已完成 |
| `X:\tianhks\scripts\generate-upper-computer-training-video.mjs` | 从课件预览图复生成静音 MP4 草稿 | 已完成 |
| `X:\tianhks\docs\upper_computer_training_material_plan.md` | 培训 PPT/视频计划和录制脚本 | 已更新 |
| `X:\tianhks\docs\upper_computer_training_attendance_and_issues.md` | 培训签到、覆盖项、问题闭环和签字模板 | 已完成 |

仍待现场完成：

| 待完成项 | 说明 |
| --- | --- |
| 现场最终版 PPTX | 用真实部署地址、真实截图、最终账号策略和现场联调结果更新 `upper_computer_training_deck.pptx` |
| 真实操作 MP4 视频 | 按 `upper_computer_training_video_storyboard.md` 录制 8 到 12 分钟操作视频，补旁白或字幕 |
| 培训签到 | 复制 `upper_computer_training_attendance_and_issues.md`，按日期归档并签字 |
| 问题闭环 | 培训提出的问题必须有责任人、计划日期、复测结论 |

## 4. 用户验收交付物

| 文件 | 用途 | 当前状态 |
| --- | --- | --- |
| `X:\tianhks\docs\upper_computer_user_acceptance_script.md` | 16 项 UAT 操作脚本，含步骤、预期、证据、问题编号和签字栏 | 已完成 |
| `X:\tianhks\docs\upper_computer_external_acceptance_checklist.md` | STM32、MQTT、AINAS、Modbus、RK、安全、性能等外部验收清单 | 已完成 |
| `X:\tianhks\docs\upper_computer_visual_evidence_index.md` | 当前本地 HMI 视觉证据索引 | 已更新 |
| `X:\tianhks\output\playwright\vue-browser-matrix-verification.json` | Chromium、Chrome、Edge、Firefox、WebKit 浏览器矩阵严格模式自动化结果；5 个浏览器、70 个页面/语言组合、0 skipped、0 console error | 已生成 |
| `X:\tianhks\output\acceptance\acceptance-report.json` | 本地一键验收结果 | 已生成 |

验收执行顺序：

1. 先执行 `upper_computer_user_acceptance_script.md` 的 PRE-01 到 PRE-06。
2. 执行 UAT-01 到 UAT-12，覆盖七大页面、中英、RBAC、监控、报警、控制、AI、批次、历史、审计、Modbus 和配置。
3. 有外部平台时执行 UAT-13，补 REST/AINAS/MQTT 真实证据。
4. 有多端设备时执行 UAT-14，补 macOS Safari、iOS/Android 真机证据；Firefox 已有本机 Playwright 严格矩阵通过证据，现场如有真实 Firefox 仍可补人工截图。
5. 有 RK/现场设备时执行 UAT-15，补部署、备份、恢复、离线运行和回滚证据。
6. 执行 UAT-16，汇总 P0/P1 结果、失败项、风险接受项和签字。

## 5. 现场证据目录建议

建议每次现场验收复制以下结构：

```text
X:\tianhks\output\acceptance\field-delivery-<YYYYMMDD>\
  00-summary\
  01-training\
  02-uat\
  03-hardware-stm32\
  04-ai-lora-rk\
  05-third-party\
  06-security-performance\
  07-signatures\
```

本地已提供一个可复生成的交付包草稿，用于把当前文档、培训素材、UAT 脚本、gate 报告和边界说明集中到一个目录。生成命令：

```powershell
node X:\tianhks\scripts\package-upper-computer-delivery.mjs
```

输出目录：

```text
X:\tianhks\output\acceptance\field-delivery-local-draft\
```

该目录是本地草稿包，不是最终现场签字包。最终现场包仍应按 `field-delivery-<YYYYMMDD>` 复制、补证、签字并归档。

各目录应放入：

| 目录 | 内容 |
| --- | --- |
| `00-summary` | 验收总结、版本、提交、环境、风险接受说明 |
| `01-training` | 最终 PPTX、MP4、签到表、培训问题闭环 |
| `02-uat` | UAT 脚本执行结果、页面截图、导出文件、控制台记录 |
| `03-hardware-stm32` | STM32 固件、寄存器手册、RTU 读写日志、故障注入报告 |
| `04-ai-lora-rk` | Qwen/GGUF/LoRA、训练脚本、manifest、RK 延迟报告 |
| `05-third-party` | AINAS、MQTT broker、Modbus Poll/Slave、Postman 或第三方系统截图 |
| `06-security-performance` | 证书链、密钥轮换、安全扫描、7x24、资源采样、RS485 丢包率 |
| `07-signatures` | 用户代表、项目负责人、上位机、硬件、算法、平台负责人签字 |

## 6. 现场不能混淆的口径

| 不能说 | 应该说 |
| --- | --- |
| 上位机已经完整满足 PRD | 上位机本地软件主体已达到联调准备版，最终 PRD 通过仍依赖外部证据 |
| LoRA 自进化已经完成 | 上位机已具备数据集导出、训练编排、manifest 和候选 adapter 晋级边界，真实模型资产和 RK 验收待补 |
| 多浏览器/多端已全量通过 | Chromium、Chrome、Edge、Firefox、WebKit 本地自动化严格矩阵已通过；macOS Safari、iOS/Android 真机和客户签字仍待补 |
| 培训已完成 | 培训课件源稿、PPTX 草稿和静音 MP4 草稿已完成；真实培训、真实操作录屏、签到和问题闭环待执行 |
| 用户验收已通过 | 用户验收脚本已完成；正式通过需要按脚本执行并签字 |

## 7. 最小现场闭环

如果现场时间有限，至少完成这些 P0 项：

| 顺序 | 项目 | 产出 |
| --- | --- | --- |
| 1 | HMI 七大页面 + 中英切换 | 中英截图、控制台无阻塞错误记录 |
| 2 | RBAC + 控制安全拒绝 | operator/engineer/admin 结果、拒绝截图、审计 event id |
| 3 | 实时监控 + 报警 | 正常样本、越限样本、报警截图 |
| 4 | History + 导出 | 批次查询、产品结果录入、CSV/XLSX/报告导出 |
| 5 | 审计链 | 审计 CSV、hash chain 状态 |
| 6 | Modbus 读写边界 | 寄存器读写日志、危险写入拒绝、STM32 或外部工具截图 |
| 7 | 培训和用户签字 | 签到、问题闭环、UAT 总结和签字 |

## 8. 交付后更新要求

现场执行完成后，必须回填这些文档：

| 文档 | 回填内容 |
| --- | --- |
| `X:\tianhks\docs\upper_computer_current_gap_summary_for_lizuyi.md` | 把已补现场证据从“待补”改为“已验收”，保留未完成项 |
| `X:\tianhks\docs\upper_computer_requirement_gap_matrix.md` | 更新 PRD 对照状态和证据路径 |
| `X:\tianhks\docs\upper_computer_delivery_readiness_index.md` | 更新交付物状态、最终 PPTX/MP4/签字路径 |
| `X:\tianhks\docs\upper_computer_test_report.md` | 并入外部验收、性能、安全和用户验收结果 |
| `X:\tianhks\docs\upper_computer_visual_evidence_index.md` | 并入真实浏览器、移动端和现场截图 |
