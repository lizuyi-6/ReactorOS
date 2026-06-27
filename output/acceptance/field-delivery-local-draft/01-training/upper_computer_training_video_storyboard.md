# 星宿智能反应釜上位机培训视频 Storyboard

日期：2026-06-07

对象：李祖祎负责的 RK/PC 上位机培训视频交付物。

本文档用于把 `X:\tianhks\docs\upper_computer_training_deck.pptx` 转成视频教程草稿，并指导后续录制真实现场操作 MP4。当前可自动生成的 `X:\tianhks\outputs\manual-20260607-training\video\upper_computer_training_video_draft.mp4` 是静音课件轮播草稿，不是真实现场录屏、不是带讲解的最终培训视频，也不能作为用户签字或 PRD 验收证据。

## 1. 自动生成草稿

```powershell
node X:\tianhks\scripts\generate-upper-computer-training-video.mjs
```

默认参数：

| 项目 | 值 |
| --- | --- |
| 输入预览图 | `X:\tianhks\outputs\manual-20260607-training\presentations\xingshu-upper-computer-training\preview\slide-01.png` 到 `slide-16.png` |
| 输出视频 | `X:\tianhks\outputs\manual-20260607-training\video\upper_computer_training_video_draft.mp4` |
| 输出 manifest | `X:\tianhks\outputs\manual-20260607-training\video\upper_computer_training_video_manifest.json` |
| 默认时长 | 每页 30 秒，总计 480 秒 |
| 声音 | 静音 AAC 音轨，仅用于保证播放器兼容 |
| 可调参数 | `XINGSHU_TRAINING_VIDEO_SECONDS_PER_SLIDE` |

示例：每页 20 秒生成 320 秒草稿。

```powershell
$env:XINGSHU_TRAINING_VIDEO_SECONDS_PER_SLIDE='20'
node X:\tianhks\scripts\generate-upper-computer-training-video.mjs
Remove-Item Env:\XINGSHU_TRAINING_VIDEO_SECONDS_PER_SLIDE
```

## 2. 页级讲解脚本

| 时间段 | 页码 | 标题 | 讲解重点 | 现场录屏补充 |
| --- | --- | --- | --- | --- |
| 00:00-00:30 | 1 | 上位机系统定位 | 上位机负责 Web HMI、REST API、CLI、SQLite、审计、安全门控、第三方接口和 AI 推荐入口；不要宣称 PRD 全量完成 | 打开 HMI 首页和交付就绪索引 |
| 00:30-01:00 | 2 | 系统架构和数据流 | 说明采集链路、控制链路、AI 链路、审计链路和第三方链路都要进入受控路径 | 展示 `/health`、监控页和审计页 |
| 01:00-01:30 | 3 | 登录、角色和权限 | operator、engineer、admin 的边界；高权限操作需要审计 reason | 演示三角色登录和受限操作 |
| 01:30-02:00 | 4 | 实时监控页面 | 温度、压力、转速、流量、pH、系统健康和传感器新鲜度 | 启动样本流或现场 STM32 数据 |
| 02:00-02:30 | 5 | 手动控制和安全门控 | 控制不是直接写设备，必须过权限、reason、步长、禁区、急停和人工锁 | 演示合法目标通过、禁区目标被拒绝 |
| 02:30-03:00 | 6 | AI 建议、AI 主控和 SOP 草案 | AI 是建议和受控执行入口；真实 Qwen/GGUF/LoRA/RK 延迟仍需外部验收 | 展示 AI 页和 local_ai readiness |
| 03:00-03:30 | 7 | 工艺探索与批次生命周期 | 批次准备、运行、采样、结束、报告和异常恢复流程 | 展示批次列表和单批次详情 |
| 03:30-04:00 | 8 | 历史数据、筛选和导出 | 历史筛选、CSV/XLSX/Markdown 导出和导出证据 | 演示一次导出并打开文件 |
| 04:00-04:30 | 9 | 审计日志和追溯 | control_events hash chain、审计窗口和异常追溯 | 展示审计链状态和最近事件 |
| 04:30-05:00 | 10 | Modbus 调试 | 寄存器映射、读写边界、admin-only 调试写入和外部工具验收缺口 | 展示 Modbus 页面和寄存器表 |
| 05:00-05:30 | 11 | AINAS、MQTT 和 REST 对接 | 第三方任务下发、回执、报警快照、AES 静态加密和外部验收证据 | 展示设置页集成状态和 API 手册 |
| 05:30-06:00 | 12 | 系统配置和安全配置 | `device.toml`、`safety.toml`、`integration.toml`、`ai_memory.toml` 和证书路径 | 展示配置摘要，遮挡密钥 |
| 06:00-06:30 | 13 | 异常处理和应急流程 | 页面打不开、实时数据为空、控制失败、AI 不可用、第三方断连等处理路径 | 模拟一个被拒绝控制或断连状态 |
| 06:30-07:00 | 14 | 部署、备份和维护 | RK/PC 部署、systemd、自动备份、恢复演练、升级回滚 | 展示维护手册和备份报告 |
| 07:00-07:30 | 15 | 用户验收范围 | 七大页面、控制安全、历史导出、审计、AI、第三方接口的验收边界 | 打开 UAT 脚本并说明签字字段 |
| 07:30-08:00 | 16 | 常见问题 | 讲清哪些问题归上位机、硬件、算法、第三方平台或运维 | 展示问题闭环模板 |

## 3. 最终现场版要求

| 项目 | 最终要求 |
| --- | --- |
| 画面 | 必须包含真实部署地址或本地验收地址、HMI 页面、导出文件、审计记录和外部工具截图 |
| 声音 | 必须有讲解旁白，或提供同步字幕 |
| 敏感信息 | 不得出现 StepFun key、证书私钥、数据库加密 key、真实账号密码 |
| 证据 | 视频旁边应归档 `X:\tianhks\docs\upper_computer_user_acceptance_script.md` 执行记录和签字页 |
| 边界 | AI 生成图片、静音草稿和课件轮播只能作为培训素材，不能作为真实硬件、真实 HMI 操作或用户签字证据 |
