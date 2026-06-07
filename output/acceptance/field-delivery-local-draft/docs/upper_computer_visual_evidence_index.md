# 星宿智能反应釜上位机视觉验证证据索引

日期：2026-06-04；最近一次视觉复核：2026-06-07

对象：李祖祎负责的 RK/PC 上位机 Web HMI。

本索引用于归档当前本地浏览器视觉验证截图和文字审计结果。截图均来自本地服务 `http://127.0.0.1:8000/`。

当前 Vue 生产 HMI 以 `monitor/control/ai/history/audit/modbus/settings` 七个 hash 路由承载 PRD 七大页面；旧版 `static/index.html` fallback 的 9-tab 截图作为历史/回退证据保留，正式验收以 Vue 七路由证据为主。

## 1. 中英切换核心截图

| 截图 | 验证点 | 结论 |
| --- | --- | --- |
| `output/visual-i18n/upper-computer-i18n-audit-20260605.json` | legacy `static/index.html` 9 个 tab × 中英双语自动化文字审计报告 | 通过；英文可见中文残留 0，乱码字块 0，空页面文本 0 |
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
| `output/playwright/vue-parity-ai-zh.png` | Vue AI 页中文状态，包含 AI 结果复核、决策摘要、动作复核、安全门控和推荐目标字块 | 通过 |
| `output/playwright/vue-parity-ai-en.png` | Vue AI 页英文状态，包含 AI Result Review、Decision Summary、Action Review、Safety Gate 和 Recommended Targets 字块 | 通过 |
| `output/playwright/vue-mobile-phone-ai-zh.png` | Vue AI 页手机视口中文状态，结构化复核入口和暗色描述表可读 | 通过 |
| `output/playwright/vue-parity-monitor-alarm-zh.png` | Vue Monitor 页中文状态，正式 `/api/v1/reactor/:device_id/samples` 越限样本触发 `/api/live` 的 `temperature_limit`/`pressure_limit` 报警并验证 `level/type/message/current_value/limit_value/suggestion` 字段显示 | 通过 |
| `output/playwright/vue-parity-monitor-alarm-en.png` | Vue Monitor 页英文状态，正式 `/api/v1/reactor/:device_id/samples` 越限样本触发 `/api/live` 的 `temperature_limit`/`pressure_limit` 报警并验证 `level/type/message/current_value/limit_value/suggestion` 字段显示 | 通过 |
| `output/playwright/vue-parity-history-zh.png` | Vue History 页中文状态，包含历史筛选、产物比例、产物结果录入、保存产物结果、产率和目标温度字块 | 通过 |
| `output/playwright/vue-parity-history-en.png` | Vue History 页英文状态，包含 History Filters、Product ratio、Product Result Entry、Save Product Result、Yield % 和 Target temperature 字块 | 通过 |
| `output/playwright/vue-parity-modbus-zh.png` | Vue Modbus 页中文状态，包含集成接口状态、基础模型入口、LoRA 推理闭环和 PRD LoRA/RK 闭环字块 | 通过 |
| `output/playwright/vue-parity-modbus-en.png` | Vue Modbus 页英文状态，包含 Integration Surface、Base inference、LoRA inference 和 PRD LoRA/RK 字块 | 通过 |
| `output/playwright/vue-history-xlsx-export-zh.png` | Vue History 页中文导出状态，验证“导出 CSV / 导出 XLSX”按钮字块可见 | 通过 |
| `output/playwright/vue-history-xlsx-export-en.png` | Vue History 页英文导出状态，验证 `Export CSV / Export XLSX` 按钮字块可见 | 通过 |
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

## 3. 移动/平板/浏览器矩阵自动化证据

| 文件 | 验证点 | 结论 |
| --- | --- | --- |
| `output/playwright/vue-mobile-verification.json` | Chromium 手机 `390x844` 与平板 `820x1180` 视口，七个 Vue 路由 × 中英双语导航点击、标题可见、横向溢出、滚动可达性、console error 和 `[object Object]` 渲染占位符检查 | 通过 |
| `output/playwright/vue-mobile-phone-*-*.png` | 手机视口七路由中英文 full-page 截图 | 通过 |
| `output/playwright/vue-mobile-tablet-*-*.png` | 平板视口七路由中英文 full-page 截图 | 通过 |
| `output/playwright/vue-mobile-phone-history-en.png` / `output/playwright/vue-mobile-phone-history-zh.png` | History 页批次搜索、状态筛选、产物比例筛选、产物结果录入和筛选结果联动在手机视口下可见 | 通过 |
| `output/playwright/vue-mobile-phone-modbus-en.png` / `output/playwright/vue-mobile-phone-modbus-zh.png` / `output/playwright/vue-mobile-tablet-modbus-en.png` / `output/playwright/vue-mobile-tablet-modbus-zh.png` | Modbus 页集成接口状态和 LoRA readiness 字段在手机/平板中英文视口下可见、可滚动且无横向溢出 | 通过 |
| `output/playwright/vue-browser-matrix-verification.json` | Playwright 浏览器矩阵脚本覆盖 bundled Chromium、系统 Chrome、系统 Microsoft Edge、Firefox 和 WebKit；2026-06-07 严格模式复跑结果为 5 个浏览器全通过、0 skipped、70 个页面/语言组合通过、console error 为 0 | 通过 |
| `output/playwright/vue-browser-matrix-chromium-*-*.png` / `output/playwright/vue-browser-matrix-chrome-*-*.png` / `output/playwright/vue-browser-matrix-msedge-*-*.png` / `output/playwright/vue-browser-matrix-firefox-*-*.png` / `output/playwright/vue-browser-matrix-webkit-*-*.png` | Chromium、Chrome、Microsoft Edge、Firefox 与 WebKit 桌面视口七路由中英文 full-page 截图，覆盖标题可见、导航可达、无横向溢出和无 `[object Object]` 渲染占位符 | 通过 |
| `output/acceptance/acceptance-report.json` | 本地一键验收 `20 pass / 0 fail / 20 total`，含 Vue release 资源、生产 safety guard、备份/恢复、培训交付物、ops preflight、RBAC/load、Vue parity、History CSV/XLSX、工艺生命周期、手机/平板响应式、浏览器矩阵、CLI 运维、AINAS/MQTT、真实 mosquitto broker round-trip 和 AINAS/STM32 mock | 通过 |
| `output/playwright/vue-history-xlsx-export-verification.json` | `scripts/verify-vue-history-xlsx.mjs` 独立验证 History CSV/XLSX 浏览器下载事件，并检查中英按钮字块和截图 | 通过 |

这组证据是本地 Chromium 桌面/手机/平板视口、系统 Chrome、系统 Microsoft Edge、Playwright Firefox 和 WebKit 桌面视口自动化验收，并新增了可复跑的 Playwright 浏览器矩阵脚本；当前仍不等同于 macOS Safari、iOS/Android 真机或客户最终签字。

## 4. AI SOP 草案文字审计

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

## 5. 工具与边界

- 首选 Windows Computer Use 插件连接时返回 `Computer Use native pipe path is unavailable`，当前 Codex 运行时没有暴露 native pipe；没有伪称已使用该插件完成桌面自动化。
- 最终视觉与文字审计由 in-app Browser 截图复核和 Playwright 本地页面巡检完成。
- 2026-06-05 legacy `static/index.html` 自动化巡检登录 `engineer` 后覆盖 `monitor / recipes / program / ai / materials / alarms / audit / modbus / settings` 九页。报告摘要：`englishPagesWithUnexpectedCjk=[]`、`pagesWithMojibake=[]`、`pagesWithEmptyViewText=[]`、`unexpectedConsoleMessages=[]`；`/api/live` 的 503 均为当前没有新鲜 pipeline 样本的预期业务状态。本轮同时复核 Modbus admin-only 写入面板中英文文案可见且无布局溢出。
- 2026-06-05 追加复核 StepFun 配置下的本地缓存推荐状态：注入 `provider.mode = stale_local_recommendation` 后，Settings 页英文显示 `Refresh cached local recommendation before AI master control`，中文显示 `AI 主控前需刷新缓存的本地推荐`，且中文模式无该英文残留。
- 2026-06-07 追加复核 History 页批次搜索、状态筛选、产物比例筛选、产物结果录入、产率/产物比例/目标参数展示和 CSV/XLSX 下载点击；`scripts/verify-vue-parity.mjs` 会实际等待 History CSV/XLSX download 事件，`scripts/verify-vue-history-xlsx.mjs` 会独立检查中英“导出 CSV / 导出 XLSX”按钮字块和下载事件，`scripts/verify-vue-mobile.mjs` 与 parity gate 会拒绝 `[object Object]` 渲染占位符。
- 2026-06-07 追加复核 Monitor 正式样本报警流：`scripts/verify-vue-parity.mjs` 通过 `/api/v1/reactor/reactor_001/samples` 注入 `temperature_c=170`、`pressure_mpa=1.2` 的越限样本，确认 `/api/live` 返回 `temperature_limit` 和 `pressure_limit`，并在中英文页面验证报警级别、类型、说明、当前/限值和建议字段。
- 2026-06-07 追加复核 Modbus 集成接口状态和 LoRA readiness：`scripts/verify-vue-parity.mjs`、`scripts/verify-vue-mobile.mjs` 和 `scripts/verify-vue-browser-matrix.mjs` 均检查 Modbus 路由中的 `Integration Surface` / `集成接口状态`、`Base inference` / `基础模型入口`、`LoRA inference` / `LoRA 推理闭环`、`PRD LoRA/RK` / `PRD LoRA/RK 闭环`，并继续拒绝 `[object Object]` 渲染占位符。
- 2026-06-07 追加浏览器矩阵复核：`scripts/verify-vue-browser-matrix.mjs` 会按 bundled Chromium、系统 Chrome、系统 Microsoft Edge、Firefox 和 WebKit 检查七个 Vue 路由 × 中英双语标题、导航可达、横向溢出、console error 和 `[object Object]` 渲染占位符；严格模式复跑结果为 Chromium/Chrome/Edge/Firefox/WebKit 各完整通过 14 个页面检查，0 skipped，70 个页面/语言组合通过。
- 2026-06-07 完整本地一键验收结果为 `20 pass / 0 fail / 20 total`；`verify-mosquitto-broker` 本轮已完成真实 broker status/task/receipt round-trip，未 skipped。
- 当前截图证明本地 HMI 页面和动态字块可正确切换，并新增手机/平板 Chromium 视口自动化检查、Chromium/Chrome/Edge/Firefox/WebKit 桌面浏览器矩阵检查；不等价于 macOS Safari、iOS/Android 真机或生产用户验收签字。

## 6. 非截图验收证据

| 文件 | 验证点 | 结论 |
| --- | --- | --- |
| `output/upper-computer-perf-smoke.json` | 本机只读 API 往返和安全计算性能冒烟 | 通过；API p95 最高 4ms，`safety_compute` p95=1ms |
| `output/upper-computer-sample-ingest.json` | CLI 通过正式 v1 样本入口注入无硬件演示样本 | 通过；`duration_s=3`，`samples_pushed=6` |
| `output/upper-computer-resource-snapshot.json` | Windows debug 本地演示进程资源快照 | 通过；working set 26.977MB，CPU max 1.533% |
| `docs/upper_computer_security_key_lifecycle.md` | 密钥生命周期、证书/token 和敏感字段清单 | 已文档化；生产轮换演练和安全扫描待验收 |

性能冒烟不证明 STM32/RS485 采集延迟、真实执行器控制延迟、Qwen/LoRA 推理训练延迟、7x24、MTBF 或外部 MQTT/Modbus 工具性能。
