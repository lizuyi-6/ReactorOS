# 星宿智能反应釜上位机视觉验证证据索引

日期：2026-06-04

对象：李祖祎负责的 RK/PC 上位机 Web HMI。

本索引用于归档当前本地浏览器视觉验证截图和文字审计结果。截图均来自本地服务 `http://127.0.0.1:8000/`。

## 1. 中英切换核心截图

| 截图 | 验证点 | 结论 |
| --- | --- | --- |
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

- 首选 Windows Computer Use 插件连接时返回 `Computer Use native pipe path is unavailable`，未继续用桌面 UI 自动化绕过。
- 最终视觉与文字审计由 Playwright 在本地页面完成。
- 当前截图证明本地 HMI 页面和动态字块可正确切换，不等价于多浏览器、移动端真机或生产用户验收签字。

## 5. 非截图验收证据

| 文件 | 验证点 | 结论 |
| --- | --- | --- |
| `output/upper-computer-perf-smoke.json` | 本机只读 API 往返和安全计算性能冒烟 | 通过；API p95 最高 4ms，`safety_compute` p95=1ms |
| `output/upper-computer-sample-ingest.json` | CLI 通过正式 v1 样本入口注入无硬件演示样本 | 通过；`duration_s=3`，`samples_pushed=6` |
| `output/upper-computer-resource-snapshot.json` | Windows debug 本地演示进程资源快照 | 通过；working set 26.977MB，CPU max 1.533% |
| `docs/upper_computer_security_key_lifecycle.md` | 密钥生命周期、证书/token 和敏感字段清单 | 已文档化；生产轮换演练和安全扫描待验收 |

性能冒烟不证明 STM32/RS485 采集延迟、真实执行器控制延迟、Qwen/LoRA 推理训练延迟、7x24、MTBF 或外部 MQTT/Modbus 工具性能。
