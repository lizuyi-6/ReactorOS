// CLAUDE.md 安全不变量检查:所有 commit_* 提交路径必须在提交前调用
// ensure_safety_latches_unchanged_for_commit 复查 generation 快照(审计窗口内 latch 未变)。
//
// 用法(IDEA MCP):run_inspage_kts 读本文件内容作为 inspectionKtsCode,contextPath=src/api.rs。
// 或 IDEA 自定义 inspection.kts 机制加载。
// 文本级 PSI(不依赖 Rust 语义索引),对任何语言文件可跑。
import com.intellij.psi.*

val check = localInspection { psiFile, inspection ->
    val text = psiFile.text
    val marker = "ensure_safety_latches_unchanged"
    val fnRegex = Regex("fn\\s+(commit_\\w+)\\s*\\(")
    for (m in fnRegex.findAll(text)) {
        val fnName = m.groupValues[1]
        val openBrace = text.indexOf('{', m.range.first)
        if (openBrace < 0) continue
        var depth = 0
        var end = -1
        var i = openBrace
        while (i < text.length) {
            val c = text[i]
            if (c == '{') depth++
            else if (c == '}') { depth--; if (depth == 0) { end = i; break } }
            i++
        }
        if (end < 0) continue
        val body = text.substring(openBrace, end)
        if (!body.contains(marker)) {
            val el = psiFile.findElementAt(m.range.first)
            val msg = fnName + " does NOT call " + marker + " (CLAUDE.md invariant: must recheck generation snapshot before commit)"
            if (el != null) inspection.registerProblem(el, msg) else inspection.registerProblem(psiFile, msg)
        }
    }
}

listOf(
    InspectionKts(
        id = "commit-generation-recheck",
        localTool = check,
        name = "commit_* must recheck safety-latch generations",
        htmlDescription = "<html><body>CLAUDE.md invariant: commit_*_after_final_interlock must call ensure_safety_latches_unchanged_for_commit.</body></html>",
        level = HighlightDisplayLevel.WARNING
    )
)
