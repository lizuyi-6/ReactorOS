// CLAUDE.md 不变量(控制流,仅 probe 结构,精确二分需人工 review):
//   start 写失败 → 现场从未开启 → 只 finish_batch、不发 stop;
//   start 成功后失败 → rollback_*_activation(重发 stop,且 stop 也失败时保留 active_batch_id)。
// 对 api.rs 跑。报告 rollback/activate/batch_start 的 stop_write/finish_batch 模式。
import com.intellij.psi.*

val check = localInspection { psiFile, inspection ->
    val text = psiFile.text
    val fnRegex = Regex("fn\\s+(rollback\\w*|\\w*batch_start\\w*|\\w*activate\\w*|\\w*activation\\w*)\\s*\\(")
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
        val hasStopWrite = body.contains("write") && (body.contains("stop") || body.contains("Stop"))
        val hasFinishBatch = body.contains("finish_batch")
        val hasStartWrite = body.contains("start") && body.contains("write")
        val msg = fnName + " | stop_write=" + hasStopWrite + " | finish_batch=" + hasFinishBatch + " | start_write=" + hasStartWrite
        val el = psiFile.findElementAt(m.range.first)
        if (el != null) inspection.registerProblem(el, msg) else inspection.registerProblem(psiFile, msg)
    }
}

listOf(InspectionKts(id = "start-failure-dichotomy-probe", localTool = check, name = "start failure dichotomy probe", htmlDescription = "<html><body>Probe: rollback should stop_write; batch_start failure should finish_batch not stop_write.</body></html>", level = HighlightDisplayLevel.WARNING))
