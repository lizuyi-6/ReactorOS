# ReactorOS Project Overview

> 简版导览,完整内容看仓库根 `README.md` 和 `docs/` 下的产品/协议文档。本文件是给 reins 干活前的速读材料。

## 一句话定位

跑在树莓派 / LubanCat 2 (RK3568 ARM64) / 通用 A55 Debian 10 上的**反应釜边缘上位机**。`reactor-edge-daemon` 单进程,采集 + 安全限幅 + SQLite + AI 记忆 + HTTP API + 静态 Web 上位机 + Qt HMI 客户端。

## 模块边界 (reins 各自 Own)

| 目录 | Own 的 rein | 不要碰除非你是它 |
|---|---|---|
| `src/`、`tests/`、`config/`、`Cargo.toml` | `reactor-daemon-expert` | 其他人 |
| `firmware/esp32_reactor_bridge/` | `reactor-firmware-expert` | 其他人 |
| `qt-client/` | `reactor-qt-expert` | 其他人 |
| `static/`、`e2e/`、`kiosk/`(启动 / Chromium 部分)、`playwright.config.mjs` | `reactor-hmi-expert` | 其他人 |
| `scripts/`、`deploy/`、`Dockerfile`、`docker-compose.yml`、`target-*-arm64/`、`dist/` | `reactor-build-expert` | 其他人 |
| 跨模块改 | 拆给 2+ reins,各自 Own 自己那块 | — |

## 关键约定 (产品级硬约束,不能违反)

1. **不在板子上编译** — LubanCat 2 / A55 性能低,电脑侧 Docker 交叉编译
2. **真实优先** — 没有 `state.json` / ESP32 / 外部管线数据时,`/api/live` 返回 503,前端显示空值和错误码,**不静默造假**
3. **StepFun step-3.6 demo key** 是客户演示私钥,只能进 demo 包,**不能进公开产物**
4. **急停 / 限幅** 走 daemon 的安全层 + 审计,**不在前端绕过**
5. **协议帧**改格式是 breaking change,daemon + 固件 + 文档三边同步
6. **依赖少** — 后端 Rust 是单二进制,前端是单页 HTML/JS,无打包器
7. **生产硬件** — 控制 / 安全的任何改动先想"会不会让设备超温超压",改 safety.toml 尤其要慎

## 常用命令速记

```powershell
# 后端
cargo run -- --config config/device.toml --safety config/safety.toml --memory config/ai_memory.toml --db data/reactor.sqlite3 --assets static --bind 127.0.0.1:8000
cargo test

# 前端
npm run frontend:dev          # vite dev
npm run frontend:build        # vite build
npm run e2e                   # playwright 全量
npm run simulate:device       # 灌传感器样本
npm run simulate:device:once  # 灌一帧

# 构建 / 部署
docker compose up --build reactor-edge
npm run qemu:lubancat2        # 模拟 LubanCat 2
npm run qemu:lubancat2:smoke  # smoke
powershell -ExecutionPolicy Bypass -File scripts\build-lubancat2-debian10.ps1
```

## 跟 reins 说的话

- 你是 reins,先看自己 `agent.md` 的 Scope / Don't own
- 跨模块的活别自己拍板,回 orchestrator 让 ta 拆
- 改完要跑 `cargo test` / `npm run e2e` / 本地 docker compose,verify 通过才算"完成"
- 写完报告改了哪些文件、影响哪些对端、build/test 状态
