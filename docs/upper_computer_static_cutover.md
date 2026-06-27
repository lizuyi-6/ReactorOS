# 上位机 HMI 静态资源切换预案

本文档说明 `codex/prd-tech-stack-migration` 分支把生产 HMI 从
`static/index.html`（单文件原生 HMI）切换到 `frontend/dist/index.html`
（Vue 3 + Element Plus + ECharts + Pinia 单文件构建产物）的步骤、回滚
路径和验收要求。

## 1. 切换机制

`src/main.rs` 的 `--assets` 参数现在默认值为 `auto`：

```text
fn resolve_assets_dir(requested: &PathBuf) -> PathBuf {
    if requested_str != "auto" { return requested.clone(); }
    for candidate in [frontend/dist, static] {
        if candidate.join("index.html").is_file() { return candidate.clone(); }
    }
    static
}
```

- 如果 `frontend/dist/index.html` 存在（执行过 `npm run frontend:build`），daemon 自动托管 Vue 产物。
- 如果 `frontend/dist/index.html` 不存在（Vue 尚未构建），daemon 自动 fall back 到 `static/index.html`，生产服务不会停摆。
- 显式 `--assets static` 或 `--assets frontend/dist` 可以强制走某条路径，便于验收。

## 2. 切换步骤

```powershell
# 1. 构建 Vue 产物
npm run frontend:build

# 2. 验证产物
Test-Path frontend/dist/index.html   # 期望 True

# 3. 启动 daemon（auto 模式）
cargo run --bin reactor-edge-daemon -- `
  --config config/device.toml `
  --safety config/safety.toml `
  --memory config/ai_memory.toml `
  --integration config/integration.toml `
  --db data/reactor.sqlite3 `
  --bind 127.0.0.1:8000

# 4. 验收
curl -s -I http://127.0.0.1:8000/ | head -1   # 期望 200
curl -s http://127.0.0.1:8000/ | Select-String -Pattern 'id="app"'   # 期望命中
curl -s http://127.0.0.1:8000/api/health      # 期望 {"status":"ok"}
```

## 3. 回滚

如果 Vue 产物在生产环境表现异常，可以两种方式快速回滚：

```powershell
# A. 显式指向 legacy HMI
cargo run --bin reactor-edge-daemon -- --assets static ...   # 其余参数不变

# B. 把 Vue 产物移走让 auto 退到 static
Move-Item frontend/dist frontend/dist.disabled
```

## 4. 注意事项

- **SPA fallback 仍由 daemon 负责**：`ServeDir::new(&assets).not_found_service(ServeFile::new(assets.join("index.html")))`，前端 hash 路由（`/#/control` 等）刷新不会 404。
- **缓存头**：浏览器对 `frontend/dist/index.html` 的 1.9 MB 单文件 bundle 应保留 `cache-control: no-cache`，由 `tower-http::services::ServeDir` 默认行为保证；如有 CDN 介入请确认不缓存首页。
- **同步产物**：发布前必须先跑 `npm run frontend:build` 再启动 daemon；如果构建失败但旧的 `frontend/dist/index.html` 还在，daemon 会照旧托管旧产物（这是有意为之，避免误发布空白页）。
- **资源文件**：当前 Vue 构建使用 `vite-plugin-singlefile` 把 JS/CSS 内联进 `index.html`，没有额外的 chunk 文件需要清理。

## 5. 验收清单

- [x] `npm run frontend:build` 成功
- [x] `cargo check --bin reactor-edge-daemon` 成功
- [x] `frontend/dist/index.html` 存在（构建后自动）
- [x] daemon auto 模式启动时优先用 `frontend/dist`
- [x] daemon auto 模式 fall back 到 `static`（构建前）
- [x] SPA fallback 工作：访问 `/#/control` 不会 404
- [x] 七大页面、Pinia 中英切换、控制写入、审计导出、Modbus 调试、工艺生命周期 6 个 Vue 验证脚本全部通过

## 6. 后续

- Vue 完整 parity 已经覆盖 PRD 七大页面；本切换预案让生产 daemon 默认托管 Vue 产物，legacy `static/index.html` 保留作为回滚基线和 PoC 演示入口。
- 后续如要做 A/B 灰度，可以在 daemon 启动脚本里加 `XINGSHU_HMI_VARIANT=vue|legacy` 环境变量并在 `resolve_assets_dir` 中判断。
