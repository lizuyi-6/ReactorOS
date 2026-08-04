# 虚拟传感器数据源

## 功能目的

为前端开发、产品演示、系统联调和异常场景测试提供一套完整的虚拟传感器数据源。虚拟传感器从后端数据接入层注入，经过与真实传感器完全相同的解析、清洗、聚合、缓存、规则判断、告警和推送管线。前端始终调用正式接口（`/api/live`、WebSocket），不感知数据来自真实传感器还是虚拟传感器。

## 架构设计

```
真实模式：   真实传感器 → ReactorDevice → 控制循环 → DB/RuntimeState → API/WebSocket → 前端
模拟模式：   VirtualSensorDevice → 控制循环 → RuntimeState → API/WebSocket → 前端
```

`VirtualSensorDevice` 实现了 `ReactorDevice` trait，是 `DeviceMode::Simulation` 对应的设备实现。控制循环以相同方式轮询 `read_sample_and_status()`，虚拟传感器返回的 `SensorSnapshot` 经过 `validate_sensor_snapshot()` 校验后写入 `RuntimeState`，由 `/api/live`、WebSocket 等正式接口暴露给前端。

核心模块：
- `src/virtual_sensor.rs` — 虚拟传感器设备、场景系统、会话管理
- `src/config.rs` — `DeviceMode::Simulation` 枚举、`SimulationConfig` 配置
- `src/device.rs` — `build_device()` 工厂方法
- `src/state.rs` — `SensorSourceType` 来源标记
- `src/api.rs` — 模拟控制接口
- `src/main.rs` — 控制循环集成、数据持久化门控

## 如何启用

### 方式一：配置文件启动（推荐）

使用模拟设备配置文件启动 daemon：

```powershell
reactor-edge-daemon --config config/device.simulation.toml
```

或覆盖默认配置路径：

```powershell
reactor-edge-daemon --config config/device.simulation.toml --bind 127.0.0.1:8000
```

### 方式二：现有配置文件中切换

在 `config/device.toml` 中设置：

```toml
mode = "simulation"

[simulation]
scenario = "normal"
seed = 20260804
interval_ms = 1000
speed = 1.0
persist_data = false
```

## 如何关闭

将配置文件中的 `mode` 改回 `"pipeline"`（或 `"esp32_serial"` / `"json_bridge"` / `"modbus"`），重启 daemon。

或通过 API 停止模拟会话（仅暂停数据生成，不切换设备模式）：

```bash
curl -X POST http://127.0.0.1:8000/api/simulation/stop -H "Authorization: Bearer <admin-token>"
```

## 场景系统

内置 10 个场景，通过 `scenario` 字段或 API 切换：

| 场景 | 说明 | 关键参数 |
|------|------|----------|
| `normal` | 正常运行，微小噪声波动 | `noise` |
| `slow_rise` | 温度缓慢上升 | `start_value`, `target_value`, `period_seconds` |
| `slow_fall` | 温度缓慢下降 | `start_value`, `target_value`, `period_seconds` |
| `sudden_spike` | 突变跳变后恢复 | `start_value`(触发tick), `target_value`(峰值), `period_seconds`(持续tick) |
| `out_of_range` | 产生超出有效范围的值 | `target_value` |
| `frozen_value` | 数值冻结不变 | `start_value` |
| `sensor_disconnect` | N 个 tick 后停止产生数据 | `start_value`(断连前tick) |
| `noisy_signal` | 高噪声信号 | `noise` |
| `intermittent_data` | 交替产生和缺失数据 | `start_value`(有数据tick), `target_value`(无数据tick) |
| `recovery` | 从故障值恢复到正常 | `start_value`(故障持续tick), `target_value`(故障温度) |

## 配置项

`[simulation]` 配置段：

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `scenario` | string | `"normal"` | 场景名称 |
| `seed` | u64 | `42` | 随机种子，相同种子产生相同序列 |
| `interval_ms` | u64 | `1000` | 数据生成间隔（毫秒），信息性字段 |
| `speed` | f64 | `1.0` | 模拟速度倍率 |
| `duration_seconds` | u64? | 无 | 模拟持续时间（秒），到期自动停止 |
| `persist_data` | bool | `false` | 是否持久化到 sensor_samples 表 |
| `parameters` | object | `{}` | 场景参数（见上表） |

## API 示例

### 查询模拟状态

```bash
curl http://127.0.0.1:8000/api/simulation/status
```

```json
{
  "code": 0,
  "message": "success",
  "data": {
    "active": true,
    "session_id": "sim-1722768000000",
    "scenario": "normal",
    "seed": 20260804,
    "interval_ms": 1000,
    "speed": 1.0,
    "tick": 42,
    "elapsed_seconds": 42.3,
    "last_sample": { "temperature_c": 45.12, ... },
    "source_type": "simulation"
  }
}
```

### 启动/恢复模拟

```bash
curl -X POST http://127.0.0.1:8000/api/simulation/start \
  -H "Authorization: Bearer <admin-token>"
```

### 停止模拟

```bash
curl -X POST http://127.0.0.1:8000/api/simulation/stop \
  -H "Authorization: Bearer <admin-token>"
```

### 切换场景

```bash
curl -X POST http://127.0.0.1:8000/api/simulation/scenario \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{
    "scenario": "sudden_spike",
    "seed": 999,
    "parameters": {
      "start_value": 10,
      "target_value": 150,
      "period_seconds": 5
    }
  }'
```

## 生产环境限制

- 模拟控制接口 (`/api/simulation/*`) 需要 **admin 权限**
- `persist_data` 默认为 `false` — 模拟数据不会写入 `sensor_samples` 表
- 生产部署应使用 `mode = "pipeline"` 或真实设备模式，不应使用 `simulation`
- 非模拟模式下调用模拟控制接口返回 400 Bad Request
- `/api/live` 响应中 `runtime.source_type` 字段标识当前数据来源 (`"real"` / `"simulation"`)

## 如何新增场景

在 `src/virtual_sensor.rs` 的 `generate_for_scenario()` 函数中添加新的 match 分支：

```rust
"my_custom_scenario" => {
    let temp = ctx.parameters.start_value.unwrap_or(50.0);
    Some(make_sample(
        temp + ctx.rng.gen_range(-noise..noise),
        // ... 其他字段
    ))
}
```

同时在 `validate_scenario_name()` 的 `VALID` 数组和 `available_scenarios()` 中注册场景名。

## 如何运行测试

```powershell
# 仅虚拟传感器测试
scripts\cargo-x.ps1 test --test virtual_sensor_tests

# 模块内单元测试
scripts\cargo-x.ps1 test --lib virtual_sensor

# 全部测试
scripts\cargo-x.ps1 test
```

## 如何确认前端使用的是完整正式管线

1. 启动 daemon 时使用 `config/device.simulation.toml`
2. 打开前端 `http://127.0.0.1:8000/`
3. 检查 `/api/live` 返回的 `runtime.source_type` 为 `"simulation"`
4. 监控页面应显示实时变化的传感器数值
5. 模拟数据通过 `ReactorDevice::read_sample_and_status()` → `validate_sensor_snapshot()` → `RuntimeState` 管线
6. 前端代码未做任何修改，使用原有 API 调用和数据转换逻辑

## 数据持久化

- **默认不持久化**：`persist_data = false` 时，控制循环跳过 `db.insert_sample_sqlx()`，模拟数据仅更新 `RuntimeState` 供实时展示
- **可选持久化**：设置 `persist_data = true` 后，模拟数据写入 `sensor_samples` 表，可用于历史趋势分析
- **来源标记**：`RuntimeState.source_type` 字段标识数据来源，序列化在 `/api/live` 响应中

## 完整启动示例

```powershell
# 使用过热场景演示
reactor-edge-daemon --config config/device.simulation.toml --bind 127.0.0.1:8000

# 启动后切换到过热场景
curl -X POST http://127.0.0.1:8000/api/simulation/scenario \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"scenario": "slow_rise", "seed": 20260804, "parameters": {"start_value": 35, "target_value": 95, "period_seconds": 120}}'
```

## 异常场景示例

模拟传感器断连：

```bash
# 切换到断连场景（5个采样后停止产生数据）
curl -X POST http://127.0.0.1:8000/api/simulation/scenario \
  -H "Authorization: Bearer <admin-token>" \
  -H "Content-Type: application/json" \
  -d '{"scenario": "sensor_disconnect", "parameters": {"start_value": 5}}'
```

前端监控页面将先显示正常数据，随后出现"等待外部数据"提示和传感器错误码，验证系统的 fail-closed 行为。
