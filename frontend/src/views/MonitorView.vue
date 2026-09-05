<template>
  <div class="monitor-page">
    <!-- 页头：标题 + 实时状态/时钟 -->
    <header class="page-head">
      <h2 class="title">Reactor Overview <span class="zh">反应釜总览</span></h2>
      <div class="head-meta">
        <span class="meta">
          <span class="status-dot" :class="isFresh ? 'ok' : 'warn'"></span>
          {{ isFresh ? tr("实时在线", "Live") : tr("数据不可用", "No data") }}
        </span>
        <span class="meta mono">{{ clockText }}</span>
      </div>
    </header>

    <!-- 上部大区（55%）：反应釜示意图 / 参数卡 / 批次信息 -->
    <section class="upper">
      <!-- 左列：反应釜示意图 -->
      <PanelCard en="Reactor Vessel" zh="反应釜示意" icon="flask">
        <div class="reactor-wrap">
          <svg class="reactor-svg" viewBox="0 0 360 292" preserveAspectRatio="xMidYMid meet">
            <defs>
              <radialGradient id="mon-vessel-grad" cx="50%" cy="35%" r="75%">
                <stop offset="0%" stop-color="#1b3a5e" />
                <stop offset="60%" stop-color="#0f2440" />
                <stop offset="100%" stop-color="#0a1a30" />
              </radialGradient>
              <linearGradient id="mon-liquid-grad" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stop-color="rgba(56,200,242,0.45)" />
                <stop offset="100%" stop-color="rgba(47,155,255,0.12)" />
              </linearGradient>
              <clipPath id="mon-vessel-clip">
                <rect x="132" y="64" width="96" height="146" rx="14" />
              </clipPath>
              <filter id="mon-glow" x="-30%" y="-30%" width="160%" height="160%">
                <feGaussianBlur stdDeviation="2.6" result="b" />
                <feMerge>
                  <feMergeNode in="b" />
                  <feMergeNode in="SourceGraphic" />
                </feMerge>
              </filter>
            </defs>

            <!-- 夹套 + 冷却管嘴 -->
            <rect x="122" y="56" width="116" height="162" rx="18" fill="none" stroke="rgba(56,200,242,0.4)" stroke-width="1.2" />
            <rect x="106" y="182" width="16" height="7" rx="2" fill="none" stroke="rgba(56,200,242,0.6)" stroke-width="1.2" />
            <rect x="238" y="88" width="16" height="7" rx="2" fill="none" stroke="rgba(56,200,242,0.6)" stroke-width="1.2" />

            <!-- 釜体 -->
            <rect x="132" y="64" width="96" height="146" rx="14" fill="url(#mon-vessel-grad)" stroke="#2f9bff" stroke-width="1.6" filter="url(#mon-glow)" />
            <g clip-path="url(#mon-vessel-clip)">
              <rect x="132" y="118" width="96" height="92" fill="url(#mon-liquid-grad)" />
              <line x1="132" y1="118" x2="228" y2="118" stroke="#38c8f2" stroke-width="1.2" opacity="0.85" />
              <circle cx="162" cy="150" r="3" fill="#38c8f2" opacity="0.3" />
              <circle cx="188" cy="172" r="2" fill="#38c8f2" opacity="0.25" />
              <circle cx="206" cy="140" r="2.5" fill="#38c8f2" opacity="0.3" />
            </g>

            <!-- 搅拌电机 + 轴 + 桨叶 -->
            <rect x="162" y="28" width="36" height="20" rx="3" fill="#0e1c30" stroke="#38c8f2" stroke-width="1.2" />
            <line x1="169" y1="32" x2="169" y2="44" stroke="rgba(56,200,242,0.5)" stroke-width="1" />
            <line x1="180" y1="32" x2="180" y2="44" stroke="rgba(56,200,242,0.5)" stroke-width="1" />
            <line x1="191" y1="32" x2="191" y2="44" stroke="rgba(56,200,242,0.5)" stroke-width="1" />
            <line x1="180" y1="48" x2="180" y2="172" stroke="#9db4cf" stroke-width="3" />
            <g fill="#38c8f2" opacity="0.9">
              <g v-if="isRunning">
                <animateTransform attributeName="transform" type="rotate" from="0 180 140" to="360 180 140" dur="2.4s" repeatCount="indefinite" />
                <rect x="146" y="137" width="16" height="6" rx="3" />
                <rect x="198" y="137" width="16" height="6" rx="3" />
              </g>
              <g v-else>
                <rect x="146" y="137" width="16" height="6" rx="3" />
                <rect x="198" y="137" width="16" height="6" rx="3" />
              </g>
              <rect x="146" y="165" width="16" height="6" rx="3" />
              <rect x="198" y="165" width="16" height="6" rx="3" />
            </g>

            <!-- 支腿 + 底部卸料阀 -->
            <line x1="148" y1="210" x2="140" y2="230" stroke="#5a7396" stroke-width="2.5" />
            <line x1="212" y1="210" x2="220" y2="230" stroke="#5a7396" stroke-width="2.5" />
            <rect x="170" y="210" width="20" height="5" fill="#0e1c30" stroke="#5a7396" stroke-width="1" />
            <line x1="180" y1="215" x2="180" y2="240" stroke="#5a7396" stroke-width="2" />
            <path d="M174 240 L186 240 L180 248 Z" fill="none" stroke="#5a7396" stroke-width="1.4" />

            <!-- 传感器标注引线 + 标签 -->
            <g class="callout">
              <line x1="132" y1="88" x2="120" y2="56" stroke="#2f9bff" stroke-width="1" />
              <circle cx="132" cy="88" r="2.5" fill="#2f9bff" />
              <rect x="8" y="38" width="112" height="36" rx="6" fill="rgba(16,31,51,0.9)" stroke="#2f9bff" stroke-width="1" />
              <text x="16" y="51" class="tag-label">TIC-101 · {{ tr("温度", "Temp") }}</text>
              <text x="16" y="67" class="tag-val">{{ callouts.tic }}</text>
            </g>
            <g class="callout">
              <line x1="228" y1="88" x2="240" y2="56" stroke="#2fd47b" stroke-width="1" />
              <circle cx="228" cy="88" r="2.5" fill="#2fd47b" />
              <rect x="240" y="38" width="112" height="36" rx="6" fill="rgba(16,31,51,0.9)" stroke="#2fd47b" stroke-width="1" />
              <text x="248" y="51" class="tag-label">PIC-101 · {{ tr("压力", "Press") }}</text>
              <text x="248" y="67" class="tag-val">{{ callouts.pic }}</text>
            </g>
            <g class="callout">
              <line x1="134" y1="182" x2="120" y2="224" stroke="#f5a623" stroke-width="1" />
              <circle cx="134" cy="182" r="2.5" fill="#f5a623" />
              <rect x="8" y="222" width="112" height="36" rx="6" fill="rgba(16,31,51,0.9)" stroke="#f5a623" stroke-width="1" />
              <text x="16" y="235" class="tag-label">LIT-101 · {{ tr("液位", "Level") }}</text>
              <text x="16" y="251" class="tag-val">{{ callouts.lit }}</text>
            </g>
            <g class="callout">
              <line x1="226" y1="182" x2="240" y2="224" stroke="#38c8f2" stroke-width="1" />
              <circle cx="226" cy="182" r="2.5" fill="#38c8f2" />
              <rect x="240" y="222" width="112" height="36" rx="6" fill="rgba(16,31,51,0.9)" stroke="#38c8f2" stroke-width="1" />
              <text x="248" y="235" class="tag-label">FIT-101 · {{ tr("流量", "Flow") }}</text>
              <text x="248" y="251" class="tag-val">{{ callouts.fit }}</text>
            </g>
          </svg>

          <div class="reactor-foot">
            <span class="pill" :class="runState.cls">
              <span class="pill-dot"></span>{{ runState.label }}
            </span>
            <span class="sample-time mono">
              {{ tr("采样", "Sample") }} {{ latestSample?.captured_at ? formatTime(latestSample.captured_at) : "--" }}
            </span>
          </div>
        </div>
      </PanelCard>

      <!-- 中列：7 张参数卡 -->
      <PanelCard en="Process Parameters" zh="工艺参数" icon="gauge">
        <div class="param-grid">
          <div
            v-for="card in paramCards"
            :key="card.key"
            class="param-card"
            :class="{ wide: card.wide }"
            :style="{ '--c': card.color }"
          >
            <div class="param-head">
              <span class="en">{{ card.en }}</span>
              <span class="zh">{{ card.zh }}</span>
            </div>
            <div class="param-value">
              <span class="pv mono">{{ card.value }}</span>
              <span v-if="card.unit" class="unit">{{ card.unit }}</span>
            </div>
            <div class="param-sp mono">{{ card.sp }}</div>
            <!-- height=0：由 flex 吃掉卡片剩余高度，卡片偏矮时收缩而不是被 overflow 裁掉 -->
            <SparkLine :points="card.spark" :color="card.color" :height="0" />
          </div>
        </div>
      </PanelCard>

      <!-- 右列：批次信息 -->
      <PanelCard en="Batch Information" zh="批次信息" icon="batch">
        <template #actions>
          <span class="batch-chip mono" v-if="activeBatch">#{{ activeBatch.id }}</span>
        </template>
        <dl v-if="activeBatch" class="kv-list batch-kv">
          <dt>Batch ID <span class="zh">批次号</span></dt>
          <dd class="mono">#{{ activeBatch.id }}</dd>
          <dt>Recipe <span class="zh">配方名称</span></dt>
          <dd :title="recipeName ?? ''">{{ recipeName ?? "--" }}</dd>
          <dt>Product <span class="zh">产品名称</span></dt>
          <dd>--</dd>
          <dt>Start Time <span class="zh">开始时间</span></dt>
          <dd class="mono">{{ formatTimestamp(activeBatch.started_at) }}</dd>
          <dt>Elapsed <span class="zh">已运行</span></dt>
          <dd class="mono">{{ formatElapsed(elapsedMs) }}</dd>
          <dt>Status <span class="zh">状态</span></dt>
          <dd>
            <el-tag size="small" :type="batchRunning ? 'success' : 'info'">
              {{ batchRunning ? tr("运行中", "Running") : tr("已完成", "Completed") }}
            </el-tag>
          </dd>
          <dt>Step <span class="zh">当前步骤</span></dt>
          <dd>{{ batchStepName }}</dd>
        </dl>
        <div v-else class="empty-state">
          <AppIcon name="batch" :size="34" />
          <span>{{ tr("当前无活动批次", "No active batch") }}</span>
        </div>
        <div class="batch-foot">
          <el-button size="small" plain @click="router.push('/history')">
            {{ tr("查看批次详情", "View Batch Details") }}
          </el-button>
        </div>
      </PanelCard>
    </section>

    <!-- 下部（45%）：趋势 / 工艺流程 / 报警 / AI 推荐 / 急停 -->
    <section class="lower">
      <PanelCard en="Live Trend" zh="实时趋势" icon="live">
        <template #actions>
          <span class="panel-note">{{ tr("最近 2 小时", "Last 2h") }}</span>
        </template>
        <div class="chart-wrap" data-testid="monitor-main-trend">
          <TrendChart :series="trendSeries" height="100%" />
        </div>
      </PanelCard>

      <PanelCard en="Process Timeline" zh="工艺流程" icon="clock">
        <div class="stepper">
          <div
            v-for="(it, i) in stepperItems"
            :key="it.key"
            class="step"
            :class="{ done: timeline.idx > i, current: timeline.idx === i }"
            :title="it.title"
          >
            <div class="node">
              <AppIcon v-if="timeline.idx > i" name="check" :size="10" />
              <span v-else class="num">{{ i + 1 }}</span>
            </div>
            <div class="slabel">
              <span class="en">{{ it.en }}</span>
              <span v-if="it.zh" class="zh">{{ it.zh }}</span>
            </div>
          </div>
        </div>
        <div class="step-detail">
          <div class="sd-row">
            <span class="lbl">{{ tr("目标温度", "Target Temp") }}</span>
            <span class="val mono">{{ fixed(currentStep?.target_temperature_c, 1) }} °C</span>
          </div>
          <div class="sd-row">
            <span class="lbl">{{ tr("目标转速", "Target RPM") }}</span>
            <span class="val mono">{{ fixed(currentStep?.target_stirrer_rpm, 0) }} rpm</span>
          </div>
          <div class="sd-row">
            <span class="lbl">{{ tr("预计剩余", "Est. Remaining") }}</span>
            <span class="val mono">{{ formatRemain(timeline.remainMin) }}</span>
          </div>
          <div class="step-progress">
            <div class="bar" :style="{ width: stepProgressPct }"></div>
          </div>
        </div>
      </PanelCard>

      <PanelCard en="Alarms & Events" zh="报警与事件" icon="alarm" scrollable>
        <template #actions>
          <RouterLink class="view-all" to="/audit">{{ tr("查看全部", "View All") }}</RouterLink>
        </template>
        <div v-if="alarmRows.length" class="alarm-rows">
          <div v-for="(row, i) in alarmRows" :key="i" class="alarm-row" :class="row.cls">
            <span class="lv-ic"><AppIcon name="alarm" :size="13" /></span>
            <div class="alarm-main">
              <div class="alarm-msg">{{ row.message }}</div>
              <div v-if="row.detail" class="alarm-sub mono">{{ row.detail }}</div>
            </div>
            <el-tag size="small" :type="row.tagType">{{ row.tagText }}</el-tag>
          </div>
        </div>
        <div v-else class="empty-state">
          <AppIcon name="shield" :size="34" />
          <span>{{ tr("无活动报警", "No active alarms") }}</span>
        </div>
      </PanelCard>

      <PanelCard en="AI Recommendation" zh="智能推荐" icon="ai">
        <template #actions>
          <span v-if="aiModel" class="ai-model mono">{{ aiModel }}</span>
        </template>
        <div v-if="hasRecommendation" class="ai-body clickable" role="button" tabindex="0"
          :title="tr('前往 AI 决策页', 'Go to AI Decision')"
          @click="router.push('/ai')" @keydown.enter="router.push('/ai')">
          <p class="ai-basis">
            {{ tr("基于当前工艺状态与历史数据分析", "Based on current process state and historical data") }}
          </p>
          <div class="ai-val">
            <span class="lbl">Target Temp <span class="zh">目标温度</span></span>
            <span class="big mono">
              {{ fixed(recommendation?.target_temperature_c, 1) }}<span class="unit">°C</span>
            </span>
            <span class="sub">
              <span v-if="dTemp" class="delta mono" :class="dTemp.dir">{{ dTemp.arrow }} {{ dTemp.txt }}</span>
              <span class="cur mono">{{ tr("当前", "Now") }} {{ fixed(latestSample?.temperature_c, 1) }}</span>
            </span>
          </div>
          <div class="ai-val">
            <span class="lbl">Target Stirrer <span class="zh">目标转速</span></span>
            <span class="big mono">
              {{ fixed(recommendation?.target_stirrer_rpm, 0) }}<span class="unit">rpm</span>
            </span>
            <span class="sub">
              <span v-if="dRpm" class="delta mono" :class="dRpm.dir">{{ dRpm.arrow }} {{ dRpm.txt }}</span>
              <span class="cur mono">{{ tr("当前", "Now") }} {{ fixed(latestSample?.stirrer_rpm, 0) }}</span>
            </span>
          </div>
          <div v-if="hasScore" class="ai-score mono">
            {{ tr("预期得分", "Expected score") }} {{ fixed(recommendation?.expected_score, 1) }}
          </div>
          <span class="ai-go">{{ tr("前往 AI 决策", "Go to AI Decision") }} →</span>
        </div>
        <div v-else class="empty-state clickable" role="button" tabindex="0"
          :title="tr('前往 AI 决策页', 'Go to AI Decision')"
          @click="router.push('/ai')" @keydown.enter="router.push('/ai')">
          <AppIcon name="ai" :size="34" />
          <span>{{ tr("暂无推荐", "No recommendation yet") }}</span>
          <span class="ai-go">{{ tr("前往 AI 决策", "Go to AI Decision") }} →</span>
        </div>
      </PanelCard>

      <EmergencyStopPanel />
    </section>
  </div>
</template>

<script setup lang="ts">
// Monitor 页（参考稿 6：Reactor Overview 反应釜总览）。
// 数据全部来自 useLiveStore（实时样本/批次/报警/推荐）与 usePlantStore（工艺/批次列表）。
// 后端无液位传感器、报警无时间戳、runtime 无当前步索引 —— 缺失字段一律显示 "--"，
// 工艺步骤进度由 batch.started_at + step.duration_minutes 推导。
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useRouter } from "vue-router";
import { storeToRefs } from "pinia";
import PanelCard from "../components/PanelCard.vue";
import SparkLine from "../components/SparkLine.vue";
import TrendChart from "../components/TrendChart.vue";
import AppIcon from "../components/AppIcon.vue";
import EmergencyStopPanel from "../components/EmergencyStopPanel.vue";
import { useLiveStore } from "../stores/live";
import { usePlantStore } from "../stores/plant";
import { useLanguage } from "../i18n";
import { fixed, formatTime, formatTimestamp } from "../utils/format";
import type { Batch, ProcessStep } from "../api/types";

interface TrendSeries {
  name: string;
  data: Array<[number, number | null]>;
  color?: string;
  unit?: string;
  yAxisIndex?: number;
  smooth?: boolean;
  dashed?: boolean;
  id?: string;
}

const router = useRouter();
const liveStore = useLiveStore();
const plant = usePlantStore();
const { tr } = useLanguage();
const { live, runtime, latestSample, recentSamples, alarms, recommendation, liveStatus } =
  storeToRefs(liveStore);

const isFresh = computed(() => liveStatus.value === "fresh");
const isRunning = computed(
  () => isFresh.value && !runtime.value?.emergency_stop && !runtime.value?.manual_lock
);

// ---- 时钟（页头时钟 / 已运行计时 / 步骤进度共用 1s tick） ----
const nowTick = ref(Date.now());
const timer = window.setInterval(() => {
  nowTick.value = Date.now();
}, 1000);
const clockText = computed(() => {
  const d = new Date(nowTick.value);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
});

// ---- 反应釜示意图：标注值（压力 MPa → bar ×10；液位无来源 → "--"） ----
const pressureBar = computed(() => {
  const p = latestSample.value?.pressure_mpa;
  return typeof p === "number" && Number.isFinite(p) ? p * 10 : null;
});
const callouts = computed(() => {
  const s = latestSample.value;
  return {
    tic:
      typeof s?.temperature_c === "number" && Number.isFinite(s.temperature_c)
        ? `${s.temperature_c.toFixed(1)} °C`
        : "--",
    pic: pressureBar.value !== null ? `${pressureBar.value.toFixed(2)} bar` : "--",
    lit: "--",
    fit:
      typeof s?.flow_rate_l_min === "number" && Number.isFinite(s.flow_rate_l_min)
        ? `${s.flow_rate_l_min.toFixed(2)} L/min`
        : "--"
  };
});
const runState = computed(() => {
  if (runtime.value?.emergency_stop) return { cls: "bad", label: tr("紧急停止", "E-STOP") };
  if (isFresh.value) return { cls: "ok", label: tr("运行中", "Running") };
  return { cls: "idle", label: tr("待机 · 数据不可用", "Standby · No data") };
});

function finiteOrNull(value: number | null | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

interface SampleColumns {
  temp: number[];
  press: number[];
  rpm: number[];
  shake: number[];
  flow: number[];
  conc: number[];
  ph: number[];
  trendTemp: Array<[number, number | null]>;
  trendPress: Array<[number, number | null]>;
  trendRpm: Array<[number, number | null]>;
  trendPh: Array<[number, number | null]>;
}

// recentSamples 只遍历一次：7 个 sparkline 列与 4 个主趋势列共享同一批派生结果。
const sampleColumns = computed<SampleColumns>(() => {
  const columns: SampleColumns = {
    temp: [], press: [], rpm: [], shake: [], flow: [], conc: [], ph: [],
    trendTemp: [], trendPress: [], trendRpm: [], trendPh: []
  };
  for (const sample of recentSamples.value) {
    const temp = finiteOrNull(sample.temperature_c);
    const pressureMpa = finiteOrNull(sample.pressure_mpa);
    const press = pressureMpa === null ? null : pressureMpa * 10;
    const rpm = finiteOrNull(sample.stirrer_rpm);
    const shake = finiteOrNull(sample.shake_speed_cpm);
    const flow = finiteOrNull(sample.flow_rate_l_min);
    const conc = finiteOrNull(sample.product_concentration_percent);
    const ph = finiteOrNull(sample.ph);
    if (temp !== null) columns.temp.push(temp);
    if (press !== null) columns.press.push(press);
    if (rpm !== null) columns.rpm.push(rpm);
    if (shake !== null) columns.shake.push(shake);
    if (flow !== null) columns.flow.push(flow);
    if (conc !== null) columns.conc.push(conc);
    if (ph !== null) columns.ph.push(ph);

    const time = tsMs(sample.captured_at);
    if (time === null) continue;
    columns.trendTemp.push([time, temp]);
    columns.trendPress.push([time, press]);
    columns.trendRpm.push([time, rpm]);
    columns.trendPh.push([time, ph]);
  }
  return columns;
});

// ---- 参数卡（7 张，蓝/绿/黄/紫/青/粉紫/红轮换；仅温度/转速/振荡有 SP） ----
const paramCards = computed(() => {
  const s = latestSample.value;
  const t = runtime.value?.targets ?? null;
  const columns = sampleColumns.value;
  const pv = (v: number | null | undefined, d: number): string =>
    typeof v === "number" && Number.isFinite(v) ? v.toFixed(d) : "--";
  const num = (v: unknown): number | null =>
    typeof v === "number" && Number.isFinite(v) ? v : null;
  return [
    {
      key: "temp", en: "Temperature", zh: "温度", unit: "°C", color: "#2f9bff",
      value: pv(s?.temperature_c, 1),
      sp: num(t?.temperature_c) !== null ? `SP ${num(t?.temperature_c)!.toFixed(1)}` : "",
      spark: columns.temp
    },
    {
      key: "press", en: "Pressure", zh: "压力", unit: "bar", color: "#2fd47b",
      value: pv(pressureBar.value, 2), sp: "", spark: columns.press
    },
    {
      key: "rpm", en: "Stirrer RPM", zh: "搅拌转速", unit: "rpm", color: "#f5a623",
      value: pv(s?.stirrer_rpm, 0),
      sp: num(t?.stirrer_rpm) !== null ? `SP ${Math.round(num(t?.stirrer_rpm)!)}` : "",
      spark: columns.rpm
    },
    {
      key: "shake", en: "Shake Speed", zh: "振荡速度", unit: "cpm", color: "#b068f0",
      value: pv(s?.shake_speed_cpm, 0),
      sp: num(t?.shake_speed_cpm) !== null ? `SP ${Math.round(num(t?.shake_speed_cpm)!)}` : "",
      spark: columns.shake
    },
    {
      key: "flow", en: "Flow Rate", zh: "流量", unit: "L/min", color: "#38c8f2",
      value: pv(s?.flow_rate_l_min, 2), sp: "", spark: columns.flow
    },
    {
      key: "conc", en: "Product Conc.", zh: "产品浓度", unit: "%", color: "#e06bd8",
      value: pv(s?.product_concentration_percent, 1), sp: "", spark: columns.conc
    },
    {
      key: "ph", en: "pH", zh: "酸碱度", unit: "", color: "#ff5252",
      value: pv(s?.ph, 2), sp: "", spark: columns.ph, wide: true
    }
  ];
});

// ---- 批次信息（live.recent_batches + runtime.active_batch_id + plant 数据） ----
const activeBatchId = computed(() => {
  const id = runtime.value?.active_batch_id;
  if (id === null || id === undefined) return null;
  const n = Number(id);
  return Number.isFinite(n) ? n : null;
});
const activeBatch = computed<Batch | null>(() => {
  const id = activeBatchId.value;
  if (id === null) return null;
  const fromLive = (live.value?.recent_batches ?? []).find((b) => b.id === id);
  if (fromLive) return fromLive;
  return (plant.batches?.batches ?? []).find((b) => b.id === id) ?? null;
});
const batchRunning = computed(() => activeBatchId.value !== null && !activeBatch.value?.finished_at);
const recipeName = computed(() => {
  const fromRuntime = runtime.value?.active_process_name;
  if (fromRuntime) return fromRuntime;
  const pid = activeBatch.value?.process_id;
  const proc = typeof pid === "number" ? plant.processes.find((p) => p.id === pid) : null;
  return proc?.name ?? null;
});
const elapsedMs = computed(() => {
  const st = activeBatch.value?.started_at;
  if (!st) return null;
  const t = new Date(st).getTime();
  if (!Number.isFinite(t)) return null;
  return Math.max(0, nowTick.value - t);
});

function formatElapsed(ms: number | null): string {
  if (ms === null || !Number.isFinite(ms)) return "--";
  const total = Math.max(0, Math.floor(ms / 1000));
  const d = Math.floor(total / 86400);
  const h = Math.floor((total % 86400) / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const p = (n: number) => String(n).padStart(2, "0");
  return d > 0 ? `${d}d ${p(h)}:${p(m)}:${p(s)}` : `${p(h)}:${p(m)}:${p(s)}`;
}

// ---- 工艺流程（selectedProcess.steps 优先；否则静态 6 步骨架 + "--"） ----
const CANON_STEPS = [
  { en: "Charge", zh: "投料" },
  { en: "Heat Up", zh: "升温" },
  { en: "Reaction", zh: "反应" },
  { en: "Hold", zh: "保温" },
  { en: "Cool Down", zh: "降温" },
  { en: "Discharge", zh: "出料" }
];
const processSteps = computed<ProcessStep[]>(() => {
  const steps = plant.selectedProcess?.steps;
  if (!Array.isArray(steps) || steps.length === 0) return [];
  return [...steps].sort((a, b) => (a.step_index ?? 0) - (b.step_index ?? 0));
});
// 由批次开始时间 + 各步时长推导当前步 / 步内进度 / 剩余分钟。
const timeline = computed(() => {
  const steps = processSteps.value;
  let idx = -1;
  let progress = 0;
  let remainMin: number | null = null;
  const el = elapsedMs.value;
  if (steps.length > 0 && el !== null) {
    const elMin = el / 60000;
    let cum = 0;
    let found = false;
    for (let i = 0; i < steps.length; i++) {
      const d = steps[i].duration_minutes;
      if (d === null || d === undefined || !Number.isFinite(d) || d <= 0) {
        idx = i;
        found = true;
        break;
      }
      if (elMin < cum + d) {
        idx = i;
        progress = (elMin - cum) / d;
        remainMin = Math.max(0, cum + d - elMin);
        found = true;
        break;
      }
      cum += d;
    }
    if (!found) {
      idx = steps.length - 1;
      progress = 1;
      remainMin = 0;
    }
  }
  return { steps, idx, progress, remainMin };
});
const stepperItems = computed(() => {
  const steps = timeline.value.steps;
  if (steps.length > 0) {
    return steps.map((s, i) => ({
      key: `s${s.id ?? i}`,
      en: s.name ?? `Step ${i + 1}`,
      zh: "",
      title: s.name ?? ""
    }));
  }
  return CANON_STEPS.map((c, i) => ({ key: `c${i}`, en: c.en, zh: c.zh, title: `${c.en} ${c.zh}` }));
});
const currentStep = computed<ProcessStep | null>(() =>
  timeline.value.idx >= 0 ? timeline.value.steps[timeline.value.idx] : null
);
const stepProgressPct = computed(() => {
  const p = timeline.value.progress;
  return `${Math.round(Math.min(1, Math.max(0, p)) * 100)}%`;
});
const batchStepName = computed(() => {
  if (!batchRunning.value) return "--";
  return currentStep.value?.name ?? "--";
});
function formatRemain(min: number | null): string {
  if (min === null || !Number.isFinite(min)) return "--";
  if (min < 1) return "< 1 min";
  return `~${Math.ceil(min)} min`;
}

// ---- 实时趋势（4 序列；压力换算 bar，转速走右轴） ----
function tsMs(v: unknown): number | null {
  if (v === null || v === undefined) return null;
  const t = new Date(String(v)).getTime();
  return Number.isFinite(t) ? t : null;
}
const trendSeries = computed<TrendSeries[]>(() => {
  const columns = sampleColumns.value;
  return [
    { id: "temp", name: tr("温度", "Temp"), unit: "°C", color: "#2f9bff", yAxisIndex: 0, data: columns.trendTemp },
    { id: "pressure", name: tr("压力", "Pressure"), unit: "bar", color: "#2fd47b", yAxisIndex: 0, data: columns.trendPress },
    { id: "stirrer", name: tr("搅拌转速", "Stirrer"), unit: "rpm", color: "#f5a623", yAxisIndex: 1, data: columns.trendRpm },
    { id: "ph", name: tr("pH", "pH"), unit: "", color: "#b068f0", yAxisIndex: 0, data: columns.trendPh }
  ];
});

// ---- 报警与事件（level: high/medium；无时间戳字段，不编造） ----
const alarmRows = computed(() =>
  (alarms.value ?? []).map((a) => {
    const high = a.level === "high" || a.severity === "critical";
    const medium = a.level === "medium" || a.severity === "warning";
    const detail =
      a.current_value !== null && a.current_value !== undefined
        ? `${tr("当前", "Cur")} ${a.current_value}${a.limit_value !== null && a.limit_value !== undefined ? " / " + tr("限值", "Limit") + " " + a.limit_value : ""}`
        : "";
    return {
      message: a.message ?? a.type ?? tr("报警", "Alarm"),
      detail,
      cls: high ? "high" : medium ? "medium" : "info",
      tagType: (high ? "danger" : medium ? "warning" : "info") as "danger" | "warning" | "info",
      tagText: high ? tr("高", "High") : medium ? tr("中", "Medium") : tr("信息", "Info")
    };
  })
);

// ---- AI 推荐（provider 可能是对象或字符串） ----
const hasRecommendation = computed(() => recommendation.value !== null);
const aiModel = computed(() => {
  const p = recommendation.value?.provider;
  if (!p) return null;
  if (typeof p === "string") return p || null;
  const parts = [p.mode, p.model].filter(
    (x): x is string => typeof x === "string" && x.length > 0
  );
  return parts.length > 0 ? parts.join(" · ") : null;
});
interface Delta {
  arrow: string;
  txt: string;
  dir: string;
}
function makeDelta(
  target: number | null | undefined,
  current: number | null | undefined,
  digits: number
): Delta | null {
  const a = typeof target === "number" && Number.isFinite(target) ? target : null;
  const b = typeof current === "number" && Number.isFinite(current) ? current : null;
  if (a === null || b === null) return null;
  const d = a - b;
  if (Math.abs(d) < Math.pow(10, -digits) / 2) return { arrow: "≈", txt: "0", dir: "eq" };
  return {
    arrow: d > 0 ? "▲" : "▼",
    txt: (d > 0 ? "+" : "") + d.toFixed(digits),
    dir: d > 0 ? "up" : "down"
  };
}
const dTemp = computed(() =>
  makeDelta(recommendation.value?.target_temperature_c, latestSample.value?.temperature_c, 1)
);
const dRpm = computed(() =>
  makeDelta(recommendation.value?.target_stirrer_rpm, latestSample.value?.stirrer_rpm, 0)
);
const hasScore = computed(() => {
  const v = recommendation.value?.expected_score;
  return typeof v === "number" && Number.isFinite(v);
});

onMounted(async () => {
  try {
    await plant.loadProcesses();
  } catch {
    /* 工艺列表加载失败 → 静态 6 步骨架降级 */
  }
  try {
    await plant.loadBatches();
  } catch {
    /* 批次列表加载失败 → 批次面板空态降级 */
  }
  const wantId = Number(
    runtime.value?.active_process_id ?? activeBatch.value?.process_id ?? plant.processes[0]?.id
  );
  if (Number.isFinite(wantId) && plant.selectedProcess?.process?.id !== wantId) {
    try {
      await plant.loadProcessDetail(wantId);
    } catch {
      /* 详情加载失败 → 步骤详情显示 "--" */
    }
  }
});

onUnmounted(() => {
  window.clearInterval(timer);
});
</script>

<style scoped>
.monitor-page {
  display: grid;
  grid-template-rows: auto minmax(0, 55fr) minmax(0, 45fr);
  gap: var(--spacing);
  height: 100%;
  min-height: 0;
  overflow: hidden;
}

/* ---- 页头 ---- */
.page-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--spacing);
  flex: none;
}
.page-head .title {
  margin: 0;
  font-size: var(--fs-lg);
  font-weight: 700;
  color: var(--text-primary);
  display: flex;
  align-items: baseline;
  gap: 8px;
}
.page-head .title .zh {
  font-size: var(--fs-sm);
  font-weight: 400;
  color: var(--text-tertiary);
}
.head-meta {
  display: flex;
  align-items: center;
  gap: 14px;
}
.head-meta .meta {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-sm);
  color: var(--text-secondary);
}

/* ---- 上部三列 ---- */
.upper {
  display: grid;
  grid-template-columns: minmax(0, 1.05fr) minmax(0, 1.25fr) minmax(0, 0.9fr);
  gap: var(--spacing);
  min-height: 0;
}

/* 反应釜示意图 */
.reactor-wrap {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.reactor-svg {
  flex: 1;
  min-height: 0;
  width: 100%;
}
.reactor-svg .tag-label {
  font-size: 8.5px;
  fill: #9db4cf;
  letter-spacing: 0.4px;
}
.reactor-svg .tag-val {
  font-size: 15px;
  font-weight: 700;
  fill: #e8f1fb;
  font-family: var(--font-data);
}
.reactor-foot {
  flex: none;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.pill {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  padding: 4px 12px;
  border-radius: 999px;
  font-size: var(--fs-sm);
  font-weight: 700;
  letter-spacing: 0.5px;
}
.pill .pill-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: currentColor;
}
.pill.ok {
  color: var(--ind-green);
  background: rgba(47, 212, 123, 0.12);
  border: 1px solid rgba(47, 212, 123, 0.45);
}
.pill.ok .pill-dot {
  animation: pill-pulse 1.6s infinite;
}
.pill.idle {
  color: var(--text-tertiary);
  background: rgba(90, 115, 150, 0.12);
  border: 1px solid var(--ind-gray);
}
.pill.bad {
  color: var(--ind-red);
  background: rgba(255, 82, 82, 0.12);
  border: 1px solid rgba(255, 82, 82, 0.5);
  animation: pill-pulse 1.2s infinite;
}
@keyframes pill-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.35; }
}
.sample-time {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
}

/* 参数卡网格：2 列 × 4 行，第 7 张跨两列 */
.param-grid {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  grid-template-rows: repeat(4, minmax(0, 1fr));
  gap: 9px;
}
.param-card {
  position: relative;
  display: flex;
  flex-direction: column;
  /* 顶部顺排，剩余空间由底部 sparkline 吸收；矮卡自动让位给 SP 行 */
  justify-content: flex-start;
  gap: 1px;
  padding: 5px 9px 4px 13px;
  background: var(--bg-inset);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-md);
  min-height: 0;
  min-width: 0;
  overflow: hidden;
}
.param-card::before {
  content: "";
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 3px;
  background: var(--c);
  box-shadow: 0 0 8px var(--c);
}
.param-card.wide {
  grid-column: 1 / -1;
}
.param-head {
  display: flex;
  align-items: baseline;
  gap: 6px;
  min-width: 0;
  line-height: 1.15;
}
.param-head .en {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-secondary);
  /* V30：窄屏允许断行（原 ellipsis 仍被计为裁切） */
  white-space: normal;
  overflow-wrap: anywhere;
}
.param-head .zh {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  white-space: nowrap;
}
.param-value {
  display: flex;
  align-items: baseline;
  gap: 5px;
  min-width: 0;
}
.param-value .pv {
  font-size: clamp(14px, 1.3vw, 19px);
  font-weight: 700;
  color: var(--text-primary);
  line-height: 1.1;
}
.param-value .unit {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
}
.param-sp {
  min-height: 12px;
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  line-height: 12px;
}
.param-card :deep(.sparkline) {
  flex: 1 1 auto;
  /* 高度不够时整条让位（优先保住 SP 行），空间充裕再展开 */
  min-height: 0;
  opacity: 0.9;
}

/* 批次信息 */
.batch-chip {
  font-size: var(--fs-xs);
  color: var(--accent-strong);
  background: var(--accent-dim);
  border: 1px solid rgba(47, 155, 255, 0.4);
  border-radius: var(--radius-sm);
  padding: 1px 7px;
}
.batch-kv {
  flex: none;
}
.batch-kv dt .zh {
  color: var(--text-tertiary);
  opacity: 0.8;
}
.batch-foot {
  margin-top: auto;
  padding-top: 10px;
  display: flex;
  justify-content: flex-end;
}

/* ---- 下部五列 ---- */
.lower {
  display: grid;
  grid-template-columns: minmax(0, 1.85fr) minmax(0, 1.3fr) minmax(0, 1fr) minmax(0, 1.05fr) minmax(0, 0.72fr);
  gap: var(--spacing);
  min-height: 0;
}
.panel-note {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
}
.chart-wrap {
  flex: 1;
  min-height: 0;
}

/* 工艺流程 stepper */
.stepper {
  flex: none;
  display: flex;
  align-items: flex-start;
  padding: 6px 2px 10px;
}
.step {
  position: relative;
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 5px;
}
.step:not(:last-child)::after {
  content: "";
  position: absolute;
  top: 13px;
  left: calc(50% + 13px);
  width: calc(100% - 26px);
  height: 2px;
  background: var(--border-glass);
}
.step.done:not(:last-child)::after {
  background: var(--accent);
  opacity: 0.65;
}
.step .node {
  width: 20px;
  height: 20px;
  border-radius: 50%;
  border: 1.5px solid var(--ind-gray);
  background: var(--bg-inset);
  display: flex;
  align-items: center;
  justify-content: center;
  font-family: var(--font-data);
  font-size: 9px;
  font-weight: 700;
  color: var(--text-tertiary);
  transition: transform 0.2s, box-shadow 0.2s, border-color 0.2s;
  z-index: 1;
}
.step.done .node {
  border-color: var(--ind-green);
  color: var(--ind-green);
}
.step.current .node {
  transform: scale(1.3);
  border-color: var(--accent-strong);
  color: var(--accent-strong);
  box-shadow: 0 0 12px rgba(47, 155, 255, 0.55);
}
.step .slabel {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 1px;
  max-width: 100%;
  min-width: 0;
}
.step .slabel .en {
  font-size: 9px;
  font-weight: 600;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 100%;
}
.step.current .slabel .en {
  color: var(--accent-strong);
}
.step .slabel .zh {
  font-size: 9px;
  color: var(--text-tertiary);
}
.step-detail {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 7px;
  padding: 10px 12px;
  background: var(--bg-inset);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-md);
}
.sd-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 8px;
}
.sd-row .lbl {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  white-space: nowrap;
}
.sd-row .val {
  font-size: var(--fs-md);
  font-weight: 700;
  color: var(--text-primary);
}
.step-progress {
  height: 5px;
  border-radius: 3px;
  background: #22364f;
  overflow: hidden;
  margin-top: 2px;
}
.step-progress .bar {
  height: 100%;
  border-radius: 3px;
  background: linear-gradient(90deg, var(--accent), var(--accent-cyan));
  box-shadow: 0 0 8px rgba(47, 155, 255, 0.6);
  transition: width 0.5s;
}

/* 报警列表 */
.view-all {
  font-size: var(--fs-xs);
  color: var(--accent);
  text-decoration: none;
  white-space: nowrap;
}
.view-all:hover {
  color: var(--accent-strong);
  text-decoration: underline;
}
.alarm-rows {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.alarm-row {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  padding: 8px 9px;
  background: var(--bg-inset);
  border: 1px solid var(--border-glass);
  border-left-width: 3px;
  border-radius: var(--radius-md);
}
.alarm-row.high {
  border-left-color: var(--ind-red);
}
.alarm-row.medium {
  border-left-color: var(--ind-amber);
}
.alarm-row.info {
  border-left-color: var(--ind-gray);
}
.alarm-row .lv-ic {
  flex: none;
  margin-top: 1px;
  color: var(--ind-red);
}
.alarm-row.medium .lv-ic {
  color: var(--ind-amber);
}
.alarm-row.info .lv-ic {
  color: var(--ind-gray);
}
.alarm-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.alarm-msg {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-primary);
  line-height: 1.45;
  overflow-wrap: anywhere;
}
.alarm-sub {
  font-size: 10px;
  color: var(--text-tertiary);
}
.alarm-row :deep(.el-tag) {
  flex: none;
}

/* AI 推荐 */
.ai-model {
  font-size: 10px;
  color: var(--text-tertiary);
  max-width: 130px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.ai-body {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.ai-basis {
  margin: 0;
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  line-height: 1.5;
  border-bottom: 1px dashed var(--border-glass);
  padding-bottom: 8px;
}
.ai-val {
  display: flex;
  flex-direction: column;
  gap: 3px;
  background: var(--bg-inset);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-md);
  padding: 8px 11px;
}
.ai-val .lbl {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-secondary);
}
.ai-val .lbl .zh {
  font-weight: 400;
  color: var(--text-tertiary);
}
.ai-val .big {
  font-size: var(--fs-2xl);
  font-weight: 700;
  color: var(--accent-strong);
  line-height: 1.1;
}
.ai-val .big .unit {
  font-size: var(--fs-sm);
  font-weight: 400;
  color: var(--text-tertiary);
  margin-left: 4px;
}
.ai-val .sub {
  display: flex;
  align-items: center;
  gap: 10px;
}
.ai-val .delta {
  font-size: var(--fs-xs);
  font-weight: 700;
}
.ai-val .delta.up {
  color: var(--ind-amber);
}
.ai-val .delta.down {
  color: var(--accent-cyan);
}
.ai-val .delta.eq {
  color: var(--text-tertiary);
}
.ai-val .cur {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
}
.ai-score {
  margin-top: auto;
  font-size: var(--fs-xs);
  color: var(--text-secondary);
  text-align: right;
}

/* 窄屏压缩 */
@media (max-width: 1280px) {
  .lower {
    grid-template-columns: minmax(0, 1.8fr) minmax(0, 1.25fr) minmax(0, 0.95fr) minmax(0, 1fr) minmax(0, 0.68fr);
  }
  .ai-val .big {
    font-size: var(--fs-xl);
  }
}

/* V32：移动端单列堆叠、整页可滚动 */
@media (max-width: 900px) {
  .monitor-page { display: flex; flex-direction: column; height: auto; overflow: visible; }
  .upper, .lower { display: flex; flex-direction: column; min-height: 0; }
  .upper > *, .lower > * { flex: none; }
  /* 移动端卡片按内容定高，没有剩余空间可吸收，sparkline 需要固定高度 */
  .param-card :deep(.sparkline) { flex: none; height: 22px; }
}
/* AI 推荐卡整体可点，跳 AI 决策页 */
.ai-body.clickable, .empty-state.clickable { cursor: pointer; }
.ai-body.clickable:hover, .empty-state.clickable:hover { filter: brightness(1.15); }
.ai-go { display: inline-block; margin-top: 6px; font-size: 11px; color: var(--ind-blue, #2f9bff); }
</style>
