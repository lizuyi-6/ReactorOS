//! CLAUDE.md 安全不变量检查 —— CI 友好版(对应 `.claude/inspections/*.inspection.kts`)。
//!
//! 文本级检查(不依赖 IDEA PSI):找函数定义 + 花括号配对提取函数体 + 查 marker。
//! `cargo test` 自动跑(docker compose test 容器原生),违规则测试 fail(CI 红)。
//! 与 `.claude/inspections/` 的 KTS 逻辑等价 —— KTS 给交互式用(CC + IDEA MCP),
//! 这里给 CI 用(无头、cargo test)。
//!
//! start 失败二分是控制流分支,文本级不可靠断言,这里不做硬 assert(留 KTS probe 人工 review)。
use std::fs;
use std::path::PathBuf;

fn src(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join(name)
}

fn read_src(name: &str) -> String {
    let p = src(name);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {:?}: {}", p, e))
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// 从 open 位置的 `{` 找匹配的 `}` offset。
fn find_matching_brace(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

struct FnDef {
    name: String,
    body: String,
}

/// 提取所有 `fn <name>(` 定义及其函数体(花括号配对)。
/// 词边界匹配 `fn `(前一个字符非标识符),覆盖 `pub fn`/`async fn`/`pub(crate) async fn`。
fn extract_fns(text: &str) -> Vec<FnDef> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 3 <= bytes.len() {
        if &bytes[i..i + 3] == b"fn " && (i == 0 || !is_ident_byte(bytes[i - 1])) {
            let mut j = i + 3;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let name_start = j;
            while j < bytes.len() && is_ident_byte(bytes[j]) {
                j += 1;
            }
            let name = text[name_start..j].to_string();
            // name 后必须跟 "("(排除 "fn " 出现在注释/文档里的误命中)
            let after = text[j..].trim_start();
            if name.is_empty() || !after.starts_with('(') {
                i += 1;
                continue;
            }
            // 函数体第一个 "{"
            let body_open = match text[j..].find('{') {
                Some(o) => j + o,
                None => {
                    i = j;
                    continue;
                }
            };
            let body_end = match find_matching_brace(text, body_open) {
                Some(e) => e,
                None => {
                    i = body_open + 1;
                    continue;
                }
            };
            out.push(FnDef {
                name,
                body: text[body_open..=body_end].to_string(),
            });
            i = body_end + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// 不变量 1:所有 commit_* 提交路径必须在函数体内调用
/// ensure_safety_latches_unchanged(提交前复查 generation 快照)。
#[test]
fn commit_paths_recheck_generation() {
    let text = read_src("api.rs");
    let marker = "ensure_safety_latches_unchanged";
    let violations: Vec<String> = extract_fns(&text)
        .into_iter()
        .filter(|f| f.name.starts_with("commit_") && !f.body.contains(marker))
        .map(|f| f.name)
        .collect();
    assert!(
        violations.is_empty(),
        "commit_* 提交路径未复查 generation(CLAUDE.md 不变量,提交前必须 ensure_safety_latches_unchanged): {:?}",
        violations
    );
}

/// 不变量 2:clear_*/reset_* 绝不能推进 generation(否则复位会伪造"新实例"计数)。
/// 注意:不强求 engage 必推进 —— 有 from_safety 这类 read-only 恢复(读 generation 填字段)。
#[test]
fn clear_reset_must_not_advance_generation() {
    let text = read_src("state.rs");
    let violations: Vec<String> = extract_fns(&text)
        .into_iter()
        .filter(|f| {
            f.body.contains("_generation")
                && (f.name.contains("clear") || f.name.contains("reset"))
                && f.body.contains("saturating_add")
        })
        .map(|f| format!("{} (clear/reset) ADVANCES generation", f.name))
        .collect();
    assert!(
        violations.is_empty(),
        "clear/reset 推进了 generation(CLAUDE.md 不变量,复位类不得推进计数): {:?}",
        violations
    );
}

/// 不变量 3:reset_* 必须在审计前后比对 generation/故障文本(证明处理同一实例)。
#[test]
fn reset_rechecks_same_instance() {
    let text = read_src("api.rs");
    let violations: Vec<String> = extract_fns(&text)
        .into_iter()
        .filter(|f| {
            f.name.starts_with("reset")
                && !(f.body.contains("ensure_safety_latches_unchanged")
                    || f.body.contains("generation")
                    || f.body.contains("fault")
                    || f.body.contains("changed"))
        })
        .map(|f| f.name)
        .collect();
    assert!(
        violations.is_empty(),
        "reset_* 未比对 generation/fault(CLAUDE.md:复位必须证明同一实例): {:?}",
        violations
    );
}

/// 不变量 4:构造 SafeCommand::Write 的函数必须先经安全限幅器
/// (clamp_operator_targets / forbidden_control_zone)。
#[test]
fn write_paths_are_clamped() {
    let text = read_src("control.rs");
    let violations: Vec<String> = extract_fns(&text)
        .into_iter()
        .filter(|f| {
            let constructs_write = f.body.contains("SafeCommand::Write")
                || f.body.contains("ControlDecision::Write")
                || f.body.contains("Write(SafeCommand");
            constructs_write
                && !(f.body.contains("clamp_operator_targets")
                    || f.body.contains("forbidden_control_zone"))
        })
        .map(|f| f.name)
        .collect();
    assert!(
        violations.is_empty(),
        "构造 SafeCommand::Write 但未限幅(CLAUDE.md:所有设备写入必须经 clamp): {:?}",
        violations
    );
}

/// 不变量 5(审计完整性):所有直接 `INSERT INTO control_events` 的函数,
/// 必须在函数体内调用 `control_event_hash`。防退化 —— 不准再加不 hash 的 insert 路径
/// (历史 db 里的 device_write/emergency_stop NO_HASH 就是旧版不 hash 路径残留,
/// 当前 4 处 insert 已全部 hash:insert_control_event / _sqlx / _in_rusqlite_tx / _in_sqlx_tx)。
#[test]
fn all_control_event_inserts_compute_hash() {
    let text = read_src("db.rs");
    let violations: Vec<String> = extract_fns(&text)
        .into_iter()
        .filter(|f| {
            f.body.contains("INSERT INTO control_events") && !f.body.contains("control_event_hash")
        })
        .map(|f| f.name)
        .collect();
    assert!(
        violations.is_empty(),
        "control_events insert 未计算 hash(CLAUDE.md:所有审计事件必须进 hash 链): {:?}",
        violations
    );
}
