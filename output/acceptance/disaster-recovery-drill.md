# 备份 / 恢复演练报告

- 时间: `2026-06-06T17:45:32+08:00`
- 提交: `659e9195`
- 演练 DB: `/x/tianhks/output/acceptance/drill.sqlite3`
- 备份文件: `/x/tianhks/output/acceptance/drill.backup.sqlite3`
- 审计链事件数: **5** (灾前) → **5** (灾后)
- 最终结果: **PASS**

## 步骤

1. 启 daemon @ 127.0.0.1:18400 (`/x/tianhks/output/acceptance/logs/drill-daemon.log`) → ✓
2. 工程师登录拿 token
3. 记下灾前审计事件数: `5`
4. 备份 DB (`/x/tianhks/output/acceptance/drill.backup.sqlite3`)
5. 停 daemon，`xingshu ops wipe` 覆盖主文件 + WAL/SHM/key
6. `xingshu ops restore` 复制回主文件
7. 重启 daemon @ 127.0.0.1:18400
8. 记下灾后审计事件数: `5`

## 验证结论

✅ 演练通过：灾前灾后审计链事件数一致 (`5`)。

## 后续行动

- 复盘 `/x/tianhks/output/acceptance/logs/drill-wipe.log` 和 `/x/tianhks/output/acceptance/logs/drill-restore.log`
- 把演练纳入季度复盘（PRD §10）
