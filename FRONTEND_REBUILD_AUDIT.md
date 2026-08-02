# FRONTEND_REBUILD_AUDIT — 前端重构审计文档

> 生成日期：2026-07-21。本文档是前端重构的**前置事实依据**，重构完成后可删除。
> 项目：星宿智能反应釜边缘上位机（ReactorOS / reactor-edge-daemon），Rust axum 后端 + Vue 3 前端，部署目标 LubanCat 2 / RK3568。

---

## 0. 总体架构

```
浏览器 (Kiosk)                    Rust daemon (reactor-edge-daemon, 默认 127.0.0.1:8000)
┌──────────────────┐   HTTP/WS   ┌─────────────────────────────────────────┐
│ Vue3 SPA          │ ──────────► │ axum Router (/api/*, /ws/*)             │
│ 构建为单文件       │             │ ServeDir: frontend/dist → (fallback)    │
│ frontend/dist/   │ ◄────────── │           static/ (SPA fallback)        │
│ index.html       │   同源伺服   │ SQLite (data/reactor.sqlite3, WAL)      │
└──────────────────┘             └─────────────────────────────────────────┘
```

- 后端：`axum 0.7` + `axum-server`(rustls 可选 TLS) + `rusqlite`/`sqlx` (SQLite) + tokio。入口 `src/main.rs`，路由 `src/api.rs`（router 定义在 587–686 行）。
- 前端：Vue 3.5 + TS 5.9 + Vite 7 + `vite-plugin-singlefile`（整包内联成单个 `frontend/dist/index.html`）+ Element Plus 2.14（全量）+ ECharts 5.6（按需）+ Pinia 2.3 + vue-router 4（hash 模式）。
- 启动：后端 `cargo run`（CLI 见下）；前端 dev `npm run frontend:dev`（vite 5173，代理 `/health` `/api` `/ws` → 127.0.0.1:8000）；前端构建 `npm run frontend:build` 输出 `frontend/dist`，后端 `--assets auto` 优先 serve `frontend/dist`，回退 `static/`。所有非 `/api/*` 路径 SPA fallback 到 index.html。
- 环境变量（后端）：`XINGSHU_OPERATOR_PASSWORD`/`XINGSHU_ENGINEER_PASSWORD`/`XINGSHU_ADMIN_PASSWORD`（默认 operator123/engineer123/admin123）、`XINGSHU_AUTH_SECRET`（非 loopback 绑定必须 ≥32 字符且非默认）、`XINGSHU_DB_ENCRYPTION_KEY`、`STEPFUN_*`、`XINGSHU_SEED_DEMO_CONTEXT`、`RUST_LOG`。
- 环境变量（前端，前缀 `XINGSHU_VITE_`）：`XINGSHU_VITE_API_TARGET`（dev 代理目标）、`XINGSHU_VITE_REFRESH_MS`（轮询间隔，默认 15000，钳 5000–60000）、`XINGSHU_VITE_LIVE_SAMPLE_LIMIT`（样本窗口，默认 24，钳 1–120）。
- 无 `.env.example` 于前端目录；根 `.env.example` 描述 StepFun 变量。CORS 未配置 → 必须同源或走 vite 代理。
- 硬编码约束：所有 v1 接口仅接受 device_id = `reactor_001`。

---

## 1. 页面清单（现有前端）

路由定义：`frontend/src/router.ts`（hash 模式）。无路由守卫，权限仅靠页面内按钮禁用（真正鉴权在后端）。

| 页面 | 路由 | 用途 | 主要操作 | 依赖接口 | 需登录 | 角色 | 新前端保留 |
|---|---|---|---|---|---|---|---|
| 实时监控 | `/monitor` | 传感器读数、温度趋势图、AI 建议摘要、当前批次、告警 | 只读浏览 | `/api/live`、`/api/recommendations/latest`、WS `/ws/v1/reactor/reactor_001/realtime` | 否（公开读） | 全部 | ✅ 保留 |
| 参数配置 | `/control` | 目标写入、运行开关（auto/manual-lock/急停）、工艺 CRUD 与启停、批次列表、安全边界 | 写目标、开关 auto/lock、急停/复位、故障复位、工艺增改/步骤增改/应用/启动/停止 | `/api/control/*`、`/api/processes*`、`/api/batches`、`/api/config/summary` | 写操作需登录 | 写：operator+；工艺编辑：engineer/admin | ✅ 保留 |
| AI 决策 | `/ai` | 本地 AI 状态、推荐详情、AI 主控 dry-run/execute、SOP 草案 | 刷新推荐、dry-run、execute、查看实验计划 | `/api/ai/control`、`/api/ai/experiment-plan`、`/api/recommendations/latest`、`/api/config/summary` | dry-run/execute 需登录 | apply_ai_suggestion（operator+） | ✅ 保留 |
| 历史数据 | `/history` | 批次筛选/详情/导出、样本时序分页查询、产物结果录入、独立批次启停 | 查询历史、筛选、导出 CSV/XLSX/报告、录入产率、启动/结束批次 | `/api/batches*`、`/api/v1/reactor/:id/history`、`/api/product-results`、导出接口 | 导出与写需登录 | export_reports / edit_process / start_stop_process | ✅ 保留 |
| 审计日志 | `/audit` | 哈希链校验指标、事件筛选分页、CSV 导出 | 查询、筛选、翻页、导出 | `/api/audit/logs`、`/api/audit/export.csv` | 是 | view_audit（engineer+） | ✅ 保留 |
| Modbus 调试 | `/modbus` | 寄存器/线圈/离散输入映射、读寄存器、管理员写寄存器、集成状态 | 读寄存器、写寄存器（admin+原因） | `/api/modbus/registers*`、`/api/config/summary` | 写需 admin | modbus_debug + admin 写 | ✅ 保留 |
| 系统配置 | `/settings` | 设备/组件控制、AINAS 任务、演示上下文、权限矩阵、集成状态、安全限幅、端点矩阵 | 组件控制、AINAS 任务创建/查看 | `/api/devices/*`、`/api/integrations/ainas/tasks*`、`/api/demo/context`、`/api/permissions/roles`、`/api/config/summary` | 组件控制/AINAS 需登录 | set_safe_targets / apply_integration_task | ✅ 保留（拆分重组） |
| 登录 | 无独立页（侧栏硬编码密码按钮） | — | — | `/api/auth/login` | — | — | ✅ 改为独立登录页（用户名+密码表单，不再硬编码密码） |

---

## 2. 后端接口清单

统一约定：
- **信封**：多数 `/api/*` 返回 `{code, message, data}`；**裸返回例外**：`/health`、`/api/live`、`/api/batches/start`(201 裸 Batch)、`/api/batches/:id/finish`(204)、`POST /api/control/*`(204 或裸 ControlTargets)、`/api/product-results`、`/api/recommendations/latest`(GET/POST 裸)、`/api/v1/reactor/:id/realtime`(裸)、`/api/test/pipeline-sample`(裸)。
- **错误**：统一信封 `{code, message, data:{error}}`，HTTP 状态码与 code 一致。400 参数/JSON 错、401 缺/坏/过期 token、403 权限不足或禁区、404 不存在、409 状态冲突（急停/人工锁/活动批次/恢复中）、503 设备/样本/AI 不可用、500 内部错。
- **显式 null 陷阱**：大量可选字段是 `Option<Option<T>>` —— 缺省=默认，`null`=400。前端**不要序列化 null 字段**。
- **认证**：`Authorization: Bearer <token>`；WS 用 `?token=`。token = 自定义签名串（非 JWT），12h 过期，无刷新，无登出接口。
- **分页**：仅 `/api/audit/logs`（page/page_size/event_type，有 total）、`/api/v1/reactor/:id/history`（start_time/end_time 必填，page/page_size，**无 total**，靠"返回数 < page_size"判尾页）、`/api/integrations/ainas/tasks`（limit，无元数据）。
- **device_id**：一律 `reactor_001`。

### 2.1 健康/聚合/演示（公开）

| 方法 | 路径 | 说明 | 认证 | 响应要点 |
|---|---|---|---|---|
| GET | `/health` | 存活探针 | 公开 | `{ok, service}` |
| GET | `/api/live` | 实时聚合大屏 | 公开（无新鲜样本→503 信封，**常态，需按"数据不可用"处理**） | `{runtime, device_status, latest_recommendation, ai_provider, processes, recent_samples, recent_batches, recent_outcomes, recent_events, alarms[], ai_memory, field_scenario, production_line}`；query: `sample_limit`(≤480), `include_processes/batches/events` |
| GET | `/api/demo/context` | 演示上下文 | 公开 | `{demo, sensor_data_policy, latest_recommendation, ai_provider, processes, recent_batches, recent_outcomes, recent_events, demo_alarms, ai_memory}` |

### 2.2 认证与权限

| 方法 | 路径 | 说明 | 认证 | 请求/响应 |
|---|---|---|---|---|
| POST | `/api/auth/login` | 登录 | 公开 | `{username,password}` → `{token, user:{username,role,permissions[]}, expires_at(RFC3339,+12h)}` |
| GET | `/api/auth/me` | 当前用户 | Bearer | `AuthUser{username,role,permissions[]}` |
| GET | `/api/permissions/roles` | 角色权限矩阵 | 公开 | `{mode, session_ttl_hours:12, default_users[], roles:[{role,label,can[],blocked[]}]}` |

权限矩阵（15 权限）：operator = view_monitor, view_history, export_reports, start_stop_process, set_safe_targets, apply_ai_suggestion, emergency_stop；engineer = operator + view_audit, edit_process, modbus_debug, ingest_sensor_sample, apply_integration_task；admin = 全部。

### 2.3 设备状态/能力/组件控制

| 方法 | 路径 | 说明 | 认证 | 要点 |
|---|---|---|---|---|
| GET | `/api/devices/status`（及 `/api/v1/...`） | 设备在线汇总 | 公开 | `DeviceStatusSummary{total_count, online_count, devices[DeviceStatusItem], sensors[], components[]}`；DeviceStatusItem 含 online/status(idle/running/stale/offline/error)/auto_enabled/manual_lock/emergency_stop/last_seen_age_ms/stale_after_ms/active_batch_id/unfinished_batch_ids/last_command_* 等 |
| GET | `/api/devices/capabilities`（及 v1） | 可控组件与动作 | 公开 | 组件动作 `{action,label,value_type("none"/"number"),min,max,unit}` |
| POST | `/api/devices/:id/components/:cid/control`（及 v1） | 单组件控制 | set_safe_targets | `{action, value?, reason?}`；组件：shake_stepper(start/stop/speed_up/speed_down/set_speed 0-60cpm)、heater_relay(on/off)、stirrer_motor(set_rpm 0-2000)、temperature_controller(set_target_temperature，仅 json_bridge) |

### 2.4 v1 文档版控制/工艺/数据

| 方法 | 路径 | 说明 | 认证 | 要点 |
|---|---|---|---|---|
| POST | `/api/v1/reactor/:id/control` | 目标写入/启动 | set_safe_targets | `{command_id?, timestamp?, params:{heat_time?,hold_time?,cool_time?,stir_speed?,shake_speed?,target_temp?,target_pressure?}(秒), priority?, auto_start?}` → `{command_id, status:"accepted", estimated_duration}` |
| POST | `/api/v1/reactor/:id/samples` | 上行传感器样本 | ingest_sensor_sample | 8 字段 SensorSnapshot |
| GET | `/api/v1/reactor/:id/realtime` | 实时数据 | view_monitor | **裸**：`{device_id, timestamp, status, device_online, device_status, data:{current_temp, current_pressure, stir_speed, shake_speed, tilt_state, tilt_angle, tilt_angle_source, flow_rate, product_concentration_percent, ph, phase, progress}, alarms[]}` |
| GET | `/api/v1/reactor/:id/history` | 历史采样 | 公开 | query `start_time*`、`end_time*`(RFC3339)、`interval?`、`page`、`page_size(1-500,默认100)` → `{items[], records[], page, page_size, ...}`（items==records，**无 total**） |
| POST | `/api/v1/reactor/:id/process` | 载入 phases 工艺 | edit_process | `{process_id, name, phases:[{phase:"heating"/"holding"/"cooling", params:{...}}]}` |

### 2.5 AI 控制与推荐

| 方法 | 路径 | 说明 | 认证 | 要点 |
|---|---|---|---|---|
| POST | `/api/ai/control`（及 v1） | AI 主控 | apply_ai_suggestion | `{intent?/mode?, dry_run?(默认true), allow_process_start?, allow_process_stop?, allow_component_control?, allow_target_adjustment?, preferred_process_id?}` → `AiControlResponse{mode, dry_run, decision, rationale, recommended_targets?, safety, actions:[{action_type,target,status,message,result?}]}` |
| GET | `/api/ai/experiment-plan`（及 v1） | 实验 SOP 草案 | 公开（无结果时 503） | `ExperimentPlanResponse{plan_id,title,status,source,recommendation,objective,sop_summary,steps[],acceptance_criteria[],safety_notes[],model_boundary[],next_actions[]}` |
| GET | `/api/recommendations/latest` | 读最新推荐（不触发模型） | 公开 | **裸** `Option<AiRecommendationEnvelope>` 或 null：`{based_on_batch_count, target_temperature_c, target_stirrer_rpm, heating_minutes, stirring_minutes, expected_score, rationale, provider:{mode,model,fallback_reason}}` |
| POST | `/api/recommendations/latest` | 重新生成推荐 | 公开 | 同上 |

### 2.6 工艺（processes）CRUD 与生命周期

| 方法 | 路径 | 说明 | 认证 |
|---|---|---|---|
| GET | `/api/processes` | 列表（draft/applied/archived） | 公开 |
| POST | `/api/processes` | 创建 `{name?, description?}` | edit_process |
| GET | `/api/processes/:id` | 详情 `{process, steps[]}` | 公开 |
| PUT | `/api/processes/:id` | 更新 `{name?, description?, status?}` | edit_process |
| POST | `/api/processes/:id/steps` | 加步骤（8 字段） | edit_process |
| PUT | `/api/processes/:id/steps/:sid` | 改步骤 | edit_process |
| POST | `/api/processes/:id/apply`（及 v1） | 应用并启动 | start_stop_process |
| POST | `/api/processes/:id/start`（及 v1） | 启动 | start_stop_process |
| POST | `/api/processes/:id/stop`（及 v1） | 停止 `{reason?}` | start_stop_process |
| POST | `/api/processes/current/stop`（及 v1） | 停止当前活动批次 | start_stop_process |

ProcessStep 字段：`step_index, name, target_temperature_c, ramp_rate_c_min, duration_minutes, target_stirrer_rpm, target_shake_speed_cpm, target_pressure_mpa, cooling_mode`。

### 2.7 批次（batches）与产品结果

| 方法 | 路径 | 说明 | 认证 | 要点 |
|---|---|---|---|---|
| GET | `/api/batches` | 最近 100 批次+结果 | 公开 | `{batches:Batch[], outcomes:BatchOutcome[]}` |
| POST | `/api/batches/start` | 启动批次（至少一个目标字段） | start_stop_process | **裸** Batch |
| GET | `/api/batches/:id` | 详情 | 公开 | `{batch, outcome?, samples[](≤480), events[](≤100)}` |
| POST | `/api/batches/:id/finish` | 结束批次 | start_stop_process | **204** |
| POST | `/api/product-results` | 录入产率/产物比→触发推荐 | edit_process | `{batch_id, yield_percent(0-100), product_ratio(0-1), notes?}` → **裸** AiRecommendationEnvelope |
| GET | `/api/batches/export.csv` | 导出 CSV | export_reports | 附件 |
| GET | `/api/batches/export.xlsx` | 导出 XLSX | export_reports | 附件 |
| GET | `/api/batches/:id/report.md` | 批次报告 Markdown | export_reports | 附件 |

### 2.8 手动控制

| 方法 | 路径 | 说明 | 认证 | 请求/响应 |
|---|---|---|---|---|
| POST | `/api/control/targets` | 写目标 | set_safe_targets | `{temperature_c, stirrer_rpm, shake_speed_cpm?}` → **裸** ControlTargets |
| POST | `/api/control/auto` | 自动控制开关 | set_safe_targets | `{enabled}` → 204 |
| POST | `/api/control/manual-lock` | 人工锁 | set_safe_targets | `{locked}` → 204（解锁被拒 409） |
| POST | `/api/control/fault/reset` | 清除控制故障 | set_safe_targets | 204 |
| POST | `/api/control/emergency-stop` | 急停 | emergency_stop | 204 |
| POST | `/api/control/emergency-stop/reset` | 复位急停 | emergency_stop | 204 |

### 2.9 审计

| 方法 | 路径 | 说明 | 认证 | 要点 |
|---|---|---|---|---|
| GET | `/api/audit/logs` | 分页查询 | view_audit | query `page,page_size(≤500,默认50),event_type?` → `{page,page_size,total,events:ControlEvent[],chain:AuditChainStatus}`；ControlEvent 含 previous_hash/event_hash（哈希链） |
| GET | `/api/audit/export.csv` | 导出 CSV | export_reports | query `event_type?` |

### 2.10 配置/集成/Modbus

| 方法 | 路径 | 说明 | 认证 | 要点 |
|---|---|---|---|---|
| GET | `/api/config/summary` | 配置总览 | 公开 | `{device_mode, device, safety(含 forbidden_control_zones), field_scenario, production_line, ai_memory, ai_provider, local_ai, permissions, data_security, integrations{rest_api,cli,mqtt,mqtt_status,ainas_*,modbus_*}}` —— 前端能力探测中心 |
| GET | `/api/integrations/ainas/tasks`（及 v1） | 任务列表 | view_audit | query `limit(≤200,默认50)` |
| POST | `/api/integrations/ainas/tasks`（及 v1） | 创建并执行 | apply_integration_task + 动作权限 | `{external_task_id?, action:"set_targets"/"start_process"/"stop_process", process_id?, 目标字段..., reason?}` |
| GET | `/api/integrations/ainas/tasks/:id`（及 v1） | 任务详情 | view_audit | — |
| GET | `/api/modbus/registers` | 点位映射总览 | 公开 | `{device_id, mode, slave_id, serial, tcp, read_registers[], write_registers[], coils[], discrete_inputs[]}` |
| GET | `/api/modbus/registers/:r/read` | 读单点 | 公开 | — |
| POST | `/api/modbus/registers/:r/write` | 写可写点位 | **admin** | `{value, reason(必填非空)}` |

### 2.11 实时通道

- **WS `GET /ws/v1/reactor/:device_id/realtime`**：认证 `Bearer` header 或 `?token=`。每 1s 推一帧，payload 与 `/api/v1/reactor/:id/realtime` 相同。失败时先发一帧错误信封 `{code,message,data:{error}}` 再关闭。服务端不读客户端消息。
- 无 SSE。MQTT（`xingshu/reactor_001/tasks` 等 4 topic）与 Modbus TCP 为设备侧通道，前端不直连。

### 2.12 未被旧前端使用的后端能力（新前端可考虑接入，但不虚构）

- `GET /api/auth/me`（token 校验/会话恢复）
- `GET /api/v1/reactor/:id/realtime`（HTTP 版，WS 之外的兜底单帧）
- `POST /api/v1/reactor/:id/control`（文档版目标写入，秒级时长+auto_start）
- `POST /api/v1/reactor/:id/process`（phases 三段式工艺）
- `GET /api/demo/context`（旧前端在 settings 页用过一次）
- `POST /api/test/reset`、`/api/test/pipeline-sample`（仅 --enable-test-reset，e2e 用，前端 UI 不暴露）

---

## 3. 核心业务流程

### 3.1 登录与会话
1. 用户提交 `{username, password}` → `POST /api/auth/login` → 得 `{token, user, expires_at}`。
2. token/user 存 localStorage；之后受保护请求带 `Authorization: Bearer`；WS 用 `?token=`。
3. token 12h 过期；401（missing/expired/invalid）→ 前端应登出并跳登录页。
4. 会话恢复：启动时若 localStorage 有 token，调 `GET /api/auth/me` 验证并刷新 user；失败则清除。
5. 登出：清本地状态 + 断 WS（无后端登出接口）。

### 3.2 实时监控（公开）
`/api/live` 轮询 + WS 1Hz 推送合并。样本缺失→503→显示"数据不可用"降级态（保留最近一次 runtime 供参考）。告警按 level 渲染。ECharts 温度趋势（实测/目标/推荐）。

### 3.3 目标写入与运行控制（operator+）
写目标 → `POST /api/control/targets`（安全门控：禁区 403、急停/人工锁/恢复中 409、样本过期 503）。开关 auto/manual-lock、急停/复位、故障复位。全部操作后 refreshLive。错误文案直接透传后端 message（英文）。

### 3.4 工艺生命周期（engineer 编辑 / operator+ 启停）
创建工艺 → 加步骤（多步，step_index 唯一）→ apply/start（创建批次+写设备，409 若已有活动批次）→ 运行 → stop（可选 reason）→ 批次 finish。`processes(current)/stop` 停当前。

### 3.5 批次与产物闭环
batches 列表 → 详情（samples+events）→ finish → 录入 product-results（yield/ratio）→ 后端自动生成新 AI 推荐 → 推荐指导下一批。导出 CSV/XLSX/报告（fetch+Blob 下载，因需 Authorization header）。

### 3.6 AI 主控（operator+）
dry_run=true 预演（decision/actions planned）→ 操作员复核 → dry_run=false 执行（受安全门控：execute 在急停/锁/恢复/live 不可用时禁用）。实验计划为只读 SOP 草案。

### 3.7 审计（engineer+）
分页+event_type 筛选；展示哈希链完整性（chain.valid/window_valid/broken_events）；导出 CSV。

### 3.8 Modbus 调试（读公开 / 写 admin）
读寄存器；写需 admin + 必填 reason，写后读回；展示 coils/discrete_inputs/TCP 状态。

### 3.9 AINAS 集成任务（engineer+ 查看 / engineer+ 提交）
任务列表/详情；创建 set_targets/start_process/stop_process 任务；状态机 received→executing→executed/failed/rejected。

### 3.10 组件控制（operator+）
devices/capabilities 拉取可控组件 → 按 value_type 渲染（none=按钮，number=数值输入带 min/max/unit）→ POST control（reason 可选）→ 刷新设备状态。

### 3.11 异常处理通则
- 401 → 清会话跳登录；403 → 提示权限不足（透传 `role '<role>' lacks permission '<perm>'`）；409 → 透传冲突原因（急停/锁/活动批次/恢复）；503 → "数据/设备不可用"降级态；网络错 → 友好提示+重试。
- `/api/live` 503 是常态降级，不算错误。

---

## 4. 必须保留的功能（新前端兼容清单）

1. 七个业务页面全部功能（监控/控制/AI/历史/审计/Modbus/设置），不允许只做首页。
2. 登录（三角色，改为真实用户名+密码表单，不硬编码密码）、会话保持、401 自动登出。
3. WS 实时推送 + 轮询兜底；`/api/live` 503 降级态。
4. 监控页：关键传感器读数、温度趋势 ECharts（实测/目标/推荐三线）、AI 建议摘要、当前批次、告警列表（中英文）、数据新鲜度（FRESH/STALE/OFFLINE/ERROR）。
5. 控制页：目标写入（带安全边界提示）、auto/manual-lock/急停/复位/故障复位、工艺 CRUD（含步骤）、工艺 apply/start/stop、停止当前、最近批次、安全 gating（未登录/提交中/恢复中/live 不可用禁用写操作）。
6. AI 页：本地 AI 状态、推荐详情、dry-run/execute（意图+允许开关）、结果复核（decision/safety/actions/原始 JSON）、SOP 草案展示。
7. 历史页：批次列表+本地筛选（搜索/状态/产物比段）、详情、样本时序分页查询（start/end/page/page_size）、产物结果录入（6 级锁定原因）、批次独立启停、CSV/XLSX/report.md 导出。
8. 审计页：哈希链指标卡、event_type 筛选、分页、CSV 导出、链完整性标识。
9. Modbus 页：寄存器/线圈/离散输入表、读、写（admin+reason+写后读回）、集成状态。
10. 设置页：设备状态/能力、组件控制、AINAS 任务 CRUD、权限矩阵、演示上下文、场景/产线识别、配置摘要、安全限幅、集成状态、存储加密信息。
11. 中文/英文双语切换（沿用 `tr(zh,en)` 模式或等价）。
12. 响应式（桌面 + ~393px 移动无横向滚动）。
13. 深色工业主题（Kiosk 场景），可读性优先。
14. 所有写操作的错误透传与操作反馈（消息提示）。
15. 文件导出必须走 fetch+Blob（带 Authorization header）。

---

## 5. 旧前端坏味道（重构必须解决）

1. **密码硬编码** `rolePasswords`（plant.ts:149-153）→ 改为登录表单。
2. **样式失控**：7300+ 行 CSS、13 文件、三套主题层层覆盖（base 荧光绿 → visual-rebuild → refined-industrial）→ 单一设计 token 体系。
3. **HMI 固定屏分页器 hack**：`hmiPageCounts` + `hmi-page-N` 类把多页塞进一路由 → 拆为真实区块/子路由。
4. **上帝 store**：plant.ts 950 行混合 http/auth/i18n/WS/全业务 → 按域拆分（api client、auth store、live store、业务 composables）。
5. **类型安全缺失**：全部 `ApiRecord = Record<string, unknown>` → 手写后端 DTO TypeScript 类型。
6. **device_id 硬编码多处** → 常量收敛。
7. **无 token 过期处理/401 拦截** → 统一 http 层处理。
8. **双数据源漂移**：live vs runtimeFallback、config vs live 的 field_scenario 散落各处 → 统一 selector。
9. **Element Plus 全量打包** → 保留 Element Plus（交互复杂度高，重写成本大），但确认按需或接受全量（单文件场景仍可接受；决策：保留全量，与旧版一致，避免引入回归）。
10. **巨型视图组件**（SettingsView 54KB、AiView 43KB 含 300 行翻译字典）→ 拆分为子组件。
11. **遗留兼容层**（legacy-hmi-shell.css、hmiNavItems 双导航）→ 删除。
12. **翻译正则脆弱**（后端英文句子→中文）→ 保留透传英文原文 + 仅对固定枚举值做 key 翻译，不做整句正则翻译。

## 6. 重构技术决策（已确认）

- 目录：继续在 `frontend/` 内重构（后端 serve `frontend/dist`，无需改后端）。
- 保留：Vue 3 + TS + Vite + singlefile + Pinia + vue-router(hash) + Element Plus + ECharts(按需)。
- 新增：`src/api/`（http client + DTO 类型 + 按域 API 模块）、`src/stores/`（auth、live、session 拆分）、`src/design/`（tokens.css 单一主题）、`src/components/`（通用组件）、`src/composables/`。
- 删除：13 个旧 CSS、app-shell.ts hmi 分页 hack、旧 views 实现（功能迁移后）。
- 路由：沿用 7 路由 + 新增 `/login`；路由守卫（受保护页未登录跳登录，登录后回跳）。
- 主题：深色工业风（Kiosk），单一 tokens，响应式断点 1200/768/480。
- 验证：`npm run frontend:build` + `tsc --noEmit`（vue-tsc 若可用）+ 现有 e2e（vue-acceptance）作为回归参照（注意：e2e 中的部分选择器/水印断言可能需随新 UI 更新——属测试更新，非功能删除）。
