# LubanCat 2 部署反馈处理账本（2026-07-19，部分完成）

反馈输入：`ReactorOS-部署问题反馈-2c672f7f.md`，对应旧包
`reactor-os-lubancat2-rk3568-debian10-chromium-kiosk-20260718-164354-2c672f7f`。

替代包：`reactor-os-lubancat2-rk3568-debian10-chromium-kiosk-20260719-134831-2c672f7f.tar.gz`，sha256
`d36f70e56e6bfa0c2139d20e60a210a05449bbab8b89f0bd27f8a632b6cc57f9`。该包同时包含 Monitor 反应釜搅拌轴固定、仅桨叶绕自身中心旋转的前端纠偏。

## 反馈逐项处理

1. backend boot-check 权限：`reactor-edge.service` 的语义从“boot-check 随 `cat` 运行并失败”改为“仅 `ExecStartPre` 用 systemd `+` 前缀以 root 运行，daemon 仍为 `cat`”。
2. 网络认证密钥：安装器在无强密钥时生成独立 `reactor-edge.auth.env`（64 hex、0600）；已有强密钥和 StepFun 环境文件不覆盖，重复安装不轮换。
3. 旧 SQLite 迁移：先补列、再保守修复重复幂等键、最后建索引；旧批次和所有 integration task 行保留。缺 `id` 主键的未知 schema 明确拒绝自动迁移，避免猜测式丢数据。
4. 离线 kiosk：不再主动 `Wants/After=graphical.target`，改为等待 `display-manager.service`；不 mask `systemd-time-wait-sync`，避免破坏审计时钟同步语义。
5. demo seed：安装 drop-in 改用 daemon 实际读取的 `XINGSHU_SEED_DEMO_CONTEXT=true`；真实 daemon 测试断言新库得到 2 工艺、6 批次、6 结果和 1 推荐。
6. Chromium 日志：launcher 仅在真实 session bus socket 存在时设置 DBus 地址，否则清除无效地址；GPU 保持启用，RK-MPP/EGL 非致命提示未通过禁用 GPU 掩盖。

## 可复现证据

- `bash scripts/verify-install-board-preflight.sh`：exit 0；安装/重复安装、密钥文件、demo drop-in 与 unit 源码门禁通过。
- `node scripts/verify-ota-systemd-boot-gate.mjs`：exit 0。
- `node scripts/verify-ota-ab-release-path.mjs`：exit 0。
- `migration_adds_legacy_integration_columns_before_indexes_and_preserves_batches`：1 passed；旧批次名、迁移后任务字段和唯一索引均作字段断言。
- `migration_preserves_duplicate_legacy_tasks_and_keeps_earliest_idempotency_record`：1 passed；两行均保留，id=1 保留 external ID，id=2 仅清除重复 external ID，重放仍返回 id=1。
- `daemon_environment_flag_seeds_demo_processes_batches_and_recommendation`：1 passed；断言 `(processes,batches,results,recommendations)==(2,6,6,1)`。
- release 包内全仓 `cargo test`：451 passed、0 failed、0 ignored。
- `bash scripts/verify-packaged-lubancat2-deployment-fixes.sh <tar>`：exit 0；从 tar 解包后在隔离根目录安装两次，字段断言全部通过。
- 三个二进制均为 stripped ARM aarch64 ELF，最高 glibc symbol version 均为 `GLIBC_2.28`。

## 未覆盖范围

- 反馈板 `192.168.100.2` 在本轮不可达（SSH connect timeout），新包尚未在该实体板安装。
- 未用现场 `.old-*` SQLite 文件复演；回归 fixture 覆盖反馈描述的缺列和重复活动任务两类失败，但未知第三方旧 schema 仍可能触发明确拒绝。
- 未验证板上 systemd 241 对 `ExecStartPre=+` 的现场日志、LightDM 实际启动顺序、真实旧库迁移耗时、kiosk DBus 日志量和 A/B 回滚。
- 包元数据仍为 `GIT_SHA=2c672f7f`、`GIT_DIRTY=true`；tar/前端分别有 sha256 锚点，但尚无包含本轮修复的 commit。
