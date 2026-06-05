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
| 测试入口 | `/api/test/*` 仅在 `--enable-test-reset` 启用时可用，生产禁止开启 |

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
| engineer | operator 权限 + 审计、工艺编辑、Modbus 调试 |
| admin | 全部权限 |

## 3. 监控和设备状态

| 方法 | 路径 | 用途 | 当前状态 |
| --- | --- | --- | --- |
| `GET` | `/api/live` | HMI 实时聚合数据 | 本地通过；无新鲜样本时 503 是预期安全行为 |
| `GET` | `/api/devices/status` | 当前设备在线数量和状态 | 本地通过 |
| `GET` | `/api/v1/devices/status` | 文档版设备状态接口 | 本地通过 |
| `GET` | `/api/devices/capabilities` | 设备能力摘要 | 本地通过 |
| `GET` | `/api/v1/devices/capabilities` | 文档版设备能力接口 | 本地通过 |
| `POST` | `/api/v1/reactor/:device_id/samples` | 外部管线上行传感器样本 | 本地通过；`xingshu data sample` 复用该入口 |
| `GET` | `/api/v1/reactor/:device_id/realtime` | 文档版实时数据，需 `Authorization: Bearer <token>` 且具备监控权限 | 本地通过 |
| `GET` | `/api/v1/reactor/:device_id/history` | 文档版历史数据 | 本地通过 |
| `WS` | `/ws/v1/reactor/:device_id/realtime` | 文档版实时 WebSocket，需 `Authorization: Bearer <token>` 且具备监控权限 | 本地具备；需浏览器/第三方验收 |

## 4. 控制和工艺流程

| 方法 | 路径 | 用途 | 当前状态 |
| --- | --- | --- | --- |
| `POST` | `/api/control/targets` | 更新目标温度、转速、摇罐、压力等 | 本地通过 |
| `POST` | `/api/control/auto` | 开关自动控制 | 本地通过 |
| `POST` | `/api/control/manual-lock` | 开关人工锁定 | 本地通过 |
| `POST` | `/api/control/emergency-stop` | 急停 | 本地通过 |
| `POST` | `/api/control/emergency-stop/reset` | 复位急停 | 本地通过 |
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
| `POST` | `/api/processes/:id/stop` | 停止指定流程的活动批次 | 本地通过 |
| `POST` | `/api/processes/current/stop` | 停止当前活动流程 | 本地通过 |

控制类写入统一要求：

- bearer token 具备对应权限。
- 未处于急停或人工锁禁止状态。
- 传感器数据新鲜度满足 `sensor_timeout_ms`。
- 目标在范围、步长和 `forbidden_control_zones` 内。
- 写入审计链。

## 5. 数据、批次和审计

| 方法 | 路径 | 用途 | 当前状态 |
| --- | --- | --- | --- |
| `GET` | `/api/batches` | 列出批次 | 本地通过 |
| `POST` | `/api/batches/start` | 启动批次并写入目标 | 本地通过 |
| `GET` | `/api/batches/:id` | 批次详情 | 本地通过 |
| `POST` | `/api/batches/:id/finish` | 结束批次 | 本地通过 |
| `GET` | `/api/batches/export.csv` | 导出批次 CSV | 本地通过 |
| `GET` | `/api/batches/export.xlsx` | 导出 Excel 工作簿 | 本地通过 |
| `GET` | `/api/batches/:id/report.md` | 单批次 Markdown 报告 | 本地通过 |
| `POST` | `/api/product-results` | 录入产率和产物比例 | 本地通过 |
| `GET` | `/api/audit/logs` | 查看审计链事件 | 本地通过 |
| `GET` | `/api/audit/export.csv` | 导出审计 CSV | 本地通过 |

## 6. AI 和本地模型边界

| 方法 | 路径 | 用途 | 当前状态 |
| --- | --- | --- | --- |
| `GET` | `/api/recommendations/latest` | 读取最新已缓存 AI/优化器建议，不触发模型调用或写库 | 本地通过 |
| `POST` | `/api/recommendations/latest` | 生成并持久化最新 AI/优化器建议 | 本地通过 |
| `GET` | `/api/ai/experiment-plan` | 只读实验 SOP 草案 | 本地通过 |
| `GET` | `/api/v1/ai/experiment-plan` | 文档版只读实验 SOP 草案 | 本地通过 |
| `GET` | `/api/config/summary` | 返回 `local_ai.ready_for_inference` 和 `ready_for_training` | 本地通过 |

当前 AI 边界：

- `local-ga-sa-pid` 本地优化器已作为传统算法建议路径。
- SOP 草案是只读安全门控输出，不启动工艺、不写目标。
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
| `set_targets` | 设置控制目标 | 经过 RBAC、安全限幅、禁区和审计 |
| `start_process` | 启动工艺流程 | 经过流程启动安全门 |
| `stop_process` | 停止工艺流程 | 写停止审计 |

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
- `local_ai.ready_for_inference`
- `local_ai.ready_for_training`
- `integrations.mqtt_status`
- `integrations.modbus_tcp_status`
- `permissions.authentication`
- `safety.forbidden_control_zones`

## 10. 本地测试专用 API

| 方法 | 路径 | 用途 | 生产要求 |
| --- | --- | --- | --- |
| `POST` | `/api/test/reset` | 重置本地验收数据 | 生产禁用 |
| `POST` | `/api/test/pipeline-sample` | 写入测试样本 | 生产禁用 |

这些接口只有 daemon 启动参数包含 `--enable-test-reset` 时可用。

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
