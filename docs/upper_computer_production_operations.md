# 上位机生产部署与运维操作手册

本文档覆盖 `reactor-edge-daemon` 在生产环境（RK3568/RK3588、x86_64、
ARM64）上需要的运维配套：watchdog、低权限用户、iptables/防火墙建议、
物理急停信号接入路径、生产密钥托管、备份/擦除运行手册。

它假设部署目标满足 `docs/upper_computer_rk_deployment_acceptance_guide.md`
列出的基本要求（rootfs、证书、libssl、Tokio 1.x）。

## 1. 守护进程 systemd unit

`/etc/systemd/system/reactor-edge-daemon.service`：

```ini
[Unit]
Description=Xingshu Reactor Edge Daemon
Documentation=https://github.com/lizuyi-6/ReactorOS
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=xingshu
Group=xingshu
WorkingDirectory=/opt/xingshu
EnvironmentFile=/etc/xingshu/daemon.env
ExecStartPre=/usr/bin/test -f /opt/xingshu/config/safety.toml
ExecStart=/opt/xingshu/bin/reactor-edge-daemon \
  --config /opt/xingshu/config/device.toml \
  --safety /opt/xingshu/config/safety.toml \
  --memory /opt/xingshu/config/ai_memory.toml \
  --integration /opt/xingshu/config/integration.toml \
  --db /opt/xingshu/data/reactor.sqlite3 \
  --assets auto \
  --bind 0.0.0.0:8443
ExecStartPost=/usr/bin/systemctl --no-block try-restart xingshu-hmi-cutover.service
Restart=on-failure
RestartSec=5
StartLimitIntervalSec=600
StartLimitBurst=5
TimeoutStopSec=20
KillMode=mixed
KillSignal=SIGTERM
LimitNOFILE=65536
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/opt/xingshu/data /opt/xingshu/log /opt/xingshu/backups
PrivateTmp=true
ProtectKernelTunables=true
ProtectControlGroups=true
RestrictSUIDSGID=true
RestrictNamespaces=true
RestrictRealtime=true
LockPersonality=true
MemoryDenyWriteExecute=true
SystemCallArchitectures=native

[Install]
WantedBy=multi-user.target
```

启用：

```bash
systemctl daemon-reload
systemctl enable --now reactor-edge-daemon.service
systemctl status reactor-edge-daemon.service
```

## 2. 低权限用户

```bash
# 1. 创建系统用户（不允许登录）
useradd --system --home /opt/xingshu --shell /usr/sbin/nologin --user-group xingshu

# 2. 目录权限
install -d -o xingshu -g xingshu -m 0750 /opt/xingshu
install -d -o xingshu -g xingshu -m 0750 /opt/xingshu/data
install -d -o xingshu -g xingshu -m 0750 /opt/xingshu/log
install -d -o xingshu -g xingshu -m 0750 /opt/xingshu/backups
install -d -o xingshu -g xingshu -m 0750 /etc/xingshu
chmod 0640 /etc/xingshu/daemon.env

# 3. SQLite 数据库文件需要读写
chown xingshu:xingshu /opt/xingshu/data/reactor.sqlite3
chmod 0640 /opt/xingshu/data/reactor.sqlite3

# 4. 证书和私钥（仅 xingshu 可读）
chown xingshu:xingshu /etc/xingshu/tls/*.pem
chmod 0640 /etc/xingshu/tls/*.pem
```

## 3. 环境变量

`/etc/xingshu/daemon.env`：

```bash
RUST_LOG=info,reactor_edge_daemon=info
RUST_BACKTRACE=1
XINGSHU_DB_ENCRYPTION_KEY=<32-byte hex; the daemon only reads this at startup, so plan a restart window when running `xingshu key generate`>
XINGSHU_STEPFUN_API_KEY=<optional, cloud provider integration>
XINGSHU_AINAS_API_KEY=<optional, AINAS remote dispatch>
```

文件权限：

```bash
chown root:xingshu /etc/xingshu/daemon.env
chmod 0640 /etc/xingshu/daemon.env
```

密钥轮换：先停止 daemon 并确认现场不在生产控制，再用 `xingshu key generate --db <path> --yes` 生成新 `<db>.key`。`xingshu key rekey-integration-tasks --db <path> --old-key-file <old.env> --new-key-file <new.env> --dry-run` 可先做只读预检；确认计数后仍在 daemon 停止期间改用 `--yes` 提交。`key generate` 和正式 rekey 会拒绝 daemon 活跃状态；如果 systemd 状态无法自动确认，只有已有维护记录后才能加 `--confirm-daemon-stopped`。该命令只迁移 `integration_tasks.request_json/response_json`，会把旧密文和历史明文行重写为新 key 的 AES-256-GCM 信封；完成后再重导出 `XINGSHU_DB_ENCRYPTION_KEY` 并启动 daemon。

## 4. watchdog / 健康检查

建议叠加两层 watchdog：

- **systemd 自重启**：`Restart=on-failure` + `RestartSec=5` + `StartLimitIntervalSec=600` + `StartLimitBurst=5`，处理崩溃和 OOM，同时避免无限重启刷日志或磨损存储；超过限流后进入维护介入。
- **应用层 watchdog**：`/health` 端点被外部 ping（上位机/上位工控机）每 30s 探一次，三次失败即触发 `systemctl restart reactor-edge-daemon.service`。
- **可观测性**：`xingshu perf smoke` 每 6h 跑一次；输出 p50/p95 写入 `/opt/xingshu/log/perf.log`；超过阈值则触发告警。

## 5. 物理急停信号路径

上位机不直接控制物理急停回路。急停信号在 `device.toml` 的硬件
（如 STM32 / 安全继电器）侧被读取，硬件急停触发后通过 Modbus 寄存器或
JSON bridge `state.json` 推送一个 `emergency_stop=true` 的最新样本到
daemon。daemon 在控制循环内检测到 `emergency_stop=true` 时立即停止
写入、清除 `auto_enabled`、在审计链写入 `emergency_stop` 事件，并通过
`/api/control/emergency-stop` 暴露给上位机。

部署方需要：

1. STM32 / 安全继电器读取物理急停（NC 触点或安全 PLC 输出）。
2. 急停状态以 100 Hz 写入共享 Modbus 寄存器或 JSON 文件。
3. daemon 在 `config/safety.toml` 启用 `sensor_timeout_ms = 6000`，
   保证物理急停信号丢失也触发应用层安全停机。
4. 上位机在 UI 上显示 `Emergency Stop` 状态；急停复位通过
   `POST /api/control/emergency-stop/reset`，由 operator 角色执行并
   写入审计链。复位急停前必须有 `sensor_timeout_ms` 内的新鲜现场样本；
   若下位机状态仍断连、帧校验失败、状态过期、仍报告 `last_command_ok=false`，
   或数据库/运行态仍处于未完成批次恢复状态，复位请求会被拒绝。该动作只复位急停，
   不清除设备控制写入故障，也不会重新开启自动控制。

目标值写入、会让自动控制接管的入口、以及会让执行器进入运行/升速/改设定值的组件动作按工业 fail-closed 处理：`/api/control/targets`、文档版 v1 control、v1 process 加载、自动控制开启、批次/流程启动、AI 执行目标、AINAS/MQTT `set_targets`、Modbus 目标写入、组件 `start`/`on`/`speed_up`/设定值动作都要求最新传感器样本仍在 `sensor_timeout_ms` 内、下位机 `connected/last_frame_ok/last_seen` 健康、急停未触发、人工锁未打开、上一次控制写入没有未清除故障。任一条件不满足时拒绝新目标，且不把目标缓存成“恢复后自动执行”；关闭自动控制、停止工艺、急停、组件 `stop`/`off` 入口仍保持可用。
后台自动控制对同一条目标写入只在一个 `sensor_timeout_ms` 新鲜度窗口内去重；超过该窗口后会重新经过最终互锁并重申同一目标，避免下位机重启、串口重连或执行器掉电后软件长期假定上一次易失写入仍然有效。重申前仍必须通过新鲜样本、下位机健康、急停/人工锁/控制故障和安全 guard 检查。

解除阻断/升风险操作按现场证明和审计优先处理：开启自动控制、解除人工锁、清除控制故障和急停复位必须先完成安全门检查和审计写入，审计链异常时不提交新的运行态。解除人工锁、清除控制故障和急停复位还会拒绝数据库/运行态未完成批次不一致的恢复状态，避免软件账尚未修复时把现场显示成可继续操作。解除人工锁还要求新鲜现场样本、下位机 `connected/last_frame_ok/last_seen` 健康、无急停、无未清 `last_control_error`，且下位机未报告 `last_command_ok=false`。降风险操作按保守优先生效：关闭自动控制、打开人工锁、触发急停、停止流程和结束批次会先把系统置入更安全状态，审计失败也不能成为继续生产控制的理由；其中急停触发即使审计写入失败也保持 `emergency_stop=true`，并额外锁存控制故障，要求维护确认审计链后再复归。Modbus 调试写目标也按纯目标意图处理，审计失败时不提交 `runtime.targets`。

传感器样本缺失、样本超时、JSON bridge 状态过期、下位机断连或上行帧校验失败时，daemon 会立即关闭 `auto_enabled`，记录 `field_input_fault_auto_disabled` 审计事件，并把原因保留在 `last_sensor_error`。后续样本恢复时，daemon 只在新样本已成功落库并写入 `latest_sample` 后清除旧 `last_sensor_error`，再按这帧新样本重新计算 hard limit 报警；旧现场输入错误不会污染恢复第一帧的报警计算，也不会自动恢复自动控制。操作员必须确认现场安全后重新执行开启自动控制 SOP。
外部数据管线上行样本必须先成功写入 SQLite，才会更新运行态 `latest_sample`，但它只证明传感器样本新鲜且可追溯，不等同于执行器/下位机状态健康。`config/safety.toml` 默认启用 `require_device_status_for_control = true`；目标写入、自动控制、人工锁解除、控制故障复归和急停复位还必须看到下位机 `connected/last_frame_ok/last_seen` 健康，且未报告 `last_command_ok=false`。若样本落库失败，daemon 会清除可用样本、关闭 `auto_enabled` 并保留 `last_sensor_error`，避免用不可追溯的瞬时内存样本放行目标写入。设备直读模式下，即使样本落库失败，同一帧下位机状态里的 `last_command_ok=false` 仍会锁存控制故障，不能被现场输入故障掩盖。
报警输出也按该边界处理：`/api/live`、AI safety 摘要和 MQTT retained alert 快照在样本缺失或超过 `sensor_timeout_ms` 时生成 `sensor_data_unavailable` 高危报警；MQTT 中的 `sensor_fresh=false` 只是摘要位，不能替代报警项本身。

新鲜样本如果触发 `ai_memory.toml` 中 `sensor_limits` 的 hard limit 高报警（如温度或压力超过硬上限），daemon 会关闭 `auto_enabled` 并记录 `high_sensor_alarm_auto_disabled`。normal range warning 只进入报警和 HMI 提示，不自动关闭控制。硬限报警后即使后续样本恢复正常，也需要操作员按 SOP 重新开启自动控制。

批次/流程启动和 `auto_start=true` 的文档版控制入口会先向设备写入目标，但软件活动批次/自动控制状态只在审计链、流程状态提交和最后一次现场互锁都通过后才提交给 `runtime`，避免后台控制循环看到一个尚未可追溯的运行态。若设备写入失败、审计链写入失败或流程状态标记失败，daemon 会回滚软件运行态，并尽力向设备写入停止/降风险目标：温度降到安全下限、加热/保温/冷却/摇床清零、搅拌降到安全最小值、压力目标归零，避免“软件认为已运行但硬件状态不确定”或“硬件已运行但审计缺失”的分叉状态。启动前设备写入失败会记录 `process_start_failed` 审计事件但不会激活运行态；设备已启动后若审计、状态提交或最终互锁失败，会锁存 `last_control_error`；回滚后的 `runtime.targets` 同步为这组停止目标，不能把失败启动目标留作后续自动控制基线。

停止流程和结束当前活动批次都按设备停机动作处理，而不是只改数据库状态：daemon 会先向设备写入同一组停止/降风险目标，设备停止成功后立即关闭自动控制并同步停止目标，但 `active_batch_id` 会保留到数据库完成标记和停止审计都成功后才清除，防止停止收尾尚未可追溯时新启动插队。如果设备停止写入失败，批次不会被标记完成，系统只锁存控制故障并关闭自动控制，必须按执行器链路异常处理。如果设备已经停下但数据库完成标记或审计写入失败，也会保持停止态并锁存控制故障，按“硬件已变更但软件账/审计账缺失”处理，不能继续生产控制。若当前正在运行另一个批次，禁止结束非活动批次，防止错 ID 操作干扰生产态。

人工锁不是“自动控制暂停键”。打开人工锁会立即关闭 `auto_enabled`；
解除人工锁只解除现场人工接管状态，不会恢复此前的自动控制状态。若确需恢复
自动控制，操作员必须在确认现场安全、新鲜样本、下位机状态健康、无急停、无控制写入故障且无下位机命令失败后，
单独执行开启自动控制 SOP。进程启动、断电恢复、systemd 自动重启和 OTA 切槽后
一律以 `auto_enabled=false` 初始化，即使 `safety.toml` 配置了
`auto_enabled_default=true` 也不会自动接管现场；若同时配置
`manual_lock_default=true`，人工锁仍按默认值打开。

设备控制写入失败会锁存 `last_control_error`、写入 `device_write_failed`
审计事件并立即关闭 `auto_enabled`。后台自动控制循环如果设备写入已经成功但
`device_write` 审计失败，也会锁存 `last_control_error` 并关闭 `auto_enabled`，
不能继续自动下发。后台自动控制每次写设备前还会重新读取当前运行态并复算安全命令；如果急停、人工锁、控制故障、样本新鲜度、目标值或 forbidden zone 让复算结果不再是同一个命令，本轮直接跳过设备写入，等待下一轮用新的现场状态重新决策。启动、停止、结束批次若设备动作已经成功但后续数据库状态提交或审计写入失败，也会锁存控制故障。AI master control 若真实执行了目标调整、流程启停或组件动作，但最终 `ai_master_decision` 审计写入失败，也按设备动作后审计失败处理；即使只执行了目标调整，也因为目标已写入设备而锁存控制故障。组件控制如果设备动作已经成功但后续审计写入失败，或审计成功后、提交 `runtime.targets` 前现场互锁发生变化，也会
锁存 `last_control_error`、关闭 `auto_enabled`，并且不会把新组件目标提交到
`runtime.targets`。AINAS/MQTT `start_process` / `stop_process` 若设备动作已经执行
但 `integration_tasks` 回执更新失败，也会锁存控制故障并关闭自动控制；MQTT 任务已经执行成功但向 broker 发布 `task_receipts` 失败时，也按外部回执缺失处理，`set_targets` 会锁存目标意图回执故障，`start_process` / `stop_process` 会按设备动作后回执缺失处理。该状态必须按“硬件状态不确定/审计或回执缺失”
处理，不能继续生产控制。后续传感器样本恢复、急停复位或人工锁开关都不会
自动清除该故障；维护人员确认执行器通信/驱动链路恢复后，
使用 `POST /api/control/fault/reset` 或 `xingshu control fault-reset`
显式复归；若数据库仍有非 runtime 当前活动批次的未完成记录，或 runtime 活动批次已不被数据库未完成记录支撑，复归会拒绝。
复归只清故障并记录 `control_fault_reset` 审计事件，
自动控制仍保持关闭，必须再由操作员按 SOP 单独开启。
复归审计成功后还会重新检查现场样本、下位机状态和锁存故障内容；只有审计前确认的同一条故障仍然存在时才会清除。如果审计期间出现新的 `last_control_error`、下位机重新报告 `last_command_ok=false` 或现场状态变坏，请求会失败并保持自动控制关闭。急停复位同样在审计后复查现场状态，审计期间重新出现的下位机命令失败或状态异常不能被复位动作覆盖。如果下位机状态仍报告 `last_command_ok=false`，复归请求会被拒绝，直到现场确认并让下位机状态不再报告上一条命令失败。
状态展示也按该口径处理：只要下位机仍报告 `last_command_ok=false`，设备状态和实时聚合里的设备摘要显示 `online=false/status=error`，组件状态显示 `error`，Modbus `device_connected` 离散输入为 false，不能因为 `connected=true/last_frame_ok=true` 就把设备当作健康在线。
Modbus `alarm_active` 离散输入与 `/api/live`/MQTT 的统一报警数组保持一致，只要存在样本缺失/过期、严格模式下位机状态缺失、下位机命令失败、急停、锁存控制故障或 hard limit 报警，PLC 侧都应看到 `alarm_active=true`。

## 6. iptables / 防火墙

上位机在 `0.0.0.0:8443` 监听 TLS（`systemd` unit 已传 `--tls-cert`/`--tls-key` 指向 `/etc/xingshu/tls/server.pem` 与 `server-key.pem`，并对证书目录设 `ReadOnlyPaths=/etc/xingshu/tls`）。如未启用 TLS，请把 daemon 改回 `127.0.0.1:8000` 并让 nginx / caddy / 工业网关在 8443 终止 TLS。建议：

```bash
# 默认入站丢弃
iptables -P INPUT DROP
iptables -P FORWARD DROP
iptables -P OUTPUT ACCEPT

# 允许本地回环
iptables -A INPUT -i lo -j ACCEPT

# 允许已建立连接
iptables -A INPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT

# 允许 Modbus TCP（仅内部网段）
iptables -A INPUT -p tcp --dport 502 -s 10.0.0.0/8 -j ACCEPT

# 允许 MQTT（仅内部 broker）
iptables -A INPUT -p tcp --dport 8883 -s 10.0.0.0/8 -j ACCEPT

# 允许运维网段访问 HTTPS 上位机
iptables -A INPUT -p tcp --dport 8443 -s 10.0.0.0/8 -j ACCEPT

# 拒绝 ICMP（避免信息泄露；可选）
iptables -A INPUT -p icmp -j DROP

# 持久化
netfilter-persistent save
```

如使用云端 VPN/WireGuard，daemon 只绑定到 `wg0` 隧道地址（`--bind 10.10.0.1:8443`）。

## 7. 备份 / 恢复 / 擦除

见 `docs/upper_computer_maintenance_manual.md` 和 `xingshu ops`：

```bash
# release 包默认安装每日 systemd timer：
systemctl status reactor-edge-backup.timer
systemctl list-timers reactor-edge-backup.timer

# 立即触发一次在线 SQLite 快照。内部使用 VACUUM INTO，不需要停 daemon。
systemctl start reactor-edge-backup.service

# 手动备份示例：
xingshu ops backup --db /opt/xingshu/data/reactor.sqlite3 \
  --out /opt/xingshu/backups/reactor.sqlite3.$(date -u +%Y%m%d-%H%M%S).snapshot

# 季度全量归档（脱机存储）
xingshu ops backup --db /opt/xingshu/data/reactor.sqlite3 \
  --out /mnt/nfs/xingshu-archive/reactor.sqlite3.$(date -u +%Y%m%d-%H%M%S).snapshot

# 灾备恢复必须进入停机维护窗口。restore 会拒绝在 daemon 活跃时覆盖运行库；
# 如果目标库仍可读且有未完成批次，也会拒绝覆盖以保留生产证据。
# --confirm-daemon-stopped 只允许用于 systemd 状态无法自动确认、且已有维护记录的场景。
systemctl stop reactor-edge-daemon reactor-edge
xingshu ops restore --backup /opt/xingshu/backups/reactor.sqlite3.<timestamp>.snapshot \
  --db /opt/xingshu/data/reactor.sqlite3 --yes
systemctl start reactor-edge-daemon
/opt/reactor-edge/health-check.sh --production

# 退役设备前安全擦除。必须保持停机维护窗口；ops wipe 会拒绝
# daemon 活跃状态和仍有未完成批次的可读数据库，随后覆盖/删除
# SQLite 主文件、WAL/SHM/JOURNAL、<db>.key 和同级 backups/ 中匹配快照。
systemctl stop reactor-edge-daemon reactor-edge
xingshu ops wipe --db /opt/xingshu/data/reactor.sqlite3 --yes
blkdiscard /dev/nvme0n1   # SSD 全盘 TRIM，物理擦除
```

## 8. 监控与告警

- 暴露 `/health` 和 `/metrics`（如需）。建议接入 Prometheus + AlertManager。
- 关键告警：
  - `audit_chain_valid == false`（防篡改链断裂）
  - `emergency_stop` 触发后 5 分钟内未复位
  - `auto_enabled == true && sensor_freshness > 6000ms`
  - `forbidden_zone_hit_count > 0`（AI 越过禁区）
- 告警通过 MQTT `xingshu/reactor_001/alerts` 主题发布（如果启用）。

## 9. 升级与回滚

- 升级前先 `xingshu ops backup` 一次 SQLite `VACUUM INTO` 在线快照；恢复时必须停止 daemon。
- 部署新二进制到 `/opt/xingshu/bin/reactor-edge-daemon.new`，
  `chmod 0750`，`chown xingshu:xingshu`。
- 切流：`systemctl stop reactor-edge-daemon && mv ...new ... && systemctl start`。
- 回滚：把上一个版本的二进制拷回 `bin/reactor-edge-daemon`，
  `systemctl restart reactor-edge-daemon`。

## 10. 安全审计与漏洞扫描

- 季度运行 `cargo audit` / `cargo outdated`。
- 半年跑一次外部漏洞扫描（Nessus / OpenVAS）。
- 关键 CVE 公告后 24h 内评估影响并发布补丁。

## 11. 文档化运维角色

| 角色 | 权限 | 操作 |
|---|---|---|
| Operator | 看/操作/急停/Modbus 写 | 启动/停止工艺、查看报警 |
| Engineer | Operator + 工艺编辑/AI 复核 | 创建工艺、添加步骤、触发 AI |
| Admin | 全权限 | Modbus 写、用户管理、密钥轮换 |
| SRE | 部署/回滚/备份/擦除 | systemd、xingshu ops、systemctl |
| Auditor | 只读审计 + 导出 | CSV / Markdown 报告、链校验 |

## 12. 故障演练

至少每季度演练一次：

1. daemon 崩溃 → systemd 自动重启 → 上位机自动恢复（30s 内）。
2. 数据库被损坏 → 停止 daemon → `xingshu ops restore` 从备份恢复 → 启动 daemon → `/opt/reactor-edge/health-check.sh --production` 通过后再恢复生产；如果 daemon 活跃或状态不可证明，restore 必须拒绝，`--confirm-daemon-stopped` 仅用于已有维护记录的人工确认。若目标库仍可读且有未完成批次，先按生产状态修复/取证处理，不能直接 restore 覆盖。退役擦除同样必须停 daemon 后执行 `xingshu ops wipe`，服务明确 active 或数据库仍有未完成批次时不能用确认参数绕过。
3. 物理急停触发 → 急停信号到 UI 提示 ≤ 1s；复位流程验证。
4. 密钥泄露 → 停止 daemon → 确认没有未完成批次 → 备份 SQLite → `xingshu key generate --db <path> --yes` → `xingshu key rekey-integration-tasks --db <path> --old-key-file <old.env> --new-key-file <new.env> --dry-run` → 确认后 `--yes` 提交 → 用新 `XINGSHU_DB_ENCRYPTION_KEY` 重启 daemon → 验证历史 integration task 可读；若 daemon 明确 active 或数据库仍有未完成批次，key generate / rekey 正式提交不能用确认参数绕过。
5. Modbus TCP 中断 → 上位机降级显示"PLC offline"。

## 13. 参考

- `docs/upper_computer_rk_deployment_acceptance_guide.md`
- `docs/upper_computer_maintenance_manual.md`
- `docs/upper_computer_security_key_lifecycle.md`
- `docs/upper_computer_static_cutover.md`
- `xingshu ops --help` / `xingshu key --help`
