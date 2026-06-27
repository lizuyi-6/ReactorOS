# 星宿智能反应釜上位机交付就绪索引

日期：2026-06-07

对象：李祖祎负责的 RK/PC 上位机交付物。

对照来源：

- PRD 第十章交付物清单。
- 团队分工文档中李祖祎负责的上位机页面、工艺探索、AI 参数对接、数据曲线、日志、导出、调试、自测和测试报告。

## 1. 当前结论

上位机本地软件、接口文档、测试报告、缺口矩阵和外部验收清单已具备“联调准备版”交付能力。正式最终交付仍依赖硬件、算法、外部平台、工业环境和用户验收补齐证据。

不能把本文档理解为 PRD 全量交付完成证明，原因包括：

- STM32 Modbus RTU 固件和真实整机联调不属于当前本地证据范围。
- Qwen3.5-2B + LoRA 真实推理、自进化、生产训练脚本和 RK 延迟未完成；上位机侧训练数据集导出、训练入口编排、manifest 归档和显式候选 adapter 晋级/备份已具备。
- 当前 HMI、数据库和 Modbus 实现与 PRD 指定技术栈存在偏离，统一说明见 `docs/architecture-deviations.md`。
- MQTT.fx、Modbus Poll/Slave、AINAS 真实平台、Postman/第三方系统验收未完成。
- 30 天工业环境、安全扫描、用户签字、现场最终版 PPTX 和真实操作录屏 MP4 培训视频仍待补；培训课件源稿、PPTX 草稿、静音 MP4 草稿、验收脚本和签到问题模板见 `docs/upper_computer_training_deck.md`、`docs/upper_computer_training_deck.pptx`、`outputs/manual-20260607-training/video/upper_computer_training_video_draft.mp4`、`docs/upper_computer_user_acceptance_script.md`、`docs/upper_computer_training_attendance_and_issues.md`。
- 培训交付物完整性已增加自动校验门禁：`scripts/verify-training-deliverables.mjs`，报告输出 `output/acceptance/training-deliverables-report.json`，并纳入 `scripts/acceptance/accept-all.ps1` / `scripts/acceptance/accept-all.sh`。本地草稿交付包由 `scripts/package-upper-computer-delivery.mjs` 生成到 `output/acceptance/field-delivery-local-draft/`。

## 2. 软件交付物对照

| PRD 交付物 | 上位机当前证据 | 当前状态 | 还缺什么 |
| --- | --- | --- | --- |
| 星宿边缘中枢 Rust 守护程序 ARM64/AMD64 | `src/main.rs`、`src/api.rs`、`src/modbus_registers.rs`、`docs/lubancat2_debian10_deploy.md`、`docs/upper_computer_rk_deployment_acceptance_guide.md` | 部分完成 | 需最终 ARM64/RK release 包、SHA256、RK 实机运行记录 |
| Web 控制台静态资源 | `frontend/dist/index.html`、`static/index.html` legacy fallback、`scripts/verify-vue-release-assets.mjs`、`scripts/verify-vue-mobile.mjs`、`scripts/verify-vue-browser-matrix.mjs`、视觉证据 `output/playwright/vue-*.png`、`output/playwright/vue-mobile-verification.json`、`output/playwright/vue-browser-matrix-verification.json` 和 `output/visual-i18n/*.png`；Chromium/Chrome/Edge/Firefox/WebKit 本地严格矩阵已通过 | 本地完成 | 需 release/RK、macOS Safari、iOS/Android 真机和用户最终验收截图 |
| 配置文件模板 | `config/device.toml`、`config/safety.toml`、`config/integration.toml`、`config/ai_memory.toml` | 本地完成 | 需生产脱敏配置、真实证书、STM32 串口和寄存器最终值 |
| 安装脚本与升级脚本 | `docs/lubancat2_debian10_deploy.md`、`deploy/install-board.sh`、`deploy/reactor-edge-backup.service`、`deploy/reactor-edge-backup.timer`、`deploy/reactor-edge-backup.sh` | 部分完成 | 安装和每日备份 timer 路径已补；仍需在全新 RK/PC 设备执行部署计时、升级/回滚和恢复演练 |
| STM32 Modbus RTU 从站控制器固件 | 非上位机范围 | 不计入李祖祎本地完成项 | 需硬件负责人提供固件、寄存器手册和联调记录 |

## 3. 文档交付物对照

| PRD 文档交付物 | 当前文件 | 当前状态 | 还缺什么 |
| --- | --- | --- | --- |
| 技术设计文档 | `docs/upper_computer_development_doc.md` | 本地完成 | 最终版需并入真实硬件、外部接口、RK 和 LoRA 验收结果 |
| PRD 偏离说明 | `docs/architecture-deviations.md` | 本地完成 | 随 Vue/SQLx/tokio-modbus/LoRA/生产安全等后续排期结果更新 |
| 用户手册 | `docs/upper_computer_user_manual.md` | 本地完成 | 需最终部署、角色账号、现场 SOP 和客户操作流程确认 |
| 部署手册 | `docs/lubancat2_debian10_deploy.md`、`docs/upper_computer_rk_deployment_acceptance_guide.md` | 部分完成 | 需全新 RK/PC 部署计时、生产证书、systemd 和资源采样证据 |
| 维护手册 | `docs/upper_computer_maintenance_manual.md`、`docs/upper_computer_security_key_lifecycle.md`、`docs/upper_computer_rk_deployment_acceptance_guide.md` | 本地完成 / 生产待验收 | 手动 `xingshu ops backup`、release backup timer、脚本和本地 daemon 重启恢复演练已补；仍需现场/RK 恢复演练、异地归档/保留策略、密钥托管、证书链、watchdog/权限隔离、安全扫描和升级回滚演练证据 |
| API 文档 | `docs/upper_computer_api_acceptance_manual.md` | 本地完成 | 需 Postman/第三方系统调用证据 |
| CLI 命令参考手册 | `docs/upper_computer_cli_reference.md`、`xingshu --help` | 本地完成 | 需随最终发布版本更新命令输出和现场验收记录 |
| Modbus 寄存器映射手册 | `docs/upper_computer_modbus_register_map.md` | 本地完成 / 待硬件确认 | 需 STM32 最终手册逐项确认地址、单位、缩放系数 |
| 现场交付执行包说明 | `docs/upper_computer_field_delivery_execution_pack.md` | 本地完成 | 需按现场日期复制执行包、填充证据和签字 |
| 现场证据签收清单 | `docs/upper_computer_field_evidence_checklist.md`、`docs/upper_computer_field_evidence_checklist.json` | 本地完成 | 需现场按 `local_ready`、`external_required`、`signature_required` 和 `draft_only` 状态逐项补证 |
| PRD | 用户提供的 `PRD v2.2.md` | 外部输入 | 不由本仓库生成 |

## 4. 测试交付物对照

| PRD 测试交付物 | 当前文件 | 当前状态 | 还缺什么 |
| --- | --- | --- | --- |
| 测试计划 | `docs/upper_computer_test_plan_traceability.md`、`docs/upper_computer_external_acceptance_checklist.md` | 本地完成 | 需验收方确认最终计划 |
| 测试用例 | `tests/*.rs`、`docs/upper_computer_external_acceptance_checklist.md` | 部分完成 | 需外部工具、硬件、RK、工业环境和用户验收用例执行记录 |
| 测试报告 | `docs/upper_computer_test_report.md`、`docs/third_party_interface_acceptance_report.md` | 本地完成 / 外部待补 | 需并入 STM32、MQTT.fx、Modbus Poll/Slave、AINAS、RK、真实 LoRA 模型/训练/推理、安全扫描、30 天和用户签字证据 |

## 5. 培训交付物对照

| PRD 培训交付物 | 当前证据 | 当前状态 | 还缺什么 |
| --- | --- | --- | --- |
| 系统操作培训 PPT | `docs/upper_computer_training_deck.md` 已给出 16 页课件源稿；`docs/upper_computer_training_deck.pptx` 已生成 16 页可编辑 PPTX 草稿并嵌入 AI 生成视觉资产；`docs/assets/upper-computer-training/README.md` 说明图片资产边界；`scripts/generate-upper-computer-training-deck.mjs` 可复生成 | 源稿和 PPTX 草稿完成 / 现场最终版待更新 | 需随最终部署截图和真实验收口径更新 PPTX；AI 生成图片不得作为真实验收照片 |
| 视频教程 | `docs/upper_computer_training_material_plan.md`、`docs/upper_computer_training_deck.md` 和 `docs/upper_computer_training_video_storyboard.md` 已给出录制脚本、讲解重点和证据路径；`outputs/manual-20260607-training/video/upper_computer_training_video_draft.mp4` 已生成静音课件轮播草稿 | 草稿完成 / 真实操作录屏未录制 | 需录制登录、监控、控制、AI、历史、审计、Modbus、系统配置和异常处理流程，并补旁白或字幕 |
| 培训签到与问题记录 | `docs/upper_computer_training_attendance_and_issues.md` 已给出签到、覆盖项、问题闭环和签字模板 | 模板完成 / 培训待执行 | 需真实培训签到、问题闭环、复测记录和签字 |
| 用户验收操作脚本 | `docs/upper_computer_user_acceptance_script.md` 已给出 16 项 UAT 脚本、证据字段、问题闭环和签字栏 | 脚本完成 / 待执行签字 | 需验收方逐项执行并签署通过、条件通过或不通过结论 |
| 培训交付物自动校验 | `scripts/verify-training-deliverables.mjs` 已校验课件源稿、16 页 PPTX、媒体资产、UAT 脚本、签到模板、现场交付执行包、video storyboard、静音 MP4 草稿、manifest 和 16 张预览图；报告 `output/acceptance/training-deliverables-report.json` | 本地完成 / 已纳入一键验收 | 仍需现场最终版 PPTX、真实操作录屏 MP4、真实培训签到和用户签字 |
| 本地交付包草稿 | `scripts/package-upper-computer-delivery.mjs` 已生成 `output/acceptance/field-delivery-local-draft/` 和 `00-summary/upper_computer_delivery_manifest.json`，集中归档当前文档、培训素材、UAT 脚本和 gate 报告 | 本地完成 / 草稿包 | 现场仍需按日期复制、补真实证据和签字 |

## 6. 交付证据目录建议

最终交付建议按以下结构归档：

```text
delivery/
  software/
  config/
  docs/
  tests/
  acceptance/
  training/
  signatures/
```

当前本地证据可先映射为：

| 目录 | 当前可放入内容 |
| --- | --- |
| `software/` | 编译产物、安装包、SHA256、启动命令 |
| `config/` | `config/*.toml` 脱敏副本、证书链说明 |
| `docs/` | `docs/upper_computer_*.md`、`docs/third_party_interface_acceptance_report.md`，含 `docs/upper_computer_current_gap_summary_for_lizuyi.md`、`docs/architecture-deviations.md` 和 `docs/upper_computer_field_delivery_execution_pack.md` |
| `tests/` | Cargo 测试输出、`output/upper-computer-perf-smoke.json`、`output/upper-computer-resource-snapshot.json`、`output/local-run/upper-computer-local-gate-20260607.json`、`output/upper-computer-local-gate-20260606.json`、`output/acceptance/restore-drill/restore-drill-report.json` |
| `acceptance/` | HMI 截图、外部接口工具截图、RK 运行日志、用户签字 |
| `training/` | `docs/upper_computer_training_material_plan.md`、`docs/upper_computer_training_deck.md`、`docs/upper_computer_training_deck.pptx`、`docs/upper_computer_training_attendance_and_issues.md`、最终现场版 PPTX、视频、培训签到和问题记录 |
| `signatures/` | `docs/upper_computer_user_acceptance_script.md` 执行后的用户验收签字页、条件通过说明和风险接受记录 |

当前本地草稿包已按上述思路生成到 `output/acceptance/field-delivery-local-draft/`，其中 manifest 为 `output/acceptance/field-delivery-local-draft/00-summary/upper_computer_delivery_manifest.json`。

## 7. 下一步交付优先级

1. 用 RK 实机生成部署验收包和资源报告。
2. 用 STM32、MQTT.fx/mosquitto、Modbus Poll/Slave、AINAS 真实平台补外部验收。
3. 接入真实 Qwen3.5-2B + LoRA/GGUF 和生产训练脚本后，用现有 `xingshu ai train` 导出、manifest、显式晋级/备份链路更新 AI/RK 验收。
4. 用 `docs/upper_computer_training_deck.pptx` 作为培训 PPTX 草稿，按最终部署截图更新现场版，按 `docs/upper_computer_training_material_plan.md` 录制视频，并用 `docs/upper_computer_user_acceptance_script.md` 完成用户签字验收。
5. 随最终发布版本更新 CLI 参考、维护手册和用户手册。
