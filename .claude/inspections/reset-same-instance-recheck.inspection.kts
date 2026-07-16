// CLAUDE.md 不变量:reset_* 必须在审计前快照 generation/故障文本,审计后变了就拒绝清除
// (人工复位必须证明处理的是同一实例)。
// 对 api.rs 跑。文本级 PSI。
import com.intellij.psi.*

val check = localInspection { psiFile, inspection ->
    val text = psiFile.text
    val fnRegex = Regex("fn\\s+(reset\\w*)\\s*\\(")
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
        val hasRecheck = body.contains("ensure_safety_latches_unchanged") || body.contains("generation") || body.contains("fault") || body.contains("changed")
        if (!hasRecheck) {
            val el = psiFile.findElementAt(m.range.first)
            val msg = fnName + " (reset) lacks generation/fault recheck after audit (CLAUDE.md: must prove same instance before clearing)"
            if (el != null) inspection.registerProblem(el, msg) else inspection.registerProblem(psiFile, msg)
        }
    }
}

listOf(InspectionKts(id = "reset-same-instance-recheck", localTool = check, name = "reset must recheck same instance", htmlDescription = "<html><body>CLAUDE.md: reset_* must snapshot generation/fault before audit and refuse if changed.</body></html>", level = HighlightDisplayLevel.WARNING))
