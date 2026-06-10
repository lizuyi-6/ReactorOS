# 当前对话导出

导出时间：2026-06-10 Asia/Shanghai

线程 ID：`019ea707-45c5-7880-8e10-acdd840dbd13`

线程标题：选择最佳更新方式

工作区：`X:\tianhks`

说明：本文件由 Codex 线程读取工具和当前工作树证据生成。线程读取工具当前每页只返回 1 个 turn，且部分早期分页为压缩/空记录；因此早期 OTA 讨论按当前可见上下文与已生成文档摘要整理，后续代码加固部分按可读取到的线程消息和工作树证据整理。

## 原始问题脉络

用户最初围绕工业设备升级维护提出问题：

- “这个项目为了工业更新方便，哪种方式最好”
- “OTA 不行吗”
- “有没有 OTA 失败方案”
- “现在是怎么做的”
- “你知不知道安卓以前提出过一种 A/B 更新的方式我不太清楚原理，不过好像是两套固件轮次更新”

随后目标扩展为：现在在软件上做改动时，必须按工业现场异常场景默认设计，避免不确定状态下继续生产控制。

## OTA / A-B 更新讨论摘要

侧聊补充中已经形成文档：

- `X:\tianhks\docs\ab_ota_hardware_discussion.md`

该文档给硬件伙伴解释：

- A/B OTA 不需要两套主控。
- 需要可靠存储、可恢复启动链、断电保护、watchdog、recovery/UART/USB/以太网救援入口、FRAM/LED 状态记录。
- 更新不能覆盖当前可用版本。
- 新版本只有健康检查成功后才提交。
- 失败后自动回滚。
- recovery/fallback/golden image 只负责救援，不负责继续生产控制。

推荐路线：

1. 先做应用级 A/B OTA。
2. 后续再考虑 rootfs 人工镜像升级或传统 rootfs A/B。
3. 批量设备成熟后再评估 RAUC / Mender / SWUpdate / OSTree / 类 Virtual A/B。

## 工业异常场景设计原则

本线程后续软件改动统一采用这些约束：

- 不确定就失败闭锁。
- 风险增加动作不能基于旧状态提交。
- 风险降低动作，例如停机、急停、上锁、关闭自动控制，应尽量保持可用。
- 设备已经停机，不等于生产记录可以被错误关闭。
- 审计成功，不等于现场状态仍然是审计前确认过的状态。
- recovery/fallback 只救援，不继续生产控制。

## 已完成的软件加固记录

### 1. 停机后 active batch 被异步改写

问题：

`process stop` 或 `finish batch` 在写入停机目标后，如果 `runtime.active_batch_id` 被另一个现场动作、恢复线程、误操作或看门狗逻辑改成别的批次，旧请求不能继续关闭原批次生产记录。

改动：

- `src/api.rs`
  - `process stop` 写入停机目标后，在 DB finish/audit 前重新核对 `runtime.active_batch_id`。
  - `finish batch` 同样在设备停机后、DB finish/audit 前重查 active batch。
  - 若批次身份不一致，返回 `409`，关闭自动控制，保持停机目标，记录控制故障，不关闭旧批次。
- `tests/api_tests.rs`
  - 新增 `ChangeActiveBatchOnWriteDevice`，确定性模拟 `write_targets` 成功时 active batch 被改写。
  - 新增 `process_stop_rejects_active_batch_change_after_stop_before_finish`。
  - 新增 `finish_batch_rejects_active_batch_change_after_stop_before_finish`。

验证：

- `cargo test --test api_tests process_stop -- --nocapture`
- `cargo test --test api_tests finish_batch -- --nocapture`
- `cargo fmt --check`
- `git diff --check -- src/api.rs tests/api_tests.rs`
- `cargo check`

### 2. 手动锁解锁不能覆盖新的重新上锁

问题：

手动锁解锁请求在写入审计后，如果另一个现场动作又把手动锁打开，旧解锁请求不能把这个新锁清掉。

改动：

- `src/state.rs`
  - 新增 `manual_lock_generation`。
  - 新增 `engage_manual_lock()` / `clear_manual_lock()`。
- `src/api.rs`
  - 解锁前记录确认过的 `manual_lock_generation`。
  - 审计后若 generation 变化，返回 `409`，保持 `manual_lock=true`，强制 `auto_enabled=false`。
- `tests/api_tests.rs`
  - 新增 `manual_lock_unlock_rejects_lock_generation_change_after_audit`。

验证：

- `cargo test --test api_tests manual_lock_unlock_rejects_lock_generation_change_after_audit -- --nocapture`
- `cargo test --test api_tests manual_lock -- --nocapture`
- `cargo test --test api_tests risk_ -- --nocapture`
- `cargo fmt --check`
- `git diff --check -- src/api.rs src/state.rs tests/api_tests.rs`
- `cargo check`

### 3. 急停复位不能覆盖新的急停

问题：

急停复位请求审计后，如果现场又触发一次新的急停，旧复位请求不能清掉新急停。单看 `emergency_stop == true` 无法区分新旧急停。

改动：

- `src/state.rs`
  - 新增 `emergency_stop_generation`。
  - 新增 `engage_emergency_stop()` / `clear_emergency_stop()`。
  - generation 字段使用 `#[serde(default, skip_serializing)]`，避免暴露到 runtime JSON。
- `src/api.rs`
  - 触发急停时推进 generation。
  - 急停复位审计前记录 generation。
  - 审计后若 generation 变化，返回 `409`，保持 `emergency_stop=true`，关闭自动控制。
- `tests/api_tests.rs`
  - 新增 `emergency_reset_rejects_stop_generation_change_after_audit`。

验证：

- `cargo test --test api_tests emergency_reset_rejects_stop_generation_change_after_audit -- --nocapture`
- `cargo test --test api_tests emergency_reset -- --nocapture`
- `cargo test --test api_tests manual_lock -- --nocapture`
- `cargo test --test api_tests risk_ -- --nocapture`
- `cargo fmt --check`
- `git diff --check -- src/api.rs src/state.rs tests/api_tests.rs`
- `cargo check`

### 4. 控制故障复位不能清掉审计后新发生的同名故障

问题：

控制故障复位原来只比对错误文本。如果同一个 `write timeout` 在复位审计后再次发生，旧复位请求可能把新故障清掉。

改动：

- `src/state.rs`
  - 新增 `control_fault_generation`。
  - `latch_control_fault()` 每次调用都推进 generation，即使故障文本相同。
  - 新增 `clear_control_fault()`。
- `src/api.rs`
  - 设备写入失败统一走 `latch_control_fault()`。
  - 组件控制审计失败统一走 `latch_control_fault()`。
  - 控制故障复位审计前记录故障文本和 generation。
  - 审计后文本或 generation 任一变化，返回 `409`。
- `tests/api_tests.rs`
  - 新增 `control_fault_reset_rechecks_fault_generation_after_audit`。

验证：

- `cargo test --test api_tests control_fault_reset_rechecks_fault_generation_after_audit -- --nocapture`
- `cargo test --test api_tests control_fault_reset -- --nocapture`
- `cargo test --test api_tests manual_lock -- --nocapture`
- `cargo test --test api_tests emergency_reset -- --nocapture`
- `cargo test --test api_tests risk_ -- --nocapture`
- `cargo fmt --check`
- `git diff --check -- src/api.rs src/state.rs tests/api_tests.rs`
- `cargo check`

### 5. 自动启用不能忽略审计窗口内短暂出现过的安全状态变化

问题：

`auto_enabled=true` 会让控制回路开始写设备。即使最终布尔状态看起来安全，如果审计窗口内出现过急停、手动锁或控制故障，又被清掉，旧自动启用请求也不应继续提交。

改动：

- `src/api.rs`
  - 新增 `SafetyLatchGenerations` 快照结构。
  - 自动启用审计前记录 `manual_lock_generation`、`emergency_stop_generation`、`control_fault_generation`。
  - 审计后最终提交前，若任一 generation 变化，返回 `409`，保持 `auto_enabled=false`。
- `tests/api_tests.rs`
  - 新增 `auto_enable_rejects_safety_generation_change_after_audit`。
  - 测试中审计钩子 latch 一个控制故障再清掉，使最终布尔状态看似安全，但 generation 已变化。

验证：

- `cargo test --test api_tests auto_enable_rejects_safety_generation_change_after_audit -- --nocapture`
- `cargo test --test api_tests auto_enable -- --nocapture`
- `cargo test --test api_tests risk_ -- --nocapture`
- `cargo test --test api_tests manual_lock -- --nocapture`
- `cargo test --test api_tests emergency_reset -- --nocapture`
- `cargo test --test api_tests control_fault_reset -- --nocapture`
- `cargo fmt --check`
- `git diff --check -- src/api.rs src/state.rs tests/api_tests.rs`
- `cargo check`

### 6. 风险增加目标提交防瞬态安全闩

问题：

第 5 项加固给 `set_auto` 加了 `SafetyLatchGenerations` 快照,在审计窗口后拒绝瞬态安全闩(急停/手动锁/控制故障"短暂出现又被清除")。但风险增加的**目标提交路径**——`set_targets`、`execute_component_control`、`start_process_lifecycle`、`start_batch`、v1 control、v1 process load、AINAS 远程目标下发和 Modbus 调试写入——仍然只在最终联锁中重新比对布尔状态。如果审计窗口内安全闩瞬态触发又被清除(布尔回到 false 但 generation 已自增),最终提交仍会通过。工艺/批次/v1 启动尤其严重:它们在审计后直接提交 `auto_enabled=true`,等于绕过了已加固的 `set_auto`。

改动：

- `src/api.rs`
  - 新增 `ensure_safety_latches_unchanged_for_commit()` 共享辅助函数:generation 变化则设 `auto_enabled=false` 并返回 409。
  - `SafetyLatchGenerations` 增加 `#[derive(Clone, Copy)]`,提升为 `pub(crate)`,`from_runtime` 提升为 `pub(crate)`,供跨模块构造。
  - `commit_process_activation_after_final_interlock`、`commit_targets_after_final_interlock`、`commit_component_targets_after_final_interlock` 三个函数各新增 `acknowledged_safety_latches: Option<SafetyLatchGenerations>` 参数;在布尔联锁重查之后、写 runtime 之前,调用 `ensure_safety_latches_unchanged_for_commit`。
  - 7 处 api.rs 调用点在确认联锁清空时捕获 `SafetyLatchGenerations::from_runtime` 快照,传入对应 commit。
- `src/api_integrations.rs`
  - AINAS `apply_ainas_targets` 在确认联锁时捕获快照,传入 `commit_targets_after_final_interlock`。
- `src/modbus_registers.rs`
  - `apply_modbus_register_write` 在 commit 调用处构造快照(确认后的 runtime 克隆仍在作用域),传入 commit。
- `tests/api_tests.rs`
  - 新增 `set_targets_rejects_safety_generation_change_after_audit`:审计钩子 latch+clear 控制故障(推进 generation),断言 409 + 目标未提交 + 审计已记录。
  - 新增 `batch_start_rejects_safety_generation_change_after_audit`:同样 latch+clear,断言 409 + active_batch=None + 设备回滚写(2 次)+ 未完成批次为空。

验证：

- `cargo test --test api_tests rejects_safety_generation_change_after_audit` (3 个 generation 测试全通过)
- `cargo test --test api_tests` (203 passed / 3 pre-existing failures / 0 regression)
- 19 个聚焦回归测试(走我 commit 路径 + 审计钩子的既有测试)全通过
- `cargo fmt --check` (clean)
- `cargo check --tests` (clean)

注意：完整运行有 3 个在途未提交测试失败(400 vs 409 预期、active_batch Some vs None 预期),全部是未提交工作里的既存问题(上游客户端 payload 缺显式控制字段、回滚 stop 写失败的保守保留 active_batch),与本次改动无关。

## 当前工作树相关文件

本线程主要涉及：

- `X:\tianhks\src\api.rs`
- `X:\tianhks\src\state.rs`
- `X:\tianhks\tests\api_tests.rs`
- `X:\tianhks\docs\ab_ota_hardware_discussion.md`
- `X:\tianhks\docs\current_thread_export.md`

注意：工作树本身存在大量已有未提交修改和未跟踪文件。本导出只记录本线程可读取到的主要对话与本轮相关工程事实，不代表完整 git diff 审计。

## 当前状态

线程目标仍未标记完成。原因是用户目标是持续性工程约束：“软件改动一定要考虑工业奇怪场景”，该目标不能仅凭某一轮补丁证明全项目已经完全覆盖。

当前已经形成的默认工程约束：

- 更新不覆盖当前可用版本。
- 新版本健康检查成功才提交。
- 失败自动回滚。
- fallback/recovery 只救援，不继续生产控制。
- 风险增加动作必须基于审计后仍一致的现场状态提交。
- 风险降低动作保持可用，但不能误写成功生产记录。
- 人工复位类动作必须证明处理的是同一个故障/锁/急停实例，而不是审计后新发生的实例。

