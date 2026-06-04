# 星宿智能反应釜上位机测试报告

测试对象：李祖祎负责的上位机软件。

测试日期：2026-06-04。

测试环境：

- 工作区：`X:/tianhks`
- 本地服务：`http://127.0.0.1:8000/`
- 最近验证目标目录：`target-tls-check`
- 数据库：`data/reactor.sqlite3`
- 静态资源：`static`
- 集成配置：`config/integration.toml`

## 1. 测试范围

PRD 第八章测试计划与团队分工测试职责的逐项追踪见 `docs/upper_computer_test_plan_traceability.md`。

| 范围 | 状态 |
| --- | --- |
| Web HMI 七大页面 | 已测主体流程，含中英切换视觉验证 |
| RBAC 登录与权限 | 已由 API/CLI 测试覆盖 |
| 安全控制 | 已覆盖目标写入、急停、人工锁、传感器超时、步长/范围 |
| 温度-转速安全禁区 | 已覆盖 `forbidden_control_zones` 自动控制阻断和手动目标拒绝 |
| 本地 GA/SA/PID 参数寻优 | 已覆盖 `local-ga-sa-pid` 本地优化器边界、禁区避让和策略说明 |
| 批次与数据导出 | 已覆盖 CSV/XLSX/Markdown 报告 |
| 审计日志 | 已覆盖审计链、审计导出和写操作记录 |
| AI 实验 SOP 草案 | 已覆盖只读 `GET /api/ai/experiment-plan`，验证三段式 SOP、安全说明和不写控制事件 |
| AINAS REST 任务 | 已覆盖 `set_targets`、`start_process`、`stop_process` |
| MQTT bridge | 已覆盖配置、CA/客户端证书字段解析、payload 执行、receipt 持久化、alert 快照；外部 broker 未验收 |
| Modbus 映射 | 已覆盖 8 读、7 写、coils、discrete inputs |
| Modbus TCP PDU/MBAP/TLS | 已覆盖 `01/02/03/06` PDU 处理、本地真实 TCP/MBAP 客户端读请求、本地 TLS 握手 + MBAP 读请求；外部 Modbus Poll/Slave 未验收 |
| HTTP/HTTPS 入口 | 已覆盖 CLI 参数和本地自签证书启动；`https://127.0.0.1:18443/health` 返回健康状态 |
| 本地性能冒烟 | 已覆盖本机只读 API 往返和安全计算延迟；真实硬件链路、LoRA、7x24 不在本次证明范围 |
| 本地 LoRA | 未实现，测试不通过项记录为产品缺口 |

## 2. 自动化测试结果

最近一次相关测试命令：

```powershell
$env:CARGO_TARGET_DIR='target-alert-check'
cargo fmt --check
cargo test --test api_tests -- --nocapture --test-threads=1
cargo test --test cli_tests -- --nocapture
cargo test --test db_tests -- --nocapture
cargo test --test control_tests -- --nocapture
cargo test --test config_tests -- --nocapture
```

结果：

| 测试集 | 结果 |
| --- | --- |
| `cargo fmt --check` | 通过 |
| `api_tests` | 42 passed |
| `cli_tests` | 10 passed |
| `db_tests` | 6 passed |
| `control_tests` | 4 passed |
| `config_tests` | 5 passed |

Windows 下偶发 Cargo incremental 目录 finalizing 权限警告，但测试结果为通过。

本轮新增针对性测试：

| 测试 | 结果 |
| --- | --- |
| `cargo test --test control_tests control_blocks_forbidden_temperature_stirrer_zone -- --nocapture` | 通过 |
| `cargo test --test api_tests operator_target_update_rejects_forbidden_temperature_stirrer_zone -- --nocapture` | 通过 |
| `cargo test --test optimizer_tests -- --nocapture` | 通过，4 passed |
| `cargo test --test cli_tests xingshu_perf_help_exposes_smoke_check -- --nocapture` | 通过 |

## 3. 本地性能冒烟

执行命令：

```powershell
& 'C:\tmp\xingshu-target-cli-perf\debug\xingshu.exe' --json perf smoke `
  --iterations 20 `
  --api-threshold-ms 100 `
  --safety-threshold-ms 100 `
  --guard 'C:\tmp\xingshu-target-cli-perf\debug\reactor-safety-guard.exe'
```

输出归档：`output/upper-computer-perf-smoke.json`。

| 项目 | 样本数 | p95 | 结论 |
| --- | --- | --- | --- |
| `/health` | 20 | 4ms | 通过 |
| `/api/config/summary` | 20 | 1ms | 通过 |
| `/api/devices/status` | 20 | 0ms | 通过 |
| `/api/live` 轻量请求 | 20 | 0ms | 往返通过；当前无外部 pipeline 样本，业务状态为 503 |
| `safety_compute` | 20 | 1ms | 通过 |
| `safety_guard_process_spawn` | 20 | 315ms | 诊断项；包含 Windows 进程启动和 stdin/stdout JSON 往返，不用于 <100ms 安全计算结论 |

结论：本机只读 API 往返和安全计算满足 <100ms 冒烟阈值。该结果不证明 STM32/RS485 采集延迟、真实执行器控制延迟、RK 端 Qwen/LoRA 推理训练延迟、7x24、MTBF 或外部 broker/工具性能。

资源快照：

| 项目 | 结果 | 结论 |
| --- | --- | --- |
| Windows debug 演示进程 working set | 26.977MB | 本地快照低于 30MB |
| Windows debug 演示进程 private memory | 6.102MB | 记录 |
| CPU 5 秒采样 max | 1.533% | 本地快照低于 3% |

输出归档：`output/upper-computer-resource-snapshot.json`。该快照不等价于 release/RK3568/RK3588 长时间稳态验收。

## 4. Web 视觉验证

已使用本地浏览器验证：

| 截图 | 验证点 |
| --- | --- |
| `output/ainas-integration-en-visible-final.png` | AINAS/集成接口英文状态可见 |
| `output/ainas-integration-zh-visible-final.png` | AINAS/集成接口中文状态可见 |
| `output/modbus-expanded-points-zh-visible.png` | 扩展 Modbus 点位中文可见 |
| `output/mqtt-integration-status-visible.png` | MQTT 状态中文可见 |
| `output/modbus-tcp-integration-status-visible.png` | Modbus TCP 状态中文可见 |
| `output/upper-computer-modbus-en-recheck.png` | 本轮复核英文 Modbus/集成动态字块 |
| `output/upper-computer-modbus-zh-recheck.png` | 本轮复核中文 Modbus/集成动态字块 |
| `output/upper-computer-mqtt-alert-modbus.png` | MQTT alert 补强后重启 daemon 并复核 Modbus/集成区 |
| `output/upper-computer-modbus-tcp-stream-reload.png` | Modbus TCP 网络流测试后重启 daemon 并复核 Modbus/集成区 |
| `output/visual-i18n/*.png` | 监控、控制、审计、Modbus、设置等页面中英切换回归截图 |
| `output/upper-computer-i18n-monitor-en-final.png` | 最终英文监控页截图 |
| `output/upper-computer-i18n-monitor-zh-final.png` | 最终中文监控页截图 |
| `output/upper-computer-i18n-modbus-en-final.png` | 最终英文 Modbus/集成状态截图 |
| `output/upper-computer-i18n-modbus-zh-final.png` | 最终中文 Modbus/集成状态截图 |
| `output/upper-computer-aes-modbus-en-playwright.png` | AES-256 静态加密状态英文可见 |
| `output/upper-computer-aes-modbus-zh-playwright.png` | AES-256 静态加密状态中文可见，字段名可切换 |
| `output/upper-computer-sop-zh.png` | AI 实验 SOP 草案中文状态，摘要、步骤、边界说明均完成切换 |
| `output/upper-computer-sop-en.png` | AI 实验 SOP 草案英文状态，切回英文后无中文残留 |

截图和文字审计索引见 `docs/upper_computer_visual_evidence_index.md`。

结论：静态文本与主要动态字块均能随语言切换刷新；AI SOP 草案不再出现词级替换导致的中英混文；Modbus 页面窄屏/纵向滚动问题已修复，底部集成接口区可达。
本轮最终浏览器巡检覆盖 `Monitor / Batches / Control / AI Lab / History / Alarms / Audit / Modbus / Settings` 九个页面：英文模式无可见中文残留，中文模式九页均有中文界面文本。

## 5. API 验收项

| 测试项 | 期望 | 结果 |
| --- | --- | --- |
| `GET /api/config/summary` | 返回设备、安全、AI、权限、集成摘要和 `data_security.storage_encryption` | 通过 |
| `POST /api/auth/login` | 默认用户可登录并返回 token | 通过 |
| `GET /api/auth/me` | token 可解析角色和权限 | 通过 |
| `POST /api/control/targets` | 目标写入受安全限幅和审计保护 | 通过 |
| `POST /api/processes/:id/start` | 拒绝急停、人工锁、已有活动批次、无新鲜传感器数据 | 通过 |
| `POST /api/processes/:id/stop` | 停止流程并写审计事件 | 通过 |
| `GET /api/ai/experiment-plan` | 基于批次结果和安全边界生成只读 SOP 草案，不写控制事件 | 通过 |
| `GET /api/modbus/registers` | 返回读/写寄存器、coils、discrete inputs 和 TCP 状态 | 通过 |
| `POST /api/modbus/registers/:name/write` | 只允许可写寄存器且走安全路径 | 通过 |
| `POST /api/integrations/ainas/tasks` | 任务持久化并执行 | 通过 |
| `GET /api/integrations/ainas/tasks/:id` | 可查询执行状态和回执 | 通过 |

## 6. CLI 验收项

| 命令 | 结果 |
| --- | --- |
| `xingshu status` | 可获取服务、设备、联锁和 AI 状态 |
| `xingshu config --local --json` | 可读取本地 device/safety/integration 摘要 |
| `xingshu data export` / `export-xlsx` / `report` | 支持数据导出 |
| `xingshu data sample --duration-s 180 --interval-ms 500` | 可通过正式 v1 样本入口驱动无硬件实时监控演示 |
| `xingshu data delete --yes` | 支持清理本地 SQLite 运行数据，需显式确认 |
| `xingshu ai plan` | 支持查看安全门控实验 SOP 草案 |
| `xingshu perf smoke` | 支持生成本地 API 和安全计算性能冒烟报告 |
| `xingshu audit list` / `export` | 支持审计查询和导出 |
| `xingshu modbus map/read/write` | 支持寄存器映射、读写调试 |
| `xingshu ai train` | 按预期报告 LoRA 训练接口未开放 |

无硬件实时监控演示验证：

- `xingshu data sample --duration-s 180 --interval-ms 500` 启动后，`GET /api/live?sample_limit=1&include_processes=false&include_batches=false&include_events=false` 返回 200。
- 浏览器显示 `SYSTEM HEALTH: NORMAL`、`DEVICES 1/1 IDLE`、实时温度 `35.9degC`、压力 `0.47MPa`，控制台 0 error。
- 截图归档：`output/upper-computer-hmi-live-sample-final.png`。
- 停止样本流后，超过 `sensor_timeout_ms=6000` 再返回 503 属于安全新鲜度检查预期行为。

## 7. Modbus 测试

当前 `/api/modbus/registers` 映射：

- 8 个读寄存器：温度、搅拌转速、压力、摇罐速度、倾角、流量、产物浓度、pH。
- 7 个写寄存器：目标温度、目标搅拌转速、目标摇罐速度、目标压力、加热时间、保温时间、冷却时间。
- coils/discrete inputs：运行、自动控制、急停、人工锁、传感器状态等。

Modbus TCP 测试覆盖：

| 功能码 | 状态 |
| --- | --- |
| `01` Read Coils | 通过 |
| `02` Read Discrete Inputs | 通过 |
| `03` Read Holding Registers | 通过 |
| `06` Write Single Holding Register | 通过 |
| 本地 TCP/MBAP 客户端读请求 | 通过 |
| 本地 TLS/MBAP 客户端读请求 | 通过 |

未完成：502 端口正式监听、Modbus Poll/Slave 外部工具联调、现场真实证书链验收。

## 8. MQTT 测试

自动化测试已覆盖：

- `integration.toml` 默认 MQTT disabled + TLS/8883 + CA/客户端证书模板。
- MQTT task payload 解析。
- 无效 JSON 报错。
- 有效任务复用 AINAS 执行路径。
- 任务 receipt 持久化。
- alert topic retained 报警快照生成。

未完成：

- 外部 broker 连接。
- MQTT.fx 验收。
- 断线重连和 backoff 验收。
- MQTT.fx / 生产 broker 证书链验收。

## 9. 安全测试

已覆盖：

- 控制目标范围。
- 单次步长限制。
- 温度-转速组合禁区：`forbidden_control_zones` 会在自动控制决策中阻断，也会在手动目标、AI 执行、AINAS 任务、Modbus 调试写入、工艺步骤和 v1 控制入口拒绝禁区组合。
- 急停阻断。
- 人工锁阻断。
- 传感器超时阻断。
- RBAC 权限。
- 审计链。
- 第三方任务复用安全路径。
- 本地 GA/SA/PID 参数寻优：`src/optimizer.rs` 在历史批次和 AI memory 边界内执行 GA 风格交叉/变异、SA 接受/降温搜索和 PID 风格误差校正，推荐仍受安全边界和 forbidden zones 限制。
- HTTP HTTPS 入口参数和本地自签证书健康检查。
- Modbus TCP TLS 本地握手和 MBAP 读请求。
- AES-256-GCM 集成任务请求/回执静态加密；原始 SQLite 列不含请求/回执明文。
- 加密开启后仍可兼容读取历史明文任务行。
- 密钥生命周期与敏感字段清单已文档化：`docs/upper_computer_security_key_lifecycle.md`。
- 独立 `reactor-safety-guard` 进程 stdin/stdout JSON 协议。
- `xingshu safety check` 通过独立进程完成目标夹紧验收。
- daemon 暴露 `--safety-guard` 参数，可委托自动控制安全判定并保留进程内回退。
- `xingshu perf smoke` 显示本地 `safety_compute` p95=1ms；独立进程启动往返作为诊断项记录为 p95=315ms。

未完成：

- MQTT 证书链、Modbus TCP 外部工具 TLS 验收。
- safety guard 生产 watchdog、权限隔离、故障演练。
- 生产密钥托管/轮换演练、全量敏感字段验收签字和正式安全报告。
- 正式渗透/漏洞扫描报告。

## 10. 结论

上位机主体功能、Web HMI、CLI、REST API、数据管理、审计、基础 AINAS/MQTT/Modbus 接口已经达到本地 PoC/联调验收状态。

剩余外部验收项已经拆成可执行清单，见 `docs/upper_computer_external_acceptance_checklist.md`。该清单覆盖 STM32/Modbus RTU 实机、Modbus Poll/Slave、MQTT.fx/mosquitto、AINAS 真实平台、Qwen3.5-2B + LoRA、RK 性能、7x24、RS485 丢包率、安全扫描、多浏览器/移动端和用户签字验收。

不能宣称完全满足 PRD 的部分：

- 本地 Qwen3.5-2B + LoRA 自进化。
- MQTT/Modbus TCP 外部证书链验收。
- AES-256 已覆盖第三方集成任务载荷，密钥生命周期/敏感字段清单已文档化；仍需生产密钥托管/轮换演练和验收签字。
- 独立 safety guard 已具备本地能力，本地安全计算冒烟通过；仍需生产 watchdog、权限隔离和故障演练。
- 外部 MQTT.fx、Modbus Poll/Slave、第三方上位机系统验收。
- 真实 STM32/反应釜整机联调。

下一步建议优先级：

1. 用真实或仿真 STM32 固化 Modbus 地址与缩放系数。
2. 启用实验室 Modbus TCP 端口并用 Modbus Poll/Slave 做外部验收。
3. 接入外部 MQTT broker，用 MQTT.fx 做任务下发和回执验收。
4. 完成 MQTT 证书链和 Modbus TCP 外部工具 TLS 验收。
5. 明确 LoRA 模型部署边界，由算法负责人提供模型与训练管线后接入上位机。
