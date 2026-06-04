# 星宿智能反应釜上位机 CLI 命令参考手册

日期：2026-06-04

对象：`xingshu` 上位机命令行工具。

边界说明：CLI 复用 Web HMI 和 REST API 的同一安全链路，不绕过 RBAC、安全限幅、急停、人工锁、传感器新鲜度和审计。当前 `xingshu ai train` 只用于暴露本地 LoRA 训练缺口，不代表真实 Qwen3.5-2B + LoRA 训练已完成。

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
- `local_ai.ready_for_inference`
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
| `xingshu ai train` | 检查本地 LoRA 训练是否可用 | 当前按预期失败并列出缺失资产 |

`xingshu ai train` 要变成真实训练能力，仍需 Qwen3.5-2B/GGUF、LoRA adapter、PEFT 训练脚本、转换脚本和 RK 验收报告。

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

## 12. 验收建议

CLI 验收至少保存：

- `xingshu --help`
- `xingshu data --help`
- `xingshu control --help`
- `xingshu ai --help`
- `xingshu modbus --help`
- 关键命令的 `--json` 输出
- 对应审计导出

当前已验证 `data` help 暴露 `export-xlsx`、`sample` 和 `delete`。
