# 前端视觉重构账本

## 2026-07-17：哑光工业 HMI（部分完成）

目标：把霓虹绿、像素字、游戏化网格和无效操作外观收敛为安静的工业操作台，同时保持急停、告警、联锁、固定屏分页和后端契约。

### 本轮改动

- 新增 `frontend/src/styles/refined-industrial.css`，作为最后一层主题覆盖：中性炭灰表面、低饱和青色信息色、独立正常/警告/危险色、等宽数字、无发光和窄屏双行安全栏。
- `frontend/src/views/MonitorView.vue` 删除伪 FFT/U-value 展示和无处理器操作按钮；AI/批次入口改为真实路由链接。
- 零告警不再显示为 1；未知/过期传感器不再硬编码 `NORMAL`。
- `control_loop_terminated` 改为读取后端布尔字段的字符串表示，避免对象读取函数把布尔值丢成 `null`。
- 窄屏显式保留联锁与命令回执；全局后端错误和禁用按钮保持可见差异。
- Settings 增加第 7 个固定屏页，暴露原先没有分页入口的 Endpoint Matrix。
- 新增 `scripts/verify-refined-hmi-contract.mjs`，固化 15 条本轮源码/产物契约。

### 可复现证据

- `npm run frontend:build`：exit 0；2176 modules transformed；生成 `frontend/dist/index.html` 2186.49 kB（gzip 617.63 kB）；构建耗时 4m 4s。
- `node scripts/verify-refined-hmi-contract.mjs`：exit 0；`Refined HMI contract passed: 15 assertions`。
- `node scripts/verify-vue-release-assets.mjs`：exit 0；`Vue release assets gate passed`。
- `git diff --check`：exit 0；仅报告既有 Windows LF→CRLF 提示，无空白错误。

### 未证明事项 / 阻断后续验收的范围

- 本轮没有对改后页面执行截图、DOM 尺寸或像素差异复核；“视觉更好”尚无改后截图证据。
- 未在 1440×900、1366×768、800×480、393×851 和真实 RK kiosk 上逐路由验证。
- 未操作真实急停、目标下发、工艺启动或 AI 建议应用；构建成功不证明这些行为正确。
- 未用真实设备的告警、离线、控制环终止和后端拒绝状态检查最终计算样式。
- 新主题作为第 13 个样式层降低了本轮回归风险，但没有清偿原有 12 层 CSS 的结构债；后续应在独立变更中合并旧主题。

### 本轮改动可能引入的新风险

- Settings 页数从 6 变为 7；源码映射与契约测试已覆盖，但目标 kiosk 上的分页按钮可达性尚未做浏览器验证。
- 920px 以下顶栏从 56px 变为 96px 双行结构；安全信息得以保留，但可用内容高度减少 40px，需在 800×480 上实测。
- CSS 末层使用高优先级覆盖旧主题；若后续旧文件继续追加 `!important` 规则，可能重新覆盖本主题。

## 2026-07-18：Workshop 前端接入当前 daemon（部分完成）

目标：把 `workshop/frontend` 的七路由重构版接到仓库当前 Rust daemon 的 HTTP/WebSocket 契约，修复可确定的断线，并列出尚无 UI 入口或尚未做真实环境验证的范围。工作范围保持在 workshop 前端、核对脚本和文档；没有切换 `--assets auto` 的生产默认资源，也没有修改后端控制逻辑。

### 安全语义变更

- 语义变更：风险增加按钮的前端条件从“`liveStatus === fresh` 且急停/人工锁/控制环终止未触发”改为“前述条件成立且 `device_status.devices[*].online === true`”。触发依据是本轮用户要求接入当前后端后，隔离 daemon 在“样本新鲜但下游设备状态不可证明”场景对 Modbus 写入返回 503 `device status unavailable`；前端此前仍显示按钮可用，与后端 fail-closed 状态不一致。
- 失败语义：下游设备在线状态不可证明时，目标下发、工艺应用/启动、批次启动、AI 推荐应用/执行和 Modbus 写入在 UI 禁用；停止、急停、人工锁等风险降低动作没有因此被禁用。
- 文档已在本节同步；后端安全默认值、`requires_*`、急停/限幅、generation 和降级策略均未修改。

### 本轮改动

- 修正 Modbus 调试请求为后端真实路由 `GET /api/modbus/registers/:register/read` 与 `POST /api/modbus/registers/:register/write`；读/写寄存器分别从 `read_registers`/`write_registers` 映射，写入后执行读回。
- 修正工艺步骤读取：不再调用不存在的 `GET /api/processes/:id/steps`，改为读取 `GET /api/processes/:id` 的 `{ process, steps }`。
- 批次 `outcomes` 进入独立 store 状态；History 刷新后不再只依赖 `/api/live` 中的旧快照。
- 持久化 bearer 会话通过 `/api/auth/me` 复核；401 会清空无效会话。页面动作错误通过全局错误条展示，不再只留未捕获 Promise。
- 修正 Settings 接口矩阵与 Modbus/MQTT/AINAS/local AI 字段映射；补内嵌 favicon，消除 daemon 托管单文件时的浏览器 404。
- 新增静态契约、隔离后端行为和真实浏览器到后端的三层核对脚本；修正 `render-check.mjs` 在 Windows 下残留 keep-alive 连接导致不退出的问题。

### 可复现证据

- `npm run typecheck`（`workshop/frontend`）：exit 0；`vue-tsc --noEmit` 无错误。
- `npm run verify:contract`（`workshop/frontend`）：exit 0；`Workshop/backend contract: 17 assertions passed`，包含真实 Modbus 路由、process detail/steps、auth/me、代理与设备在线闭锁源码断言。
- `npm run build`（`workshop/frontend`，最终 favicon 产物）：exit 0；654 modules transformed；`dist/index.html` 736.19 kB、gzip 247.57 kB；`built in 1m 19s`。
- `node workshop/scripts/render-check.mjs`：exit 0；monitor/control/ai/history/audit/modbus/settings 7/7 路由均为 `mounted=true`、`horizOverflow=false`、`unexpectedErrors=0`。这只证明浏览器渲染层。
- `scripts/cargo-x.ps1 build --bin reactor-edge-daemon`：exit 0；当前源码 debug daemon 构建耗时 30.03s。
- `npm run verify:backend`（loopback `127.0.0.1:18080`、独立 SQLite、`--enable-test-reset --seed-demo-context`、当前默认 `config/safety.toml`）：exit 0；11 个字段级行为断言，包含 `health.ok==true`、`auth/me.role==admin`、实时温度 42.5、工艺详情 steps、Modbus 压力 `value==0.18/raw==18`、缺下游状态写入 503、WebSocket `current_temp==42.5/current_pressure==0.18`。
- `npm run verify:ui`（同一 loopback 隔离 daemon 托管最终 `workshop/frontend/dist`）：exit 0；8 个真实浏览器行为断言，包含登录、Monitor 后端温度、Modbus 读回、离线状态下写入/目标下发禁用、Settings 真实端点，以及 `pageerror=[]`、`consoleErrors=[]`、`HTTP >=400 responses=[]`。
- `scripts/cargo-x.ps1 -CargoArgs @('test','--test','api_tests','--','--test-threads=1')`：exit 0；208 passed、0 failed、0 ignored、0 measured、0 filtered out；覆盖 Modbus 成功写入/读回、越界拒绝、缺审计原因拒绝和安全联锁等后端分支。

### 未覆盖范围（本轮未触及的相关路径/测试/分支）

- 本节首次检查时尚无入口的工艺编辑、组件控制、AINAS 任务和 v1 时间窗历史，已在下方“补齐缺失 UI”轮次接入；真实设备成功动作的未覆盖范围仍保留。
- 未在真实 `device_status.online=true` 的硬件/桥接器环境通过浏览器点击完成 Modbus 成功写入或目标/工艺启动；后端成功写入/读回由完整 `api_tests` 的字段断言覆盖，但不构成真实硬件或前端成功按钮链路证明。
- 未验证 WebSocket 断线 3 秒重连、15 秒轮询兜底、token 真实过期后的页面跳转、导出文件内容、AI 云端/本地真实推荐生成、操作员/工程师两种角色的完整浏览器矩阵。
- 未在 800×480、移动端、RK kiosk、真实 STM32/ESP32/Modbus RTU/TCP、TLS 和长期运行环境验证；七路由截图是 1440×900 Chromium 渲染证据。
- 生产默认 `--assets auto` 仍优先 `frontend/dist`；本轮只证明 `--assets workshop/frontend/dist` 可由 daemon 托管并与后端同源工作，没有把 workshop 切成生产默认前端。
- 同语义检索命令：`rg -n --no-heading '"/(api|ws|health)' src tests frontend/src workshop/frontend/src static/index.html` 与 `rg -n '^\\s*\\.route|Router::new|route\\(' src/api.rs src/api_auth.rs src/api_integrations.rs`。未读的相关大类包括 AINAS 任务各 handler 的完整实现、组件控制页面设计和 v1 history 前端筛选交互。

### 本轮改动可能引入的新风险

- 设备在线前端闭锁比原 workshop 更严格：仅有新鲜 pipeline 样本、但无设备状态证明的演示环境会看到目标/启动/写入禁用；这是与当前默认后端安全配置一致的结果，但会降低“纯样本演示”的可操作性。
- 登录后每次受保护刷新增加一次 `/api/auth/me` 请求；能及时清理过期 token，但慢盘/慢网络下增加一个往返。
- Modbus 写入后同时刷新 live、寄存器和审计，保证读回/审计界面一致，但会增加一次操作后的请求数。
- 测试产物 `output/workshop-live-check.sqlite3{,-shm,-wal}` 与 `.stdout/.stderr.log` 因本地策略拒绝删除仍保留；隔离 daemon 已停止，18080/5417 均无监听。它们是 loopback 测试数据，不是生产数据库。

### 2026-07-18 补齐缺失 UI（部分完成）

- Control 增加工艺创建、元数据更新、步骤新增/更新和按 `/api/devices/capabilities` 动态生成的组件控制表单。pipeline 模式的组件能力数组为空时显示明确空态。
- History 增加 `/api/v1/reactor/reactor_001/history` 的开始/结束时间、分页和样本表格。
- Settings 增加 AINAS `set_targets/start_process/stop_process` 表单、任务列表和详情；engineer/admin 才可提交/读取。
- 安全语义变更：组件 `stop/off` 与 AINAS `stop_process` 作为风险降低动作保留入口；其他组件/AINAS 动作在现场数据不新鲜、设备在线不可证明或安全闩触发时禁用。后端权限和最终联锁未修改。
- `npm run typecheck`：exit 0；`vue-tsc --noEmit` 无错误。
- `npm run verify:contract`：exit 0；26 个前后端路由/UI 接线断言。
- `npm run verify:backend`：exit 0；15 组行为断言，包含工艺/步骤字段持久化、v1 时间窗历史、pipeline 组件空能力与 AINAS 拒绝任务入表。
- `npm run verify:ui`：exit 0；15 个真实浏览器行为断言，包含工艺创建/更新/加步骤、时间窗样本、组件空态、AINAS 闭锁与任务列表，且无页面/控制台/HTTP 错误。
- `npm run build`：exit 0；654 modules；最终 `dist/index.html` 760.79 kB、gzip 253.76 kB。
- `node workshop/scripts/render-check.mjs`：exit 0；7/7 路由 `mounted=true`、`horizOverflow=false`、`unexpectedErrors=0`。
- 未覆盖范围：真实 Modbus/ESP32/JSON Bridge 的非空组件能力浏览器动作未执行；AINAS 成功执行升风险任务未在健康设备状态下点击；步骤更新 UI 有源码/类型/后端字段证据，但本轮浏览器只点击了步骤新增。

## 2026-07-18：LubanCat 2 生产机专项构建（部分完成）

目标机档案：RK3568、4× Cortex-A55、2 GiB LPDDR4、Debian 10/glibc 2.28、HS200 eMMC、单核 Mali-G52。生产构建保持电脑侧 Docker 交叉编译，不在板端编译。

### 本轮调整

- LubanCat 专用 PowerShell/Unix 构建包装器默认构建 `workshop/frontend`，打包器通过 `FRONTEND_DIST` 显式复制到 release 包内的 `frontend/dist`；没有修改 daemon 的 `--assets auto` 选择语义。
- ARM64 编译仍使用 `target-cpu=cortex-a55`，并根据目标机实测指令集启用 LLVM 支持的 AES（含 PMULL 指令族）、SHA2、CRC 和 LSE target features；这使该包只面向档案中的 RK3568，不作为通用 ARM64 包。
- rusqlite 与 SQLx 连接统一增加 64 MiB `mmap_size`，用于按需映射热历史/索引页；WAL、`synchronous=NORMAL`、400 页 checkpoint、4 MiB page cache 和 busy timeout 保持不变。
- Workshop 趋势图关闭 ECharts 过渡动画、启用 dirty-rect，并把 DPR 上限设为 1.25；告警、急停和设备状态判定未修改。
- 未设置 daemon/safety-guard 的 `MemoryMax`：当前没有生产板 release RSS 峰值证据，硬上限可能同时杀死后端与安全子进程。
- 部署语义变更：从“安装时包内 `reactor-edge.env` 无条件覆盖目标机文件”改为“首次安装复制模板，已存在的目标机环境文件保持原字节不变”；依据是生产机档案明确记录该文件含真实 StepFun key，旧行为会在迁移时丢失密钥。其他 TOML 仍按包内容更新。

### 可复现证据

- `npm run typecheck`（`workshop/frontend`）：exit 0；`vue-tsc --noEmit` 无错误。`npm run verify:contract`：exit 0；26 assertions passed。
- `npm run build`（`workshop/frontend`）：exit 0；654 modules；单文件 `dist/index.html` 760.90 kB、gzip 253.80 kB；打包后 HMI sha256=`8b08d8125d4a7d846b2ea57dd31d59c84efb09d99f3c9f988b21ec7fe8bae022`，与源码构建产物相等。
- `cargo test --lib db::tuning_tests:: -- --nocapture`：exit 0；2 passed、0 failed、0 ignored；rusqlite/SQLx 均断言 `journal_mode==wal`、`synchronous==1`、`wal_autocheckpoint==400`、`temp_store==2`、`cache_size==-4096`、`mmap_size==67108864`。
- LubanCat 打包脚本内全仓 `cargo test`：exit 0；合计 448 passed、0 failed、0 ignored（各测试二进制统计相加）。
- `bash scripts/verify-install-board-preflight.sh`：exit 0；现有 `reactor-edge.env` 安装前后 sha256 相等、`STEPFUN_API_KEY=existing-production-key` 仍存在，且 `device.toml` 更新为包内容。
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-lubancat2-debian10.ps1 -SkipBuilderImage`：exit 0；最终包 `dist/reactor-os-lubancat2-rk3568-debian10-chromium-kiosk-20260718-164354-2c672f7f.tar.gz`，8,509,931 bytes，sha256=`7590371a5e7f60f48457b6e5fe89a12ed84b917b8793334db6fc44af1ccefba1`。
- 三个 release 二进制经 `file`/`readelf --version-info` 检查均为 stripped ARM aarch64 ELF，最高 glibc symbol version 均为 `GLIBC_2.28`；构建元数据断言 target=`aarch64-unknown-linux-gnu`、flags=`-C target-cpu=cortex-a55 -C target-feature=+aes,+sha2,+crc,+lse`、frontend source=`workshop/frontend`。

### 未覆盖范围

- 包尚未 scp/安装到生产 LubanCat 2；未在该板执行 `/health`、设备状态、Chromium kiosk、safety-guard 子进程、A/B OTA 或回滚。
- 未采集生产板 release RSS/CPU、控制延迟、eMMC 写放大、温度/降频或 7×24 稳态数据；因此没有设置 `MemoryHigh/MemoryMax`，也不声称性能指标达标。
- 未在真实 Modbus/ESP32/JSON Bridge 设备上执行升风险控制动作；交叉编译与 x86_64 容器测试不构成真实硬件行为证明。
- 当前包元数据为 `GIT_SHA=2c672f7f`、`GIT_DIRTY=true`；tar 通过独立 HMI/包 sha256 锚定，但尚无提交 hash 能单独重建全部工作树改动。

### 本轮改动可能引入的新风险

- A55 与 AES/SHA2/CRC/LSE 特性使二进制专用于档案中的 RK3568；不得下发到缺少这些指令的通用 ARM64 设备。
- SQLite 64 MiB mmap 是按需虚拟映射而非预分配 RSS，但数据库热读工况下仍可能增加文件页驻留；需在 2 GiB 板上测量 daemon + safety-guard + Chromium 总 RSS。
- 安装器不再覆盖已有 `reactor-edge.env`，可保住生产密钥；代价是包内新增环境模板不会自动替换旧文件，环境变量迁移必须显式人工合并。

## 2026-07-19：反应釜搅拌动画纠偏（部分完成）

目标：修正 Monitor 反应釜中搅拌轴随桨叶整体绕圈的问题，使轴保持固定、仅桨叶围绕自身中心旋转。

### 本轮改动

- `ReactorVessel.vue` 将固定搅拌轴与 `impeller-rotor` 旋转组拆开；旋转动画不再施加到包含长轴的父组。
- `render-check.mjs` 增加搅拌动画几何门禁和 `STIRRER_ONLY=1` 快速模式，分别断言轴矩阵不变、桨叶旋转矩阵变化、桨叶中心不漂移。

### 可复现证据

- `npm run typecheck`（`workshop/frontend`）：exit 0；`vue-tsc --noEmit` 无错误。
- `node node_modules/vite/bin/vite.js build`（`workshop/frontend`）：exit 0；654 modules；`dist/index.html` 761.00 kB、gzip 253.83 kB。
- `$env:STIRRER_ONLY='1'; node workshop/scripts/render-check.mjs`：exit 0；`shaftFixed==true`、`rotorTurned==true`、`rotorCentered==true`、`unexpectedErrors==0`。
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-lubancat2-debian10.ps1 -SkipBuilderImage`：exit 0；451 passed、0 failed、0 ignored；生成 `20260719-134831` ARM64 包。
- 新 tar sha256=`d36f70e56e6bfa0c2139d20e60a210a05449bbab8b89f0bd27f8a632b6cc57f9`；包内 HMI sha256=`edbd102f8571e3e35309173c9c66a83a0932463074c31cdc8d89e7f069b20818`，与本地构建产物相等，且包内可检索到 `impeller-rotor` 与 `impeller-spin`。
- `bash scripts/verify-packaged-lubancat2-deployment-fixes.sh <20260719-134831 tar>`：exit 0；最终 tar 解包后连续安装两次的部署字段门禁通过。
- `git diff --check`：exit 0；仅报告既有换行符转换提示。

### 未覆盖范围

- 几何门禁使用桌面 Chromium，并通过添加 `spinning` class 直接触发动画；未在 RK3568 板载 Chromium 91 和真实实时 RPM 数据流上复验。
- 新包已包含动画资源，但尚未安装到实体 LubanCat 2。

### 本轮改动可能引入的新风险

- SVG 动画依赖 Chromium 对 `transform-box: fill-box` 的实现；桌面 Chromium 字段断言成立，但板载 Chromium 91 尚无现场证据。
