// CLAUDE.md 不变量:engage_*/latch_* 必须 saturating_add(1) 推进 generation;
// clear_*/reset_* 绝不能推进 generation(否则复位会伪造"新实例"计数)。
// 对 state.rs 跑。文本级 PSI。
import com.intellij.psi.*

val check = localInspection { psiFile, inspection ->
    val text = psiFile.text
    val fnRegex = Regex("fn\\s+(\\w+)\\s*\\(")
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
        if (!body.contains("_generation")) continue
        val advances = body.contains("saturating_add")
        val isClear = fnName.contains("clear") || fnName.contains("reset")
        if (isClear && advances) {
            val el = psiFile.findElementAt(m.range.first)
            val msg = fnName + " (clear/reset) ADVANCES generation via saturating_add - INVARIANT VIOLATION"
            if (el != null) inspection.registerProblem(el, msg) else inspection.registerProblem(psiFile, msg)
        }
        if (!isClear && !advances) {
            val el = psiFile.findElementAt(m.range.first)
            val msg = fnName + " touches _generation but does NOT saturating_add (engage/latch should advance; or read-only)"
            if (el != null) inspection.registerProblem(el, msg) else inspection.registerProblem(psiFile, msg)
        }
    }
}

listOf(InspectionKts(id = "generation-advance-invariant", localTool = check, name = "engage advances / clear does not", htmlDescription = "<html><body>CLAUDE.md: engage_*/latch_* must saturating_add(1); clear_* must NOT advance.</body></html>", level = HighlightDisplayLevel.WARNING))
