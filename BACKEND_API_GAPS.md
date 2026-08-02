# BACKEND_API_GAPS — 前端重构中发现的后端问题

> 2026-07-21。本次重构**未修改任何后端代码**。以下问题在前端做了容错处理，建议后端后续跟进。

## 1. `GET /api/integrations/ainas/tasks` 空表也 500（建议后端修复）

- **现象**：数据库 `integration_tasks` 表为空、且未设置 `XINGSHU_DB_ENCRYPTION_KEY` 时，该接口返回 500 `XINGSHU_DB_ENCRYPTION_KEY is required to read encrypted integration task payloads`。
- **期望**：表为空（或行未加密）时应直接返回空数组 `[]`，不应要求加密密钥。
- **影响**：设置页 AINAS 任务列表在全新部署/未配置加密时显示为接口错误。
- **前端处理**：`stores/plant.ts::loadAinasTasks` 已 try/catch 静默降级为空列表，不阻断页面。
- **建议**：后端在查询路径上，仅当行确实被加密标记时才要求密钥；空结果集短路返回。

## 2. 无登出接口 / token 无刷新机制（记录，不阻塞）

- token 12 小时过期后只能重新登录；无 `POST /api/auth/logout`（前端只能清本地）。可接受，已在前端按 401 自动登出处理。

## 3. 无 SSE；WebSocket 为唯一推送通道（记录）

- 仅 `/ws/v1/reactor/:id/realtime`，认证需 `?token=` query（会进访问日志）。可接受，前端已实现。

## 4. 信封不统一（记录，前端已适配）

- 约 10 个接口裸返回/204（见 `FRONTEND_REBUILD_AUDIT.md` §2）。前端 http 层已统一处理。建议后端未来统一信封，但不强制。
