# 星宿智能反应釜上位机 API 验收手册

日期：2026-06-04

对象：李祖祎负责的上位机 REST API、WebSocket、AINAS 任务接口和本地验收入口。

边界说明：本文档记录当前上位机 API 交付面和验收方式。REST/CLI/AINAS 已具备本地验收基础；MQTT、Modbus TCP、AINAS 真实平台、STM32 RTU 和第三方系统仍需按 `docs/upper_computer_external_acceptance_checklist.md` 补外部证据。

## 1. 通用约定

| 项目 | 说明 |
| --- | --- |
| Base URL | `http://127.0.0.1:8000`，生产可改为 HTTPS |
| 健康检查 | `GET /health` |
| 数据格式 | JSON；导出类接口返回 CSV/XLSX/Markdown |
| 认证 | 写入、审计、权限、集成、控制类接口以及 v1 实时数据/实时 WebSocket 使用 bearer session |
| 登录入口 | `POST /api/auth/login` |
| Token 使用 | `Authorization: Bearer <token>` |
| 默认角色 | `operator`、`engineer`、`admin` |
| 测试入口 | `/api/test/*` 仅在本机 loopback 监听、`--enable-test-reset` 启用且请求带 `X-Xingshu-Test-Confirm: local-e2e` 时可用，生产禁止开启 |

默认本地登录账号用于开发和验收演示，生产必须通过环境变量替换密码：

| 角色 | 默认用户名 | 默认密码环境变量 | 默认密码 |
| --- | --- | --- | --- |
| operator | `operator` | `XINGSHU_OPERATOR_PASSWORD` | `operator123` |
| engineer | `engineer` | `XINGSHU_ENGINEER_PASSWORD` | `engineer123` |
| admin | `admin` | `XINGSHU_ADMIN_PASSWORD` | `admin123` |

## 2. 认证和权限

| 方法 | 路径 | 用途 | 当前状态 |
| --- | --- | --- | --- |
| `POST` | `/api/auth/login` | 用户名/密码登录并返回 bearer token | 本地通过 |
| `GET` | `/api/auth/me` | 查看当前 token 对应角色和权限 | 本地通过 |
| `GET` | `/api/permissions/roles` | 查看角色允许和阻断的权限 | 本地通过 |

权限覆盖：

| 角色 | 主要权限 |
| --- | --- |
| operator | 监控、历史、导出、启停流程、安全目标、AI 建议、急停 |
| engineer | operator 权限 + 审计、工艺编辑、Modbus 调试、外部样本写入 |
| admin | 全部权限 |

## 3. 监控和设备状态

| 方法 | 路径 | 用途 | 当前状态 |
| --- | --- | --- | --- |
| `GET` | `/api/live` | HMI 实时聚合数据 | 本地通过；无新鲜样本时 503 是预期安全行为；生产严格模式缺少下位机状态时返回传感器值但 `device_status.online_count=0/status=offline` 并给出高危报警 |
| `GET` | `/api/devices/status` | 当前设备在线数量和状态 | 本地通过 |
| `GET` | `/api/v1/devices/status` | 文档版设备状态接口 | 本地通过 |
| `GET` | `/api/devices/capabilities` | 设备能力摘要 | 本地通过 |
| `GET` | `/api/v1/devices/capabilities` | 文档版设备能力接口 | 本地通过 |
| `POST` | `/api/devices/:device_id/components/:component_id/control` | 单组件控制入口 | 本地通过；启动、升速、设定值等危险动作走 fail-closed 门槛，`stop`/`off` 保持可用 |
| `POST` | `/api/v1/reactor/:device_id/samples` | 外部管线上行传感器样本，需 `Authorization: Bearer <token>` 且具备 `ingest_sensor_sample` 权限 | 本地通过；`xingshu data sample` 复用该入口 |
| `GET` | `/api/v1/reactor/:device_id/realtime` | 文档版实时数据，需 `Authorization: Bearer <token>` 且具备监控权限 | 本地通过；生产严格模式缺少下位机状态时 `status=offline/device_online=false/data.phase=offline` 并给出高危报警 |
| `GET` | `/api/v1/reactor/:device_id/history` | 文档版历史数据 | 本地通过 |
| `WS` | `/ws/v1/reactor/:device_id/realtime` | 文档版实时 WebSocket，需 `Authorization: Bearer <token>` 且具备监控权限 | 本地具备；与 v1 realtime 使用同一严格模式设备状态和样本新鲜度语义，样本缺失/过期时发送错误信封并断开，不继续推送伪实时帧；需浏览器/第三方验收 |

## 4. 控制和工艺流程

| 方法 | 路径 | 用途 | 当前状态 |
| --- | --- | --- | --- |
| `POST` | `/api/control/targets` | 更新目标温度、转速、摇罐、压力等 | 本地通过 |
| `POST` | `/api/control/auto` | 开关自动控制 | 本地通过 |
| `POST` | `/api/control/manual-lock` | 开关人工锁定；打开时关闭自动控制，解除需新鲜现场样本、下位机状态健康、无急停/控制故障/命令失败，且不自动恢复自动控制 | 本地通过 |
| `POST` | `/api/control/fault/reset` | 现场确认后清除锁存的设备控制写入故障；不会开启自动控制 | 本地通过 |
| `POST` | `/api/control/emergency-stop` | 急停 | 本地通过 |
| `POST` | `/api/control/emergency-stop/reset` | 无活动批次、有新鲜现场样本、下位机状态健康且未报告命令失败后复位急停；不会清除设备控制写入故障，也不会开启自动控制 | 本地通过 |
| `POST` | `/api/v1/reactor/:device_id/control` | 文档版控制接口 | 本地通过 |
| `POST` | `/api/ai/control` | AI 控制 dry-run / 执行入口 | 本地具备；真实执行仍需人工确认和硬件联调 |
| `POST` | `/api/v1/ai/control` | 文档版 AI 控制入口 | 本地具备 |
| `GET` | `/api/processes` | 列出工艺流程 | 本地通过 |
| `POST` | `/api/processes` | 创建工艺流程 | 本地通过 |
| `GET` | `/api/processes/:id` | 查看流程详情 | 本地通过 |
| `PUT` | `/api/processes/:id` | 更新流程 | 本地通过 |
| `POST` | `/api/processes/:id/steps` | 新增步骤 | 本地通过 |
| `PUT` | `/api/processes/:id/steps/:step_id` | 更新步骤 | 本地通过 |
| `POST` | `/api/processes/:id/apply` | 兼容入口，内部走同一安全路径 | 本地通过 |
| `POST` | `/api/processes/:id/start` | 启动流程，创建活动批次 | 本地通过；真实硬件闭环待验收 |
| `POST` | `/api/processes/:id/stop` | 停止指定流程的活动批次；先写设备停止目标，清活动批次、关闭自动控制并同步 `runtime.targets`；若活动批次记录缺失则拒绝，避免错停 | 本地通过 |
| `POST` | `/api/processes/current/stop` | 停止当前活动流程；先写设备停止目标，清活动批次、关闭自动控制并同步 `runtime.targets`；若 runtime 仍 active 但批次记录缺失，会执行救援停止并返回 `batch: null`、`recovery` | 本地通过 |

控制类写入统一要求：

- bearer token 具备对应权限。
- 未处于急停或人工锁禁止状态，且上一次设备控制写入没有未清除故障。
- 传感器数据新鲜度满足 `sensor_timeout_ms`；没有新鲜样本时，系统默认无法证明现场安全并拒绝目标变更。
- 正式外部样本入口只有在样本成功写入数据库后才更新运行态 `latest_sample`，只作为“传感器样本新鲜且可追溯”的证明来源；如果样本落库失败，系统清除可用样本、关闭自动控制并记录现场输入故障。只有 engineer/admin 或具备 `ingest_sensor_sample` 权限的第三方数据源 token 可写入，operator 和匿名请求会被拒绝。
- `config/safety.toml` 默认启用 `require_device_status_for_control = true`；目标写入、自动控制、人工锁解除、控制故障复归和急停复位还必须有下位机状态，且 `connected`、`last_frame_ok` 和 `last_seen_at` 健康，`last_command_ok` 未报告失败。状态缺失、断连、帧校验失败或状态过期时拒绝升风险动作并保持 fail-closed；`/api/devices/status` 不会仅凭新鲜样本把设备显示为 `online=true/status=idle`，`/api/live` 和 v1 realtime 也会把未证明的下位机状态显示为 offline 并产生 `device_status_unavailable` 高危报警。只有实验室纯 Pipeline 演示才应显式关闭该开关。
- 目标在范围、步长和 `forbidden_control_zones` 内。
- 写入审计链。
- 纯目标意图入口（手动 targets、v1 非启动控制、v1 process 载入、AI 目标调整、AINAS/MQTT `set_targets`、Modbus 调试目标写入）必须先完成安全校验和审计写入，审计失败时不提交 `runtime.targets`；AI 目标调整在审计失败时也不会下发设备写入。
- 纯目标意图入口在审计成功后、提交 `runtime.targets` 前还会重新检查急停、人工锁、控制故障、现场样本和下位机状态；最终互锁失败时不提交新目标。若 AI 目标调整已经写入设备但最终互锁失败，会锁存 `last_control_error` 并要求维护复归。
- `POST /api/control/targets`、`POST /api/v1/reactor/:device_id/control`、`POST /api/v1/reactor/:device_id/process`、自动控制开启、批次/流程启动、AI 执行目标、AINAS/MQTT `set_targets`、Modbus 目标写入和组件危险动作共用上述 fail-closed 门槛；关闭自动控制、停止工艺、急停、组件 `stop`/`off` 入口保持可用。
- 当前上位机运行态只对应 `reactor_001`；所有 `/api/v1/reactor/:device_id/*` 路径必须匹配该设备 ID，错误 ID 返回 `404`，不能把外部样本、目标或历史回显混入当前反应釜状态。
- 批次/流程启动和 `auto_start=true` 的文档版控制入口必须先写入设备目标；软件 `active_batch_id/auto_enabled` 只在启动审计、必要数据库状态提交和最终互锁都通过后提交给 `runtime`。若设备写入、最终互锁、审计或数据库标记失败，软件运行态会回滚，并尽力向设备写入停止/降风险目标（温度安全下限、各计时/摇床为 0、搅拌安全最小值、压力目标为 0）。启动前设备写入失败会记录 `process_start_failed` 审计事件但不会激活运行态；设备已启动后若最终互锁、审计或状态提交失败，会锁存 `last_control_error`；回滚后的 `runtime.targets` 也同步为停止目标，避免下次开启自动控制时沿用失败启动的高目标。
- 组件控制若设备动作已经成功但审计写入失败，或审计成功后、提交 `runtime.targets` 前最终互锁失败，不提交新的 `runtime.targets`，同时锁存 `last_control_error` 并关闭 `auto_enabled`；该状态必须按控制故障处理，不能继续生产控制。
- 设备控制写入失败，或设备动作成功后的审计/数据库状态提交失败，会锁存 `last_control_error` 并关闭 `auto_enabled`；这包括后台自动控制循环的 `device_write` 审计失败、启动后流程状态提交失败、停止/结束批次后的完成标记或审计失败。后续传感器样本、急停复位、人工锁切换都不会自动清除该故障。现场确认执行器链路恢复后，使用 `POST /api/control/fault/reset` 显式清除，自动控制仍保持关闭。
- 后台自动控制对同一条目标写入只在一个 `sensor_timeout_ms` 新鲜度窗口内去重；超过窗口后会重新经过最终互锁并重申同一目标，避免下位机重启、串口重连或执行器掉电后软件长期假定上一条易失写入仍有效。
- AI master control 若真实执行了目标调整、流程启停或组件动作，但最终 `ai_master_decision` 审计写入失败，也按设备动作后审计失败处理：即使只执行了目标调整，也会因为目标已写入设备而锁存 `last_control_error`、关闭自动控制，并要求维护复归。
- 下位机状态报告 `last_command_ok=false` 时按控制故障处理；复归接口会拒绝仍在报告失败的状态，`/api/devices/status` 和 `/api/live.device_status` 会把设备显示为 `online=false/status=error`，组件状态显示为 `error`，Modbus `device_connected` 也不计入健康连接。
- 传感器或下位机状态链路异常会关闭 `auto_enabled` 并记录 `field_input_fault_auto_disabled`；样本恢复必须先成功落库、更新 `latest_sample`，再清除旧 `last_sensor_error` 并按新样本重新计算报警，不会继承旧现场输入错误误触发 hard alarm，也不会自动重新开启自动控制。若运行态出现 `last_control_error` 已锁存但 `auto_enabled=true` 的不一致状态，样本摄入/后台控制循环会强制关闭自动控制并记录 `control_fault_auto_disabled`。
- 新样本触发 `sensor_limits` 的 hard limit 高报警时，会关闭 `auto_enabled` 并记录 `high_sensor_alarm_auto_disabled`；normal range warning 只报警，不自动停用控制。
- 解除阻断/升风险操作必须先证明现场安全、写入审计再提交运行态：开启自动控制、解除人工锁、清除控制故障和急停复位在审计失败时不生效；解除人工锁、清除控制故障和急停复位还会拒绝未完成批次恢复状态，不能在数据库/运行态不一致时把现场显示成可继续操作。清除控制故障和急停复位在审计成功后还会重新检查现场样本、下位机状态、当前锁存状态和未完成批次恢复状态，审计期间出现的新控制故障、下位机命令失败、状态异常或 DB/runtime 批次不一致不会被清除。其中解除人工锁还要求新鲜现场样本、下位机状态健康、无急停、无未清控制故障且 `last_command_ok` 未报告失败。关闭自动控制、打开人工锁、触发急停、停止流程和结束批次属于降风险操作，即使审计链异常也保持保守状态，不继续生产控制。
- 急停触发属于最高优先级降风险动作：即使审计链异常，`emergency_stop=true` 和 `auto_enabled=false` 也会立即生效；若急停审计写入失败，会额外锁存 `last_control_error`，要求维护确认审计链后再复归。
- 停止流程或结束当前活动批次时会先向设备写入停止目标；设备停止成功后立即关闭自动控制并同步停止目标，但 `active_batch_id` 保留到数据库完成标记和停止审计都成功后才清除，避免停止收尾尚未可追溯时新启动插队。如果设备停止写入失败，批次不会被标记完成，只锁存控制故障并关闭自动控制。如果设备已经停下但数据库完成标记或审计写入失败，也会保持停止态并锁存控制故障，避免硬件已变更但软件账/审计账缺失时继续生产控制。当前存在其他活动批次时，禁止结束非活动批次，避免错 ID 操作影响正在运行的生产态。
- 人工锁打开会立即关闭自动控制；人工锁解除只解除锁，不会恢复此前的自动控制状态。若现场样本缺失/过期、下位机断连/状态过期/帧校验失败、急停未复位、`last_control_error` 未清除、`last_command_ok=false` 仍在报告，或数据库仍有需要恢复处理的未完成批次，解除请求会被拒绝，必须由维护人员先排除原因，再由操作员重新执行开启自动控制的 SOP。
- daemon 启动、断电恢复、systemd 自动重启和 OTA 切槽后，运行态一律以 `auto_enabled=false` 初始化；即使配置文件里保留 `auto_enabled_default=true`，也不能跳过现场证明、审计和操作员重新开启自动控制。

## 5. 数据、批次和审计

| 方法 | 路径 | 用途 | 当前状态 |
| --- | --- | --- | --- |
| `GET` | `/api/batches` | 列出批次 | 本地通过 |
| `POST` | `/api/batches/start` | 启动批次并写入目标 | 本地通过 |
| `GET` | `/api/batches/:id` | 批次详情 | 本地通过 |
| `POST` | `/api/batches/:id/finish` | 结束批次；若为当前活动批次则先写设备停止目标并关闭自动控制；若请求 ID 正是 runtime 当前活动批次但批次记录缺失，会执行救援停止并记录 `batch_finish_recovery_missing_batch` 审计 | 本地通过 |
| `GET` | `/api/batches/export.csv` | 导出批次 CSV | 本地通过 |
| `GET` | `/api/batches/export.xlsx` | 导出 Excel 工作簿 | 本地通过 |
| `GET` | `/api/batches/:id/report.md` | 单批次 Markdown 报告 | 本地通过 |
| `POST` | `/api/product-results` | 录入产率和产物比例 | 本地通过 |
| `GET` | `/api/audit/logs` | 查看审计链事件 | 本地通过 |
| `GET` | `/api/audit/export.csv` | 导出审计 CSV | 本地通过 |

结束批次只允许作用于存在且尚未完成的批次；不存在或已完成批次会被拒绝，不生成新的 `batch_finished` 审计。唯一例外是请求 ID 正好等于运行态 `active_batch_id`、但数据库批次记录已经缺失的恢复场景：系统会先向设备写入停止目标，关闭自动控制并同步停止目标，写入 `batch_finish_recovery_missing_batch` 审计成功后才清除运行态活动批次；若该救援审计失败，则保持 `active_batch_id` 供维护修复后重试。当前存在其他活动批次时，只能结束该活动批次，避免错 ID 操作影响生产账。

产品结果只能录入到已完成且不是当前活动态的批次；未完成批次、活动批次或不存在的批次会被拒绝，避免把仍在生产中的数据写入 AI 推荐依据。

## 6. AI 和本地模型边界

| 方法 | 路径 | 用途 | 当前状态 |
| --- | --- | --- | --- |
| `GET` | `/api/recommendations/latest` | 读取最新已缓存 AI/优化器建议，不触发模型调用或写库 | 本地通过 |
| `POST` | `/api/recommendations/latest` | 生成并持久化最新 AI/优化器建议 | 本地通过 |
| `GET` | `/api/ai/experiment-plan` | 只读实验 SOP 草案 | 本地通过 |
| `GET` | `/api/v1/ai/experiment-plan` | 文档版只读实验 SOP 草案 | 本地通过 |
| `GET` | `/api/config/summary` | 返回 `local_ai.ready_for_base_inference`、`ready_for_lora_inference`、`ready_for_training` 和 `ready_for_prd_lora` | 本地通过 |

当前 AI 边界：

- `local-ga-sa-pid` 本地优化器已作为传统算法建议路径。
- SOP 草案是只读安全门控输出，不启动工艺、不写目标。
- `local_ai.ready_for_base_inference` 只证明基础 GGUF/HTTP 推理入口可用。
- `local_ai.ready_for_lora_inference` 才证明 LoRA adapter 也存在；兼容字段 `ready_for_inference` 与它保持同义。
- `local_ai.ready_for_prd_lora` 还要求训练边界和 RK 延迟报告同时存在，才可作为 PRD LoRA/RK 侧证据。
- 真实 Qwen3.5-2B + LoRA 推理、训练、自进化、GGUF 转换和 RK 延迟仍未完成。

## 7. AINAS 和第三方任务

| 方法 | 路径 | 用途 | 当前状态 |
| --- | --- | --- | --- |
| `POST` | `/api/integrations/ainas/tasks` | 创建 AINAS/第三方任务 | 本地通过 |
| `GET` | `/api/integrations/ainas/tasks` | 列出任务 | 本地通过 |
| `GET` | `/api/integrations/ainas/tasks/:id` | 查询任务详情和回执 | 本地通过 |
| `POST` | `/api/v1/integrations/ainas/tasks` | 文档版创建任务 | 本地通过 |
| `GET` | `/api/v1/integrations/ainas/tasks` | 文档版任务列表 | 本地通过 |
| `GET` | `/api/v1/integrations/ainas/tasks/:id` | 文档版任务详情 | 本地通过 |

支持动作：

| action | 说明 | 安全要求 |
| --- | --- | --- |
| `set_targets` | 设置控制目标 | 经过 RBAC、新鲜样本、急停/人工锁/控制故障互锁、安全限幅、禁区和审计 |
| `start_process` | 启动工艺流程 | 经过流程启动安全门 |
| `stop_process` | 停止工艺流程 | 写停止审计 |

`start_process` / `stop_process` 若设备动作已经执行但 `integration_tasks` 回执状态更新失败，会锁存 `last_control_error` 并关闭 `auto_enabled`，按“第三方回执缺失、现场状态不可完全追溯”处理；`set_targets` 按纯目标意图审计优先处理，审计失败不提交目标。MQTT 任务若已经 `executed` 但向 broker 发布 `task_receipts` 失败，也会按外部回执缺失闭锁：`set_targets` 锁存目标意图回执故障，`start_process` / `stop_process` 按设备动作后回执缺失处理。

MQTT retained alert 快照与 `/api/live` 使用同一报警生成逻辑。传感器样本缺失、样本过期或现场输入错误会生成 `sensor_data_unavailable` 高危报警；`sensor_fresh=false` 只是摘要字段，第三方系统不能只看该布尔值而忽略报警数组。

启用 `XINGSHU_DB_ENCRYPTION_KEY` 后，`integration_tasks.request_json` 与 `integration_tasks.response_json` 会以 AES-256-GCM 信封写入 SQLite。

## 8. Modbus 调试 API

| 方法 | 路径 | 用途 | 当前状态 |
| --- | --- | --- | --- |
| `GET` | `/api/modbus/registers` | 返回读/写寄存器、coils、discrete inputs、TCP 状态 | 本地通过 |
| `GET` | `/api/modbus/registers/:name/read` | 读取一个映射点位 | 本地通过 |
| `POST` | `/api/modbus/registers/:name/write` | 写一个可写点位 | 本地通过；HTTP 调试写入口仅允许 admin bearer session，且请求体必须提供非空 `reason`，写入走安全链路和审计 |

寄存器详细表见 `docs/upper_computer_modbus_register_map.md`。外部 Modbus Poll/Slave 验收仍需补齐。

## 9. 配置摘要和安全状态

| 方法 | 路径 | 用途 | 当前状态 |
| --- | --- | --- | --- |
| `GET` | `/api/config/summary` | 设备、安全、AI、权限、集成和加密状态摘要 | 本地通过 |

验收重点字段：

- `data_security.storage_encryption.enabled`
- `local_ai.ready_for_base_inference`
- `local_ai.ready_for_lora_inference`
- `local_ai.ready_for_inference`
- `local_ai.ready_for_training`
- `local_ai.ready_for_prd_lora`
- `integrations.mqtt_status`
- `integrations.modbus_tcp_status`
- `permissions.authentication`
- `safety.forbidden_control_zones`

## 10. 本地测试专用 API

| 方法 | 路径 | 用途 | 生产要求 |
| --- | --- | --- | --- |
| `POST` | `/api/test/reset` | 重置本地验收数据 | 生产禁用 |
| `POST` | `/api/test/pipeline-sample` | 写入测试样本 | 生产禁用 |

这些接口只有 daemon 启动参数包含 `--enable-test-reset`、HTTP 监听地址是 loopback（如 `127.0.0.1` 或 `::1`）、且请求带 `X-Xingshu-Test-Confirm: local-e2e` 时可用。若测试模式绑定到 `0.0.0.0` 或其他非本机地址，daemon 会拒绝启动。`/api/test/reset` 还会拒绝活动批次、数据库未完成批次、自动控制已启用、急停中或控制故障未清除的运行态；它只用于本地验收清理，不是生产恢复路径。

## 11. API 验收步骤

本地基础验收：

```powershell
Invoke-RestMethod http://127.0.0.1:8000/health
Invoke-RestMethod http://127.0.0.1:8000/api/config/summary
Invoke-RestMethod http://127.0.0.1:8000/api/devices/status
```

登录：

```powershell
$login = Invoke-RestMethod -Method Post `
  -Uri http://127.0.0.1:8000/api/auth/login `
  -ContentType application/json `
  -Body '{"username":"engineer","password":"engineer123"}'
$headers = @{ Authorization = "Bearer $($login.data.token)" }
Invoke-RestMethod -Headers $headers http://127.0.0.1:8000/api/auth/me
```

只读接口验收：

```powershell
Invoke-RestMethod http://127.0.0.1:8000/api/modbus/registers
Invoke-RestMethod http://127.0.0.1:8000/api/ai/experiment-plan
Invoke-RestMethod http://127.0.0.1:8000/api/audit/logs -Headers $headers
```

外部验收必须补充：

- Postman 或第三方系统调用记录。
- AINAS 真实平台任务记录。
- MQTT.fx/mosquitto broker 任务、回执、报警、断线重连记录。
- Modbus Poll/Slave 读写和 TLS 证书链记录。
- STM32/RS485 实机闭环和异常工况记录。

## 12. 当前结论

REST API、Web HMI 和 CLI 共用同一安全链路，本地 API 验收已具备基础。正式 PRD 交付仍需要第三方工具、真实平台、真实硬件、生产证书链和用户验收签字作为外部证据。
