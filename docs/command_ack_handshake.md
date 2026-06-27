# 命令下发握手协议契约(命令级 ACK)

日期：2026-06-27

适用范围：星宿上位机(`reactor-edge-daemon`)向下位控制器下发目标参数时的命令级确认握手。对象包括 ESP32 串口桥、JSON Bridge、Modbus RTU 主站,以及外部数据管线(不适用)。

本文档供**上位机 + 下位机固件双方**对齐协议契约。上位机侧实现见 `src/device.rs` 的 `write_targets_acknowledged` 与 `src/main.rs` 的 `control_loop`;安全语义见 `docs/architecture-deviations.md`。

## 1. 为什么需要握手

`write_targets` 在 ESP32 / JSON Bridge 模式下返回 `Ok` **只代表字节已发出 / 文件已写盘**,不代表下位机收到并执行。确认原本要靠下一轮采样读回 `last_command_ok`,存在一个采样间隔的盲区,且回执可能滞后误关联。

命令级握手把确认改成**本次命令本次确认**:每条命令带唯一 `request_id`,下位机处理后回显式 ACK,上位机收到**匹配 rid 且成功**的 ACK 才算完成;否则在 `command_ack_timeout_ms` 后 fail-closed 闭锁自动控制。

## 2. request_id 规则

- **生成方**:上位机。`control_loop` 内单调计数生成,格式 `auto-<timestamp_ms>-<seq>`(见 `src/main.rs` 的 `command_seq`)。
- **传给下位机**:随命令帧/控制文件下发。
- **回填要求**:下位机在 ACK / 状态回执中**原样回填同一个 rid**。上位机严格匹配:`request_id` 不符的回执视为陈旧,忽略,继续等待。
- **字符集**:ESP32 帧里 rid 出现在 `rid=...` 字段,固件应原样回填(不做改写、截断或重排)。

## 3. ESP32 串口

### 3.1 命令帧(上位机 → 下位机,握手路径)

```
<command_prefix>|v=1|rid=<request_id>|heat_time=<f>|hold_time=<f>|cool_time=<f>|target_temp=<f>|stir_speed=<f>|shake_speed=<f>|target_pressure=<f>|chk=<hex>\n
```

- 与 legacy 命令帧(`build_esp32_command`,无 `rid`)的区别:**多了 `rid=<request_id>` 字段**,位于 `v=1` 之后。
- `chk` 为整行(除 `chk=` 段外)的 checksum(同样本帧算法,见 `checksum_hex`)。`checksum` 配置关闭时省略。
- **固件识别**:含 `rid=` 字段的命令帧必须回 ACK;无 `rid=` 的 legacy 命令帧可不回 ACK(上位机 `write_targets` 不等待)。

### 3.2 ACK 帧(下位机 → 上位机)

```
<frame_prefix>|v=1|type=ack|rid=<request_id>|ok=<0|1>[|err=<text>]|chk=<hex>\n
```

- `<frame_prefix>` 与样本帧前缀相同(下位机→上位机方向)。
- `type=ack` 标识 ACK 帧(上位机据此区分 ACK 与样本帧)。
- `ok=1` 已接受并执行;`ok=0` 拒绝(越限 / 忙 / 故障),此时应附 `err=<简短原因>`。
- `chk` 同命令帧算法。
- **超时契约**:上位机等待 `command_ack_timeout_ms`(默认 2000,可配)。固件应在该窗口内回 ACK,否则上位机判 Timeout 并 fail-closed。

### 3.3 半双工时序

ESP32 串口为半双工。握手期间上位机在持锁的串口上:发命令帧 → 在 cloned reader 上(独立 100ms read timeout)读行,跳过样本帧,直到读到匹配 rid 的 ACK 或超时。**固件无需特殊半双工处理**,只要在处理完命令后正常发送 ACK 帧(会与样本帧交错,上位机能识别)。

## 4. JSON Bridge

JSON Bridge 协议层**已具备握手所需全部字段**(`config/integration.toml` 关联的 state.json / control.json):

### 4.1 control.json(上位机 → 下位机)

```json
{ "request_id": "<rid>", "command": "<atomic>", "value": <json>, "name": "<opt>" }
```

上位机在握手路径用传入 rid 覆盖 control.request_id(见 `JsonBridgeDevice::write_targets_acknowledged`)。

### 4.2 state.json(下位机 → 上位机)回填契约

下位机处理 control.json 后**必须及时回填**以下字段:

| 字段 | 含义 |
| --- | --- |
| `last_command_request_id` | 最近处理的 control 的 `request_id`(原样回填) |
| `last_command_ok` | `true` 已接受执行 / `false` 拒绝 |
| `last_command_error` | `ok=false` 时的原因;`ok=true` 时可省略或 null |

上位机轮询 state.json(约 50ms 间隔),直到 `last_command_request_id == 本次 rid` 且 `last_command_ok.is_some()`,或 `command_ack_timeout_ms` 超时。

**及时性要求**:固件应在远小于 `command_ack_timeout_ms`(默认 2000ms)的时间内回填——建议 < 500ms,留出通信与轮询余量。

## 5. Modbus RTU

Modbus FC06(写单寄存器)**本身就是 request-response**:从站应答即传输层 ACK。握手额外做**读回验证**:

1. 上位机写 `target_temperature_c` / `target_stirrer_rpm` 目标寄存器(FC06)。
2. 立即读回这两个目标寄存器(FC03)。
3. 比对读回的 raw word == 写入的 raw word:
   - 全部匹配 → `Confirmed`,`accepted_targets` 填读回的工程值。
   - 任一不符 → `Rejected`(从站 clamp / 覆盖 / 拒绝写入)。
   - FC06 exception code → 调用返回 `Err`(上位机 fail-closed)。

**固件要求**:目标寄存器必须可读(作为 holding register 暴露),否则读回失败 → `Err` → 闭锁。Modbus 无 `request_id` 概念(事务 id 由 tokio-modbus 内部管理),rid 仅用于审计关联。

## 6. 配置

`safety.toml` 的 `[control]` 段:

| 字段 | 默认 | 说明 |
| --- | --- | --- |
| `require_command_ack` | `false` | `true`:要求握手,ACK 超时/拒绝即 fail-closed;`false`:legacy fire-and-forget。**生产应设 `true`**,`xingshu ops preflight --production` 会检查。 |
| `command_ack_timeout_ms` | `2000` | 握手等待窗口。需覆盖下位机处理 + 通信往返;过小过度闭锁,过大失去意义。 |

## 7. 失败语义(fail-closed)

| ACK 结果 | 上位机动作 |
| --- | --- |
| `Confirmed` | 记 `device_write` 审计(含 rid + ack 状态);命令标记为已下发 |
| `Rejected(reason)` | `latch_control_fault` + `auto_enabled=false` + `device_write_rejected` 审计 |
| `Timeout` | `latch_control_fault` + `auto_enabled=false` + 设 retry_after 退避 + `device_write_unconfirmed` 审计 |
| `Unverified`(设备未实现握手,且 `require_command_ack=true`) | `latch_control_fault`(配置错误)+ `device_write_unconfirmed` 审计 |
| `Err`(IO 错误) | 同 legacy `device_write_failed`:latch + 退避 + 审计 |

**第二道防线**:现有的"下一轮 `last_command_ok` / device_status 超时检测"保留,覆盖 ACK 帧本身丢失但命令已执行等边角。

## 8. 未覆盖(需现场)

- ESP32 固件侧 ACK 帧实现(本文档为契约,固件由硬件团队实现)。
- 真机时序实测:ACK 往返延迟分布(决定现场 `command_ack_timeout_ms` 调参)、半双工 ACK 与样本帧交错密度。
- Modbus 读回的"合法 clamp vs 错误"判定阈值(需与固件方对齐寄存器语义)。
- 跨进程重启的 pending-ack 持久化(当前 timeout 在单次调用内,不跨重启)。
