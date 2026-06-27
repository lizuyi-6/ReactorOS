# 星宿智能反应釜上位机 RK 平台部署与验收指南

日期：2026-06-04

适用范围：RK3568/RK3588 工业边缘设备上的上位机部署、启动、接口联调和验收取证。

相关文档：`docs/lubancat2_debian10_deploy.md` 已覆盖 LubanCat 2 RK3568 Debian 10 打包、安装、systemd 和 kiosk 运行步骤。本文档面向 PRD 交付验收，补充验收证据、外部接口、安全和本地 AI 边界。

## 1. 交付边界

当前本地上位机已经能在 PC 上运行并通过 HMI/CLI/API 自测。RK 平台正式验收必须单独证明：

- ARM64 release 包可在 RK3568/RK3588 上启动。
- `reactor-edge-daemon`、HMI 静态资源、SQLite、配置文件和 systemd 服务可持续运行。
- STM32/RS485、Modbus TCP、MQTT、AINAS 或第三方 REST 能按现场配置联通。
- 不含模型时资源满足 PRD：内存 < 30MB，单核 CPU 稳态 < 3%。
- 本地 Qwen3.5-2B + LoRA 若纳入验收，必须补真实模型文件、推理入口、训练/转换脚本和 RK 延迟报告。

## 2. PC 侧构建

Windows PowerShell：

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-lubancat2-debian10.ps1
```

WSL/Linux/macOS with Docker：

```bash
./scripts/build-lubancat2-debian10.sh
```

构建完成后记录：

- 构建机器系统和时间。
- Git 提交或工作区版本说明。
- 包名和 SHA256。
- `dist/latest-lubancat2-debian10-package.txt` 内容。

## 3. RK 设备准备

在 RK 设备上安装运行依赖：

```bash
sudo apt-get update
sudo apt-get install -y ca-certificates libudev1 curl x11-xserver-utils
sudo apt-get install -y chromium || sudo apt-get install -y chromium-browser
```

可选：

```bash
sudo apt-get install -y unclutter
```

记录设备信息：

```bash
uname -a
cat /etc/os-release
lscpu
free -m
df -h
ip addr
```

## 4. 安装和启动

复制包到 RK：

```bash
scp dist/reactor-os-lubancat2-rk3568-debian10-chromium-kiosk-*.tar.gz cat@BOARD_IP:/home/cat/
```

安装：

```bash
tar -xzf reactor-os-lubancat2-rk3568-debian10-chromium-kiosk-*.tar.gz
cd reactor-os-lubancat2-rk3568-debian10-chromium-kiosk-*
sudo ./install.sh --install-deps
```

检查服务：

```bash
systemctl status reactor-edge
systemctl status reactor-edge-backup.timer
systemctl list-timers reactor-edge-backup.timer
systemctl status reactor-os-chromium
journalctl -u reactor-edge --no-pager -n 100
journalctl -u reactor-edge-backup.service --no-pager -n 50
```

验收通过条件：

- `reactor-edge` 为 active。
- `reactor-edge-backup.timer` 为 active，并能在 `systemctl list-timers` 中看到下一次触发时间。
- HMI 可访问 `http://127.0.0.1:8000/`。
- 日志无启动级 panic、端口占用或配置解析错误。

## 5. 基础健康检查

```bash
curl http://127.0.0.1:8000/health
curl http://127.0.0.1:8000/api/config/summary
curl http://127.0.0.1:8000/api/devices/status
```

预期：

- `/health` 返回 `{"ok":true,"service":"reactor-edge-daemon"}`。
- `/api/config/summary` 返回权限、安全、集成、数据加密和本地 AI readiness 状态。
- 无真实样本时 `/api/live` 可能返回 503，这是传感器新鲜度保护，不作为启动失败。

## 6. 配置落地检查

正式部署前检查：

| 配置 | 文件或变量 | 验收要求 |
| --- | --- | --- |
| 设备和 Modbus RTU | `/etc/reactor-edge/device.toml` | 串口、slave_id、寄存器映射与 STM32 手册一致 |
| 安全边界 | `/etc/reactor-edge/safety.toml` | 温度、转速、步长、禁区符合项目安全表 |
| 集成接口 | `/etc/reactor-edge/integration.toml` | MQTT、Modbus TCP、证书路径、topic 或端口正确 |
| 数据库加密 | `XINGSHU_DB_ENCRYPTION_KEY` | 生产密钥由托管流程注入，不写入源码 |
| 登录和权限 | `XINGSHU_AUTH_SECRET`、角色密码变量 | 默认密码已替换，登录审计可追踪 |
| 本地 AI | `XINGSHU_LOCAL_AI_*` | 没有真实模型时保持 `ready_for_lora_inference=false` 和 `ready_for_prd_lora=false`，不得宣称 LoRA 完成 |

## 7. 外部接口验收

按 `docs/upper_computer_external_acceptance_checklist.md` 执行以下编号：

| 编号 | RK 侧动作 |
| --- | --- |
| P0-01 | 使用真实 STM32/RS485 验证 RTU 读写 |
| P0-04 | 启用 Modbus TCP，使用 Modbus Poll/Slave 验证功能码和 TLS |
| P0-05 | 启用 MQTT，使用 MQTT.fx/mosquitto 验证任务、回执、报警和断线重连 |
| P0-06 | 使用 AINAS 真实平台下发任务并核对审计 |

Modbus 寄存器表见 `docs/upper_computer_modbus_register_map.md`。

## 8. RK 性能验收

不含本地模型时采样：

```bash
pid=$(pidof reactor-edge-daemon)
ps -p "$pid" -o pid,comm,%cpu,rss,vsz
top -b -n 5 -p "$pid"
curl http://127.0.0.1:8000/health
```

建议至少采样：

- 启动后 5 分钟。
- 连续样本流运行 30 分钟。
- 第三方接口联调期间。
- 72 小时或 30 天稳定性测试期间。

通过标准：

- 不含模型时内存 < 30MB。
- 单核 CPU 稳态 < 3%。
- Web 响应 < 1s。
- 长时运行无崩溃、无数据丢失。

## 8.1 自动备份验收

release package 已包含 `/opt/reactor-edge/current/bin/xingshu`、`/opt/reactor-edge/current/backup.sh`、`reactor-edge-backup.service` 和 `reactor-edge-backup.timer`。安装后 `/opt/reactor-edge/bin` 和 `/opt/reactor-edge/backup.sh` 作为兼容链接指向当前 slot。RK 上需执行：

```bash
systemctl status reactor-edge-backup.timer
systemctl list-timers reactor-edge-backup.timer
sudo systemctl start reactor-edge-backup.service
ls -lh /var/lib/reactor-edge/backups
sha256sum -c /var/lib/reactor-edge/backups/latest.snapshot.sha256
```

通过标准：

- 手动启动 `reactor-edge-backup.service` 后生成 `reactor.sqlite3.<时间>.snapshot`。
- 同目录存在 `.sha256` sidecar 和 `latest.snapshot` 链接。
- `sha256sum -c` 校验通过。
- 恢复演练必须先停止 `reactor-edge`，替换数据库后再启动服务；现场恢复、保留策略和异地归档仍需单独验收记录。

## 9. 本地 Qwen3.5-2B + LoRA 验收

当前上位机提供 readiness、推理入口、训练数据集导出、训练入口编排、manifest 和显式候选 adapter 晋级/备份边界，但不声明真实本地 LoRA 已完成。若要在 RK 上验收本地 AI，必须提供：

- Qwen3.5-2B 量化模型或 GGUF 文件。
- LoRA adapter。
- llama.cpp 或等效推理二进制。
- PEFT/LoRA 训练脚本。
- GGUF 转换脚本。
- RK3568/RK3588 推理延迟报告。
- 训练后 manifest、评估、晋级和回滚报告。

验收时检查：

```bash
curl http://127.0.0.1:8000/api/config/summary
xingshu ai model
xingshu ai train --export-only --dataset output/local-ai/rk-lora-dataset.jsonl
xingshu ai train --dataset output/local-ai/rk-lora-dataset.jsonl --manifest output/local-ai/rk-train.manifest.json --dry-run
xingshu ai train --dataset output/local-ai/rk-lora-dataset.jsonl --manifest output/local-ai/rk-train.manifest.json --promote --min-eval-score 0.8
```

通过标准：

- `ready_for_base_inference=true` 只证明基础模型入口存在，不能单独作为 LoRA 完成证据。
- `ready_for_lora_inference=true` 且真实推理延迟 < 3s。
- `ready_for_training=true` 且训练、manifest、评估、显式替换、备份回滚链路可执行。
- `ready_for_prd_lora=true`，并附 RK 延迟报告。
- 不能用占位文件或只读 SOP 草案替代真实 LoRA 推理/训练。

## 10. 验收归档

建议归档到：

```text
output/acceptance/rk-deployment/
```

至少包含：

- `acceptance-summary.md`
- 构建包 SHA256。
- RK 设备信息。
- `/etc/reactor-edge/*.toml` 脱敏副本。
- systemd 状态和 journal 日志。
- HMI 截图。
- API 响应 JSON。
- 性能采样记录。
- 外部接口工具截图和日志。
- 未关闭问题及复测记录。
