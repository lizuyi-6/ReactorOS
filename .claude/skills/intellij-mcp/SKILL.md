---
name: intellij-mcp
description: 通过 IntelliJ IDEA MCP server 操作 ReactorOS(Rust)仓库的实操指南——能力边界、踩坑与绕过。当要用 IDEA 的编辑器/搜索/文件/检查能力处理本仓库代码时使用。
---

# IntelliJ IDEA MCP 在 ReactorOS(Rust)上实操

## 前提
- MCP server `intellij` 已注册(user scope),`http://127.0.0.1:64342/sse`(SSE,固定端口)。**IDEA 必须开着**,否则连不上。
- **所有工具都传 `projectPath`,用正斜杠 `X:/tianhks`**(反斜杠触发 `different root`)。
- bash 里的 `python3`(`/x/msys64/...`)标准库残缺;好 python 是 `C:/Users/Abraham/AppData/Local/Programs/Python/Python312/python.exe`。
- 构建/测试走 `scripts/cargo-x.ps1`,**别用 IDEA build**(见下)。

## 核心状态(必读)—— 已装 IntelliJ Rust 插件(2026-07),符号索引已建
- ✅ **文件级**:读、全文/正则搜索、glob、打开编辑器、VCS、目录树(指向子目录)——稳定
- ✅ **符号级(装 Rust 插件后复活)**:`search_symbol` / `get_symbol_info` / `rename_refactoring` 对 .rs 可用(已验证 rename 真落盘)。`rename` 返回 `Successfully renamed ... with N usages`。
- ❌ `generate_psi_tree`:硬编码只支持 Java/Kotlin,**装插件也无效**(工具实现限制,非索引问题)。
- ⚠️ **装插件/重启 IDEA 后会全项目重索引(dumb mode,几分钟)**:期间 `rename`/`build`/全项目 find_usages 报 `IndexNotReadyException`;`search_symbol` 用部分索引会先恢复。看 IDEA 右下角进度条判断是否跑完。
- ⚠️ **run/debug**:装插件后 main 已被识别为 runnable(`get_run_configurations(filePath=...)` 返回 runPoints,如 `xingshu.rs:572` 的 main)。但 `execute_run_configuration`/`xdebug_*` 对 Rust 走 IDEA 的 cargo run,**不经 cargo-x.ps1**(错工具链/污染 target)→ **保持用 cargo-x.ps1 / 外部调试器**。无保存的 run config(`configurations` 空),用 filePath+line 模式触发。
- ⚠️ **Developer kit 工具**(`find_lock_requirement_usages`/`find_threading_requirements_usages`/`get_project_status`):**插件不存在/未安装**(用户确认无 Developer kit MCP 插件),这3个工具不可用。
- **database 类**(需先在 IDEA 配 SQLite data source → `data/reactor.sqlite3`;MCP 无 create-connection 工具,手动配):
  - ✅ **全部可用**(授权后测过 9/10):`list_database_connections` / `test_database_connection`(Ping 6ms) / `list_database_schemas`(main) / `list_schema_object_kinds`(table/view/virtual-table) / `list_schema_objects`(11表) / `get_database_object_description`(表结构:列/类型/NN/autoincr/外键/PK) / `preview_table_data`(CSV真实数据) / `execute_sql_query`(SELECT只读) / `list_recent_sql_queries`。`cancel_sql_query` 未测(需长查询场景)。
  - ⚠️ **执行/扫描类(execute_sql/preview/list_schema_objects/list_schema_object_kinds)首次调用会 timeout** —— 实际是 IDEA 弹 SQL 执行授权确认对话框等用户点。**在 IDEA 授权一次后秒通**,不是工具坏。
  - db:`data/reactor.sqlite3`(WAL,11表:sensor_samples 21万/control_events 1.2万/ai_recommendations 4080 等);生成:`scripts/cargo-x.ps1 run --bin reactor-edge-daemon`。daemon 跑时锁 db,测时停 daemon。

## 可用(文件级,稳定)
- 读:`read_file` / `get_file_text_by_path`(支持 offset/limit)
- 搜:`search_text` / `search_regex`(带 file:line:col,`limit` 生效)。`search_in_files_by_text` / `search_in_files_by_regex` 带上下文,但 `maxUsageCount` **不生效**——用前者的 `limit` 限流
- 找文件:`find_files_by_glob` / `find_files_by_name_keyword` / `search_file`
- 结构:`list_directory_tree`(见坑)、`get_project_modules`、`get_repositories`
- 编辑器:`open_file_in_editor`
- 检查:`run_inspection_kts`(KTS 脚本,基于通用 PSI,对**任何文件类型**都能跑——这是可用的代码分析能力)、`get_file_problems`(只对 txt/json 等有效,Rust 错误拿不到)

## 可用(写,但有偶发重试)
`create_new_file` / `replace_text_in_file` / `apply_patch` 都能用,**但偶发 `StandaloneCoroutine was cancelled` 或 `operation timed out`——遇到就重试,通常第二次成功**。**不要把写操作和长耗时操作(build/xdebug)并行**,会互相取消协程。

## 不可用 / 无法测
- `build_project`:IDEA 的 Rust build 会调裸 cargo,违反 `scripts/cargo-x.ps1` 约定(污染默认 target/错工具链)→ **别用,保持 cargo-x.ps1**
- `execute_run_configuration` / `xdebug_*`:无 run config、无 runnable 入口、Rust 调试未配
- `execute_sql_query` / 数据库类:项目无 DB 连接
- `get_all_open_file_paths`:恒报 `'other' has different root`(根因未定位)→ 用 list_directory_tree + read_file 替代

## 踩坑速查
| 现象 | 绕过 |
|---|---|
| projectPath 反斜杠报 different root | 用正斜杠 `X:/tianhks` |
| `list_directory_tree(".")` 只返回根名 | 指向子目录 `directoryPath="src"` |
| `src/**/*.rs` 只返回 bin/ | 用 `**/*.rs` 或 `src/*.rs` |
| `search_in_files_*` maxUsageCount 不生效 | 改用 `search_text`/`search_regex` 的 `limit` |
| 写工具 cancelled / timeout | 重试 |
| terminal `&&` 报错 | IDEA 终端是 PowerShell 5.1,用 `;` 或 `if($?)` |
| terminal 中文乱码 | GBK 编码,以 exit code 为准 |

## 审计盲点(重要)
provenance hook(`.claude/hooks/post-tool-call.py`,POST 到 IDEA built-in server `localhost:63342/api/provenance/call`)只匹配 Claude 内置 `Write|Edit|MultiEdit|NotebookEdit`。**IDEA MCP 自己的 create/replace/patch 不触发它**——这些改动不进溯源。要审计就改用 Claude 内置 Write/Edit。

注:该 hook 的 command 必须指向好的 Python312(见前提),且 `python3` 在 bash 解析到坏的 msys2 解释器。

## 官方文档对照(权威,2026-05-13)
官方按功能分 17 章节(非"可独立启停 toolset")。可在 **Settings | Tools | MCP Server | Exposed Tools** 按工具启停。本仓库实测可用性见上各节。几个对照结论:

- **projectPath 不是格式硬要求**:官方原文"provide if known **to reduce ambiguous calls**"。但 IDEA 实现对反斜杠敏感(实测 different root),正斜杠 `X:/tianhks` 是**绕 bug**,非官方规定。
- **本仓库缺 3 个官方工具**:`find_lock_requirement_usages`、`find_threading_requirements_usages`、`get_project_status` —— 来自 **Developer kit MCP 插件**(未装)。前两个找锁/线程需求 usages,对 fail-closed 并发不变量分析有用,但依赖语义索引(需先装 IntelliJ Rust)。
- **apply_patch 官方未列**(我这有,文档可能滞后)。
- **get_all_open_file_paths 的 different root 官方未提限制** —— 倾向 IDEA 实现 bug。
- 官方未对 Rust 做特殊承诺;语义工具失效 = 没装 IntelliJ Rust 插件,符合"通用、依赖语言插件"的设计。

## 安全不变量检查套件(`.claude/inspections/*.inspection.kts`)
把 CLAUDE.md 的安全约定变成可重跑的 KTS 检查(用 `run_inspection_kts` 读 .kts 文件内容作 `inspectionKtsCode`,指定 `contextPath`):
- `commit-generation-recheck` — commit_* 提交前复查 generation(对 api.rs)
- `generation-advance-invariant` — engage 推进 / clear 不推进 generation(对 state.rs)
- `reset-same-instance-recheck` — reset 必须证明同一实例(对 api.rs)
- `start-failure-dichotomy-probe` — start 失败二分结构 probe(对 api.rs;控制流,需人工 review)
- `clamp-before-write` — SafeCommand::Write 必经 clamp_operator_targets(对 control.rs)

截至 2026-07 全部合规(commit/engage/clamp 已 probe 反证非假阴性;reset 框架同、未单独 probe)。改控制路径代码后重跑确认不变量没破。

**CI 版**(无头,cargo test 自动跑):`tests/safety_invariants.rs` 4 个 `#[test]` 守 commit/clear-reset/reset/clamp(`docker compose run --rm test cargo test` 天然包含,不用改 CI 配置)。文本级 byte-safe(处理中文/特殊字符)。start 失败二分是控制流,只在 KTS probe(人工 review),CI 不硬 assert。
