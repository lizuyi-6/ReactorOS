// CLAUDE.md 不变量:所有真实设备写入必须经安全限幅器(clamp_operator_targets /
// forbidden_control_zone)限幅后再下发。构造 SafeCommand::Write 的函数应调限幅。
// 对 control.rs / api.rs 跑。文本级 PSI(跨函数调用链不追踪,仅单函数体检查)。
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
        val constructsWrite = body.contains("SafeCommand::Write") || body.contains("ControlDecision::Write") || body.contains("Write(SafeCommand")
        if (!constructsWrite) continue
        val clamps = body.contains("clamp_operator_targets") || body.contains("forbidden_control_zone")
        if (!clamps) {
            val el = psiFile.findElementAt(m.range.first)
            val msg = fnName + " constructs SafeCommand::Write but does NOT call clamp_operator_targets/forbidden_control_zone (CLAUDE.md: all device writes must be clamped)"
            if (el != null) inspection.registerProblem(el, msg) else inspection.registerProblem(psiFile, msg)
        }
    }
}

listOf(InspectionKts(id = "clamp-before-write", localTool = check, name = "SafeCommand::Write must be clamped", htmlDescription = "<html><body>CLAUDE.md: all device writes must pass safety clamp before dispatch.</body></html>", level = HighlightDisplayLevel.WARNING))
