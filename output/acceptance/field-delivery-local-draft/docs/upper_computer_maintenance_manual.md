# 星宿智能反应釜上位机维护手册

日期：2026-06-04

对象：李祖祎负责的 RK/PC 上位机软件运维维护。

边界说明：本文档是当前上位机维护手册初版，覆盖本地服务、配置、数据库、日志、密钥、证书、升级回滚和常见故障。正式生产维护手册仍需结合现场 RK 设备、STM32 硬件、企业证书、密钥托管、备份系统和安全扫描结果更新。

## 1. 服务和端口

| 项目 | 默认值 |
| --- | --- |
| HMI/API | `http://127.0.0.1:8000/` |
| 健康检查 | `GET /health` |
| 本地数据库 | `data/reactor.sqlite3` |
| HMI 资源 | `frontend/dist/` 默认，`static/` legacy fallback |
| 设备配置 | `config/device.toml` |
| 安全配置 | `config/safety.toml` |
| 集成配置 | `config/integration.toml` |
| AI memory | `config/ai_memory.toml` |

健康检查：

```powershell
Invoke-RestMethod http://127.0.0.1:8000/health
Invoke-RestMethod http://127.0.0.1:8000/api/config/summary
Invoke-RestMethod http://127.0.0.1:8000/api/devices/status
```

RK systemd 检查见 `docs/upper_computer_rk_deployment_acceptance_guide.md`。

## 2. 配置维护

| 配置 | 维护动作 | 风险 |
| --- | --- | --- |
| `device.toml` | 修改串口、Modbus slave、寄存器地址和缩放系数 | 必须与 STM32 最终手册一致，否则数据和控制会错位 |
| `safety.toml` | 修改温度、转速、压力、步长和禁区 | 生产修改需双人复核和审计 |
| `integration.toml` | 修改 MQTT、Modbus TCP、证书、topic 和端口 | 需外部 broker/工具复测 |
| `ai_memory.toml` | 修改推荐边界和参考批次 | 不等同于 LoRA 自进化 |

生产配置应使用脱敏模板和环境变量注入密钥，不把真实证书、密码、token 写入仓库。

生产上线前必须执行预检：

```powershell
$env:XINGSHU_AUTH_SECRET = "<32+ chars production secret>"
$env:XINGSHU_OPERATOR_PASSWORD = "<production password>"
$env:XINGSHU_ENGINEER_PASSWORD = "<production password>"
$env:XINGSHU_ADMIN_PASSWORD = "<production password>"
$env:XINGSHU_DB_ENCRYPTION_KEY = "<64 hex chars or base64 32 bytes>"
cargo run --bin xingshu -- ops preflight --production --json
```

预检会检查本地配置解析、默认口令/session secret、数据库加密 key、MQTT/Modbus TLS 文件、备份 service/timer/script 路径。fail 级发现会返回非 0；warning 表示仍需现场解释或外部验收，不能直接当成最终生产签字。

## 3. 数据库备份和恢复

备份对象：

- SQLite 数据库。
- `XINGSHU_DB_ENCRYPTION_KEY`，必须与数据库备份成对托管。
- 当前 `config/*.toml` 脱敏副本。
- 审计 CSV 导出。

备份建议：

```powershell
cargo run --bin xingshu -- --db data\reactor.sqlite3 ops backup --out backup\reactor-$(Get-Date -Format yyyyMMdd-HHmmss).sqlite3.snapshot
cargo run --bin xingshu -- audit export --out backup\audit.csv
cargo run --bin xingshu -- data export-xlsx --out backup\batches.xlsx
```

ARM64 release package 会安装 `reactor-edge-backup.service` 和 `reactor-edge-backup.timer`，默认每天 02:17 通过 `/opt/reactor-edge/backup.sh` 生成在线 SQLite `VACUUM INTO` 快照。快照路径为 `/var/lib/reactor-edge/backups/reactor.sqlite3.<UTC时间>.snapshot`，同时生成 `.sha256` sidecar 和 `latest.snapshot` 链接。默认保留天数由 `REACTOR_EDGE_BACKUP_RETAIN_DAYS` 控制，默认 30 天。

本地恢复演练：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\verify-backup-restore-drill.ps1
```

该脚本会启动临时 daemon 写入真实批次、产品结果和审计事件，执行 `xingshu ops backup`，停 daemon 后 `xingshu ops restore` 到新库，再用恢复库重启 daemon 并校验 `/health`、批次详情、产品结果和审计链窗口。报告输出到 `output\acceptance\restore-drill\restore-drill-report.json` 和 `.md`。这只能证明本地恢复链路可执行；现场仍需用生产 RK/PC、真实密钥、真实备份介质和归档策略复演。

恢复前检查：

1. 停止上位机服务。
2. 确认数据库文件和 `XINGSHU_DB_ENCRYPTION_KEY` 匹配。
3. 替换数据库。
4. 启动服务。
5. 检查 `/health`、`/api/config/summary`、审计链和历史数据。

密钥丢失后，已加密的 `integration_tasks.request_json` 和 `integration_tasks.response_json` 无法恢复。

## 4. 日志和审计归档

本地审计：

```powershell
cargo run --bin xingshu -- audit list
cargo run --bin xingshu -- audit export --out audit.csv
```

RK systemd 日志：

```bash
journalctl -u reactor-edge -f
journalctl -u reactor-edge --since "1 hour ago"
```

归档要求：

- 审计日志定期导出。
- 数据库定期备份；RK 上检查 `systemctl status reactor-edge-backup.timer` 和 `systemctl list-timers reactor-edge-backup.timer`。
- systemd 日志和外部接口工具日志按验收编号归档。
- 生产需防删除策略和恢复演练。

## 5. 密钥和证书维护

详细清单见 `docs/upper_computer_security_key_lifecycle.md`。

生产必须维护：

| 项 | 维护要求 |
| --- | --- |
| `XINGSHU_DB_ENCRYPTION_KEY` | 32 字节密钥；备份、分发和轮换受控 |
| `XINGSHU_AUTH_SECRET` | 轮换会使旧 token 全部失效 |
| 角色密码 | 生产覆盖默认密码 |
| HTTP TLS cert/key | 正式 CA 或企业 CA；私钥最小权限 |
| MQTT CA/client cert/key | broker 验收后归档 |
| Modbus TCP TLS cert/key | Modbus Poll/Slave TLS 验收后归档 |
| `STEPFUN_API_KEY` | 不入库、不进日志、最小权限 |

当前尚未提供自动数据库重加密迁移工具；生产密钥轮换需人工 SOP 或新增迁移工具。

## 6. 升级和回滚

升级前：

1. 备份数据库、配置、证书和当前二进制。
2. 导出审计和批次数据。
3. 记录当前版本、包 SHA256、启动参数。
4. 在测试环境跑 `/health`、CLI、HMI 和关键 API。

升级后：

1. 检查服务启动。
2. 检查 `/api/config/summary`。
3. 检查 HMI 七大页面。
4. 检查审计链。
5. 检查 Modbus/MQTT/AINAS 状态。

回滚条件：

- 服务无法启动。
- 控制安全链路异常。
- 数据库无法读取。
- 审计链异常。
- HMI 阻塞级不可用。

## 7. 常见故障

| 现象 | 可能原因 | 处理 |
| --- | --- | --- |
| `/health` 不通 | 服务未启动、端口占用、TLS 配置错误 | 查看进程、端口和启动日志 |
| `/api/live` 返回 503 | 没有新鲜传感器样本 | 检查 STM32/JSON bridge/pipeline；本地可用 `xingshu data sample` 演示 |
| 控制写入被拒绝 | 急停、人工锁、传感器超时、步长/范围/禁区 | 查看 HMI 状态和审计事件 |
| Modbus 写入失败 | 非可写寄存器、目标越界、安全禁区 | 查 `docs/upper_computer_modbus_register_map.md` 和安全配置 |
| MQTT 无回执 | broker 未连、topic 错误、证书错误 | 查 `integration.toml`、broker 日志、证书链 |
| AINAS 任务失败 | action 不支持或安全门拒绝 | 查任务详情和 `response_json` |
| `xingshu ai train` 失败 | 本地 LoRA 训练资产、训练 HTTP 入口或生产模型资产缺失 | 先用 `--export-only` 验证数据集导出；真实训练仍需补 Qwen/GGUF/LoRA、生产训练脚本或训练服务、评估输出和 RK 报告 |
| 加密任务无法读取 | DB key 不匹配或丢失 | 恢复匹配密钥；无法恢复时只能保留密文记录 |

## 8. 维护验收清单

| 检查项 | 当前状态 | 生产验收 |
| --- | --- | --- |
| 健康检查 | 本地通过 | RK/PC 正式环境复测 |
| 生产预检 | `xingshu ops preflight --production` 已实现并纳入一键验收 | 需用真实生产密钥、口令、证书和 RK 包路径执行 |
| 数据库备份/恢复 | 手动 `xingshu ops backup`、release timer、backup script 和 `scripts\verify-backup-restore-drill.ps1` 本地恢复演练已实现 | 需现场恢复演练、保留策略和异地归档验收 |
| 审计导出 | 已实现 | 需归档和防删策略 |
| 密钥清单 | 已文档化 | 需托管、轮换和丢失恢复演练 |
| TLS/MQTT/Modbus 证书 | 本地配置和部分自签测试 | 需正式证书链和外部工具验收 |
| 升级/回滚 | 有初版 SOP | 需真实包升级和回滚演练 |
| watchdog/权限隔离 | 独立 guard 本地可用 | 需生产 watchdog、最小权限和故障演练 |

## 9. 当前结论

当前维护手册足以支撑本地联调和验收准备，自动备份发布路径和生产预检 gate 已经补齐。正式生产维护仍需补现场恢复演练、异地归档/保留策略、密钥托管、证书链、watchdog/权限隔离、安全扫描和升级回滚演练证据。
