# 小模型返工指令：PRD 技术栈迁移复查后修正项

编写人：Codex
仓库绝对路径：`X:\tianhks`
当前分支：`codex/prd-tech-stack-migration`
复查范围：`75f9afef..HEAD`
目标读者：负责继续推进本分支的低成本模型或工程同学

## 0. 当前结论

当前分支不是“坏到不能运行”的状态。基础验证已通过：

```powershell
Set-Location X:\tianhks
npm run frontend:build
git diff --check 75f9afef..HEAD
$env:CARGO_TARGET_DIR='C:\tmp\xingshu-target-bugfix'; cargo check --all-targets
$env:CARGO_TARGET_DIR='C:\tmp\xingshu-target-bugfix'; cargo test --test api_tests
$env:CARGO_TARGET_DIR='C:\tmp\xingshu-target-bugfix'; cargo test --test db_tests
$env:CARGO_TARGET_DIR='C:\tmp\xingshu-target-bugfix'; cargo test --test cli_tests -- --nocapture
```

但是这批提交仍不能按“工业级完成”或“PRD 技术栈迁移完成”对外宣称。主要问题不是编译失败，而是：

- 前端页面读取后端字段结构不对，真实使用会出现空值或错误。
- 生产运维命令的能力被文案夸大，尤其是备份、密钥轮换和安全擦除。
- systemd/生产文档宣称 HTTPS，但 unit 没有传 TLS 证书参数。
- Playwright / RBAC 验证脚本通过条件过宽，能把明显失败写成 `ok=true`。
- Vue cutover 文档之间相互矛盾，会误导后续部署。

本返工单要求先修“事实错误”和“验收脚本放水”，再谈新增能力。

## 1. 返工原则

1. 不要改动或提交 `X:\tianhks\output\` 下的截图、JSON、CSV、临时运行产物，除非用户明确要求提交证据产物。
2. 不要把 `X:\tianhks\CLAUDE.md`、`X:\tianhks\code_audit_report.md`、`X:\tianhks\docs\handoff_prd_tech_stack_migration_for_low_cost_model.md` 这类未跟踪文件顺手提交。
3. 所有新增文档里引用仓库文件必须使用绝对路径，例如 `X:\tianhks\frontend\src\views\AiView.vue`。
4. 如果只是把能力降级成真实描述，必须同步改文档和 CLI help，不能只改一边。
5. 如果声称某项能力“已完成”，必须有自动化验证或明确外部验收步骤。

## 2. 必修项 A：修 AI 推荐页字段读取错误

### 现象

`X:\tianhks\frontend\src\views\AiView.vue` 把最新推荐当成包含 `targets` 对象的结构读取：

- `X:\tianhks\frontend\src\views\AiView.vue:11`
- `X:\tianhks\frontend\src\views\AiView.vue:152`
- `X:\tianhks\frontend\src\views\AiView.vue:157`
- `X:\tianhks\frontend\src\views\AiView.vue:162`
- `X:\tianhks\frontend\src\views\AiView.vue:167`

但后端推荐结构不是这样。后端 `AiRecommendationEnvelope` 使用 `#[serde(flatten)]` 展开 `Recommendation`：

- `X:\tianhks\src\ai_provider.rs:44`
- `X:\tianhks\src\ai_provider.rs:45`
- `X:\tianhks\src\ai_provider.rs:46`
- `X:\tianhks\src\optimizer.rs:12`
- `X:\tianhks\src\optimizer.rs:14`
- `X:\tianhks\src\optimizer.rs:15`
- `X:\tianhks\src\optimizer.rs:19`

真实字段是：

```text
target_temperature_c
target_stirrer_rpm
heating_minutes
stirring_minutes
expected_score
rationale
based_on_batch_count
provider
```

因此 AI 页的推荐目标卡会显示空值，截图如果只看标题会漏掉这个问题。

### 推荐修法

优先改前端，不动后端接口：

1. 在 `X:\tianhks\frontend\src\views\AiView.vue` 中移除 `objectAt(recommendation.value, "targets")`。
2. 目标卡读取：
   - 温度：`textAt(recommendation.value, "target_temperature_c")`
   - 搅拌：`textAt(recommendation.value, "target_stirrer_rpm")`
   - 加热时间：`textAt(recommendation.value, "heating_minutes")`
   - 搅拌时间：`textAt(recommendation.value, "stirring_minutes")`
   - 预期分数：`textAt(recommendation.value, "expected_score")`
3. 不要显示后端没有给的推荐摇速和推荐压力。可以改成显示“当前摇速/当前压力”并明确来源是 runtime，或者直接显示 `--`。
4. `updated_at` 目前推荐结构里没有稳定字段，除非后端提供，否则 UI 不要暗示有更新时间。
5. `alternatives` 和 `reasons` 也不是当前 `Recommendation` 的稳定字段。要么删除这两块，要么只在数组存在时显示，并且不要把它们作为验收必需内容。

### 必加验证

在 `X:\tianhks\scripts\verify-vue-parity.mjs` 或单独新增脚本里加入真实推荐字段验证：

1. 通过 API 构造或获取一条推荐。
2. 打开 `http://127.0.0.1:5173/#/ai`。
3. 点击刷新推荐。
4. 验证页面正文包含非空 `target_temperature_c` 和 `target_stirrer_rpm` 数值。
5. 中英文都跑一遍。

验收命令：

```powershell
Set-Location X:\tianhks
npm run frontend:build
node X:\tianhks\scripts\verify-vue-parity.mjs
```

## 3. 必修项 B：修 History 批次详情结构读取错误

### 现象

`X:\tianhks\frontend\src\views\HistoryView.vue` 点击批次后，把 `store.loadBatchDetail(id)` 的返回值直接赋给 `selectedBatch`：

- `X:\tianhks\frontend\src\views\HistoryView.vue:63`

之后页面按批次本体读取：

- `X:\tianhks\frontend\src\views\HistoryView.vue:157`
- `X:\tianhks\frontend\src\views\HistoryView.vue:158`
- `X:\tianhks\frontend\src\views\HistoryView.vue:159`
- `X:\tianhks\frontend\src\views\HistoryView.vue:160`
- `X:\tianhks\frontend\src\views\HistoryView.vue:165`
- `X:\tianhks\frontend\src\views\HistoryView.vue:166`

但后端 `GET /api/batches/:id` 返回的是包装对象：

- `X:\tianhks\src\api.rs:286`
- `X:\tianhks\src\api.rs:287`
- `X:\tianhks\src\api.rs:288`
- `X:\tianhks\src\api.rs:1255`

结构是：

```json
{
  "batch": {},
  "outcome": null,
  "samples": [],
  "events": []
}
```

所以 History 页详情区会读错字段，报告下载也可能拿不到正确 id。

### 推荐修法

1. 把 `selectedBatch` 改名为 `selectedBatchDetail`，类型仍可先用 `Record<string, unknown> | null`。
2. 新增 computed：

```ts
const selectedBatch = computed(() => objectAt(selectedBatchDetail.value, "batch"));
const selectedOutcome = computed(() => objectAt(selectedBatchDetail.value, "outcome"));
const selectedSamples = computed(() => arrayAt(selectedBatchDetail.value, "samples"));
const selectedEvents = computed(() => arrayAt(selectedBatchDetail.value, "events"));
```

3. `selectBatch(id)` 中赋值给 `selectedBatchDetail.value`。
4. 详情区读取 `selectedBatch`，而不是 detail 包装对象。
5. 报告下载 id 从 `numberAt(selectedBatch.value, "id")` 取。
6. 可顺手把 samples/events 做一个小表或计数，证明详情接口不是只拿批次元信息。

### 必加验证

修改 `X:\tianhks\scripts\verify-vue-parity.mjs`：

1. 保证测试数据库里至少有 1 个 batch。
2. 打开 `/#/history`。
3. 点击第一行 batch。
4. 验证详情区出现：
   - Batch ID / 批次 ID
   - Name / 名称
   - Status / 状态
   - Download Report / 下载报告
5. 点击 Download Report，要求返回 blob size > 0。

验收命令：

```powershell
Set-Location X:\tianhks
npm run frontend:build
node X:\tianhks\scripts\verify-vue-parity.mjs
```

## 4. 必修项 C：重做或降级 `xingshu ops backup/key rotate`

### 现象

CLI help 和生产文档把备份、恢复、擦除、密钥轮换描述成生产级能力，但实现没有达到对应语义。

相关代码：

- `X:\tianhks\src\bin\xingshu.rs:353`
- `X:\tianhks\src\bin\xingshu.rs:357`
- `X:\tianhks\src\bin\xingshu.rs:360`
- `X:\tianhks\src\bin\xingshu.rs:1611`
- `X:\tianhks\src\bin\xingshu.rs:1621`
- `X:\tianhks\src\bin\xingshu.rs:1768`
- `X:\tianhks\src\bin\xingshu.rs:1774`
- `X:\tianhks\src\bin\xingshu.rs:1790`
- `X:\tianhks\src\bin\xingshu.rs:1800`

相关文档：

- `X:\tianhks\docs\upper_computer_production_operations.md:99`
- `X:\tianhks\docs\upper_computer_production_operations.md:111`
- `X:\tianhks\docs\upper_computer_production_operations.md:180`
- `X:\tianhks\docs\upper_computer_production_operations.md:181`
- `X:\tianhks\docs\upper_computer_production_operations.md:205`
- `X:\tianhks\docs\upper_computer_external_acceptance_handoff.md:22`
- `X:\tianhks\docs\upper_computer_external_acceptance_handoff.md:23`

具体问题：

1. `ops backup` 文案说 tar.gz，但实现只是 `fs::copy(db, out)`。
2. 默认输出是 `backups/reactor-backup.tar.gz`，但内容不是 tar.gz。
3. `include_ciphertext` 只是被写进输出 JSON，没有改变任何备份行为。
4. `ops wipe` 只处理 SQLite 文件，还在文案里说旁路密文要另行擦除。这不能叫完整生产擦除。
5. `key rotate` 只生成新 key 文件，没有迁移旧密文行。
6. `key rotate` 把完整密钥写进 human output 和 JSON 的 `new_key_env`，这不适合生产日志。
7. 文档里有 `xingshu ops key rotate`，但真实 CLI 是 `xingshu key rotate`。

### 可选修法 1：真实实现生产级能力

如果要保留“生产级备份/轮换”宣称，必须完成：

1. `ops backup`
   - 输出真实 tar.gz，包含 manifest。
   - manifest 至少包括 schema version、创建时间、源 DB path、sha256、是否包含配置/密文。
   - SQLite 备份必须用 SQLite backup API 或至少在 daemon 停止/只读窗口中执行，不能只在运行中裸 `fs::copy`。
   - `include_ciphertext` 必须真的收集相关密文字段或外部密文文件；如果密文只在 DB 内，也要说明该 flag 无意义并删除。
2. `ops restore`
   - 校验 manifest 和 sha256。
   - 拒绝把非本工具生成的 tar.gz 当 SQLite 恢复。
   - 恢复前保留 `.pre-restore`，并校验恢复后的 DB 能打开、schema 能迁移。
3. `ops wipe`
   - 明确擦除范围：DB、WAL、SHM、backup、key file、integration 外部目录。
   - 对 SSD/NVMe 说明 overwrite 不等于物理擦除，文档建议 `blkdiscard` 或设备级 secure erase。
4. `key rotate`
   - 新 key 文件权限应为 0600 或文档明确要求。
   - 不要把完整密钥打印到 stdout 或 JSON。
   - 如果旧行已加密，必须用旧 key 解密后用新 key 重加密，事务内完成。
   - 增加测试：旧 key 写入 integration task，rotate 后新 key 能读取旧行且新写入使用新 key。

### 可选修法 2：降级成真实描述

如果不想大改实现，就必须降级文案：

1. `ops backup` 改名或文案改为 SQLite 文件快照。
2. 默认输出改成 `.sqlite3`，不要 `.tar.gz`。
3. 删除 `include_ciphertext`，或明确写成 no-op 并不建议保留。
4. `key rotate` 改名为 `key generate`，只负责生成新环境变量文件，不声称 re-encrypt。
5. `X:\tianhks\docs\upper_computer_external_acceptance_handoff.md` 中这两项不能标 READY，只能标 `PARTIAL` 或 `SCRIPT-ONLY`。

### 必加验证

至少补 CLI 测试：

```powershell
Set-Location X:\tianhks
$env:CARGO_TARGET_DIR='C:\tmp\xingshu-target-bugfix'
cargo test --test cli_tests -- --nocapture
```

如果选择真实实现，还要补 DB 加密轮换测试：

```powershell
Set-Location X:\tianhks
$env:CARGO_TARGET_DIR='C:\tmp\xingshu-target-bugfix'
cargo test --test db_tests integration_task_payloads_encrypt_at_rest_when_key_is_configured -- --nocapture
```

## 5. 必修项 D：修 systemd HTTPS 配置矛盾

### 现象

生产 unit 监听 `0.0.0.0:8443`：

- `X:\tianhks\deploy\reactor-edge-daemon.service:14`
- `X:\tianhks\deploy\reactor-edge-daemon.service:20`
- `X:\tianhks\deploy\reactor-edge-daemon.service:21`

文档说这是 HTTPS：

- `X:\tianhks\docs\upper_computer_production_operations.md:142`
- `X:\tianhks\docs\upper_computer_production_operations.md:162`

但 unit 没传：

```text
--tls-cert
--tls-key
```

结果是 8443 上可能跑明文 HTTP。工业现场这属于危险误导。

### 推荐修法

二选一。

方案 A：daemon 自己终止 TLS。

修改 `X:\tianhks\deploy\reactor-edge-daemon.service`：

```text
  --tls-cert /etc/xingshu/tls/server.pem \
  --tls-key /etc/xingshu/tls/server-key.pem \
  --bind 0.0.0.0:8443
```

并确认 `ReadOnlyPaths` 或普通读取权限允许 `xingshu` 用户读证书。

方案 B：反向代理终止 TLS。

修改 unit：

```text
  --bind 127.0.0.1:8000
```

然后在 `X:\tianhks\docs\upper_computer_production_operations.md` 写清楚 nginx/caddy/设备网关负责 8443 TLS，daemon 只监听 localhost。

### 必加验证

如果选方案 A：

```powershell
Set-Location X:\tianhks
openssl s_client -connect 127.0.0.1:8443 -tls1_3
```

如果选方案 B：

```powershell
Set-Location X:\tianhks
curl http://127.0.0.1:8000/health
```

并在文档中不要再把 daemon 的明文端口叫 HTTPS。

## 6. 必修项 E：重写验证脚本的通过条件

### 现象

当前验证脚本会把明显失败算通过。

`X:\tianhks\scripts\verify-vue-parity.mjs`：

- `X:\tianhks\scripts\verify-vue-parity.mjs:117` 记录 `open-en fail`，但没有让脚本失败。
- `X:\tianhks\scripts\verify-vue-parity.mjs:153` 允许 English 缺 3 个文案、Chinese 缺 1 个文案。

`X:\tianhks\scripts\verify-vue-process-lifecycle.mjs`：

- `X:\tianhks\scripts\verify-vue-process-lifecycle.mjs:286` 允许英文缺 2 个。
- `X:\tianhks\scripts\verify-vue-process-lifecycle.mjs:287` 允许中文缺 2 个。

`X:\tianhks\scripts\verify-load-and-rbac.ps1`：

- `X:\tianhks\scripts\verify-load-and-rbac.ps1:159` operator 调 AINAS 期望 false。
- `X:\tianhks\scripts\verify-load-and-rbac.ps1:167` 只判断“是否 2xx”，导致 500 也被当成“拒绝成功”。
- `X:\tianhks\output\load-and-rbac-report.json` 中已经出现 operator 调 AINAS 返回 500 但 `ok=true` 的证据。

### 必须修改

1. 所有 `open-en`、`open-zh` 失败必须导致 `result.ok=false`。
2. 所有必检文案缺失必须导致脚本失败，不允许 missing 容忍。
3. RBAC 脚本必须区分：
   - 期望允许：必须是 2xx。
   - 期望拒绝：必须是 401 或 403。
   - 500、502、503、0 一律 fail。
4. 所有验证报告中的 `ok=true` 必须意味着没有任何 fail step。
5. 不要把“已知问题”写到 findings 里但仍让 `ok=true`。有 findings 就应该 fail，或改名为 `notes` 且只放非阻断事项。

### 必加验证

```powershell
Set-Location X:\tianhks
node X:\tianhks\scripts\verify-vue-parity.mjs
node X:\tianhks\scripts\verify-vue-process-lifecycle.mjs
powershell -ExecutionPolicy Bypass -File X:\tianhks\scripts\verify-load-and-rbac.ps1
```

如果脚本依赖运行中的服务，文档必须写清楚启动命令、端口、账号和数据准备步骤。

## 7. 必修项 F：统一 Vue cutover 文档口径

### 现象

`X:\tianhks\frontend\README.md` 仍说：

- `X:\tianhks\frontend\README.md:5`
- `X:\tianhks\frontend\README.md:19`

大意是生产板端仍托管 `static/index.html`，不要指向 `frontend/dist`。

但新切换文档和 daemon auto 逻辑说：

- `X:\tianhks\src\main.rs:23`
- `X:\tianhks\src\main.rs:28`
- `X:\tianhks\src\main.rs:34`
- `X:\tianhks\docs\upper_computer_static_cutover.md:22`
- `X:\tianhks\docs\upper_computer_static_cutover.md:23`
- `X:\tianhks\docs\upper_computer_static_cutover.md:81`

大意是如果 `frontend/dist/index.html` 存在，daemon 默认优先托管 Vue。

这两个口径互相冲突。

### 推荐修法

必须选一个真实状态。

方案 A：承认已经 cutover。

1. `X:\tianhks\frontend\README.md` 改成：生产 daemon 的 `--assets auto` 会优先托管 `X:\tianhks\frontend\dist`。
2. 文档写清回滚方式：显式传 `--assets X:\tianhks\static`。
3. 所有“不要指向 frontend/dist”的旧表述删除或改成历史说明。

方案 B：还没 cutover。

1. `X:\tianhks\src\main.rs` 的默认 `--assets` 改回 `static`。
2. `auto` 只作为显式 opt-in。
3. `X:\tianhks\docs\upper_computer_static_cutover.md` 不得写“默认托管 Vue”。

当前更推荐方案 A，因为代码已经按 auto 优先 Vue 写了。

## 8. 建议项 G：SQLx migration 不要吞掉启动错误

### 现象

`X:\tianhks\src\main.rs` 中：

- `X:\tianhks\src\main.rs:104`
- `X:\tianhks\src\main.rs:105`

`db.migrate_sqlx().await` 失败只打 warn，然后 daemon 继续启动。对于生产服务，这可能导致启动成功但部分 SQLx 路径运行时 500。

### 建议修法

1. 生产启动时 migration 失败应直接返回错误并终止启动。
2. 如果担心兼容旧环境，可以加显式参数，例如 `--allow-sqlx-migration-warning`，默认不允许。
3. 增加测试：损坏 DB/schema 时 daemon 不应该静默启动。

## 9. 建议项 H：生产运维文档存在终端编码风险

### 现象

在 PowerShell 里读取 `X:\tianhks\docs\upper_computer_production_operations.md` 时曾出现乱码显示。用 Python UTF-8 读取源码时部分前端源码正常，所以这不一定是文件损坏，可能是终端编码问题。

### 要求

1. 不要盲目“修复乱码”导致二次破坏。
2. 用下面命令确认文档真实编码：

```powershell
Set-Location X:\tianhks
python - <<'PY'
from pathlib import Path
for p in [
    r"X:\tianhks\docs\upper_computer_production_operations.md",
    r"X:\tianhks\docs\upper_computer_external_acceptance_handoff.md",
    r"X:\tianhks\frontend\src\views\AiView.vue",
    r"X:\tianhks\frontend\src\views\HistoryView.vue",
]:
    text = Path(p).read_text(encoding="utf-8")
    print(p, text[:120].encode("unicode_escape").decode())
PY
```

3. 只有确认文件本身含 mojibake 时才改文档内容。

## 10. 完整验收命令

返工完成后，必须从干净工作区运行：

```powershell
Set-Location X:\tianhks
git status --short --branch
npm run frontend:build
git diff --check 75f9afef..HEAD
$env:CARGO_TARGET_DIR='C:\tmp\xingshu-target-bugfix'; cargo check --all-targets
$env:CARGO_TARGET_DIR='C:\tmp\xingshu-target-bugfix'; cargo test --test api_tests
$env:CARGO_TARGET_DIR='C:\tmp\xingshu-target-bugfix'; cargo test --test db_tests
$env:CARGO_TARGET_DIR='C:\tmp\xingshu-target-bugfix'; cargo test --test cli_tests -- --nocapture
```

如果本机已启动 dev 服务，再跑：

```powershell
Set-Location X:\tianhks
node X:\tianhks\scripts\verify-vue-parity.mjs
node X:\tianhks\scripts\verify-vue-process-lifecycle.mjs
powershell -ExecutionPolicy Bypass -File X:\tianhks\scripts\verify-load-and-rbac.ps1
```

验收报告必须说明：

- 当前 commit hash。
- 每条命令是否通过。
- 若脚本跳过，必须说明跳过原因，不允许写“默认通过”。
- 不允许把 500 当作 RBAC 拒绝成功。
- 不允许把缺失文案的截图验证标成 `ok=true`。

## 11. 最小可接受交付

如果时间有限，至少完成这 4 件：

1. 修 `X:\tianhks\frontend\src\views\AiView.vue` 的推荐字段读取。
2. 修 `X:\tianhks\frontend\src\views\HistoryView.vue` 的批次详情结构读取。
3. 修 `X:\tianhks\scripts\verify-load-and-rbac.ps1`，让 500 必定失败。
4. 修 `X:\tianhks\scripts\verify-vue-parity.mjs`，让缺文案/打开失败必定失败。

这 4 件完成前，不要再写“工业级完成”“READY”“完整 parity”。

## 12. 不要碰的事项

除非用户另行要求，本轮不要处理：

- `X:\tianhks\output\` 下未跟踪截图和报告的清理。
- `X:\tianhks\static\index.html` 的大规模重写。
- `X:\tianhks\src\api.rs` 上帝模块拆分。
- `round2` 重复清理。
- 手写 XLSX 替换。
- Modbus TCP server 改成第三方库。

这些是后续工程债，不应和本次“修正小模型错误交付”混在一个提交里。
