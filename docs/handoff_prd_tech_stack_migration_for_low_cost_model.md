# PRD 技术栈切换交接文档：给低成本模型继续推进用

本文档用于把当前 `codex/prd-tech-stack-migration` 分支的真实状态、下一步任务、实现边界、验证方式和提交要求交给另一个模型继续推进。
请严格按本文档执行。不要凭记忆猜，不要为了“看起来完成”改小目标。

## 0. 一句话目标

在现有上位机工程上，继续把 PRD 要求的技术栈落地：

- 前端：从生产 `static/index.html` 逐步迁移到 `frontend/` 的 Vue 3 + Vite + Element Plus + ECharts + Pinia + Vue Router。
- 后端：继续推进 SQLx、tokio-modbus 等 PRD 技术栈差距。
- 验收：所有新增能力必须能本地运行、构建通过、浏览器视觉验证通过，并把边界写进文档。

当前最建议低成本模型做的下一步是：

> 完成 Vue 版“工艺管理 / 批次流程启动停止”迁移切片。

原因：这是当前 Vue 前端替换生产 HMI 前最明显的 parity 缺口之一，后端 API 已经具备，适合低成本模型接力实现。

## 1. 仓库和分支状态

工作目录：

```text
X:\tianhks
```

当前分支：

```text
codex/prd-tech-stack-migration
```

远端：

```text
origin https://github.com/lizuyi-6/ReactorOS.git
```

当前已推送到远端的最新提交：

```text
75f9afef Wire Vue audit export actions
```

最近关键提交：

```text
75f9afef Wire Vue audit export actions
3fdf8174 Wire Vue Modbus debug actions
8b4bf854 Wire Vue control page actions
4d6ceade Localize Vue HMI route views
b2ac5bfd Add Vue HMI language switch
051c79a2 Migrate batch detail reads to SQLx
f72f6cab Migrate process configuration to SQLx
a6276527 Migrate audit event writes to SQLx
```

## 2. 当前工作区特别注意

我在开始推进“工艺管理 / 批次流程”时已经改了一个文件，但还没有完成、没有构建验证、没有提交：

```text
frontend/src/stores/plant.ts
```

这份未提交改动是有意留下的半成品，方向是正确的：给 Pinia store 增加工艺、步骤、批次、启动/停止流程相关 API。
低成本模型可以继续沿用它，但必须认真检查、补全、构建验证后再提交。

当前该文件新增的方向包括：

- `CreateProcessPayload`
- `ProcessStepPayload`
- `processes`
- `selectedProcess`
- `batches`
- `loadProcesses`
- `loadProcessDetail`
- `createProcess`
- `addProcessStep`
- `startProcess`
- `stopCurrentProcess`

注意：这份半成品可能还需要调整类型、错误处理、刷新逻辑、UI 调用方式，不要直接假设它已经完成。

## 3. 不要提交的本地杂项

工作区有很多未跟踪文件是验证产物、报告草稿、截图或临时输出。不要提交它们，除非用户明确要求。

不要提交这些：

```text
CLAUDE.md
code_audit_report.md
output/*
upper-computer-*.png
```

其中 `output/playwright/*.png/json/csv` 是浏览器验证证据，可以保留在本地，但不要纳入 git commit。

## 4. 已完成的能力

### 4.1 Vue 前端基础栈

`frontend/` 已经是一个真实 Vue 应用，不是占位页面：

- Vue 3
- Vite
- TypeScript
- Element Plus
- ECharts
- Pinia
- Vue Router
- 单文件构建输出到 `frontend/dist/index.html`

构建命令：

```powershell
npm run frontend:build
```

### 4.2 Vue 七个 PRD 页面已存在

路由位于：

```text
frontend/src/router.ts
```

当前七页：

- `/monitor` 实时监控
- `/control` 参数配置 / 控制
- `/ai` AI 决策
- `/history` 历史数据
- `/audit` 审计日志
- `/modbus` Modbus 调试
- `/settings` 系统配置

### 4.3 中英切换已经接入

Pinia store 中维护语言状态：

```text
frontend/src/stores/plant.ts
```

核心函数：

```ts
function tr(zh: string, en: string): string
function setLanguage(nextLanguage: UiLanguage): void
function toggleLanguage(): void
```

语言持久化 localStorage key：

```text
reactoros.vue.language
```

此前已经用浏览器验证七个路由的关键中英字块。

### 4.4 控制页已经接入基础安全写入

文件：

```text
frontend/src/views/ControlView.vue
```

已接入：

- `POST /api/control/targets`
- `POST /api/control/auto`
- `POST /api/control/manual-lock`
- `POST /api/control/emergency-stop`
- `POST /api/control/emergency-stop/reset`

Pinia 对应方法：

- `updateTargets`
- `setAutoEnabled`
- `setManualLocked`
- `triggerEmergencyStop`
- `resetEmergencyStop`

### 4.5 审计页已经接入链状态和 CSV 导出

文件：

```text
frontend/src/views/AuditView.vue
```

已完成：

- 审计链指标
- 事件类型过滤
- page size
- 上一页 / 下一页
- bearer 授权 CSV 导出
- 中英文视觉验证
- 横向溢出检查

Pinia 对应方法：

- `loadAudit`
- `exportAuditCsv`

验证证据本地路径：

```text
output/playwright/vue-audit-export-verification.json
output/playwright/vue-audit-export-en.png
output/playwright/vue-audit-export-zh.png
```

不要提交这些证据文件。

### 4.6 Modbus 调试页已经接入

文件：

```text
frontend/src/views/ModbusView.vue
```

已完成：

- `GET /api/modbus/registers`
- `GET /api/modbus/registers/:name/read`
- admin-only `POST /api/modbus/registers/:name/write`
- 写入必须带非空 audit reason
- 写入后 read-back
- 中英文视觉验证

Pinia 对应方法：

- `readModbusRegister`
- `writeModbusRegister`

### 4.7 后端 SQLx 已覆盖大量运行路径

SQLx 已经覆盖很多真实运行 API：

- 审计 total/list/chain/export
- 审计事件写入
- 工艺流程列表/详情/创建/更新
- 工艺步骤新增/更新
- 工艺应用标记
- 批次创建/结束
- 批次详情/报告读取
- demo alarm 读取
- AI 推荐输入/缓存/写入
- 产品结果写入
- 实时样本读写
- AINAS/MQTT 集成任务查询/创建/更新

但还没完全替换：

- schema migration
- 内存测试库
- 部分兼容路径

所以不能说 SQLx 已经完全完成。

## 5. 当前明确未完成的大项

这些不要假装已经完成：

1. Vue 生产替换还没完成
   生产 daemon 仍托管 `static/index.html`，不是 `frontend/dist/index.html`。

2. Vue 完整 parity 还没完成
   重点缺口包括工艺管理/批次流程、更完整历史报表交互、AI 深度动作、系统设置动作等。

3. 本地 Qwen3.5-2B + LoRA 真接入没完成
   缺模型、adapter、推理服务、训练脚本、GGUF 转换、RK 延迟报告。

4. SQLx 没完全替代 rusqlite
   schema migration 和部分兼容路径还在 rusqlite。

5. Modbus TCP server 仍是自实现
   RTU 主站已用 `tokio-modbus`，但 TCP server 还没切到 tokio-modbus server feature。

6. 外部/硬件验收没完成
   缺 STM32 实机、RS485、Modbus Poll/Slave、MQTT.fx/mosquitto、AINAS 真实任务、生产证书链。

7. 生产安全运维没完成
   缺 watchdog、低权限运行、密钥托管/轮换、正式漏洞扫描、备份/归档/安全擦除演练。

8. 正式性能可靠性没完成
   缺 release/RK 稳态 CPU/内存、7x24、MTBF、真实控制延迟、RS485 丢包率、本地 LoRA <3s 延迟。

## 6. 推荐低成本模型立刻做的任务

### 任务名称

完成 Vue 工艺管理 / 批次流程迁移切片。

### 任务目标

在 `frontend/` Vue HMI 中，让用户可以：

1. 查看已有工艺列表。
2. 查看选中工艺详情和步骤。
3. 创建一个新工艺。
4. 给工艺添加步骤。
5. 启动一个已有工艺。
6. 停止当前运行工艺。
7. 查看当前 active batch / recent batches 摘要。
8. 所有可见文字都支持中英切换。
9. 浏览器视觉验证通过，不能有明显文本重叠、横向溢出、按钮不可见。

### 推荐放置页面

优先放在：

```text
frontend/src/views/ControlView.vue
```

理由：

- PRD 七页中的“参数配置”页天然包含工艺执行前复核和启动停止。
- 后端流程 API 本身是安全门控执行路径。
- 当前 `ControlView.vue` 已经有目标写入、自动控制、人工锁定、急停，新增工艺生命周期面板顺手。

不要新增第八个路由。保持 PRD 七页结构。

## 7. 后端 API 参考

所有写操作都需要 bearer token。建议用 engineer 登录，因为 engineer 有：

- `edit_process`
- `start_stop_process`
- `set_safe_targets`
- `view_audit`

登录接口：

```http
POST /api/auth/login
Content-Type: application/json

{
  "username": "engineer",
  "password": "engineer123"
}
```

Pinia 已有 `store.login("engineer")` 快捷方式。

### 7.1 工艺列表

```http
GET /api/processes
Authorization: Bearer <token>
```

响应 envelope 解包后是数组：

```ts
ProcessDefinition[]
```

字段：

```ts
{
  id: number;
  name: string;
  description: string;
  status: string;        // draft | applied | archived
  version: number;
  step_count: number;
  created_at: string;
  updated_at: string;
  applied_at: string | null;
}
```

### 7.2 创建工艺

```http
POST /api/processes
Authorization: Bearer <token>
Content-Type: application/json

{
  "name": "Vue acceptance process",
  "description": "Created from Vue HMI"
}
```

权限：

```text
Permission::EditProcess
```

返回：

```ts
ProcessDefinition
```

### 7.3 工艺详情

```http
GET /api/processes/:id
Authorization: Bearer <token>
```

返回：

```ts
{
  process: ProcessDefinition;
  steps: ProcessStep[];
}
```

`ProcessStep` 字段：

```ts
{
  id: number;
  process_id: number;
  step_index: number;
  name: string;
  target_temperature_c: number;
  ramp_rate_c_min: number;
  duration_minutes: number;
  target_stirrer_rpm: number;
  target_shake_speed_cpm: number;
  target_pressure_mpa: number;
  cooling_mode: string;
  created_at: string;
  updated_at: string;
}
```

### 7.4 添加步骤

```http
POST /api/processes/:id/steps
Authorization: Bearer <token>
Content-Type: application/json

{
  "name": "Heat",
  "target_temperature_c": 65,
  "ramp_rate_c_min": 2,
  "duration_minutes": 20,
  "target_stirrer_rpm": 320,
  "target_shake_speed_cpm": 30,
  "target_pressure_mpa": 0.5,
  "cooling_mode": "natural"
}
```

重要安全范围：

- `target_temperature_c` 必须通过 safety config。
- `target_stirrer_rpm` 必须通过 safety config。
- 温度-转速组合不能落入 forbidden control zones。
- `duration_minutes` 最小 1。
- `target_shake_speed_cpm` 范围 0 到 60。
- `target_pressure_mpa` 范围 0 到 10。

低成本模型不要绕过后端校验。

### 7.5 启动工艺

```http
POST /api/processes/:id/start
Authorization: Bearer <token>
```

权限：

```text
Permission::StartStopProcess
```

启动前要求：

- 当前没有 active batch。
- 工艺必须至少有一个 step。
- 当前 runtime 不能处于 emergency stop。
- 当前 runtime 不能 manual lock。
- 传感器新鲜度等后端条件必须满足。

返回：

```ts
{
  process: ProcessDefinition;
  batch: Batch;
  applied_targets: ControlTargets;
  status: "running";
}
```

启动后：

- runtime targets 会更新。
- runtime active_batch_id 会更新。
- auto_enabled 会变 true。
- 审计事件会写入。

### 7.6 停止当前工艺

```http
POST /api/processes/current/stop
Authorization: Bearer <token>
```

返回：

```ts
{
  stopped_batch_id: number;
  process_id: number | null;
  batch: Batch;
  active_batch_id: number | null;
  auto_enabled: boolean;
  stopped_targets: ControlTargets;
}
```

停止后：

- active_batch_id 应为 null。
- auto_enabled 应为 false。
- stopped_targets 里 shake speed 等应被安全停止。
- batch finished_at 应有值。

### 7.7 批次列表

```http
GET /api/batches
Authorization: Bearer <token>
```

返回：

```ts
{
  batches: Batch[];
  outcomes: BatchOutcome[];
}
```

## 8. 需要改的前端文件

### 8.1 必改：Pinia store

文件：

```text
frontend/src/stores/plant.ts
```

当前已有半成品改动。低成本模型应该继续补齐，而不是重写全文件。

建议最终 store 至少提供：

```ts
processes: Ref<ApiRecord[]>
selectedProcess: Ref<ApiRecord | null>
batches: Ref<ApiRecord | null>

loadProcesses(): Promise<ApiRecord[]>
loadProcessDetail(processId: number): Promise<ApiRecord>
createProcess(payload: CreateProcessPayload): Promise<ApiRecord>
addProcessStep(processId: number, payload: ProcessStepPayload): Promise<ApiRecord>
startProcess(processId: number): Promise<ApiRecord>
stopCurrentProcess(): Promise<ApiRecord>
```

注意点：

1. `refreshProtected()` 可以并发加载 `/api/processes` 和 `/api/batches`。
2. `startProcess()` 成功后应刷新 live、protected 数据。
3. `stopCurrentProcess()` 成功后应刷新 live、protected 数据。
4. `runtimeFallback` 可以用于 `/api/live` stale 时显示刚刚启动/停止的结果。
5. 不要让 `request<T>` 处理 CSV blob，CSV 已经有 `requestBlob()`。

### 8.2 必改：Control 页面

文件：

```text
frontend/src/views/ControlView.vue
```

建议新增一个或两个 panel：

1. 工艺管理 / Process Recipes
2. 批次生命周期 / Batch Lifecycle

建议 UI 结构：

- 左侧/上方：创建工艺表单
  - name
  - description
  - create button
- 中间：工艺列表
  - id
  - name
  - status
  - step_count
  - applied_at
  - 操作：查看 / 启动
- 右侧/下方：选中工艺详情
  - 基础信息
  - steps table
  - 添加步骤表单
- 底部：当前运行
  - active_batch_id
  - auto_enabled
  - 当前 targets
  - stop current process button
- 最近批次摘要
  - id
  - name
  - process_id
  - status
  - started_at
  - finished_at

不要过度复杂。目标是能完成真实生命周期操作，不是做花哨页面。

### 8.3 可能要改：CSS

文件：

```text
frontend/src/styles.css
```

如果新增表单/布局，需要复用已有风格：

- `panel`
- `control-panel`
- `control-form`
- `control-actions`
- `data-table`
- `target-summary`

可以新增类似：

```css
.process-panel
.process-form
.process-grid
.process-detail-grid
```

要求：

- 不允许文字挤出按钮。
- 不允许横向溢出。
- 不允许卡片套卡片。
- 页面窄屏能单列显示。

### 8.4 必改：文档

至少更新：

```text
frontend/README.md
docs/architecture-deviations.md
docs/upper_computer_development_doc.md
```

如果完成工艺切片，文档中原来的“工艺管理仍需迁移”要改成：

- Vue 已接入工艺列表/详情/创建/步骤添加/启动/停止。
- 生产 HMI 替换和完整 parity 仍未完成。

不要把“Vue 工艺切片完成”写成“生产交付完成”。

## 9. 推荐实现步骤

### Step 1：确认当前工作区

```powershell
git status --short --branch
git log --oneline --decorate -5
```

必须确认：

- 在 `codex/prd-tech-stack-migration`
- 远端最新提交是 `75f9afef`
- 只有 `frontend/src/stores/plant.ts` 是有意未提交改动
- 不要误提交 `output/*`

### Step 2：修完 `frontend/src/stores/plant.ts`

重点检查：

- TypeScript 类型是否通过。
- `refreshProtected()` 并发请求失败时是否会导致整页不可用。当前 store 设计是 protected 失败会进入 `store.error`，可接受。
- `startProcess()` 和 `stopCurrentProcess()` 的 runtimeFallback 是否不会写入 `undefined` 造成 UI 文本怪异。

如果要更稳，可以写一个 helper 过滤 undefined：

```ts
function mergeRuntimeFallback(patch: ApiRecord): void {
  runtimeFallback.value = Object.fromEntries(
    Object.entries({
      ...(runtimeFallback.value ?? {}),
      ...patch
    }).filter(([, value]) => value !== undefined)
  );
}
```

但不要做大重构。

### Step 3：改 `ControlView.vue`

建议新增 script state：

```ts
const processes = computed(() => store.processes);
const selectedProcess = computed(() => objectAt(store.selectedProcess, "process"));
const selectedSteps = computed(() => arrayAt(store.selectedProcess, "steps"));
const batchRows = computed(() => arrayAt(store.batches, "batches"));
const runtime = computed(() => objectAt(store.live, "runtime") ?? store.runtimeFallback);
const activeBatchId = computed(() => textAt(runtime.value, "active_batch_id"));
```

建议表单：

```ts
const processForm = reactive({
  name: "Vue acceptance process",
  description: "Created from Vue HMI"
});

const stepForm = reactive({
  name: "Heat",
  target_temperature_c: 65,
  ramp_rate_c_min: 2,
  duration_minutes: 20,
  target_stirrer_rpm: 320,
  target_shake_speed_cpm: 30,
  target_pressure_mpa: 0.5,
  cooling_mode: "natural"
});
```

建议动作：

```ts
async function createProcessFromForm(): Promise<void>
async function selectProcess(id: number): Promise<void>
async function addStepToSelectedProcess(): Promise<void>
async function startSelectedProcess(id: number): Promise<void>
async function stopCurrent(): Promise<void>
```

按钮权限：

- 创建工艺、添加步骤：`store.role === "engineer" || store.role === "admin"` 更清晰。
- 启动/停止：operator/engineer/admin 都有权限，但为了工艺编辑验收建议用 engineer。
- 所有写按钮至少要 `store.isAuthenticated`。

建议成功消息：

- `工艺已创建 / Process created`
- `步骤已添加 / Step added`
- `工艺已启动 / Process started`
- `当前工艺已停止 / Current process stopped`

### Step 4：浏览器验证脚本

必须验证真实页面，不要只跑 build。

本地通常已有后端和 Vite：

```text
http://127.0.0.1:8000/
http://127.0.0.1:5173/
```

如果没有，启动：

```powershell
$env:CARGO_TARGET_DIR='C:\tmp\xingshu-target-bugfix'
cargo run --bin reactor-edge-daemon -- `
  --config config/device.toml `
  --safety config/safety.toml `
  --memory config/ai_memory.toml `
  --integration config/integration.toml `
  --db data/reactor.sqlite3 `
  --assets static `
  --bind 127.0.0.1:8000 `
  --enable-test-reset
```

Vite：

```powershell
npm run frontend:dev
```

浏览器验证建议用 Playwright 脚本，不要手点后声称通过。

验证流程：

1. 打开 `http://127.0.0.1:5173/#/control`。
2. 登录 engineer。
3. 切英文，检查：
   - `Process Control`
   - `Process Recipes`
   - `Create Process`
   - `Add Step`
   - `Start Process`
   - `Stop Current Process`
4. 创建工艺。
5. 添加至少一个步骤。
6. 启动工艺。
7. 确认页面显示 active batch / running / applied targets。
8. 停止当前工艺。
9. 切中文，检查：
   - `参数配置`
   - `工艺管理`
   - `创建工艺`
   - `添加步骤`
   - `启动工艺`
   - `停止当前工艺`
10. 截图：
   - `output/playwright/vue-process-lifecycle-en.png`
   - `output/playwright/vue-process-lifecycle-zh.png`
11. 写验证 JSON：
   - `output/playwright/vue-process-lifecycle-verification.json`
12. 检查无横向溢出：
   - `.content.scrollWidth <= .content.clientWidth + 1`
   - `.view-stack.scrollWidth <= .view-stack.clientWidth + 1`

不要提交截图和 JSON。

## 10. 必跑检查命令

每次提交前必须跑：

```powershell
npm run frontend:build
git diff --check
$env:CARGO_TARGET_DIR='C:\tmp\xingshu-target-bugfix'; cargo check --all-targets
```

如果改了 Rust 测试相关代码，再跑相关 Rust tests。
本任务正常只改 Vue/文档，`cargo check` 足够作为后端编译守门。

## 11. 提交要求

只 stage 相关文件，不要 `git add .`。

如果完成 Vue 工艺生命周期，建议 stage：

```powershell
git add -- `
  frontend/src/stores/plant.ts `
  frontend/src/views/ControlView.vue `
  frontend/src/styles.css `
  frontend/README.md `
  docs/architecture-deviations.md `
  docs/upper_computer_development_doc.md
```

提交信息建议：

```text
Wire Vue process lifecycle actions
```

提交：

```powershell
git commit -m "Wire Vue process lifecycle actions"
```

推送：

```powershell
git -c http.version=HTTP/1.1 push origin codex/prd-tech-stack-migration
```

推送后确认：

```powershell
git status --short --branch
git rev-parse HEAD
git rev-parse origin/codex/prd-tech-stack-migration
```

两个 hash 必须一致。

## 12. 严格禁止事项

低成本模型不要做这些：

1. 不要提交 `output/*`、截图、CSV、临时报告。
2. 不要把未完成的本地 LoRA 写成已完成。
3. 不要把 Vue 迁移切片写成生产 HMI 已替换。
4. 不要删除或重写 `static/index.html`。
5. 不要为了前端方便绕过后端 RBAC/safety/audit。
6. 不要引入新的 UI 框架。
7. 不要新增第八个路由来规避 PRD 七页面结构。
8. 不要用 `git reset --hard`、`git checkout --` 清掉用户/前序模型改动。
9. 不要提交大范围格式化。
10. 不要只跑构建不做浏览器验证。

## 13. 完成标准

低成本模型完成本切片后，必须能给出以下证据：

1. Git 提交 hash。
2. 推送成功。
3. `npm run frontend:build` 通过。
4. `git diff --check` 通过。
5. `cargo check --all-targets` 通过。
6. Playwright 验证 JSON 路径。
7. 中英文截图路径。
8. 说明未提交的 `output/*` 证据文件仍保留本地。
9. 说明生产 HMI 仍未切换，目标未完成，只是向 PRD 技术栈切换推进了一步。

## 14. 做完后交给高级模型检查时要说明

请把以下内容发给高级模型：

```text
我在 codex/prd-tech-stack-migration 上完成了 Vue 工艺生命周期切片。

提交：
<commit hash> <commit message>

我改了：
- frontend/src/stores/plant.ts
- frontend/src/views/ControlView.vue
- frontend/src/styles.css
- frontend/README.md
- docs/architecture-deviations.md
- docs/upper_computer_development_doc.md

验证：
- npm run frontend:build
- git diff --check
- cargo check --all-targets
- Playwright: output/playwright/vue-process-lifecycle-verification.json
- 截图: output/playwright/vue-process-lifecycle-en.png
- 截图: output/playwright/vue-process-lifecycle-zh.png

请重点 review：
1. 是否误提交 output/ 临时文件。
2. 是否所有新增中文/英文文字都通过 store.tr。
3. 是否流程启动/停止真的走后端 API，而不是前端假状态。
4. 是否启动后 active batch 和 targets 正确回显。
5. 是否停止后 active batch 清空、auto disabled。
6. 是否页面无横向溢出、无明显文字遮挡。
7. 文档是否没有夸大为生产交付完成。
```

## 15. 如果低成本模型想继续做下一个切片

优先级建议：

1. Vue 工艺生命周期。
2. Vue 历史报表导出增强：CSV/XLSX/Markdown 下载和批次详情。
3. Vue AI 页面动作增强：生成推荐、AI master-control dry-run/execute 边界展示。
4. Vue settings 页面动作增强：权限、配置摘要、集成状态更完整。
5. 生产静态资源切换预案：先文档和构建产物，不要直接替换 daemon 默认 assets。
6. SQLx schema migration。
7. Modbus TCP server 是否迁 tokio-modbus server feature 的技术 spike。

不要让低成本模型直接碰本地 LoRA 真接入，除非已经有模型、adapter、推理服务、训练脚本和 RK 报告。那块容易写成假完成。
