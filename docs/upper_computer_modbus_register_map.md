# 星宿智能反应釜上位机 Modbus 寄存器映射手册

日期：2026-06-04

适用范围：当前上位机默认 Modbus RTU/TCP 映射、CLI 调试和第三方接口联调。

重要边界：本文档记录的是上位机当前默认映射，来源于 `config/device.toml`、`src/modbus_registers.rs` 和 `src/modbus_tcp.rs`。正式交付前必须与 STM32 固件最终寄存器手册、传感器量程、执行器单位和现场缩放系数逐项确认。

## 1. 通信参数

默认 RTU 参数：

| 参数 | 当前值 |
| --- | --- |
| 串口 | `/dev/ttyUSB0` |
| 波特率 | 9600 |
| 校验 | `N` |
| 停止位 | 1 |
| 数据位 | 8 |
| 超时 | 1000ms |
| 从站地址 | 1 |

默认 Modbus TCP 参数：

| 参数 | 当前值 |
| --- | --- |
| 监听地址 | `0.0.0.0:502` |
| Unit ID | 1 |
| TLS | 默认要求 TLS，证书路径见 `config/integration.toml` |
| 最大 PDU | 253 bytes |
| 当前默认状态 | disabled，需要显式启用 |

## 2. 数值编码规则

读寄存器解码：

```text
engineering_value = raw * scale + offset
```

写寄存器编码：

```text
raw = round((engineering_value - offset) / scale)
```

上位机会拒绝无法编码到 `u16` 的值。写目标寄存器还会经过 RBAC、目标范围、步长、安全禁区、急停、人工锁、传感器新鲜度、上一次控制写入故障和审计链路；无法证明现场状态安全时默认拒绝写入，而不是缓存目标等待后续自动执行。

## 3. Holding Registers 读点位

| 名称 | 地址 | 单位 | scale | offset | 有效范围 | 说明 |
| --- | ---: | --- | ---: | ---: | --- | --- |
| `temperature_c` | 0 | degC | 0.1 | 0.0 | 0.0 到 250.0 | 当前反应温度 |
| `stirrer_rpm` | 1 | rpm | 1.0 | 0.0 | 0.0 到 2000.0 | 当前搅拌转速 |
| `pressure_mpa` | 2 | MPa | 0.01 | 0.0 | 0.0 到 10.0 | 当前釜内压力 |
| `shake_speed_cpm` | 3 | cpm | 1.0 | 0.0 | 0.0 到 60.0 | 当前摇罐速度 |
| `tilt_angle_deg` | 4 | deg | 0.01 | -45.0 | -45.0 到 45.0 | 当前倾角 |
| `flow_rate_l_min` | 5 | L/min | 0.01 | 0.0 | 0.0 到 20.0 | 当前流量 |
| `product_concentration_percent` | 6 | percent | 0.1 | 0.0 | 0.0 到 100.0 | 当前产物浓度 |
| `ph` | 7 | pH | 0.01 | 0.0 | 0.0 到 14.0 | 当前 pH |

## 4. Holding Registers 写点位

| 名称 | 地址 | 单位 | scale | offset | 说明 |
| --- | ---: | --- | ---: | ---: | --- |
| `target_temperature_c` | 10 | degC | 0.1 | 0.0 | 目标温度 |
| `target_stirrer_rpm` | 11 | rpm | 1.0 | 0.0 | 目标搅拌转速 |
| `target_shake_speed_cpm` | 12 | cpm | 1.0 | 0.0 | 目标摇罐速度 |
| `target_pressure_mpa` | 13 | MPa | 0.01 | 0.0 | 目标压力 |
| `heat_time_s` | 14 | s | 1.0 | 0.0 | 加热阶段时长 |
| `hold_time_s` | 15 | s | 1.0 | 0.0 | 保温阶段时长 |
| `cool_time_s` | 16 | s | 1.0 | 0.0 | 冷却阶段时长 |

## 5. Coils

| 名称 | 地址 | 访问 | 说明 |
| --- | ---: | --- | --- |
| `auto_enabled` | 0 | read/write | 自动控制开关 |
| `manual_lock` | 1 | read/write | 人工锁定 |
| `emergency_stop` | 2 | read/write | 急停状态 |
| `process_running` | 3 | read | 当前是否有活动批次 |

## 6. Discrete Inputs

| 名称 | 地址 | 访问 | 说明 |
| --- | ---: | --- | --- |
| `device_connected` | 0 | read | 设备健康在线状态；生产默认 `require_device_status_for_control=true` 时必须有下位机状态，且 connected、last_frame_ok、last_seen 均健康，并且 `last_command_ok` 未报告失败；实验/非严格模式下仅允许用 `sensor_timeout_ms` 内的新鲜样本兜底，过期样本仍按离线处理 |
| `sensor_fresh` | 1 | read | 传感器数据是否在 `sensor_timeout_ms` 内 |
| `alarm_active` | 2 | read | 统一报警是否存在；与 `/api/live`/MQTT `alarms` 使用同一逻辑，覆盖急停、样本缺失/过期、严格模式下位机状态缺失/异常、下位机命令失败、锁存控制故障和 hard limit 报警 |
| `tilt_state` | 3 | read | 倾角状态是否触发 |
| `active_batch` | 4 | read | 是否存在活动批次 |

设备控制写入失败后，daemon 会锁存 `last_control_error`、关闭自动控制，并让
`alarm_active` 保持为 true。`alarm_active` 不只是急停/控制故障摘要；只要统一
报警数组非空，PLC 侧都应按报警处理。传感器恢复、急停复位或人工锁切换不会自动清除
锁存控制故障；现场确认执行器链路恢复后，通过 `POST /api/control/fault/reset`
或 `xingshu control fault-reset` 显式复归。
传感器样本缺失/过期或下位机状态断连、帧校验失败、状态过期时，daemon 会关闭
`auto_enabled` 并记录现场输入故障；新样本恢复后不会自动重新开启自动控制。
如果下位机仍报告 `last_command_ok=false`，控制故障复归会被拒绝，`device_connected` 也保持 false，PLC/第三方系统必须按设备不健康处理。

## 7. 支持的 Modbus TCP 功能码

| 功能码 | 名称 | 当前支持 |
| --- | --- | --- |
| `01` | Read Coils | 支持 |
| `02` | Read Discrete Inputs | 支持 |
| `03` | Read Holding Registers | 支持 |
| `06` | Write Single Holding Register | 支持 |

暂未声明支持 `04`、`05`、`15`、`16` 等功能码。需要第三方系统使用这些功能码时，应先扩展实现和测试。

Modbus TCP 会校验 MBAP 头中的 Unit ID，必须与 `config/integration.toml` 中的 `modbus_tcp.unit_id` 一致；Unit ID 不匹配时返回异常响应，不执行读写寄存器，避免多设备或网关场景下把其他站号的写入误落到本机运行态。

## 8. 上位机调试入口

REST：

```text
GET  /api/modbus/registers
GET  /api/modbus/registers/:name/read
POST /api/modbus/registers/:name/write
```

CLI：

```powershell
xingshu modbus map
xingshu modbus read temperature_c
xingshu modbus write target_temperature_c 65 --reason "acceptance test"
```

写入示例中 `target_temperature_c=65` 会被编码为 raw `650`，并且必须通过安全链路。HTTP REST 调试写入口仅允许 admin bearer session，且请求体必须提供非空 `reason`，避免 engineer 经调试路径绕过常规 `set_targets` 审计上下文。若急停、人工锁、传感器超时或上一次控制写入失败未清除，Modbus 目标写入会返回拒绝，现场需先进入维护排障或恢复新鲜样本。

## 9. 正式联调待确认项

| 项目 | 当前状态 | 正式交付前动作 |
| --- | --- | --- |
| STM32 寄存器地址 | 上位机默认值已定义 | 与 STM32 固件最终手册逐项核对 |
| 单位和缩放系数 | 上位机默认值已定义 | 用真实传感器/执行器标定结果修订 |
| RTU 实机读写 | 待外部验收 | 记录 RS485 读写日志、CRC 错误和异常恢复 |
| Modbus TCP TLS | 本地自签证书测试通过 | 使用生产证书链和 Modbus Poll/Slave 验收 |
| 多品牌兼容 | 待外部验收 | 输出第三方系统兼容性矩阵 |
