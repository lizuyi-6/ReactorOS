# 星宿智能反应釜上位机第三方接口验收报告

测试对象：CLI、REST API、AINAS 任务接口、MQTT bridge、Modbus RTU/TCP 映射。

测试日期：2026-06-04。

## 1. 接口总览

| 接口 | 当前状态 | 验收结论 |
| --- | --- | --- |
| CLI | 已实现 `xingshu` 二进制，覆盖状态、配置、数据、控制、AI、审计、Modbus、性能冒烟和样本入口 | 本地通过 |
| REST API | 已实现 HMI/CLI/第三方共用 API，写入受 RBAC 和安全保护 | 本地通过 |
| Safety guard | 已实现独立 `reactor-safety-guard` 进程和 `xingshu safety check` 本地验收入口 | 本地通过 |
| AINAS REST | 已实现任务创建、列表、单任务查询、执行回执持久化；启用 `XINGSHU_DB_ENCRYPTION_KEY` 后请求/回执 AES-256-GCM 加密落盘 | 本地通过 |
| MQTT | 已实现 bridge 框架、任务执行、receipt 发布逻辑、CA/客户端证书配置字段；`use_tls=true` 时缺少非空 `ca_cert` 会 fail-closed | 代码测试通过，外部 broker 未验收 |
| Modbus RTU | 已有配置映射和上位机调试接口 | 需等待 STM32 实机联调 |
| Modbus TCP | 已实现 server 框架、PDU 处理、本地 TCP/MBAP 与 TLS/MBAP 客户端测试 | 代码测试通过，外部 Modbus Poll/Slave 未验收 |

## 2. CLI 验收

入口：

```powershell
cargo run --bin xingshu -- --help
```

核心命令：

```text
xingshu start
xingshu stop
xingshu auth login
xingshu auth me
xingshu status
xingshu config
xingshu data list
xingshu data export
xingshu data export-xlsx
xingshu data report
xingshu data sample
xingshu data delete
xingshu control set
xingshu control start
xingshu control stop
xingshu control estop
xingshu ai suggest
xingshu ai model
xingshu ai train
xingshu audit list
xingshu audit export
xingshu modbus map
xingshu modbus read
xingshu modbus write
xingshu perf smoke
```

验收结论：

- CLI 通过同一 REST API 写入，不绕过安全链路。
- 受保护命令需要 bearer token。
- `xingshu data sample --duration-s ...` 通过正式 `/api/v1/reactor/:device_id/samples` 样本入口注入演示数据，不写控制目标。
- `xingshu perf smoke` 可输出本机只读 API 往返和安全计算性能冒烟报告。
- `xingshu ai train --export-only` 可从本地 SQLite 导出 LoRA 训练 JSONL；配置 `XINGSHU_LOCAL_AI_TRAIN_SCRIPT` 或 `XINGSHU_LOCAL_AI_TRAIN_URL` 后可编排训练、写入 manifest，并在显式 `--promote` 时晋级候选 adapter。真实生产训练脚本、Qwen/GGUF/LoRA 资产和 RK 验收仍未完成。

## 3. REST API 验收

完整 REST API、WebSocket、AINAS 和本地验收步骤见 `docs/upper_computer_api_acceptance_manual.md`。

基础接口：

```text
GET  /api/config/summary
GET  /api/live
GET  /api/v1/devices/status
POST /api/v1/reactor/:device_id/samples
GET  /api/v1/reactor/:device_id/realtime
GET  /api/v1/reactor/:device_id/history
POST /api/v1/reactor/:device_id/control
```

控制接口：

```text
POST /api/control/targets
POST /api/control/auto
POST /api/control/manual-lock
POST /api/control/emergency-stop
POST /api/processes/:id/start
POST /api/processes/:id/stop
```

数据与审计：

```text
GET  /api/batches/export.csv
GET  /api/batches/export.xlsx
GET  /api/batches/:id/report.md
GET  /api/audit/logs
GET  /api/audit/export.csv
```

验收结论：

- API 已覆盖 PRD 要求的数据提取、状态查询、控制下发、历史查询、审计导出。
- 写入路径统一经过 RBAC、安全限幅和审计。

## 4. AINAS REST 任务接口

创建任务：

```http
POST /api/integrations/ainas/tasks
```

示例 payload：

```json
{
  "external_task_id": "ainas-acceptance-001",
  "action": "set_targets",
  "target_temperature_c": 60,
  "target_stirrer_rpm": 300,
  "target_shake_speed_cpm": 24,
  "reason": "AINAS acceptance task"
}
```

查询：

```http
GET /api/integrations/ainas/tasks
GET /api/integrations/ainas/tasks/:id
```

支持动作：

| action | 说明 |
| --- | --- |
| `set_targets` | 设置温度/转速/摇罐等目标 |
| `start_process` | 启动指定工艺流程，受安全门限制 |
| `stop_process` | 停止当前工艺流程并写审计 |

验收结论：本地自动化测试通过，任务会持久化请求、执行状态和响应回执。启用 `XINGSHU_DB_ENCRYPTION_KEY` 时，请求与回执 JSON 以 AES-256-GCM 信封写入 SQLite；测试已确认原始列不含明文，且历史明文行仍可兼容读取。

## 5. MQTT 接口

配置文件：`config/integration.toml`。

默认 topic：

| topic | 方向 | 说明 |
| --- | --- | --- |
| `xingshu/reactor_001/tasks` | broker -> 上位机 | 第三方任务下发 |
| `xingshu/reactor_001/task_receipts` | 上位机 -> broker | 执行回执 |
| `xingshu/reactor_001/alerts` | 上位机 -> broker | retained 报警快照 |

当前实现：

- MQTT 3.1.1。
- `rumqttc` client。
- 默认 TLS/8883 模板，支持 `ca_cert`、`client_cert`、`client_key`；TLS 模式必须配置非空 `ca_cert`，否则拒绝连接而不是隐式信任系统根证书。
- bridge 启动时会同步刷新 `mqtt_status`，配置摘要不会在后台任务调度前短暂显示默认 broker/topic。
- task payload 复用 AINAS 执行路径。
- receipt 逻辑和持久化测试已覆盖。
- alert topic retained 报警快照已覆盖。

未完成验收：

- 使用 MQTT.fx 或 mosquitto 连接外部 broker。
- 断线重连和 backoff 验收。
- MQTT.fx 下证书链、用户名/密码、生产 broker 配置验收。

## 6. Modbus 接口

REST 调试入口：

```text
GET  /api/modbus/registers
GET  /api/modbus/registers/:name/read
POST /api/modbus/registers/:name/write
```

读点位：

```text
temperature_c
stirrer_rpm
pressure_mpa
shake_speed_cpm
tilt_angle_deg
flow_rate_l_min
product_concentration_percent
ph
```

写点位：

```text
target_temperature_c
target_stirrer_rpm
target_shake_speed_cpm
target_pressure_mpa
heat_time_s
hold_time_s
cool_time_s
```

Modbus TCP PDU：

| 功能码 | 当前状态 |
| --- | --- |
| `01` Read Coils | 已实现 |
| `02` Read Discrete Inputs | 已实现 |
| `03` Read Holding Registers | 已实现 |
| `06` Write Single Holding Register | 已实现 |

验收结论：

- 映射和 PDU 处理可用于上位机/第三方联调。
- 本地自动化测试已用真实 TCP/MBAP 客户端验证读请求。
- 正式 PRD 要求的 Modbus TCP over TLS 已通过本地自签证书 TLS 握手 + MBAP 读请求回归；HTTP/HTTPS 入口已通过 `--tls-cert`/`--tls-key` 本地验证。
- 外部 Modbus Poll/Slave 联调尚未完成。
- STM32 Modbus RTU 实机地址仍需和硬件侧最终寄存器手册对齐。

## 7. 配置摘要验收

`GET /api/config/summary` 已返回：

- `ainas_task_api`
- `mqtt_status`
- `modbus_tcp_status`
- `modbus_rtu`
- `json_bridge`
- `permissions`
- `safety`
- `ai`

Web HMI 的 Modbus 调试页已经把这些状态以中英双语展示。

## 8. 最终结论

第三方接口在本地代码与自动化层面已经具备联调基础：

- CLI、REST、AINAS：可作为当前可验收接口。
- Safety guard：独立进程 JSON 协议和 CLI 本地验收通过；生产 watchdog/权限隔离仍需部署验收。
- AINAS/MQTT 任务持久化：已支持 AES-256-GCM 静态加密，需在生产部署确认密钥生命周期。
- MQTT、Modbus TCP：代码路径和本地测试通过，但外部工具/真实网络验收未完成。
- Modbus RTU：需要硬件侧 STM32 寄存器最终版和实机联调。

外部接口验收的执行用例、证据字段和归档模板见 `docs/upper_computer_external_acceptance_checklist.md`，其中 P0-04、P0-05、P0-06 覆盖 Modbus TCP、MQTT 和 AINAS 的正式对外验收。

正式对外验收前必须补齐：

1. MQTT.fx/mosquitto broker 验收记录。
2. Modbus Poll/Slave 验收记录。
3. MQTT 证书链、Modbus TCP 外部工具 TLS 验收记录。
4. 生产 `XINGSHU_DB_ENCRYPTION_KEY` 生成、备份、轮换和丢失恢复策略。
5. 第三方平台实际任务下发、数据提取、回执确认记录。
