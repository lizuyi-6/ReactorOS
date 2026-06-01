# ReactorOS Coding Standards

> 给 reins 干活时参考的代码规约。**只列关键约束**,具体风格看各模块已有代码。

## Rust (`src/`、`tests/`)

- **不要**引入新重量级依赖,后端追求"单二进制、依赖少"
- 错误用 `anyhow` / `thiserror`,**不要** `unwrap()` 出现在生产路径(测试 OK)
- 并发用 `tokio`,不引 `async-std` / `smol`
- 串口读写用 `serialport` crate,不要自己写 syscall 包装
- SQLite 迁移:`db.rs` 集中管理,**不要**在每个 query 里临时建表
- API 端点 (`api.rs`):
  - 错误用结构化 JSON + HTTP 状态码,**不要**裸字符串
  - 写操作必须产生审计事件 (`audit_events` 表)
  - 真实设备写入必经 `control.rs` 安全限幅
- 测试 (`tests/`):
  - 每个新功能补一个集成测试,不要只写 `#[cfg(test)]` 单元测试
  - 协议解析类必须有边界用例 (空字段、超长、校验失败)

## ESP32 Arduino (`firmware/`)

- 单 `.ino` 文件,别拆成多 .cpp 再拼
- 串口帧格式严格按 `docs/esp32_protocol.md`,**不要**改字段顺序
- 校验和 (`chk=`) 改动时 daemon 端解析必须同步
- `Serial.println` 用于调试 OK,但**不要**留 `Serial.println` 打印完整帧在生产代码
- 二值倾角 (`tilt_state=0|1`) 是硬件上报的,曲线拟合是 daemon 端软件做,**不要**在固件里拟合

## Qt C++ (`qt-client/`)

- Qt 6,跟 qmake (`reactor-os-qt.pro`) 走,别上 CMake
- 与 daemon 通信走 HTTP API (axum 提供),别自己造协议
- **不要**绕过 daemon 直接写设备 / 串口 — 急停/限幅必须在 daemon 侧做
- UI 控件命名按 Qt 习惯 (`pushButton_stop` 之类),不改风格
- 没有 in-tree 测试框架,改动后至少要 `qmake` + `make` 通过

## Web (`static/`)

- 原生 HTML/CSS/JavaScript,**不要**引入框架 / 打包器
- 数据从 `/api/live`、`/api/devices/status`、`/api/v1/...` 拉
- 没有数据时显示空值 + 错误码,**不造假** (这是产品约束)
- JS 风格:看 `static/index.html` 已有代码保持一致

## Playwright (`e2e/`)

- Spec 文件拆桌面 / 移动端 (`*.desktop.spec.mjs` / `*.mobile.spec.mjs`)
- 公共辅助放 `reactor-os.helpers.mjs`,能复用就复用
- 每个新交互流补 case,不要只跑通就行
- 跑:`npm run e2e` 全量,`npm run e2e:headed` 调试单 case

## Config (`config/`)

- TOML 格式,加字段时给注释
- `safety.toml` 是限幅硬约束,改前想"会不会让设备超温超压"
- `ai_memory.toml` 包含 StepFun step-3.6 demo key 的部分**只进 demo 包**

## 协议 / API 改动 (跨模块硬约束)

1. 改帧格式 / API schema 是 breaking change,**先停下来**跟对应 reins 同步
2. 三处必须同步:
   - 代码 (daemon 或 firmware)
   - 测试 (daemon 端 `esp32_protocol_tests.rs` / `json_bridge_protocol_tests.rs`)
   - 文档 (`docs/esp32_protocol.md` / `docs/json_bridge_protocol.md` / `README.md` 帧示例)
3. 改完先在自己模块跑通,再叫对端 reins 验证联调

## 提交前自检

- [ ] `cargo build` / `cargo test` 通过 (Rust 改动)
- [ ] `npm run e2e` 桌面 + 移动端通过 (前端 / e2e 改动)
- [ ] `qmake` + `make` 通过 (Qt 改动)
- [ ] `docker compose up --build reactor-edge` 起得来 (构建 / 部署改动)
- [ ] 跨模块改动文档已同步
- [ ] 报告:改了哪些文件、影响哪些对端、build/test 状态
