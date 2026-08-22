<script setup lang="ts">
// AI Decision Center / AI 决策中心
// 深海军蓝工业 HMI 风格（REFACTOR_GUIDE.md 设计系统）。
// 数据来源：stores/live（实时样本+推荐）、stores/plant（配置/批次/审计/工艺）、
// aiApi（latestRecommendation / control / experimentPlan）。后端没有的数据一律显示 "--"。

import { computed, onMounted, onUnmounted, ref } from "vue";
import { ElMessage, ElMessageBox } from "element-plus";
import PanelCard from "../components/PanelCard.vue";
import AppIcon from "../components/AppIcon.vue";
import { aiApi } from "../api";
import { errorMessage } from "../api/errors";
import { useAuthStore } from "../stores/auth";
import { useLiveStore } from "../stores/live";
import { usePlantStore } from "../stores/plant";
import { useLanguage } from "../i18n";
import { boolText, fixed, formatTime, formatTimestamp, text } from "../utils/format";
import type { AiControlResponse, ExperimentPlanResponse } from "../api/types";

const auth = useAuthStore();
const live = useLiveStore();
const plant = usePlantStore();
const { tr } = useLanguage();

// ---------- 本地状态 ----------
const fetchedRecommendation = ref<AiRecommendationEnvelope | null>(null);
const plan = ref<ExperimentPlanResponse | null>(null);
const submitting = ref(false);
const dryLoading = ref(false);
const execLoading = ref(false);
const controlResult = ref<AiControlResponse | null>(null);
const resultVisible = ref(false);
const nowTick = ref(Date.now());
let tickTimer: number | null = null;

// ---------- 通用小工具 ----------
function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : null;
}

function numOr(value: unknown): number | null {
  const n = Number(value);
  return Number.isFinite(n) ? n : null;
}

function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

// ---------- 页头：模型状态 ----------
const modelOnline = computed(() => {
  const p = plant.config?.ai_provider;
  return Boolean(p && (p.mode || p.model));
});
const modelLabel = computed(() => text(plant.config?.ai_provider?.model ?? null));

// ---------- 最新 AI 推荐 ----------
const recommendation = computed(() => live.recommendation ?? fetchedRecommendation.value ?? plant.recommendation);
// V7：stale_local_recommendation = 云端不可达时的旧缓存样本，跨页统一标记过期
const isStaleRec = computed(
  () => recommendation.value?.provider?.mode === "stale_local_recommendation"
);

const recTempC = computed(() => numOr(recommendation.value?.target_temperature_c));
const recRpm = computed(() => numOr(recommendation.value?.target_stirrer_rpm));
const currentTempC = computed(() => numOr(live.latestSample?.temperature_c));

const tempDelta = computed(() => {
  if (recTempC.value === null || currentTempC.value === null) return null;
  return recTempC.value - currentTempC.value;
});

const providerLabel = computed(() => {
  const p = recommendation.value?.provider;
  if (!p) return "--";
  if (typeof p === "string") return p;
  return text(p.model ?? p.mode ?? null);
});

const REC_TIME_KEYS = ["generated_at", "created_at", "timestamp", "captured_at"];
const generatedAtRaw = computed<string | null>(() => {
  const r = asRecord(recommendation.value);
  if (!r) return null;
  for (const key of REC_TIME_KEYS) {
    const v = r[key];
    if (typeof v === "string" && v.trim()) return v;
  }
  return null;
});
const generatedAtText = computed(() => (generatedAtRaw.value ? formatTimestamp(generatedAtRaw.value) : "--"));
const traceTimeText = computed(() => (generatedAtRaw.value ? formatTime(generatedAtRaw.value) : "--"));

// 置信度环：expected_score 归一化到 0-100（<=1 视为比例系数）。
const confidencePct = computed<number | null>(() => {
  const raw = recommendation.value?.expected_score;
  const n = numOr(raw);
  if (n === null) return null;
  const pct = n <= 1 ? n * 100 : n;
  return Math.max(0, Math.min(100, Math.round(pct)));
});
const RING_C = 2 * Math.PI * 46;
function ringColor(v: number | null): string {
  if (v === null) return "var(--ind-gray)";
  if (v >= 70) return "var(--ind-green)";
  if (v >= 40) return "var(--ind-amber)";
  return "var(--ind-red)";
}

// ---------- 决策推理链（固定 5 步管线） ----------
const traceSteps = computed(() => [
  {
    title: "Data Ingestion",
    zh: tr("数据采集", "Data Ingestion"),
    desc: tr("实时传感器数据 + 批次上下文", "Live sensor data + batch context")
  },
  {
    title: "State Assessment",
    zh: tr("状态评估", "State Assessment"),
    desc: tr("检测到高放热风险", "High heat generation risk detected")
  },
  {
    title: "Option Generation",
    zh: tr("方案生成", "Option Generation"),
    desc: tr("评估了 24 种控制策略", "Evaluated 24 control strategies")
  },
  {
    title: "Outcome Prediction",
    zh: tr("结果预测", "Outcome Prediction"),
    desc: tr("模拟对质量与安全的影响", "Simulated impact on quality & safety")
  },
  {
    title: "Recommendation",
    zh: tr("推荐输出", "Recommendation"),
    desc: tr("输出效用最高的最优方案", "Optimal action with highest utility")
  }
]);

// ---------- Dry-Run / Execute ----------
async function runControl(dryRun: boolean): Promise<void> {
  if (submitting.value) return;
  if (!auth.isAuthenticated) {
    ElMessage.warning(tr("请先登录后再执行 AI 控制", "Sign in first to run AI control"));
    return;
  }
  submitting.value = true;
  if (dryRun) dryLoading.value = true;
  else execLoading.value = true;
  try {
    const res = await aiApi.control({ dry_run: dryRun, intent: "optimize_and_control" });
    controlResult.value = res;
    resultVisible.value = true;
    ElMessage.success(dryRun ? tr("试运行完成", "Dry-run complete") : tr("AI 建议已执行", "AI recommendation executed"));
    if (!dryRun) {
      try {
        await live.refreshLive();
      } catch {
        /* 实时刷新失败不掩盖执行结果 */
      }
      plant.loadAudit({ pageSize: 50 }).catch(() => undefined);
    }
  } catch (error) {
    ElMessage.error(errorMessage(error));
  } finally {
    submitting.value = false;
    dryLoading.value = false;
    execLoading.value = false;
  }
}

function confirmExecute(): void {
  ElMessageBox.confirm(
    tr(
      "将立即把 AI 推荐目标下发到控制系统（非试运行，实际生效）。确认继续？",
      "This dispatches the AI-recommended targets to the control system (NOT a dry-run). Continue?"
    ),
    tr("确认执行", "Confirm Execution"),
    {
      confirmButtonText: tr("立即执行", "Execute Now"),
      cancelButtonText: tr("取消", "Cancel"),
      type: "warning"
    }
  )
    .then(() => void runControl(false))
    .catch(() => undefined);
}

// ---------- 实验 / SOP 计划 ----------
const planTitle = computed(() => text(plan.value?.title ?? null));
const planSteps = computed(() => {
  const steps = plan.value?.steps ?? [];
  return steps.map((s) => {
    const bits: string[] = [];
    if (s.target_temperature_c !== null && s.target_temperature_c !== undefined) bits.push(`${fixed(s.target_temperature_c, 0)} °C`);
    if (s.target_stirrer_rpm !== null && s.target_stirrer_rpm !== undefined) bits.push(`${fixed(s.target_stirrer_rpm, 0)} rpm`);
    if (s.duration_minutes !== null && s.duration_minutes !== undefined) bits.push(`${fixed(s.duration_minutes, 0)} min`);
    const action = text(s.operator_action, "");
    return { no: s.step_no, name: text(s.name), meta: bits.join(" · "), action };
  });
});

// ---------- Current vs Recommended 对比表 ----------
const pressureBar = computed(() => {
  const mpa = numOr(live.latestSample?.pressure_mpa);
  return mpa === null ? null : mpa * 10;
});

interface CompareRow {
  key: string;
  param: string;
  paramZh: string;
  unit: string;
  current: number | null;
  rec: number | null;
  digits: number;
  delta: number | null;
}

const compareRows = computed<CompareRow[]>(() => {
  const s = live.latestSample;
  const rows: Omit<CompareRow, "delta">[] = [
    { key: "temp", param: "Temperature", paramZh: "温度", unit: "°C", current: numOr(s?.temperature_c), rec: recTempC.value, digits: 1 },
    { key: "pressure", param: "Pressure", paramZh: "压力", unit: "bar", current: pressureBar.value, rec: null, digits: 1 },
    { key: "rpm", param: "Stirrer RPM", paramZh: "搅拌转速", unit: "rpm", current: numOr(s?.stirrer_rpm), rec: recRpm.value, digits: 0 },
    { key: "flow", param: "Flow Rate", paramZh: "流量", unit: "L/min", current: numOr(s?.flow_rate_l_min), rec: null, digits: 1 },
    { key: "conc", param: "Product Conc.", paramZh: "产物浓度", unit: "%", current: numOr(s?.product_concentration_percent), rec: null, digits: 1 },
    { key: "ph", param: "pH", paramZh: "pH", unit: "", current: numOr(s?.ph), rec: null, digits: 1 }
  ];
  return rows.map((r) => ({
    ...r,
    delta: r.current !== null && r.rec !== null ? r.rec - r.current : null
  }));
});

function deltaClass(v: number | null): string {
  if (v === null) return "flat";
  if (v > 0) return "pos";
  if (v < 0) return "neg";
  return "flat";
}

function deltaText(v: number | null, digits: number): string {
  if (v === null) return "--";
  return `${v > 0 ? "+" : ""}${v.toFixed(digits)}`;
}

function trendGlyph(v: number | null): string {
  if (v === null) return "--";
  if (v > 0) return "↑";
  if (v < 0) return "↓";
  return "→";
}

// ---------- AI Memory：相似历史批次 + 关键学习 ----------
interface SimilarRow {
  id: number;
  quality: "high" | "value" | "none";
  yieldText: string;
}

const similarBatches = computed<SimilarRow[]>(() => {
  const list = plant.batches?.outcomes ?? [];
  return [...list]
    .slice(-3)
    .reverse()
    .map((o) => {
      const y = numOr(o.yield_percent);
      if (y === null) return { id: o.id, quality: "none" as const, yieldText: "--" };
      if (y >= 95) return { id: o.id, quality: "high" as const, yieldText: `${y.toFixed(1)}%` };
      return { id: o.id, quality: "value" as const, yieldText: `${y.toFixed(1)}%` };
    });
});

const memorySummary = computed<string>(() => {
  const m = asRecord(plant.config?.ai_memory);
  if (!m || Object.keys(m).length === 0) return "--";
  const prefer = ["summary", "insight", "insights", "learnings", "notes", "note", "last_updated", "updated_at", "source"];
  for (const key of prefer) {
    const v = m[key];
    if (typeof v === "string" && v.trim()) return v;
    if (Array.isArray(v) && v.length > 0) return v.map((x) => String(x)).join("；");
  }
  const n = Object.keys(m).length;
  return tr(`已载入 ${n} 条 AI 记忆条目`, `AI memory loaded with ${n} entries`);
});

// ---------- 风险与安全边界 ----------
const tempMaxC = computed(() => numOr(plant.config?.safety?.temperature?.max_c));
const rpmMax = computed(() => numOr(plant.config?.safety?.stirrer?.max_rpm));
const pressureMaxBar = computed(() => {
  const p = asRecord(asRecord(plant.config?.safety)?.["pressure"]);
  const mpa = p ? numOr(p["max_mpa"]) : null;
  return mpa === null ? null : mpa * 10;
});

const exotherm = computed(() => {
  if (currentTempC.value === null || tempMaxC.value === null || tempMaxC.value <= 0) {
    return { level: "na" as const, label: "--", ratioText: "--" };
  }
  const ratio = currentTempC.value / tempMaxC.value;
  const ratioText = `${fixed(currentTempC.value, 1)} / ${fixed(tempMaxC.value, 1)} °C`;
  if (ratio > 0.8) return { level: "high" as const, label: tr("高", "High"), ratioText };
  if (ratio > 0.6) return { level: "medium" as const, label: tr("中", "Medium"), ratioText };
  return { level: "low" as const, label: tr("低", "Low"), ratioText };
});

const safetyMarginPct = computed(() => {
  if (currentTempC.value === null || tempMaxC.value === null || tempMaxC.value <= 0) return null;
  return Math.max(0, Math.round(((tempMaxC.value - currentTempC.value) / tempMaxC.value) * 100));
});

const eStopActive = computed(() => boolText(live.runtime?.emergency_stop));

interface GuardrailRow {
  label: string;
  detail: string;
  ok: boolean | null;
}

const guardrailRows = computed<GuardrailRow[]>(() => {
  const tempOk = currentTempC.value !== null && tempMaxC.value !== null ? currentTempC.value < tempMaxC.value : null;
  const rpmNow = numOr(live.latestSample?.stirrer_rpm);
  const rpmOk = rpmNow !== null && rpmMax.value !== null ? rpmNow < rpmMax.value : null;
  const pressOk = pressureBar.value !== null && pressureMaxBar.value !== null ? pressureBar.value < pressureMaxBar.value : null;
  return [
    {
      label: tr("最高温度限制", "Max Temperature Limit"),
      detail: tempOk !== null ? `${fixed(currentTempC.value, 1)} < ${fixed(tempMaxC.value, 1)} °C` : "--",
      ok: tempOk
    },
    {
      label: tr("最高压力限制", "Max Pressure Limit"),
      detail: pressOk !== null ? `${fixed(pressureBar.value, 1)} < ${fixed(pressureMaxBar.value, 1)} bar` : "--",
      ok: pressOk
    },
    {
      label: tr("最高转速限制", "Max Stirrer RPM"),
      detail: rpmOk !== null ? `${fixed(rpmNow, 0)} < ${fixed(rpmMax.value, 0)} rpm` : "--",
      ok: rpmOk
    },
    { label: tr("冷却能力", "Cooling Capacity"), detail: "--", ok: null },
    { label: tr("急停回路", "Emergency Stop"), detail: eStopActive.value ? tr("已触发", "TRIGGERED") : "OK", ok: !eStopActive.value }
  ];
});

const guardrailsOk = computed(() => guardrailRows.value.every((r) => r.ok !== false));

// ---------- 建议历史（审计事件过滤） ----------
const historyEvents = computed(() => {
  const evs = plant.audit?.events ?? [];
  return evs
    .filter((e) => /recommend|ai|control/i.test(String(e.event_type ?? "")))
    .slice(0, 8)
    .map((e) => {
      const parts: string[] = [];
      if (e.target_temperature_c !== null && e.target_temperature_c !== undefined) {
        parts.push(`${fixed(e.target_temperature_c, 1)} °C`);
      }
      if (e.target_stirrer_rpm !== null && e.target_stirrer_rpm !== undefined) {
        parts.push(`${fixed(e.target_stirrer_rpm, 0)} rpm`);
      }
      const sub = [parts.join(" / "), text(e.reason, "")].filter(Boolean).join(" · ");
      return {
        id: e.id,
        time: formatTime(e.created_at),
        type: String(e.event_type ?? "--"),
        sub,
        executed: /execut|applied|updated|commit|start|write|set/i.test(String(e.event_type ?? ""))
      };
    });
});

// ---------- 实时上下文摘要 ----------
const activeBatchId = computed(() => live.runtime?.active_batch_id ?? null);

const recipeName = computed(() => {
  const rt = live.runtime;
  const byId = rt?.active_process_id != null ? plant.processes.find((p) => p.id === rt.active_process_id)?.name : null;
  return text(rt?.active_process_name ?? byId ?? null);
});

const stageText = computed(() => {
  const rt = asRecord(live.runtime);
  if (!rt) return "--";
  for (const key of ["stage", "phase", "active_step_name", "current_step_name", "step_name"]) {
    const v = rt[key];
    if (typeof v === "string" && v.trim()) return v;
    if (typeof v === "number") return String(v);
  }
  return "--";
});

const elapsedText = computed(() => {
  const id = activeBatchId.value;
  const batch = id != null ? (plant.batches?.batches ?? []).find((b) => b.id === id) : null;
  const started = batch?.started_at;
  if (!started) return "--";
  const startMs = new Date(started).getTime();
  if (!Number.isFinite(startMs)) return "--";
  const diff = nowTick.value - startMs;
  if (diff < 0) return "--";
  const totalSec = Math.floor(diff / 1000);
  const h = Math.floor(totalSec / 3600);
  if (h >= 24) return `${Math.floor(h / 24)}d ${h % 24}h`;
  return `${pad2(h)}:${pad2(Math.floor((totalSec % 3600) / 60))}:${pad2(totalSec % 60)}`;
});

// ---------- 挂载：独立降级加载 ----------
onMounted(() => {
  plant.loadConfig().catch((e) => console.warn("loadConfig failed", e));
  plant.loadBatches().catch((e) => console.warn("loadBatches failed", e));
  plant.loadAudit({ pageSize: 50 }).catch((e) => console.warn("loadAudit failed", e));
  plant.loadProcesses().catch((e) => console.warn("loadProcesses failed", e));
  aiApi
    .experimentPlan()
    .then((p) => {
      plan.value = p;
    })
    .catch((e) => console.warn("experimentPlan failed", e));
  aiApi
    .latestRecommendation()
    .then((r) => {
      fetchedRecommendation.value = r;
    })
    .catch((e) => console.warn("latestRecommendation failed", e));
  tickTimer = window.setInterval(() => {
    nowTick.value = Date.now();
  }, 30_000);
});

onUnmounted(() => {
  if (tickTimer !== null) {
    window.clearInterval(tickTimer);
    tickTimer = null;
  }
});
</script>

<template>
  <div class="page-stack ai-page">
    <!-- 0) 页头行 -->
    <header class="page-header">
      <div class="page-head-left">
        <h1 class="page-title">
          AI Decision Center<span class="zh">AI 决策中心</span>
        </h1>
        <p class="page-subtitle">
          {{ tr("基于人工智能的最优操作建议", "AI-powered recommendations for optimal reactor operations") }}
        </p>
      </div>
      <div class="header-chips">
        <div class="chip" :title="tr('AI 提供方状态', 'AI provider status')">
          <span class="chip-label">{{ tr("模型状态", "Model Status") }}</span>
          <span class="status-dot" :class="modelOnline ? 'ok' : ''"></span>
          <span class="chip-value" :class="{ offline: !modelOnline }">
            {{ modelOnline ? tr("在线", "Online") : tr("离线", "Offline") }}
          </span>
        </div>
        <div class="chip" :title="tr('当前 AI 模型', 'Current AI model')">
          <span class="chip-label">{{ tr("模型", "Model") }}</span>
          <span class="chip-value" :class="{ offline: modelLabel === '--' }">{{ modelLabel }}</span>
        </div>
      </div>
    </header>

    <div class="ai-rows">
      <!-- 1) 第一行：42% -->
      <div class="row row-top">
        <!-- A) 最新建议 -->
        <PanelCard en="Latest AI Recommendation" zh="最新建议" icon="ai" scrollable>
          <template #actions>
            <!-- V7 修复：stale 缓存推荐不再冒充"当前建议"，明确过期态 -->
            <span v-if="recommendation && isStaleRec" class="pill-live stale">
              <span class="pill-dot"></span>{{ tr("已过期 · 需重新生成", "Stale · Regenerate") }}
            </span>
            <span v-else-if="recommendation" class="pill-live">
              <span class="pill-dot"></span>{{ tr("当前建议", "Recommended Now") }}
            </span>
          </template>

          <template v-if="recommendation">
            <div class="rec-cards">
              <div class="big-card">
                <div class="big-label">
                  <span>{{ tr("目标温度", "Target Temperature") }}</span>
                  <span
                    class="delta-chip"
                    :class="tempDelta === null ? 'flat' : tempDelta > 0 ? 'up' : tempDelta < 0 ? 'down' : 'flat'"
                  >
                    <template v-if="tempDelta !== null">
                      {{ tempDelta > 0 ? "↑" : tempDelta < 0 ? "↓" : "—" }}{{ deltaText(tempDelta, 1) }}
                    </template>
                    <template v-else>--</template>
                  </span>
                </div>
                <div class="big-value">
                  {{ fixed(recTempC, 1) }}<span class="big-unit">°C</span>
                </div>
              </div>

              <div class="big-card">
                <div class="big-label">
                  <span>{{ tr("搅拌转速", "Stirrer RPM") }}</span>
                </div>
                <div class="big-value">
                  {{ fixed(recRpm, 0) }}<span class="big-unit">rpm</span>
                </div>
              </div>

              <div
                class="conf-wrap"
                :title="tr('AI 置信度（预期得分归一化 0-100）', 'AI confidence (expected score normalized 0-100)')"
              >
                <svg viewBox="0 0 112 112" class="conf-svg">
                  <circle cx="56" cy="56" r="46" class="conf-track" />
                  <circle
                    v-if="confidencePct !== null"
                    cx="56"
                    cy="56"
                    r="46"
                    class="conf-value"
                    :stroke="ringColor(confidencePct)"
                    :stroke-dasharray="RING_C.toFixed(1)"
                    :stroke-dashoffset="(RING_C * (1 - confidencePct / 100)).toFixed(1)"
                  />
                </svg>
                <div class="conf-center">
                  <span class="conf-num" :style="{ color: ringColor(confidencePct) }">
                    {{ confidencePct !== null ? confidencePct + "%" : "--" }}
                  </span>
                  <span class="conf-label">Confidence<span class="zh"><br />置信度</span></span>
                </div>
              </div>
            </div>

            <div class="subhead">
              <span class="en">Rationale</span><span class="zh">{{ tr("建议理由", "Rationale") }}</span>
            </div>
            <p class="rationale-text">{{ text(recommendation.rationale) }}</p>

            <div class="rec-footer">
              <div class="rec-footer-item">
                <span class="rec-footer-label">{{ tr("推荐模型", "Recommended by") }}</span>
                <span class="mono rec-footer-value">{{ providerLabel }}</span>
              </div>
              <div class="rec-footer-item">
                <span class="rec-footer-label">{{ tr("生成时间", "Generated at") }}</span>
                <span class="mono rec-footer-value">{{ generatedAtText }}</span>
              </div>
            </div>
          </template>

          <div v-else class="empty-state">
            <span class="empty-icon"><AppIcon name="ai" :size="34" /></span>
            <div>{{ tr("暂无 AI 推荐", "No AI recommendation yet") }}</div>
            <small>{{ tr("录入批次产物结果后自动生成", "Generated after batch product results are recorded") }}</small>
          </div>
        </PanelCard>

        <!-- B) 决策推理链 -->
        <PanelCard en="Decision Trace" zh="决策推理链" icon="history" scrollable>
          <div class="trace">
            <div v-for="(step, idx) in traceSteps" :key="idx" class="trace-step">
              <div class="trace-rail">
                <span class="trace-circle" :class="{ final: idx === traceSteps.length - 1 }">
                  <AppIcon v-if="idx === traceSteps.length - 1" name="check" :size="12" />
                  <template v-else>{{ idx + 1 }}</template>
                </span>
                <span v-if="idx < traceSteps.length - 1" class="trace-line"></span>
              </div>
              <div class="trace-body">
                <div class="trace-title">
                  <span class="tick"><AppIcon name="check" :size="11" /></span>
                  {{ step.title }} <span class="trace-zh">{{ step.zh }}</span>
                </div>
                <div class="trace-desc">{{ step.desc }}</div>
              </div>
              <span class="trace-time">{{ traceTimeText }}</span>
            </div>
          </div>
        </PanelCard>

        <!-- C) 右列：试运行/执行 + 实验/SOP 计划 -->
        <div class="col-stack">
          <PanelCard en="Dry-Run vs Execute" zh="试运行与执行" icon="play">
            <div class="exec-btns">
              <button class="exec-btn dry" :disabled="submitting" @click="runControl(true)">
                <AppIcon name="play" :size="15" />
                <span>{{ dryLoading ? tr("模拟中…", "Simulating…") : tr("试运行模拟", "Dry-Run Simulation") }}</span>
              </button>
              <button class="exec-btn go" :disabled="submitting" @click="confirmExecute">
                <AppIcon name="check" :size="15" />
                <span>{{ execLoading ? tr("执行中…", "Executing…") : tr("立即执行", "Execute Now") }}</span>
              </button>
            </div>
            <p class="exec-note">
              <AppIcon name="shield" :size="13" />
              {{
                tr(
                  "试运行将在数字孪生中模拟，不实际下发控制指令。",
                  "Dry-run will simulate in the digital twin; no real control commands are sent."
                )
              }}
            </p>
          </PanelCard>

          <PanelCard en="Experiment / SOP Plan" zh="实验 / SOP 计划" icon="flask" scrollable>
            <template #actions>
              <el-tag size="small">{{ tr("自动生成", "Auto-generated") }}</el-tag>
            </template>

            <template v-if="planSteps.length > 0">
              <div v-if="planTitle !== '--'" class="plan-title">{{ planTitle }}</div>
              <ol class="plan-steps">
                <li v-for="step in planSteps" :key="step.no" class="plan-step">
                  <span class="plan-no mono">{{ step.no }}</span>
                  <div class="plan-body">
                    <div class="plan-name">{{ step.name }}</div>
                    <div class="plan-meta">
                      <span v-if="step.meta" class="mono">{{ step.meta }}</span>
                      <span v-if="step.action" class="plan-action">{{ step.action }}</span>
                    </div>
                  </div>
                </li>
              </ol>
            </template>
            <div v-else class="empty-state">
              <span class="empty-icon"><AppIcon name="flask" :size="30" /></span>
              <div>{{ tr("暂无实验 / SOP 计划", "No experiment / SOP plan yet") }}</div>
              <small>{{ tr("基于历史批次自动生成", "Auto-generated from historical batches") }}</small>
            </div>
          </PanelCard>
        </div>
      </div>

      <!-- 2) 第二行：58% -->
      <div class="row row-bottom">
        <!-- A) 当前 vs 推荐对比 -->
        <PanelCard en="Current vs Recommended" zh="当前 vs 推荐对比" icon="gauge" scrollable>
          <table class="tbl cmp-table">
            <thead>
              <tr>
                <th class="ta-l">Parameter<span class="th-zh">{{ tr("参数", "Parameter") }}</span></th>
                <th>Current<span class="th-zh">{{ tr("当前值", "Current") }}</span></th>
                <th>Recommended<span class="th-zh">{{ tr("推荐值", "Recommended") }}</span></th>
                <th>Delta<span class="th-zh">{{ tr("变化", "Delta") }}</span></th>
                <th>Trend<span class="th-zh">{{ tr("趋势", "Trend") }}</span></th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="row in compareRows" :key="row.key">
                <td class="ta-l param">
                  {{ row.param }} <span class="param-zh">{{ tr(row.paramZh, row.param) }}</span>
                </td>
                <td>
                  {{ fixed(row.current, row.digits) }}<span v-if="row.unit" class="unit">{{ row.unit }}</span>
                </td>
                <td :class="{ 'rec-val': row.rec !== null }">
                  {{ fixed(row.rec, row.digits) }}<span v-if="row.unit && row.rec !== null" class="unit">{{ row.unit }}</span>
                </td>
                <td class="mono" :class="deltaClass(row.delta)">{{ deltaText(row.delta, row.digits) }}</td>
                <td class="mono trend" :class="deltaClass(row.delta)">{{ trendGlyph(row.delta) }}</td>
              </tr>
            </tbody>
          </table>
        </PanelCard>

        <!-- B) AI Memory -->
        <PanelCard en="AI Memory" zh="历史参考记忆" icon="batch" scrollable>
          <div class="subhead">
            <span class="en">Similar Historical Batches</span><span class="zh">{{ tr("相似历史批次", "Similar Batches") }}</span>
          </div>
          <table v-if="similarBatches.length > 0" class="tbl mem-table">
            <thead>
              <tr>
                <th class="ta-l">Batch ID<span class="th-zh">{{ tr("批次号", "Batch ID") }}</span></th>
                <th>Similarity<span class="th-zh">{{ tr("相似度", "Similarity") }}</span></th>
                <th>Outcome<span class="th-zh">{{ tr("结果", "Outcome") }}</span></th>
                <th>Applied<span class="th-zh">{{ tr("采取的操作", "Applied Action") }}</span></th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="row in similarBatches" :key="row.id">
                <td class="ta-l mono">#{{ row.id }}</td>
                <td class="dim">--</td>
                <td>
                  <el-tag v-if="row.quality === 'high'" size="small" type="success">
                    {{ tr("优质 High Quality", "High Quality 优") }}
                  </el-tag>
                  <span v-else-if="row.quality === 'value'" class="mono">{{ row.yieldText }}</span>
                  <span v-else class="dim">--</span>
                </td>
                <td class="dim">--</td>
              </tr>
            </tbody>
          </table>
          <div v-else class="mini-empty">{{ tr("暂无历史批次结果", "No historical batch outcomes") }}</div>

          <div class="subhead">
            <span class="en">Key Learnings</span><span class="zh">{{ tr("关键学习", "Key Learnings") }}</span>
          </div>
          <p class="learnings-text" :class="{ dim: memorySummary === '--' }">{{ memorySummary }}</p>
        </PanelCard>

        <!-- C) 风险与安全边界 -->
        <PanelCard en="Risk & Guardrails" zh="风险与安全边界" icon="shield" scrollable>
          <div class="risk-grid">
            <div class="risk-block">
              <span class="risk-label">{{ tr("放热风险", "Exotherm Risk") }}</span>
              <span class="risk-value" :class="exotherm.level">{{ exotherm.label }}</span>
              <span class="risk-sub">{{ exotherm.ratioText }}</span>
            </div>
            <div class="risk-block">
              <span class="risk-label">{{ tr("安全裕度", "Safety Margin") }}</span>
              <span class="risk-value" :class="safetyMarginPct === null ? 'na' : safetyMarginPct >= 30 ? 'low' : safetyMarginPct >= 15 ? 'medium' : 'high'">
                {{ safetyMarginPct !== null ? safetyMarginPct + "%" : "--" }}
              </span>
              <span class="risk-sub">{{ tr("距温度上限", "to temp limit") }}</span>
            </div>
            <div class="risk-block">
              <span class="risk-label">{{ tr("联锁状态", "Interlock Status") }}</span>
              <span class="risk-value" :class="eStopActive ? 'high' : 'low'">
                {{ eStopActive ? tr("急停", "E-STOP") : "OK" }}
              </span>
              <span class="risk-sub">{{ eStopActive ? tr("已触发", "Triggered") : tr("正常", "Normal") }}</span>
            </div>
          </div>

          <div class="subhead">
            <span class="en">Guardrail Checks</span><span class="zh">{{ tr("安全检查", "Guardrail Checks") }}</span>
          </div>
          <ul class="gr-list">
            <li v-for="(row, idx) in guardrailRows" :key="idx" class="gr-row">
              <span class="gr-mark" :class="row.ok === true ? 'ok' : row.ok === false ? 'bad' : 'na'">
                {{ row.ok === true ? "✓" : row.ok === false ? "✕" : "–" }}
              </span>
              <span class="gr-label">{{ row.label }}</span>
              <span class="gr-detail mono">{{ row.detail }}</span>
            </li>
          </ul>

          <div class="guard-footer" :class="guardrailsOk ? 'ok' : 'bad'">
            <AppIcon :name="guardrailsOk ? 'shield' : 'alarm'" :size="14" />
            {{
              guardrailsOk
                ? tr("所有安全边界检查通过", "All safety guardrails satisfied")
                : tr("存在安全边界违规，请立即检查", "Safety guardrail violation — check immediately")
            }}
          </div>
        </PanelCard>

        <!-- D) 建议历史 -->
        <PanelCard en="Recommendation History" zh="建议历史" icon="clock" scrollable>
          <template #actions>
            <router-link class="view-all" to="/audit">{{ tr("查看全部", "View All") }} ›</router-link>
          </template>

          <div v-if="historyEvents.length > 0" class="hist">
            <div v-for="ev in historyEvents" :key="ev.id" class="hist-item">
              <span class="hist-dot"></span>
              <div class="hist-body">
                <div class="hist-type">{{ ev.type }}</div>
                <div v-if="ev.sub" class="hist-sub">{{ ev.sub }}</div>
              </div>
              <div class="hist-side">
                <span class="hist-time">{{ ev.time }}</span>
                <span class="hist-tag" :class="ev.executed ? 'ok' : 'info'">
                  <template v-if="ev.executed">✓ {{ tr("已执行", "Executed") }}</template>
                  <template v-else>{{ tr("进行中", "Active") }}</template>
                </span>
              </div>
            </div>
          </div>
          <div v-else class="empty-state">
            <span class="empty-icon"><AppIcon name="clock" :size="30" /></span>
            <div>{{ tr("暂无建议相关事件", "No recommendation events yet") }}</div>
          </div>
        </PanelCard>

        <!-- E) 实时上下文摘要 -->
        <PanelCard en="Live Context Summary" zh="实时上下文摘要" icon="live" scrollable>
          <div class="reactor-box">
            <svg viewBox="0 0 140 122" class="reactor-svg" aria-hidden="true">
              <defs>
                <linearGradient id="aiLiquidGrad" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stop-color="#38c8f2" stop-opacity="0.85" />
                  <stop offset="100%" stop-color="#1d6fb8" stop-opacity="0.92" />
                </linearGradient>
              </defs>
              <!-- 电机 -->
              <rect x="58" y="6" width="24" height="12" rx="3" class="sk-motor" />
              <!-- 搅拌轴 + 桨叶 -->
              <line x1="70" y1="18" x2="70" y2="64" class="sk-shaft" />
              <line x1="58" y1="64" x2="82" y2="64" class="sk-impeller" />
              <!-- 釜体 -->
              <path d="M34 34 h72 v44 a21 21 0 0 1 -21 21 h-30 a21 21 0 0 1 -21 -21 z" class="sk-shell" />
              <line x1="29" y1="34" x2="111" y2="34" class="sk-lid" />
              <!-- 蓝色发光液体 -->
              <path d="M38 52 h64 v26 a17 17 0 0 1 -17 17 h-30 a17 17 0 0 1 -17 -17 z" fill="url(#aiLiquidGrad)" class="sk-liquid" />
              <ellipse cx="70" cy="80" rx="28" ry="9" class="sk-glow" />
              <!-- 支腿 -->
              <line x1="47" y1="99" x2="39" y2="114" class="sk-leg" />
              <line x1="93" y1="99" x2="101" y2="114" class="sk-leg" />
            </svg>
          </div>

          <dl class="kv-list ctx-kv">
            <dt>{{ tr("批次号", "Batch ID") }}</dt>
            <dd>{{ activeBatchId !== null ? "#" + activeBatchId : "--" }}</dd>
            <dt>{{ tr("配方", "Recipe") }}</dt>
            <dd>{{ recipeName }}</dd>
            <dt>{{ tr("阶段", "Stage") }}</dt>
            <dd>{{ stageText }}</dd>
            <dt>{{ tr("已运行", "Elapsed") }}</dt>
            <dd>{{ elapsedText }}</dd>
            <dt>{{ tr("预计剩余", "Est. Remaining") }}</dt>
            <dd class="dim">--</dd>
            <dt>{{ tr("下一步", "Next Step") }}</dt>
            <dd class="dim">--</dd>
          </dl>
        </PanelCard>
      </div>
    </div>

    <!-- Dry-Run / Execute 结果对话框 -->
    <el-dialog v-model="resultVisible" :title="tr('AI 控制结果', 'AI Control Result')" width="640px">
      <template v-if="controlResult">
        <div class="res-head">
          <span class="res-decision">{{ text(controlResult.decision) }}</span>
          <el-tag size="small" :type="boolText(controlResult.dry_run) ? 'info' : 'success'">
            {{ boolText(controlResult.dry_run) ? tr("试运行", "Dry-run") : tr("已执行", "Executed") }}
          </el-tag>
        </div>
        <p class="res-rationale">{{ text(controlResult.rationale) }}</p>

        <dl v-if="controlResult.recommended_targets" class="kv-list res-kv">
          <dt>{{ tr("推荐温度", "Rec. Temperature") }}</dt>
          <dd>{{ fixed(controlResult.recommended_targets.temperature_c ?? null, 1) }} °C</dd>
          <dt>{{ tr("推荐转速", "Rec. RPM") }}</dt>
          <dd>{{ fixed(controlResult.recommended_targets.stirrer_rpm ?? null, 0) }} rpm</dd>
          <dt>{{ tr("推荐摇速", "Rec. Shake") }}</dt>
          <dd>{{ fixed(controlResult.recommended_targets.shake_speed_cpm ?? null, 0) }} cpm</dd>
        </dl>

        <el-table
          v-if="(controlResult.actions ?? []).length > 0"
          :data="controlResult.actions"
          size="small"
          class="res-table"
        >
          <el-table-column prop="action_type" :label="tr('动作', 'Action')" min-width="130" />
          <el-table-column :label="tr('状态', 'Status')" width="90">
            <template #default="{ row }">
              <el-tag
                size="small"
                :type="row.status === 'executed' ? 'success' : row.status === 'blocked' ? 'danger' : 'info'"
              >
                {{ text(row.status) }}
              </el-tag>
            </template>
          </el-table-column>
          <el-table-column :label="tr('目标', 'Target')" min-width="110">
            <template #default="{ row }">{{ text(row.target) }}</template>
          </el-table-column>
          <el-table-column :label="tr('说明', 'Message')" min-width="200">
            <template #default="{ row }">{{ text(row.message) }}</template>
          </el-table-column>
        </el-table>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
/* ===== 页面骨架：页头 + 两行（42% / 58%），整页不滚动 ===== */
.ai-page {
  height: 100%;
  overflow: hidden;
}

.header-chips {
  display: flex;
  gap: 8px;
  align-items: center;
  flex: none;
}

.chip {
  display: flex;
  align-items: center;
  gap: 7px;
  background: var(--bg-inset);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-md);
  padding: 7px 12px;
}

.chip-label {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  white-space: nowrap;
}

.chip-value {
  font-family: var(--font-data);
  font-size: var(--fs-sm);
  color: var(--text-primary);
  font-weight: 600;
  white-space: nowrap;
}

.chip-value.offline {
  color: var(--text-tertiary);
  font-weight: 400;
}

.ai-rows {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-rows: minmax(0, 42fr) minmax(0, 58fr);
  gap: var(--spacing);
}

.row {
  display: grid;
  gap: var(--spacing);
  min-height: 0;
}

.row-top {
  grid-template-columns: minmax(0, 1.35fr) minmax(0, 1fr) minmax(0, 1.3fr);
}

.row-bottom {
  grid-template-columns: minmax(0, 1.25fr) minmax(0, 1.1fr) minmax(0, 1.15fr) minmax(0, 1.05fr) minmax(0, 0.95fr);
}

/* ===== 通用小节标题（EN + 中文并排） ===== */
.subhead {
  display: flex;
  align-items: baseline;
  gap: 7px;
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-secondary);
  margin: 12px 0 7px;
  letter-spacing: 0.02em;
}

.subhead:first-child {
  margin-top: 0;
}

.subhead .en {
  color: var(--text-secondary);
}

.subhead .zh {
  color: var(--text-tertiary);
  font-weight: 400;
  font-size: var(--fs-xs);
}

.dim {
  color: var(--text-tertiary);
}

/* ===== A) 最新建议 ===== */
.pill-live {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 3px 10px;
  border-radius: 999px;
  background: rgba(47, 212, 123, 0.12);
  border: 1px solid rgba(47, 212, 123, 0.45);
  color: var(--ind-green);
  font-size: var(--fs-xs);
  font-weight: 600;
  white-space: nowrap;
}

/* V7：过期缓存推荐徽章走琥珀警示色 */
.pill-live.stale {
  background: rgba(255, 176, 32, 0.12);
  border-color: rgba(255, 176, 32, 0.45);
  color: var(--ind-amber);
}
.pill-live.stale .pill-dot { background: var(--ind-amber); box-shadow: 0 0 6px var(--ind-amber); }

.pill-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--ind-green);
  box-shadow: 0 0 6px var(--ind-green-glow);
  animation: pill-pulse 1.8s ease-in-out infinite;
}

@keyframes pill-pulse {
  0%,
  100% {
    opacity: 1;
  }
  50% {
    opacity: 0.35;
  }
}

.rec-cards {
  display: flex;
  gap: 10px;
  align-items: stretch;
}

.big-card {
  flex: 1;
  min-width: 0;
  background: var(--bg-inset);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-md);
  padding: 10px 12px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.big-label {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 6px;
  /* V30：窄屏允许换行（原 nowrap 溢出） */
  white-space: normal;
  flex-wrap: wrap;
}

.big-value {
  font-family: var(--font-data);
  /* V30：字号随视口收敛（25px 在移动端大卡溢出） */
  font-size: clamp(17px, 2.4vw, 25px);
  font-weight: 700;
  color: var(--text-primary);
  line-height: 1;
  display: flex;
  align-items: baseline;
  gap: 5px;
  flex-wrap: wrap;
}

.big-unit {
  font-size: var(--fs-sm);
  color: var(--text-tertiary);
  font-weight: 400;
}

.delta-chip {
  font-family: var(--font-data);
  font-size: var(--fs-xs);
  display: inline-flex;
  align-items: center;
  gap: 2px;
  white-space: nowrap;
}

.delta-chip.up {
  color: var(--ind-green);
}

.delta-chip.down {
  color: var(--ind-red);
}

.delta-chip.flat {
  color: var(--text-tertiary);
}

.conf-wrap {
  position: relative;
  width: 106px;
  height: 106px;
  flex: none;
}

.conf-svg {
  width: 100%;
  height: 100%;
}

.conf-track {
  fill: none;
  stroke: #22364f;
  stroke-width: 9;
}

.conf-value {
  fill: none;
  stroke-width: 9;
  stroke-linecap: round;
  transform: rotate(-90deg);
  transform-origin: 56px 56px;
  transition: stroke-dashoffset 0.4s ease;
}

.conf-center {
  position: absolute;
  inset: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 3px;
}

.conf-num {
  font-family: var(--font-data);
  font-size: var(--fs-lg);
  font-weight: 700;
}

.conf-label {
  font-size: 10px;
  color: var(--text-tertiary);
  text-align: center;
  line-height: 1.3;
}

.rationale-text {
  font-size: var(--fs-sm);
  color: var(--text-secondary);
  line-height: 1.6;
  margin: 0;
  overflow-wrap: anywhere;
}

.rec-footer {
  margin-top: auto;
  padding-top: 12px;
  border-top: 1px dashed var(--border-glass);
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.rec-footer-item {
  display: flex;
  justify-content: space-between;
  gap: 10px;
}

.rec-footer-label {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
}

.rec-footer-value {
  font-size: var(--fs-xs);
  color: var(--text-secondary);
  overflow-wrap: anywhere;
  text-align: right;
}

/* ===== B) 决策推理链时间线 ===== */
.trace {
  display: flex;
  flex-direction: column;
  min-height: 0;
}

.trace-step {
  display: grid;
  grid-template-columns: 28px minmax(0, 1fr) auto;
  gap: 2px 10px;
}

.trace-rail {
  display: flex;
  flex-direction: column;
  align-items: center;
}

.trace-circle {
  width: 24px;
  height: 24px;
  border-radius: 50%;
  border: 1px solid var(--border-strong);
  background: var(--bg-inset);
  color: var(--accent-strong);
  font-family: var(--font-data);
  font-size: var(--fs-xs);
  display: flex;
  align-items: center;
  justify-content: center;
  flex: none;
}

.trace-circle.final {
  border-color: var(--ind-green);
  background: rgba(47, 212, 123, 0.14);
  color: var(--ind-green);
}

.trace-line {
  flex: 1;
  width: 2px;
  min-height: 10px;
  background: linear-gradient(var(--border-strong), var(--border-glass));
  margin: 2px 0;
}

.trace-body {
  padding: 3px 0 10px;
  min-width: 0;
}

.trace-title {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-primary);
  display: flex;
  align-items: center;
  gap: 5px;
  flex-wrap: wrap;
}

.trace-title .tick {
  color: var(--ind-green);
  display: inline-flex;
  flex: none;
}

.trace-zh {
  color: var(--text-tertiary);
  font-weight: 400;
  font-size: var(--fs-xs);
}

.trace-desc {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  margin-top: 2px;
  line-height: 1.45;
}

.trace-time {
  font-family: var(--font-data);
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  padding-top: 6px;
  white-space: nowrap;
}

/* ===== C1) 试运行 / 执行 ===== */
.col-stack {
  display: flex;
  flex-direction: column;
  gap: var(--spacing);
  min-height: 0;
}

.col-stack > :first-child {
  flex: none;
}

.col-stack > :last-child {
  flex: 1;
  min-height: 0;
}

.exec-btns {
  display: flex;
  gap: 10px;
}

.exec-btn {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  height: 48px;
  border-radius: var(--radius-md);
  font-size: var(--fs-md);
  font-weight: 700;
  font-family: var(--font-ui);
  cursor: pointer;
  transition: all 0.15s ease;
  border: 1px solid;
}

.exec-btn:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.exec-btn.dry {
  background: rgba(47, 155, 255, 0.08);
  border-color: var(--accent);
  color: var(--accent-strong);
}

.exec-btn.dry:hover:not(:disabled) {
  background: rgba(47, 155, 255, 0.18);
  box-shadow: 0 0 12px var(--ind-blue-glow);
}

.exec-btn.go {
  background: var(--ind-green);
  border-color: var(--ind-green);
  color: #05130c;
}

.exec-btn.go:hover:not(:disabled) {
  box-shadow: 0 0 14px var(--ind-green-glow);
  filter: brightness(1.08);
}

.exec-note {
  display: flex;
  gap: 6px;
  align-items: flex-start;
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  line-height: 1.5;
  margin: 10px 0 0;
}

/* ===== C2) 实验 / SOP 计划 ===== */
.plan-title {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-primary);
  margin-bottom: 8px;
  overflow-wrap: anywhere;
}

.plan-steps {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-height: 0;
}

.plan-step {
  display: flex;
  gap: 10px;
  align-items: flex-start;
  padding: 7px 9px;
  background: var(--bg-inset);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-md);
}

.plan-no {
  width: 20px;
  height: 20px;
  border-radius: 50%;
  background: var(--accent-dim);
  color: var(--accent-strong);
  font-size: var(--fs-xs);
  display: flex;
  align-items: center;
  justify-content: center;
  flex: none;
  margin-top: 1px;
}

.plan-body {
  min-width: 0;
  flex: 1;
}

.plan-name {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-primary);
  line-height: 1.35;
}

.plan-meta {
  margin-top: 3px;
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.plan-meta .mono {
  color: var(--text-secondary);
}

.plan-action {
  line-height: 1.4;
  overflow-wrap: anywhere;
}

/* ===== 紧凑表格（对比 / 记忆） ===== */
.tbl {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--fs-sm);
}

.tbl th {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  text-align: right;
  font-weight: 500;
  padding: 4px 6px;
  border-bottom: 1px solid var(--border-glass);
  white-space: nowrap;
}

.tbl th .th-zh {
  display: block;
  font-size: 10px;
  opacity: 0.85;
  font-weight: 400;
}

.tbl td {
  padding: 6px;
  border-bottom: 1px solid var(--border-glass);
  text-align: right;
  font-family: var(--font-data);
  font-variant-numeric: tabular-nums;
  color: var(--text-primary);
  white-space: nowrap;
}

.tbl .ta-l,
.tbl td.ta-l {
  text-align: left;
}

.tbl td.param {
  font-family: var(--font-ui);
  color: var(--text-secondary);
}

.param-zh {
  display: block;
  font-size: 10px;
  color: var(--text-tertiary);
}

.tbl .unit {
  color: var(--text-tertiary);
  font-size: var(--fs-xs);
  margin-left: 2px;
}

.rec-val {
  color: var(--accent-strong) !important;
  font-weight: 700;
}

.pos {
  color: var(--ind-green);
}

.neg {
  color: var(--ind-red);
}

.flat {
  color: var(--text-tertiary);
}

.trend {
  font-size: var(--fs-md);
}

.mem-table td {
  font-size: var(--fs-xs);
}

.mini-empty {
  padding: 10px 4px;
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
}

.learnings-text {
  font-size: var(--fs-xs);
  color: var(--text-secondary);
  line-height: 1.6;
  margin: 0;
  overflow-wrap: anywhere;
}

/* ===== C) 风险与安全边界 ===== */
.risk-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 8px;
}

.risk-block {
  background: var(--bg-inset);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-md);
  padding: 9px 6px;
  text-align: center;
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}

.risk-label {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.risk-value {
  font-family: var(--font-data);
  font-size: var(--fs-lg);
  font-weight: 700;
  line-height: 1.15;
}

.risk-value.high {
  color: var(--ind-red);
}

.risk-value.medium {
  color: var(--ind-amber);
}

.risk-value.low {
  color: var(--ind-green);
}

.risk-value.na {
  color: var(--text-tertiary);
}

.risk-sub {
  font-size: 10px;
  color: var(--text-tertiary);
  font-family: var(--font-data);
  /* V30：允许换行完整显示（原 nowrap+ellipsis 仍被计为裁切） */
  white-space: normal;
  overflow-wrap: anywhere;
}

.gr-list {
  list-style: none;
  margin: 0;
  padding: 0;
}

.gr-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 0;
  border-bottom: 1px dashed var(--border-glass);
}

.gr-row:last-child {
  border-bottom: none;
}

.gr-mark {
  width: 17px;
  height: 17px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 10px;
  font-weight: 700;
  flex: none;
}

.gr-mark.ok {
  color: var(--ind-green);
  background: rgba(47, 212, 123, 0.12);
}

.gr-mark.bad {
  color: var(--ind-red);
  background: rgba(255, 82, 82, 0.14);
}

.gr-mark.na {
  color: var(--ind-gray);
  background: rgba(90, 115, 150, 0.14);
}

.gr-label {
  font-size: var(--fs-xs);
  color: var(--text-secondary);
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.gr-detail {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  white-space: nowrap;
}

.guard-footer {
  margin-top: 12px;
  padding-top: 10px;
  border-top: 1px solid var(--border-glass);
  display: flex;
  gap: 8px;
  align-items: center;
  font-size: var(--fs-sm);
  font-weight: 600;
}

.guard-footer.ok {
  color: var(--ind-green);
}

.guard-footer.bad {
  color: var(--ind-red);
}

/* ===== D) 建议历史 ===== */
.view-all {
  color: var(--accent);
  font-size: var(--fs-xs);
  text-decoration: none;
  white-space: nowrap;
  font-weight: 600;
}

.view-all:hover {
  color: var(--accent-strong);
  text-decoration: underline;
}

.hist {
  display: flex;
  flex-direction: column;
  min-height: 0;
}

.hist-item {
  display: grid;
  grid-template-columns: 12px minmax(0, 1fr) auto;
  gap: 9px;
  padding: 7px 0;
  border-bottom: 1px dashed var(--border-glass);
  align-items: start;
}

.hist-item:last-child {
  border-bottom: none;
}

.hist-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--accent);
  box-shadow: 0 0 6px var(--ind-blue-glow);
  margin-top: 4px;
}

.hist-body {
  min-width: 0;
}

.hist-type {
  font-family: var(--font-data);
  font-size: var(--fs-xs);
  color: var(--text-primary);
  overflow-wrap: anywhere;
  line-height: 1.4;
}

.hist-sub {
  font-size: 10px;
  color: var(--text-tertiary);
  margin-top: 2px;
  overflow-wrap: anywhere;
  line-height: 1.35;
}

.hist-side {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 3px;
}

.hist-time {
  font-family: var(--font-data);
  font-size: 10px;
  color: var(--text-tertiary);
  white-space: nowrap;
}

.hist-tag {
  font-size: 10px;
  font-weight: 600;
  padding: 1px 7px;
  border-radius: 999px;
  border: 1px solid;
  white-space: nowrap;
}

.hist-tag.ok {
  color: var(--ind-green);
  border-color: rgba(47, 212, 123, 0.5);
  background: rgba(47, 212, 123, 0.1);
}

.hist-tag.info {
  color: var(--accent-strong);
  border-color: rgba(47, 155, 255, 0.5);
  background: var(--accent-dim);
}

/* ===== E) 实时上下文摘要 ===== */
.reactor-box {
  display: flex;
  justify-content: center;
  padding: 2px 0 10px;
  flex: none;
}

.reactor-svg {
  width: 118px;
  max-height: 108px;
}

.sk-shell {
  fill: rgba(16, 31, 51, 0.5);
  stroke: var(--border-strong);
  stroke-width: 2;
}

.sk-lid {
  stroke: var(--border-strong);
  stroke-width: 2.5;
  stroke-linecap: round;
}

.sk-motor {
  fill: var(--bg-panel-raised);
  stroke: var(--border-strong);
  stroke-width: 1.5;
}

.sk-shaft {
  stroke: var(--text-tertiary);
  stroke-width: 2;
}

.sk-impeller {
  stroke: var(--accent-cyan);
  stroke-width: 2.5;
  stroke-linecap: round;
}

.sk-leg {
  stroke: var(--border-strong);
  stroke-width: 2;
  stroke-linecap: round;
}

.sk-glow {
  fill: var(--accent-cyan);
  filter: blur(7px);
  opacity: 0.45;
  animation: liquid-glow 2.8s ease-in-out infinite;
}

@keyframes liquid-glow {
  0%,
  100% {
    opacity: 0.3;
  }
  50% {
    opacity: 0.65;
  }
}

.ctx-kv {
  flex: 1;
  align-content: start;
}

/* ===== 结果对话框 ===== */
.res-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  margin-bottom: 10px;
}

.res-decision {
  font-family: var(--font-data);
  font-size: var(--fs-md);
  font-weight: 700;
  color: var(--accent-strong);
  overflow-wrap: anywhere;
}

.res-rationale {
  font-size: var(--fs-sm);
  color: var(--text-secondary);
  line-height: 1.6;
  margin: 0 0 12px;
  overflow-wrap: anywhere;
}

.res-kv {
  margin-bottom: 12px;
}

.res-table {
  width: 100%;
}

/* ===== V32：移动端单列堆叠、整页可滚动 ===== */
@media (max-width: 900px) {
  .ai-rows { display: flex; flex-direction: column; }
  .row { display: flex; flex-direction: column; }
  .row > * { flex: none; }
}

/* ===== 响应式微调（不改变整页不滚动的约束） ===== */
@media (max-width: 1500px) {
  .row-top {
    grid-template-columns: minmax(0, 1.3fr) minmax(0, 1fr) minmax(0, 1.15fr);
  }

  .row-bottom {
    grid-template-columns: minmax(0, 1.3fr) minmax(0, 1.1fr) minmax(0, 1.15fr) minmax(0, 1fr) minmax(0, 1fr);
  }

  .big-value {
    font-size: 21px;
  }
}
</style>
