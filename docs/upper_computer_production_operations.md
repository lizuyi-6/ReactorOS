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
Restart=always
RestartSec=5
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

密钥轮换：用 `xingshu key generate --db <path> --yes`（见 `docs/upper_computer_security_key_lifecycle.md`）。该命令只生成新的 `<db>.key` 文件并设权限 0600，不会重加密现有 ciphertext 行；操作员需在 daemon 停止期间重导出 `XINGSHU_DB_ENCRYPTION_KEY` 后再启动。

## 4. watchdog / 健康检查

建议叠加两层 watchdog：

- **systemd 自重启**：`Restart=always` + `RestartSec=5`，处理崩溃和 OOM。
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
   写入审计链。

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
# 每日 02:00 cron 备份（须在 daemon 停止期间，或搭配维护窗口）
# 注意：当前实现是 fs::copy，不是 SQLite backup API；运行中调用可能
# 留下半事务状态。生产侧建议每周用 systemd 的 --quiet 停 5 分钟跑一次。
xingshu ops backup --db /opt/xingshu/data/reactor.sqlite3 \
  --out /opt/xingshu/backups/$(date +%Y%m%d).sqlite3.snapshot

# 季度全量归档（脱机存储）
xingshu ops backup --db /opt/xingshu/data/reactor.sqlite3 \
  --out /mnt/nfs/xingshu-archive/$(date +%Y%m%d).sqlite3.snapshot

# 退役设备前安全擦除（只擦 SQLite 主文件；WAL/SHM/backup/key 需手动清）
xingshu ops wipe --db /opt/xingshu/data/reactor.sqlite3 --yes
shred -vzn 3 /opt/xingshu/backups/*.snapshot
rm -f /opt/xingshu/data/reactor.sqlite3-wal /opt/xingshu/data/reactor.sqlite3-shm
rm -f /opt/xingshu/data/reactor.sqlite3.key
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

- 升级前先 `xingshu ops backup` 一次 SQLite 文件快照（daemon 必须停止）。
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
2. 数据库被损坏 → `xingshu ops restore` 从备份恢复。
3. 物理急停触发 → 急停信号到 UI 提示 ≤ 1s；复位流程验证。
4. 密钥泄露 → `xingshu key generate --db <path> --yes` → 重启 daemon → 验证新写入。注意旧 ciphertext 行将不可读，需离线脚本迁移。
5. Modbus TCP 中断 → 上位机降级显示"PLC offline"。

## 13. 参考

- `docs/upper_computer_rk_deployment_acceptance_guide.md`
- `docs/upper_computer_maintenance_manual.md`
- `docs/upper_computer_security_key_lifecycle.md`
- `docs/upper_computer_static_cutover.md`
- `xingshu ops --help` / `xingshu key --help`
