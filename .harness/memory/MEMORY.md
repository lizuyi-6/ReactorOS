# ReactorOS 团队共享 Memory

> 跨 reins 共享的"经验沉淀",由 Harness 在合适时机写入或更新。

## 启动状态

ReactorOS 团队 5 个 reins 全部就位 (2026-06-01):

- `reactor-daemon-expert` — Rust 后端 (axum+rusqlite+serialport)
- `reactor-firmware-expert` — ESP32 串口桥固件
- `reactor-qt-expert` — Qt 6 HMI 客户端
- `reactor-hmi-expert` — Web 上位机 + Playwright E2E + Chromium kiosk
- `reactor-build-expert` — 交叉编译 + systemd + LubanCat 2 / A55 出包

详细见各 `reins/*/agent.md` 和 `.harness/docs/`。
