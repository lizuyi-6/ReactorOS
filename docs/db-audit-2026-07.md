# 数据库审计发现(2026-07-16)

检查对象:`data/reactor.sqlite3`(用 IDEA MCP database 工具查 + 代码核实 + cargo test 验证)。

## ✅ 健康
- 孤儿记录(全表外键完整):`control_events` / `sensor_samples` / `product_results` / `process_steps` **全部 0 孤儿**
- 审计链(有 hash 的事件)`broken_links`:**0**
- `emergency_stop` 代码 fail-closed 正确(engage → audit hash → 审计失败 latch_control_fault)

## ⚠️ 发现 + 定性

### 1. device_write/emergency_stop 历史无 event_hash(已修,非当前 bug)
- `device_write` 12642 条 + `emergency_stop` 3 条 + 少量 process/v1_control,`event_hash IS NULL`
- **定性:旧版代码残留**。当前 `insert_control_event_sqlx`(db.rs:2269)无条件计算 hash;device_write(main.rs:576)、emergency_stop(api.rs:4353)都走它;4 处 `INSERT INTO control_events`(2248/2319/5270/5331)全部 hash
- **验证**:`audit_writes_use_sqlx_pool_without_breaking_chain` 测试 pass + `safety_invariants::all_control_event_inserts_compute_hash` 守卫
- db 最新数据 2026-06-02(2 个多月前),当前代码(07-16)没在新数据上产生过 NO_HASH

### 2. sensor_samples pressure_kpa 死列
- `pressure_kpa` 列存在但 **211394 行全为 0**(数据直接录在 `pressure_mpa`,不是从 kPa 迁移来的)
- 代码只在 db.rs:606 `has_legacy_pressure_kpa = column_exists(...)` 做兼容检测,不写不读
- **DROP 安全**(无数据丢,代码不依赖)

### 3. 2 个未完成批次堆积
- batch 84(新工艺)、86(最终管线验证工艺),`finished_at IS NULL`
- daemon 异常退出遗留,runtime_recovery recovered active(86)但 84 没告警

## 本次改进
- `tests/safety_invariants.rs`:加 `all_control_event_inserts_compute_hash` 守卫(防以后加不 hash 的 insert 路径,CI 红)
- `src/main.rs`:启动加未完成批次堆积告警(extra > 0 warn)

## 建议(运维决策,需确认后执行)
- **DROP pressure_kpa**:`ALTER TABLE sensor_samples DROP COLUMN pressure_kpa`(SQLite 3.35+,当前 3.51;列空安全)。或在 `migrate` 里加 guarded DROP(数据迁移后)
- **历史无 hash 数据**:接受(只读归档)。不回填 —— hash 链依赖顺序,回填会破坏现有链。查询区分:`SELECT ..., CASE WHEN event_hash IS NULL THEN 'pre_hash_legacy' ELSE 'hashed' END FROM control_events`
- **未完成批次**:启动告警已加(本次);可考虑 >N 自动告警外发(MQTT/AINAS)
