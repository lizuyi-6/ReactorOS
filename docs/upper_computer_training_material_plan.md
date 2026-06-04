# 星宿智能反应釜上位机培训材料计划

日期：2026-06-04

对象：李祖祎负责的 RK/PC 上位机培训交付物。本文档用于补齐 PRD 第十章中“系统操作培训 PPT”和“视频教程”的交付规划。当前只形成计划和脚本，不等同于已经产出最终 PPT 或视频。

## 1. 交付物范围

| 交付物 | 当前状态 | 目标格式 | 完成判据 |
| --- | --- | --- | --- |
| 系统操作培训 PPT | 待制作 | 15 到 20 页 PPTX | 覆盖登录、监控、控制、AI、历史、审计、Modbus、系统配置、异常处理和验收注意事项 |
| 视频教程 | 待录制 | 8 到 12 分钟 MP4 | 基于本地或 RK 实机环境完整演示核心操作，字幕或旁白与最终 UI 一致 |
| 培训签到与问题记录 | 待执行 | Markdown / Excel | 记录培训时间、参训人员、问题、责任人和闭环状态 |
| 用户验收操作脚本 | 待最终确认 | Markdown | 客户或项目负责人可按脚本逐项验收并签字 |

## 2. 培训 PPT 建议结构

| 页码 | 标题 | 重点内容 | 依赖材料 |
| --- | --- | --- | --- |
| 1 | 上位机系统概览 | 系统定位、适用环境、与 STM32/AI/第三方平台的关系 | `docs/upper_computer_development_doc.md` |
| 2 | 登录与权限 | operator / engineer / admin 权限差异、Bearer session、安全注意事项 | `docs/upper_computer_user_manual.md` |
| 3 | 实时监控 | 温度、压力、转速、流量、pH、系统健康、传感器超时 | HMI 实机截图 |
| 4 | 手动控制 | 目标值设置、步长限制、人工锁、急停、恢复边界 | `config/safety.toml` |
| 5 | AI 建议与 SOP 草案 | AI 参数建议、本地 LoRA 状态边界、SOP 只读草案 | `docs/local_ai_adapter_status_addendum.md` |
| 6 | 工艺探索与批次 | 批次生命周期、样本流、实验记录、报告生成 | `docs/upper_computer_user_manual.md` |
| 7 | 历史数据与导出 | 历史查询、CSV/XLSX、Markdown 报告、审计 CSV | `docs/upper_computer_cli_reference.md` |
| 8 | 审计日志 | 哈希链、角色操作记录、异常追溯 | `docs/upper_computer_maintenance_manual.md` |
| 9 | Modbus 调试 | 寄存器映射、读写入口、安全禁区拦截、外部工具验收 | `docs/upper_computer_modbus_register_map.md` |
| 10 | AINAS / MQTT / REST | 任务下发、回执、报警快照、第三方验收证据 | `docs/upper_computer_api_acceptance_manual.md` |
| 11 | 系统配置 | 设备配置、安全配置、AI memory、integration、证书路径 | `config/*.toml` |
| 12 | 安全与异常处理 | 急停、传感器掉线、RS485 异常、证书异常、密钥轮换边界 | `docs/upper_computer_security_key_lifecycle.md` |
| 13 | 部署与维护 | RK/PC 启动命令、systemd、备份、升级/回滚 | `docs/upper_computer_rk_deployment_acceptance_guide.md` |
| 14 | 验收清单 | STM32、LoRA、外部平台、性能、安全、用户签字 | `docs/upper_computer_external_acceptance_checklist.md` |
| 15 | 常见问题 | 登录失败、无实时数据、控制被拒绝、AI 不可用、导出失败 | `docs/upper_computer_maintenance_manual.md` |

## 3. 视频教程脚本

| 片段 | 时长 | 操作内容 | 验收画面 |
| --- | --- | --- | --- |
| 1 | 30 秒 | 打开 `http://127.0.0.1:8000/` 或 RK 部署地址，检查健康状态 | HMI 首页正常加载 |
| 2 | 60 秒 | 登录不同角色，展示权限差异 | 受限操作被禁用或拒绝 |
| 3 | 90 秒 | 启动样本流，查看实时监控、系统健康和趋势曲线 | 实时数据刷新，状态为 normal |
| 4 | 90 秒 | 设置安全范围内控制目标，再尝试禁区组合 | 合法目标通过，禁区目标被拒绝并记录审计 |
| 5 | 90 秒 | 查看 AI 建议、SOP 草案和本地 LoRA 缺口状态 | SOP 草案可读，local_ai 显示未就绪边界 |
| 6 | 90 秒 | 查询历史批次，导出 CSV/XLSX/Markdown 报告 | 导出文件生成 |
| 7 | 60 秒 | 查看审计日志和异常记录 | 审计链和事件可追溯 |
| 8 | 60 秒 | 打开 Modbus 调试页，查看寄存器 map 和读写边界 | 寄存器表清晰，写入走安全校验 |
| 9 | 60 秒 | 切换中文/英文，确认主要字块同步变化 | 页面文字切换完整 |
| 10 | 60 秒 | 展示维护入口、备份、配置和下一步外部验收项 | 观众理解哪些依赖硬件/外部平台 |

## 4. 录制前检查

| 检查项 | 目标 |
| --- | --- |
| 本地服务 | `GET /health` 返回 `{"ok": true, "service": "reactor-edge-daemon"}` |
| 样本流 | `/api/live` 在录制期间返回 200，HMI 显示实时温度/压力 |
| 中英切换 | 当前版本所有主要文字块均可切换，无明显遗漏 |
| 安全拦截 | 禁区目标和超步长目标能稳定被拒绝 |
| 导出目录 | CSV/XLSX/Markdown 导出路径可写 |
| 录屏分辨率 | 建议 1920x1080，浏览器缩放 100% |
| 敏感信息 | StepFun key、证书私钥、数据库加密 key、真实账号密码不得出现在画面中 |

## 5. 培训执行记录模板

| 字段 | 填写说明 |
| --- | --- |
| 培训时间 | 具体日期和起止时间 |
| 培训环境 | 本地 PC、RK3568、RK3588 或现场网络 |
| 培训讲师 | 默认李祖祎或上位机负责人 |
| 参训人员 | 姓名、角色、单位 |
| 覆盖模块 | 监控、控制、AI、历史、审计、Modbus、配置、异常处理 |
| 现场问题 | 问题描述、提出人、严重程度 |
| 责任人 | 负责修复或解释的人 |
| 闭环状态 | 未处理、处理中、已解决、延期 |
| 签字确认 | 项目负责人或客户代表签字 |

## 6. 与现有文档的关系

培训材料应以这些文档作为内容来源：

- 用户操作：`docs/upper_computer_user_manual.md`
- 部署维护：`docs/upper_computer_maintenance_manual.md`、`docs/upper_computer_rk_deployment_acceptance_guide.md`
- API/CLI：`docs/upper_computer_api_acceptance_manual.md`、`docs/upper_computer_cli_reference.md`
- Modbus：`docs/upper_computer_modbus_register_map.md`
- 外部验收：`docs/upper_computer_external_acceptance_checklist.md`
- 当前缺口：`docs/upper_computer_current_gap_summary_for_lizuyi.md`

