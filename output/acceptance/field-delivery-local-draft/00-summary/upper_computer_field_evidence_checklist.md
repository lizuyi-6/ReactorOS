# 星宿智能反应釜上位机现场证据签收清单

日期：2026-06-07

对象：李祖祎负责的 RK/PC 上位机最终现场验收证据归档。

本文档把 `X:\tianhks\docs\upper_computer_external_acceptance_checklist.md`、`X:\tianhks\docs\upper_computer_user_acceptance_script.md` 和当前本地交付包整理成现场签收清单。机器可读版本见 `X:\tianhks\docs\upper_computer_field_evidence_checklist.json`。

状态口径：

| 状态 | 含义 |
| --- | --- |
| `local_ready` | 本地草稿或自动化证据已具备，可放入交付包，但仍可能需要现场复核 |
| `external_required` | 必须由硬件、算法、外部平台、RK 或生产环境补真实证据 |
| `signature_required` | 必须由客户代表、项目负责人或责任人签字确认 |
| `draft_only` | 只有草稿或示意材料，不能作为最终验收通过证据 |

## 1. 本地已具备但现场需确认

| 编号 | 证据项 | 当前证据 | 现场动作 | 状态 |
| --- | --- | --- | --- | --- |
| L-01 | 本地草稿交付包 | `X:\tianhks\output\acceptance\field-delivery-local-draft\00-summary\upper_computer_delivery_manifest.json` | 复制为现场日期目录，补版本、环境和签字 | `local_ready` |
| L-02 | 培训 PPTX 草稿 | `X:\tianhks\docs\upper_computer_training_deck.pptx` | 用真实部署截图更新现场最终版 | `draft_only` |
| L-03 | 静音 MP4 课件轮播草稿 | `X:\tianhks\outputs\manual-20260607-training\video\upper_computer_training_video_draft.mp4` | 按 storyboard 录制真实操作视频并补旁白或字幕 | `draft_only` |
| L-04 | UAT 操作脚本 | `X:\tianhks\docs\upper_computer_user_acceptance_script.md` | 逐项执行、填写结果、附证据、签字 | `signature_required` |
| L-05 | 培训签到问题模板 | `X:\tianhks\docs\upper_computer_training_attendance_and_issues.md` | 填真实参训人员、问题闭环和签字 | `signature_required` |
| L-06 | 培训交付物 gate | `X:\tianhks\output\acceptance\training-deliverables-report.json` | 现场最终材料更新后重跑并归档 | `local_ready` |

## 2. 必须外部补证

| 编号 | 证据项 | 必须补齐 | 责任方 | 状态 |
| --- | --- | --- | --- | --- |
| E-01 | STM32 Modbus RTU 实机联调 | 固件版本、最终寄存器表、RS485 接线、真实读写日志、异常写入拒绝记录 | 硬件 + 上位机 | `external_required` |
| E-02 | 硬件闭环控制 | 加热、搅拌、压力、摇罐、启停、急停、人工锁、暂停/恢复真实动作记录 | 硬件 + 上位机 | `external_required` |
| E-03 | Modbus TCP 外部工具 | Modbus Poll/Slave 读写截图、TLS 证书链、并发连接和危险写入拒绝 | 上位机 + 外部工具 | `external_required` |
| E-04 | MQTT 外部 broker | MQTT.fx 或 mosquitto 任务、receipt、retained alert、断线重连和证书链记录 | 第三方平台 + 上位机 | `external_required` |
| E-05 | AINAS 真实平台 | 真实任务下发、查询、执行回执、平台截图或接口日志 | AINAS/平台 + 上位机 | `external_required` |
| E-06 | Qwen3.5-2B/GGUF/LoRA/RK | 模型文件、adapter、推理入口、训练脚本、manifest、RK 延迟和评估报告 | 算法 + RK | `external_required` |
| E-07 | 生产 TLS/密钥 | 正式证书链、密钥托管、轮换、丢失恢复和敏感信息遮挡证据 | 安全/运维 | `external_required` |
| E-08 | 长时间可靠性 | 72 小时或 30 天运行、RS485 丢包率、断电恢复、资源采样和 MTBF 论证 | 硬件 + 运维 + 上位机 | `external_required` |
| E-09 | 多浏览器/移动真机 | macOS Safari、iOS/Android 真机截图或录屏；Firefox 已有本机 Playwright 严格矩阵通过证据 | 测试/验收 | `external_required` |

## 3. 必须签字确认

| 编号 | 证据项 | 签字人 | 条件 |
| --- | --- | --- | --- |
| S-01 | 培训完成 | 培训讲师、项目负责人或客户代表 | PPT/视频讲解完成，问题闭环表无 P0 未关闭 |
| S-02 | 用户验收通过 | 客户代表或项目负责人 | UAT-01 到 UAT-16 执行完毕，失败项有关闭或风险接受 |
| S-03 | 外部依赖风险接受 | 硬件、算法、平台、项目负责人 | 未完成外部项必须明确责任人、计划日期和风险接受范围 |
| S-04 | 最终交付包归档 | 上位机负责人、项目负责人 | 现场日期目录包含软件、配置、文档、测试、培训、签字和遗留问题 |

## 4. 现场执行顺序

1. 先运行 `node X:\tianhks\scripts\package-upper-computer-delivery.mjs`，把本地草稿包复制到现场日期目录。
2. 按 `X:\tianhks\docs\upper_computer_user_acceptance_script.md` 执行 UAT。
3. 按本清单补 E-01 到 E-09 外部证据。
4. 用真实截图更新培训 PPTX，按 `X:\tianhks\docs\upper_computer_training_video_storyboard.md` 录制真实操作视频。
5. 重跑 `node X:\tianhks\scripts\verify-training-deliverables.mjs`，把最新报告放入现场目录。
6. 完成 S-01 到 S-04 签字，回填缺口矩阵和交付就绪索引。
