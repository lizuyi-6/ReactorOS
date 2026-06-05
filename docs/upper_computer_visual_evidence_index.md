# 星宿智能反应釜上位机视觉验证证据索引

日期：2026-06-04；最近一次视觉复核：2026-06-05

对象：李祖祎负责的 RK/PC 上位机 Web HMI。

本索引用于归档当前本地浏览器视觉验证截图和文字审计结果。截图均来自本地服务 `http://127.0.0.1:8000/`。

当前 HMI 以 9 个 tab 承载 PRD 七大页面，验收归档时按 `docs/architecture-deviations.md` 的页面映射合并截图：`monitor/alarms` 对应实时监控，`program/settings` 对应参数配置，`ai/monitor` 对应 AI 智能决策，`recipes/materials` 对应历史数据，其余 `audit/modbus/settings` 分别对应审计日志、Modbus 调试和系统配置。

## 1. 中英切换核心截图

| 截图 | 验证点 | 结论 |
| --- | --- | --- |
| `output/visual-i18n/upper-computer-i18n-audit-20260605.json` | 9 个 tab × 中英双语自动化文字审计报告 | 通过；英文可见中文残留 0，乱码字块 0，空页面文本 0 |
| `output/visual-i18n/upper-computer-i18n-monitor-zh-20260605.png` | 监控页中文状态 | 通过 |
| `output/visual-i18n/upper-computer-i18n-monitor-en-20260605.png` | 监控页英文状态 | 通过 |
| `output/visual-i18n/upper-computer-i18n-program-zh-20260605.png` | 控制/工艺页中文状态 | 通过 |
| `output/visual-i18n/upper-computer-i18n-program-en-20260605.png` | 控制/工艺页英文状态，动态“客户演示工艺”已切换为 Customer Demo Process | 通过 |
| `output/visual-i18n/upper-computer-i18n-audit-zh-20260605.png` | 审计页中文状态，审计链统计与动态 reason 中文化 | 通过 |
| `output/visual-i18n/upper-computer-i18n-audit-en-20260605.png` | 审计页英文状态，审计列表长事件名横向可读 | 通过 |
| `output/visual-i18n/upper-computer-i18n-modbus-zh-20260605.png` | Modbus 页中文状态，admin-only 写入面板显示“管理员安全写入测试 / 仅管理员” | 通过 |
| `output/visual-i18n/upper-computer-i18n-modbus-en-20260605.png` | Modbus 页英文状态，admin-only 写入面板显示 `ADMIN SAFETY WRITE TEST / ADMIN ONLY` | 通过 |
| `output/visual-i18n/upper-computer-i18n-settings-zh-20260605.png` | 设置页中文状态 | 通过 |
| `output/visual-i18n/upper-computer-i18n-settings-en-20260605.png` | 设置页英文状态 | 通过 |
| `output/upper-computer-sop-zh.png` | AI 实验室，实验 SOP 草案中文状态 | 通过 |
| `output/upper-computer-sop-en.png` | AI 实验室，实验 SOP 草案英文状态 | 通过 |
| `output/upper-computer-i18n-monitor-zh-final.png` | 实时监控页中文状态 | 通过 |
| `output/upper-computer-i18n-monitor-en-final.png` | 实时监控页英文状态 | 通过 |
| `output/upper-computer-i18n-modbus-zh-final.png` | Modbus/集成状态中文状态 | 通过 |
| `output/upper-computer-i18n-modbus-en-final.png` | Modbus/集成状态英文状态 | 通过 |
| `output/visual-i18n/01-monitor-zh.png` | 监控页中文回归截图 | 通过 |
| `output/visual-i18n/02-control-zh.png` | 控制页中文回归截图 | 通过 |
| `output/visual-i18n/03-audit-zh.png` | 审计页中文回归截图 | 通过 |
| `output/visual-i18n/04-modbus-zh.png` | Modbus 页中文回归截图 | 通过 |
| `output/visual-i18n/05-settings-zh.png` | 设置页中文回归截图 | 通过 |
| `output/visual-i18n/06-en-return.png` | 切回英文后的回归截图 | 通过 |
| `output/upper-computer-hmi-live-sample-final.png` | 持续 pipeline 样本流下实时监控英文状态 | 通过 |

## 2. 动态字块专项截图

| 截图 | 验证点 | 结论 |
| --- | --- | --- |
| `output/ainas-integration-en-visible-final.png` | AINAS/集成接口英文动态状态 | 通过 |
| `output/ainas-integration-zh-visible-final.png` | AINAS/集成接口中文动态状态 | 通过 |
| `output/modbus-expanded-points-zh-visible.png` | 扩展 Modbus 点位中文动态状态 | 通过 |
| `output/mqtt-integration-status-visible.png` | MQTT 集成状态中文动态状态 | 通过 |
| `output/modbus-tcp-integration-status-visible.png` | Modbus TCP 集成状态中文动态状态 | 通过 |
| `output/upper-computer-aes-modbus-zh-playwright.png` | AES-256 静态加密状态中文动态状态 | 通过 |
| `output/upper-computer-aes-modbus-en-playwright.png` | AES-256 静态加密状态英文动态状态 | 通过 |
| `output/upper-computer-local-ai-zh.png` | 本地 Qwen/LoRA readiness 中文边界状态 | 通过 |
| `output/upper-computer-local-ai-en.png` | 本地 Qwen/LoRA readiness 英文边界状态 | 通过 |
| `output/upper-computer-local-ai-settings-zh.png` | 设置页本地 AI 配置中文边界状态 | 通过 |
| `output/playwright/xingshu-stale-ai-settings-en-20260605.png` | StepFun 配置下本地缓存推荐 stale 提示英文状态；eval 结果 `hasEnglish=true` | 通过 |
| `output/playwright/xingshu-stale-ai-settings-zh-20260605.png` | 同一 stale 提示中文状态；eval 结果 `hasChinese=true`、`hasEnglish=false` | 通过 |

## 3. AI SOP 草案文字审计

中文模式抽取范围：

- `#aiPlanStatus`
- `#aiPlanSummary`
- `#aiPlanSteps`
- `#aiPlanBoundary`

中文审计结论：

- `aiPlanStatus` 显示为 `草案待操作员复核`。
- `aiPlanSummary` 显示为中文安全门控 SOP 摘要。
- 三个步骤显示为 `步骤 1/2/3`、`预检查与升温`、`反应保温与取样`、`降温与结果录入`。
- 边界说明显示本地 Qwen LoRA 尚不可执行、缺失资产和 safety guard/RBAC/审计边界。
- 混文检测表达式未命中：`based 开`、`之后`、`目标 write`、`product-result 批次`、`STEP \d`。

英文模式抽取范围相同。

英文审计结论：

- 标题显示为 `EXPERIMENT SOP DRAFT`。
- 状态、摘要、三个步骤和边界说明均为英文。
- 中文残留检测表达式未命中：`草案`、`步骤`、`预检查`、`反应保温`、`降温`、`该接口`、`真实执行`、`缺失资产`。

## 4. 工具与边界

- 首选 Windows Computer Use 插件连接时返回 `Computer Use native pipe path is unavailable`，当前 Codex 运行时没有暴露 native pipe；没有伪称已使用该插件完成桌面自动化。
- 最终视觉与文字审计由 in-app Browser 截图复核和 Playwright 本地页面巡检完成。
- 2026-06-05 自动化巡检登录 `engineer` 后覆盖 `monitor / recipes / program / ai / materials / alarms / audit / modbus / settings` 九页。报告摘要：`englishPagesWithUnexpectedCjk=[]`、`pagesWithMojibake=[]`、`pagesWithEmptyViewText=[]`、`unexpectedConsoleMessages=[]`；`/api/live` 的 503 均为当前没有新鲜 pipeline 样本的预期业务状态。本轮同时复核 Modbus admin-only 写入面板中英文文案可见且无布局溢出。
- 2026-06-05 追加复核 StepFun 配置下的本地缓存推荐状态：注入 `provider.mode = stale_local_recommendation` 后，Settings 页英文显示 `Refresh cached local recommendation before AI master control`，中文显示 `AI 主控前需刷新缓存的本地推荐`，且中文模式无该英文残留。
- 当前截图证明本地 HMI 页面和动态字块可正确切换，不等价于多浏览器、移动端真机或生产用户验收签字。

## 5. 非截图验收证据

| 文件 | 验证点 | 结论 |
| --- | --- | --- |
| `output/upper-computer-perf-smoke.json` | 本机只读 API 往返和安全计算性能冒烟 | 通过；API p95 最高 4ms，`safety_compute` p95=1ms |
| `output/upper-computer-sample-ingest.json` | CLI 通过正式 v1 样本入口注入无硬件演示样本 | 通过；`duration_s=3`，`samples_pushed=6` |
| `output/upper-computer-resource-snapshot.json` | Windows debug 本地演示进程资源快照 | 通过；working set 26.977MB，CPU max 1.533% |
| `docs/upper_computer_security_key_lifecycle.md` | 密钥生命周期、证书/token 和敏感字段清单 | 已文档化；生产轮换演练和安全扫描待验收 |

性能冒烟不证明 STM32/RS485 采集延迟、真实执行器控制延迟、Qwen/LoRA 推理训练延迟、7x24、MTBF 或外部 MQTT/Modbus 工具性能。
