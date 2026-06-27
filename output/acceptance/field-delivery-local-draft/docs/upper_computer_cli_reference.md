# 星宿智能反应釜上位机 CLI 命令参考手册

日期：2026-06-06

对象：`xingshu` 上位机命令行工具。

边界说明：CLI 复用 Web HMI 和 REST API 的同一安全链路，不绕过 RBAC、安全限幅、急停、人工锁、传感器新鲜度和审计。当前 `xingshu ai train` 已具备本地 LoRA 数据集导出、训练入口编排、manifest 归档和显式候选 adapter 晋级/备份边界；但这不代表真实 Qwen3.5-2B + LoRA 模型资产、生产训练脚本、推理链路和 RK 验收已经完成。

## 1. 全局参数

```powershell
xingshu --help
xingshu --json <command>
xingshu --api http://127.0.0.1:8000 <command>
xingshu --token <bearer-token> <command>
xingshu --db data/reactor.sqlite3 <command>
```

| 参数 | 说明 |
| --- | --- |
| `--api` | REST API 地址，默认 `http://127.0.0.1:8000` |
| `--token` | bearer session token，受保护写入和导出使用 |
| `--db` | 本地 SQLite 路径，默认 `data/reactor.sqlite3` |
| `--json` | 输出 JSON，便于脚本和验收归档 |

## 2. 启停和状态

| 命令 | 说明 | 安全边界 |
| --- | --- | --- |
| `xingshu start` | 前台启动 `reactor-edge-daemon` | 使用当前配置和启动参数 |
| `xingshu stop` | 安全停止当前反应流程并关闭自动控制 | 走 REST API 和审计 |
| `xingshu status` | 查看服务、设备、运行时和模型状态 | 只读 |

示例：

```powershell
xingshu status
xingshu --json status
```

## 3. 认证

| 命令 | 说明 |
| --- | --- |
| `xingshu auth login --username engineer --password engineer123` | 登录并输出 token |
| `xingshu auth me --token <token>` | 查看当前 token 角色和权限 |

生产必须通过环境变量覆盖默认密码：

- `XINGSHU_OPERATOR_PASSWORD`
- `XINGSHU_ENGINEER_PASSWORD`
- `XINGSHU_ADMIN_PASSWORD`

## 4. 配置

| 命令 | 说明 |
| --- | --- |
| `xingshu config` | 从 API 读取运行配置摘要 |
| `xingshu config --local` | 从本地 TOML 读取配置摘要 |
| `xingshu config --local --json` | 输出本地配置 JSON |

验收重点：

- `data_security.storage_encryption`
- `local_ai.ready_for_base_inference`
- `local_ai.ready_for_lora_inference`
- `local_ai.ready_for_inference`（兼容字段，等同 LoRA 推理闭环）
- `local_ai.ready_for_training`
- `local_ai.ready_for_prd_lora`
- `integrations.mqtt_status`
- `integrations.modbus_tcp_status`
- `permissions.authentication`

## 5. 数据

| 命令 | 说明 | 注意事项 |
| --- | --- | --- |
| `xingshu data list` | 列出最近批次 | 只读 |
| `xingshu data export --out batches.csv` | 导出批次 CSV | 需要有运行中服务 |
| `xingshu data export-xlsx --out batches.xlsx` | 导出 Excel 工作簿 | 需要有运行中服务 |
| `xingshu data report --batch-id 1` | 导出单批次 Markdown 报告 | 指定批次必须存在 |
| `xingshu data sample --duration-s 180 --interval-ms 500` | 通过正式 v1 样本入口注入演示样本 | 不写控制目标，不绕过 safety |
| `xingshu data delete --yes` | 删除本地 SQLite 运行数据 | 破坏性操作，仅本地验收或重置演示环境使用 |

`data sample` 停止后，超过 `sensor_timeout_ms` 没有新样本时 `/api/live` 返回 503 是预期安全行为。

## 6. 控制

| 命令 | 说明 | 安全边界 |
| --- | --- | --- |
| `xingshu control set --temp 60 --rpm 300` | 设置目标温度和搅拌转速 | RBAC、安全限幅、步长、禁区、审计 |
| `xingshu control set --temp 60 --rpm 300 --shake 24` | 同时设置摇罐速度 | 同上 |
| `xingshu control start --process-id 1` | 启动已保存工艺流程 | 拒绝急停、人工锁、已有活动批次、无新鲜样本 |
| `xingshu control start --name demo --temp 60 --rpm 300` | 创建基础批次并启动 | 同上 |
| `xingshu control stop` | 停止当前流程 | 写停止审计 |
| `xingshu control estop` | 触发急停 | 最高优先级 |
| `xingshu control estop --reset` | 复位急停 | 需符合现场安全流程 |

生产现场禁止用 CLI 绕过操作 SOP；CLI 与 HMI 使用同一安全路径。

## 7. AI

| 命令 | 说明 | 当前状态 |
| --- | --- | --- |
| `xingshu ai suggest` | 获取或生成最新参数建议 | 本地传统优化器/云端 provider 边界可用 |
| `xingshu ai plan` | 生成只读实验 SOP 草案 | 本地通过，不写控制 |
| `xingshu ai model` | 查看 AI provider、memory 和 local_ai readiness | 本地通过 |
| `xingshu ai train --export-only --dataset lora.jsonl` | 从真实 SQLite 批次、产品结果、样本和审计事件导出监督训练 JSONL | 本地通过，不需要模型资产 |
| `xingshu ai train --dataset lora.jsonl --manifest train.manifest.json --dry-run` | 调用 `XINGSHU_LOCAL_AI_TRAIN_SCRIPT` 并写训练 manifest | 需配置训练脚本、GGUF 和转换脚本 |
| `xingshu ai train --dataset lora.jsonl --promote --min-eval-score 0.8` | 训练脚本返回候选 adapter 和评估分数达标时，备份当前 `XINGSHU_LOCAL_AI_LORA` 并晋级候选 adapter | 必须显式 `--promote`；无分数、无候选文件或低于阈值会拒绝晋级 |

`xingshu ai train` 的训练脚本输出建议包含：

```json
{
  "status": "ok",
  "evaluation": { "score": 0.91, "metrics": { "loss": 0.12 } },
  "artifacts": { "adapter_path": "C:\\models\\candidate-adapter.gguf" }
}
```

真实 AI 交付仍需 Qwen3.5-2B/GGUF、LoRA adapter、生产 PEFT 训练脚本、推理入口、RK 延迟报告和真实批次效果验证。
其中 `ready_for_base_inference` 只能说明基础模型入口可用；只有 `ready_for_prd_lora=true` 才能作为 PRD 本地 LoRA/RK 侧证据。

## 8. 审计

| 命令 | 说明 |
| --- | --- |
| `xingshu audit list` | 查看审计事件 |
| `xingshu audit list --event-type modbus_register_write` | 按事件类型筛选 |
| `xingshu audit export --out audit.csv` | 导出审计 CSV |

审计日志带 hash chain 状态，生产需配合备份、防删和归档策略。

## 9. Modbus

| 命令 | 说明 |
| --- | --- |
| `xingshu modbus map` | 查看寄存器、coils、discrete inputs 映射 |
| `xingshu modbus read temperature_c` | 读取一个映射点位 |
| `xingshu modbus write target_temperature_c 65 --reason "acceptance test"` | 写一个可写点位 |

写入会经过安全链路和审计。寄存器详细表见 `docs/upper_computer_modbus_register_map.md`。

## 10. Safety Guard

| 命令 | 说明 |
| --- | --- |
| `xingshu safety check --temp 999 --rpm 9999 --shake 99 --pressure 99 --guard reactor-safety-guard` | 使用独立 safety guard 进程夹紧目标 |

该命令用于本地验收独立安全进程，不等同于真实执行器响应验收。

## 11. 性能冒烟

| 命令 | 说明 |
| --- | --- |
| `xingshu perf smoke --iterations 20 --api-threshold-ms 100 --safety-threshold-ms 100` | 测量只读 API 往返和本地安全计算 |

当前性能冒烟不证明 STM32/RS485 采集延迟、真实执行器控制延迟、LoRA 推理延迟、7x24 或 MTBF。

## 12. 运维预检

| 命令 | 说明 |
| --- | --- |
| `xingshu ops preflight --production --json` | 检查生产上线前的本地配置、默认口令、session secret、数据库加密 key、MQTT/Modbus TLS 路径和备份 timer 文件 |
| `xingshu ops backup --db data/reactor.sqlite3 --out backups/reactor.sqlite3.snapshot` | 使用 SQLite `VACUUM INTO` 生成在线快照 |
| `xingshu ops restore --backup backups/reactor.sqlite3.snapshot --db data/reactor.sqlite3 --yes` | 校验 SQLite magic/schema/integrity 后恢复；必须停 daemon |
| `xingshu ops wipe --db data/reactor.sqlite3 --yes` | 覆盖并删除 DB、WAL/SHM/JOURNAL、`<db>.key` 和 sibling `backups/` 中匹配快照 |
| `xingshu key generate --db data/reactor.sqlite3 --yes` | 生成新的 `XINGSHU_DB_ENCRYPTION_KEY` 文件，不在输出中泄露 key |
| `xingshu key rekey-integration-tasks --db data/reactor.sqlite3 --old-key-file old.env --new-key-file data/reactor.key --dry-run` | 离线扫描并预检 `integration_tasks.request_json/response_json` 迁移计数；确认后用 `--yes` 正式重加密 |

`ops preflight --production` 有 fail 级发现时返回非 0。默认口令、缺失 `XINGSHU_AUTH_SECRET`、缺失或无效 `XINGSHU_DB_ENCRYPTION_KEY`、启用 MQTT/Modbus 但 TLS 文件缺失都会失败。MQTT/Modbus disabled 或 `device.mode=pipeline` 会给 warning，用于保留本地联调能力，但正式 RK/现场交付需逐项解释。

示例：

```powershell
$env:XINGSHU_AUTH_SECRET = "<32+ chars production secret>"
$env:XINGSHU_OPERATOR_PASSWORD = "<production password>"
$env:XINGSHU_ENGINEER_PASSWORD = "<production password>"
$env:XINGSHU_ADMIN_PASSWORD = "<production password>"
$env:XINGSHU_DB_ENCRYPTION_KEY = "<64 hex chars or base64 32 bytes>"
xingshu ops preflight --production --json
```

该命令不替代正式 CA/企业 CA 证书链验收、broker 联调、密钥托管/轮换演练或安全扫描。

## 13. 验收建议

CLI 验收至少保存：

- `xingshu --help`
- `xingshu data --help`
- `xingshu control --help`
- `xingshu ai --help`
- `xingshu modbus --help`
- `xingshu ops preflight --production --json`
- 关键命令的 `--json` 输出
- 对应审计导出

当前已验证 `data` help 暴露 `export-xlsx`、`sample` 和 `delete`。
