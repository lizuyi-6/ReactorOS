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

ARM64 release package 会安装 `reactor-edge-backup.service` 和 `reactor-edge-backup.timer`，默认每天 02:17 通过 `/opt/reactor-edge/current/backup.sh` 生成在线 SQLite `VACUUM INTO` 快照。脚本会先用 `.reactor-edge-backup.lock` 做非阻塞互斥，若定时备份和 OTA 前备份撞在一起，后启动的一方直接失败并保持已有快照不变。脚本先写入 `reactor.sqlite3.<UTC时间>.snapshot.tmp.<pid>` 临时文件，确认非空、`.sha256` 校验通过且具备 SQLite magic header 后，才原子发布为 `reactor.sqlite3.<UTC时间>.snapshot` 并更新 `latest.snapshot` / `latest.snapshot.sha256` 链接；中途断电或进程被杀不会把半写文件发布成 latest。默认保留天数由 `REACTOR_EDGE_BACKUP_RETAIN_DAYS` 控制，默认 30 天。

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

审计链异常时按工业 fail-closed 处理。开启自动控制、解除人工锁、清除控制故障、急停复位和 Modbus 调试目标写入必须先写入审计成功才提交运行态；解除人工锁同时要求新鲜现场样本、下位机状态健康、无急停、无未清控制故障且无下位机命令失败。清除控制故障和急停复位在审计成功后还会重新检查现场状态；如果审计期间故障内容变化、下位机重新报告命令失败或状态异常，复归失败并保持自动控制关闭。如果审计写入失败，阻断状态保持不解除，目标不更新，自动控制保持关闭。关闭自动控制、打开人工锁、触发急停、停止流程和结束批次属于降风险操作，会优先把系统置入保守状态；停止/结束批次在设备停止后会先关闭自动控制和同步停止目标，但活动批次 ID 会保留到数据库完成标记和审计都成功后才清除，防止停止收尾未完成时新批次启动。维护人员必须先恢复审计链、核对现场状态和故障原因，再按 SOP 重新解除阻断。

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

当前 ARM64/RK 发布包采用应用级 A/B slot：

```text
/opt/reactor-edge/slots/a
/opt/reactor-edge/slots/b
/opt/reactor-edge/current   -> 当前运行 slot
/opt/reactor-edge/previous  -> 上一可回滚 slot
/etc/reactor-edge           # 共享配置
/var/lib/reactor-edge       # 共享 SQLite、备份、OTA 状态
/project                    # 共享 state.json/control.json
```

升级命令先跑预检，再正式切换 slot：

```bash
sudo /opt/reactor-edge/ota-update.sh \
  reactor-os-*.tar.gz \
  --sha256 reactor-os-*.tar.gz.sha256 \
  --dry-run

sudo /opt/reactor-edge/ota-update.sh \
  reactor-os-*.tar.gz \
  --sha256 reactor-os-*.tar.gz.sha256
```

升级脚本会先检查板端必要命令是否存在，避免跑到解包或切换阶段才发现缺 `tar`、`sha256sum`、`install`、`sync` 等工具；随后校验 sha256，且 `.sha256` sidecar 必须引用当前传入的 tar 包名，不能拿另一个包的合法 hash 混用；同时会检查健康检查参数必须为正整数，且 `--required-passes` 不能大于 `--health-attempts`。之后脚本会检查 tar 包成员安全性（拒绝绝对路径、`..` 路径穿越、多顶层目录、链接和设备/特殊文件）、通过 backend 和 `/api/devices/status` 证明设备明确 `online=true`、`status=idle`、没有活动批次、没有急停、`auto_enabled=false`、`manual_lock=false`、没有未清 `last_control_error` 且 `last_command_ok` 未报告失败，然后再检查磁盘空间、校验备份脚本可用性、校验候选包 `BUILD-METADATA.properties`，并解包验证候选 slot 内容。`--dry-run` 只做这些预检，不切换 `current`/`previous`、不安装 systemd unit、也不创建数据库快照。切槽前的 checksum、tar 安全、metadata、busy-state 或 dry-run 校验失败会记录为 `rejected_before_switch`，用于区分“主动拒绝坏包/不安全现场”和“断电中断”，此时原 current slot 保持运行。正式升级会执行 SQLite 在线快照，再把包解压到 inactive slot。脚本会在 OTA 状态文件、候选 staging、正式 slot 替换、systemd unit、root OTA 工具、兼容链接和 `current`/`previous` 切换后执行 `sync`，降低提交阶段断电后的状态歧义；这仍不能替代掉电检测、保持电容和可靠 eMMC。系统还会启用 `reactor-edge-ota-boot-check.service`，并在 backend 的 `ExecStartPre` 再执行 `/opt/reactor-edge/ota-boot-check.sh`，保证开机、人工重启和 systemd 自动重启都先检查 OTA 状态；backend/kiosk 也配置了 systemd 启动限流，反复崩溃不会无限重启刷日志或磨损存储，而是进入维护介入。若断电发生在切换 `current` 之前，下一次启动会记录 `interrupted_before_switch` 并继续运行原 current slot；如果断电发生在 `switching`、`health_checking` 或 `rolling_back` 状态，且 `/run/reactor-edge/` 下没有 OTA 脚本主动健康检查的临时标记，就先恢复 previous slot，再允许生产服务启动；这个临时标记会记录 OTA 脚本 PID 和进程启动身份，boot-check 只有确认该 OTA 进程仍存在时才放行，脚本被 kill 或 marker 残留时会删除 marker 并按中断 OTA 处理。如果 OTA 状态已经是 `failed`，boot-check 会非 0 退出并保持 backend 停止，设备必须留在维护态，直到 recovery 或人工回滚完成。更新或手动回滚进入 `failed` 时，脚本会先清除健康检查临时放行标记并立即停止 backend/kiosk，避免当前失败现场继续生产控制。`current`/`previous` 链接必须指向 `/opt/reactor-edge/slots/a` 或 `/opt/reactor-edge/slots/b`，若被手工改到槽外，升级和回滚都会拒绝。若 staging/extract 阶段失败，脚本会清理临时目录，避免多次失败升级把 slot 存储吃满。若 backend 未运行、状态接口不可读、设备离线/过期/错误、自动控制仍开启、人工锁仍接管、控制故障未清或下位机仍报告命令失败，默认按“无法证明现场空闲”拒绝升级，只有确认维护窗口时才允许 `--force --confirm-maintenance-window`；跳过备份也必须同时使用 `--skip-backup --confirm-skip-backup`；无 sha256 sidecar 的实验室救援包必须同时使用 `--allow-missing-checksum --confirm-unsafe-no-checksum`。切换 `/opt/reactor-edge/current` 后，OTA 健康检查也必须连续看到 `/health`、HMI 和 `/api/devices/status` 的安全 idle 证明才写入 `committed`；如果新版本只是进程能启动但设备处于急停、控制故障、自动控制异常开启、人工锁未清、下位机命令失败或离线/过期状态，同样视为健康检查失败并回滚 previous slot。新版本启动失败或 HMI/`/health` 检查失败时也会自动恢复 previous slot 和 previous slot 的 systemd unit。OTA 状态会记录 `from_version`、`to_version`、`from_git` 和 `to_git`，用于现场回滚和事故追溯。

切换阶段还有一个额外的断电兜底：正式写入 `switching` 状态前，脚本会先把 `previous` 指向当前可用槽并同步落盘。若断电刚好发生在 `switching` 状态写入之后、`current` 尚未切到新槽之前，开机 boot-check 会识别 `current` 已经等于 `previous`，重装旧槽的 systemd unit、兼容链接和 OTA 工具，并把状态收敛为 `rolled_back_on_boot`，继续旧槽而不是误判为不可恢复失败。

升级前人工检查：

1. 确认反应釜处于维护窗口，无活动批次；如必须 `--force`，需有现场负责人确认并使用 `--confirm-maintenance-window`。
2. 导出审计和批次数据。
3. 记录当前 slot、包 SHA256、启动参数和维护单号。
4. 在测试环境跑 `/health`、CLI、HMI 和关键 API。

升级后：

1. 检查服务启动。
2. 检查 `/api/config/summary`。
3. 检查 HMI 七大页面。
4. 检查审计链。
5. 检查 Modbus/MQTT/AINAS 状态。

手动回滚：

```bash
sudo /opt/reactor-edge/ota-rollback.sh
```

若 backend 或状态接口已经不可用，手动回滚也会按 fail-closed 拒绝执行。此时先在现场面板确认反应釜已停稳、维护窗口已打开，再执行 `sudo /opt/reactor-edge/ota-rollback.sh --force --confirm-maintenance-window`。

回滚只切应用 slot，不默认回滚 SQLite。数据库 schema 升级必须保持至少一个版本向后兼容；若数据库迁移失败或结构损坏，先停服务，再按备份恢复流程人工确认恢复。

OTA 状态和日志：

```text
/var/lib/reactor-edge/ota/state.json
/var/lib/reactor-edge/ota/ota.log
```

回滚条件：

- 服务无法启动。
- 控制安全链路异常。
- 数据库无法读取。
- 审计链异常。
- HMI 阻塞级不可用。
- 维护窗口内健康检查连续失败。

## 7. 常见故障

| 现象 | 可能原因 | 处理 |
| --- | --- | --- |
| `/health` 不通 | 服务未启动、端口占用、TLS 配置错误 | 查看进程、端口和启动日志 |
| `/api/live` 返回 503 | 没有新鲜传感器样本 | 检查 STM32/JSON bridge/pipeline；本地演示先登录 engineer/admin 并设置 `XINGSHU_TOKEN`，再用 `xingshu --token <engineer-token> data sample` 注入样本 |
| 控制写入被拒绝 | 急停、人工锁、传感器超时、步长/范围/禁区 | 查看 HMI 状态和审计事件 |
| 人工锁解除被拒绝 | 无新鲜样本、下位机断连/状态过期/帧校验失败、急停未复位、`last_control_error` 未清除、`last_command_ok=false` 未解除，或数据库仍有未完成批次恢复状态 | 先确认人工接管原因、现场执行器状态、下位机命令状态和批次账；必要时执行 `xingshu control fault-reset`；解除人工锁不会开启自动控制 |
| 急停复位被拒绝 | 无新鲜样本、下位机断连/状态过期/帧校验失败、`last_command_ok=false` 未解除，或数据库仍有未完成批次恢复状态 | 先确认硬件急停链、下位机命令状态和批次账恢复，再复位；复位不会开启自动控制 |
| 启动/停止/结束批次后返回 500 且出现控制故障 | 设备动作已执行，但数据库状态提交或审计写入失败，软件账/审计账不可完全追溯 | 保持设备停止或人工接管，导出日志和数据库快照，核对批次/执行器实际状态，恢复数据库或审计链路后再执行 `xingshu control fault-reset` |
| 组件动作后返回 500 且出现控制故障 | 设备动作成功但审计写入失败，硬件状态不可完全追溯 | 停止继续生产控制，导出日志和审计，现场确认执行器状态后再执行 `xingshu control fault-reset` |
| AINAS/MQTT start/stop 后回执更新失败且出现控制故障 | 第三方任务已影响设备或批次，但 `integration_tasks` 回执缺失 | 停止继续生产控制，核对现场执行器和批次状态，恢复数据库/回执链路后再复归 |
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
