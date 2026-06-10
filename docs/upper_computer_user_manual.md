# 星宿智能反应釜上位机用户手册

适用对象：现场操作员、调试工程师、第三方平台对接人员。

## 1. 启动与访问

本地开发/验收启动：

```powershell
$env:CARGO_TARGET_DIR='C:\tmp\xingshu-target-bugfix'
cargo run --bin reactor-edge-daemon -- `
  --config config/device.toml `
  --safety config/safety.toml `
  --memory config/ai_memory.toml `
  --integration config/integration.toml `
  --db data/reactor.sqlite3 `
  --assets auto `
  --bind 127.0.0.1:8000 `
  --safety-guard C:\tmp\xingshu-target-bugfix\debug\reactor-safety-guard.exe `
  --enable-test-reset
```

浏览器访问：

```text
http://127.0.0.1:8000/
```

需要本地 HTTPS 验收时，启动时同时提供证书和私钥：

```powershell
cargo run --bin reactor-edge-daemon -- `
  --config config/device.toml `
  --safety config/safety.toml `
  --memory config/ai_memory.toml `
  --integration config/integration.toml `
  --db data/reactor-tls-test.sqlite3 `
  --assets auto `
  --bind 127.0.0.1:18443 `
  --tls-cert output/tls-test/server.crt `
  --tls-key output/tls-test/server.key `
  --enable-test-reset
```

没有硬件或测试样本输入时，实时数据接口可能返回 `503`，界面会显示空值或离线状态。这是预期行为，不代表上位机进程未运行。

无硬件本地演示时，可以在另一个终端启动持续样本流：

```powershell
$login = cargo run --bin xingshu -- auth login --username engineer --password engineer123
$env:XINGSHU_TOKEN = ($login | ConvertFrom-Json).data.token
cargo run --bin xingshu -- --token $env:XINGSHU_TOKEN data sample --duration-s 180 --interval-ms 500
```

该命令通过正式 `/api/v1/reactor/:device_id/samples` 入口注入演示样本，不写控制目标。正式样本会更新 `latest_sample`，只证明传感器样本新鲜且可追溯；生产默认还要求下位机状态健康才允许危险控制，因此需要 engineer/admin token 或具备 `ingest_sensor_sample` 权限的第三方数据源 token。当前 `sensor_timeout_ms=6000`，样本流停止超过 6 秒后实时监控会重新显示 pipeline stale/503。

## 2. 登录与权限

默认本地账号：

| 角色 | 用户名 | 密码 | 权限范围 |
| --- | --- | --- | --- |
| operator | `operator` | `operator123` | 查看、基础控制 |
| engineer | `engineer` | `engineer123` | 调试、Modbus、导出、控制 |
| admin | `admin` | `admin123` | 全部本地管理权限 |

生产或正式验收前应通过环境变量覆盖默认密码，并设置 `XINGSHU_AUTH_SECRET`。

## 3. 中英切换

界面右上角提供语言切换。切换后以下内容会同步刷新：

- 页面导航。
- 实时监控和控制区文本。
- 历史、审计、配置页面文本。
- Modbus 调试页中的动态寄存器、状态、访问类型和集成接口文字。
- AINAS、MQTT、Modbus TCP 状态字块。
- AI 实验 SOP 草案中的摘要、步骤、操作说明、安全检查和模型边界说明。

已经完成本地浏览器视觉验证。截图索引位于：

```text
docs/upper_computer_visual_evidence_index.md
```

关键截图位于：

```text
output/ainas-integration-en-visible-final.png
output/modbus-expanded-points-zh-visible.png
output/mqtt-integration-status-visible.png
output/modbus-tcp-integration-status-visible.png
output/upper-computer-modbus-en-recheck.png
output/upper-computer-modbus-zh-recheck.png
output/upper-computer-hmi-live-sample-final.png
output/upper-computer-sop-zh.png
output/upper-computer-sop-en.png
```

## 4. 实时监控

实时监控页用于查看：

- 温度、搅拌转速、压力、摇罐速度、倾角、流量、产物浓度、pH。
- 当前目标温度和目标转速。
- 设备在线/离线/过期状态。
- 急停、人工锁定、自动控制状态。
- 温度曲线和运行趋势。

如果数据为空，先检查硬件/测试管线是否有新鲜样本写入。

## 5. 参数控制

参数控制页用于：

- 设置目标温度和搅拌转速。
- 开启或关闭自动控制。
- 开启或解除人工锁定。
- 触发或复位急停。

所有控制写入都会经过安全配置检查，包括最大值、单次步长、急停、人工锁和传感器超时保护。

本地开发启动时可通过 `--safety-guard` 指向已编译的 `reactor-safety-guard`；ARM64 release package 的 `run.sh` 和 systemd service 默认启用该独立安全判定子进程。也可用 CLI 单独验收夹紧结果：

```powershell
cargo run --bin xingshu -- --json safety check --temp 999 --rpm 9999 --shake 99 --pressure 99
```

## 6. AI 智能决策

AI 页面用于查看推荐参数和优化建议。当前已实现：

- 本地传统优化器建议。
- 云端 provider 配置与状态。
- 推荐上下文持久化。
- 实验 SOP 草案：`GET /api/ai/experiment-plan` 和 HMI AI 页面会基于批次结果、推荐参数与安全边界生成只读 heat/hold/cool 三段式实验方案，供操作员复核。

CLI 可查看同一份草案：

```powershell
cargo run --bin xingshu -- ai plan
```

该草案不会自动启动工艺，也不会写入硬件目标；执行前仍需操作员确认，并通过 AI master-control dry-run、安全限幅和审计链。

当前边界：

- 上位机已提供 `local_ai` readiness 展示、LoRA 训练数据导出、训练入口编排、manifest 归档和显式候选 adapter 晋级/备份边界。
- `ready_for_base_inference=true` 只表示基础模型入口存在；`ready_for_lora_inference=true` 才表示 LoRA adapter 也进入推理边界。
- `ready_for_prd_lora=true` 必须同时具备 LoRA 推理、训练边界和 RK 验收报告。
- Qwen3.5-2B/GGUF、生产 LoRA adapter、生产训练脚本、真实 llama.cpp 推理服务和 RK 端本地推理延迟小于 3 秒的验收仍未完成。

## 7. 历史数据与导出

历史页支持：

- 查看批次记录。
- 录入产品结果。
- 导出批次 CSV。
- 导出 XLSX 工作簿。
- 导出单批次 Markdown 实验报告。

CLI 也可导出：

```powershell
cargo run --bin xingshu -- data list
cargo run --bin xingshu -- data export --out batches.csv
cargo run --bin xingshu -- data export-xlsx --out batches.xlsx
cargo run --bin xingshu -- data report --batch-id 1
```

清理本地运行数据需要显式确认，仅用于本地验收或重置演示环境，生产环境不要误用：

```powershell
cargo run --bin xingshu -- data delete --yes --confirm-daemon-stopped
```

## 8. 审计日志

审计页用于查看控制、登录、导出、Modbus 写入和第三方任务事件。审计日志带哈希链状态，用于检查日志是否被篡改。

导出：

```powershell
cargo run --bin xingshu -- audit export --out audit.csv
```

## 9. Modbus 调试

Modbus 调试页用于：

- 查看当前寄存器映射。
- 读取寄存器当前值。
- 写入可写目标寄存器。
- 查看 Modbus RTU/TCP、MQTT、AINAS 等集成状态。

写寄存器同样受安全校验和 RBAC 控制；HTTP REST 调试写入口仅允许 admin 会话，并且必须填写非空写入原因。当前可写寄存器：

```text
target_temperature_c
target_stirrer_rpm
target_shake_speed_cpm
target_pressure_mpa
heat_time_s
hold_time_s
cool_time_s
```

CLI 示例：

```powershell
cargo run --bin xingshu -- modbus map
cargo run --bin xingshu -- modbus read temperature_c
cargo run --bin xingshu -- modbus write target_temperature_c 65 --reason "acceptance test"
```

## 10. 系统配置

系统配置页展示：

- 设备模式和数据源。
- 安全边界。
- AI memory/provider 状态。
- 权限和角色。
- AINAS、MQTT、Modbus TCP 等集成状态。
- 数据库存储加密状态；设置 `XINGSHU_DB_ENCRYPTION_KEY` 后，AINAS/MQTT 集成任务请求和回执会以 AES-256-GCM 加密写入 SQLite。

CLI 查看：

```powershell
cargo run --bin xingshu -- config
cargo run --bin xingshu -- config --local --json
```

## 11. AINAS/第三方任务下发

AINAS 可通过 REST 下发任务：

```http
POST /api/integrations/ainas/tasks
```

支持动作：

- `set_targets`
- `start_process`
- `stop_process`

任务执行后可查询：

```http
GET /api/integrations/ainas/tasks
GET /api/integrations/ainas/tasks/:id
```

MQTT 可在启用后向 `xingshu/reactor_001/tasks` 发布同样结构的任务；上位机会向 `xingshu/reactor_001/task_receipts` 发布回执，并按 `alert_interval_s` 向 `xingshu/reactor_001/alerts` 发布 retained 报警快照。样本缺失、样本过期或现场输入错误会在报警数组中出现 `sensor_data_unavailable` 高危报警；`sensor_fresh=false` 只是摘要位，第三方系统必须同时读取 `alarms`。

## 12. 常见问题

| 现象 | 处理 |
| --- | --- |
| 页面能打开但实时值为空 | 检查是否有硬件或测试管线样本；无样本时 `/api/live` 返回 `503` 是预期行为 |
| 实时值有更新但设备显示 offline 或高危报警 | 生产严格模式只把样本视为传感器证明；检查下位机 `connected`、`last_frame_ok`、`last_seen_at` 和 `last_command_ok`，状态未证明前不要继续生产控制 |
| 下位机报告 `last_command_ok=false` | 设备会显示 `status=error/online=false`，Modbus `device_connected=false`；先确认执行器状态和失败命令原因，清除下位机失败报告后再做控制故障复归 |
| MQTT alert 中 `sensor_fresh=false` | 检查 `alarms` 数组中的 `sensor_data_unavailable`，按样本缺失/过期或现场输入错误处理，不能只按布尔摘要忽略报警 |
| 写控制失败 | 检查是否登录、权限是否足够、是否处于急停/人工锁/传感器超时状态 |
| HTTPS 启动失败 | 检查 `--tls-cert` 和 `--tls-key` 是否成对提供，证书/私钥路径是否存在 |
| safety guard 调用失败 | 检查 `--safety-guard` 路径是否指向已编译的 `reactor-safety-guard(.exe)`；daemon 会记录错误并回退进程内安全判定 |
| Modbus TCP 未监听 | 默认 `require_tls=true`，需在 `config/integration.toml` 配置 `tls_cert` 和 `tls_key`；实验室联调可关闭 TLS 并使用非特权端口 |
| MQTT 显示 disabled | `config/integration.toml` 默认关闭 MQTT，需要配置 broker 并启用 |
| 数据库加密显示关闭 | 设置 32 字节、64 位 hex 或 base64 编码的 `XINGSHU_DB_ENCRYPTION_KEY` 后重启 daemon |
| `xingshu ai train` 失败 | 检查 `XINGSHU_LOCAL_AI_*` 模型、adapter、训练脚本或训练 HTTP 入口配置；未提供真实生产资产时失败是预期边界 |

## 13. 安全提醒

- 不要在生产部署启用 `--enable-test-reset`。测试入口只允许 loopback 监听地址启用，请求还必须带 `X-Xingshu-Test-Confirm: local-e2e`；若绑定到非本机地址，daemon 会拒绝启动。
- 默认账号密码只用于本地验收。
- 生产部署必须使用持久且可备份的 `XINGSHU_DB_ENCRYPTION_KEY`；更换或丢失密钥会导致已加密的集成任务载荷无法解密。
- 急停和硬件保护应由反应釜本体/下位机独立承担，上位机只作为可审计控制层。
- 正式第三方联调前需要补 MQTT/Modbus TCP 真实证书链、全量敏感字段清单和外部工具验收。

密钥生命周期、证书、token 和敏感字段清单见：

```text
docs/upper_computer_security_key_lifecycle.md
```
