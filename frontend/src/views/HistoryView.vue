<script setup lang="ts">
// 历史数据与批次记录（History & Batch Records）
// 布局：筛选栏 → 趋势回放(60%) + 批次详情(40%) → 批次记录 + 事件回放 + 历史对比。
// 数据全部来自真实接口：/api/batches、/api/batches/{id}、/api/v1/reactor/{device}/history。
// 后端没有的字段（操作员、产品名等）一律显示 "--"，绝不编造。
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { ElMessage } from "element-plus";
import PanelCard from "../components/PanelCard.vue";
import TrendChart from "../components/TrendChart.vue";
import AppIcon from "../components/AppIcon.vue";
import { batchApi, realtimeApi, DEVICE_ID } from "../api";
import { downloadBlob } from "../api/http";
import { errorMessage } from "../api/errors";
import { usePlantStore } from "../stores/plant";
import { useLanguage } from "../i18n";
import { fixed, formatTime, formatTimestamp } from "../utils/format";
import type { Batch, BatchDetail, ControlEvent, HistoryRecord } from "../api/types";

const plant = usePlantStore();
const { tr } = useLanguage();

// ---------- 局部类型 ----------
interface ReplayPoint {
  t: number;
  temp: number | null;
  pressBar: number | null; // MPa × 10 → bar
  ph: number | null;
  conc: number | null;
  rpm: number | null;
}

interface DetailMetrics {
  yieldPct: number | null;
  cycleMin: number | null;
  maxTempC: number | null;
  maxPressBar: number | null;
  avgPh: number | null;
  finalConcPct: number | null;
}

interface SeriesSpec {
  name: string;
  data: Array<[number, number | null]>;
  color: string;
  yAxisIndex?: number;
}

const SERIES_COLORS = {
  temp: "#ff5252",
  press: "#f5a623",
  ph: "#2fd47b",
  conc: "#b068f0",
  rpm: "#38c8f2"
};

// ---------- 工具 ----------
function num(value: unknown): number | null {
  const n = typeof value === "number" ? value : Number(value);
  return Number.isFinite(n) ? n : null;
}

function tsMs(value: unknown): number | null {
  if (value === null || value === undefined || value === "") return null;
  const t = Date.parse(String(value));
  return Number.isNaN(t) ? null : t;
}

function normId(value: number | string | null | undefined): number | null {
  if (value === "" || value === null || value === undefined) return null;
  const n = Number(value);
  return Number.isFinite(n) ? n : null;
}

function pad(n: number): string {
  return String(n).padStart(2, "0");
}

function shortTime(t: number | null | undefined): string {
  if (t === null || t === undefined) return "--";
  const d = new Date(t);
  return `${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

function formatDurationMin(mins: number | null): string {
  if (mins === null) return "--";
  if (mins < 1) return "<1m";
  const h = Math.floor(mins / 60);
  const m = Math.round(mins % 60);
  if (h <= 0) return `${m}m`;
  return `${h}h ${pad(m)}m`;
}

function stamp(): string {
  const d = new Date();
  return `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}_${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}`;
}

// ---------- 筛选栏 ----------
const filters = reactive({
  range: null as [Date, Date] | null,
  batchId: null as number | string | null,
  processId: null as number | string | null,
  deviceId: DEVICE_ID as string
});

const deviceOptions: string[] = [DEVICE_ID];

const batches = computed<Batch[]>(() => plant.batches?.batches ?? []);

const outcomeById = computed(() => {
  const map = new Map<number, { yield_percent?: number | null }>();
  for (const o of plant.batches?.outcomes ?? []) map.set(o.id, o);
  return map;
});

const processById = computed(() => {
  const map = new Map<number, { name?: string | null }>();
  for (const p of plant.processes) map.set(p.id, p);
  return map;
});

const batchOptions = computed(() =>
  batches.value
    .slice()
    .sort((a, b) => b.id - a.id)
    .map((b) => ({ id: b.id, label: `#${b.id}${b.name ? " · " + b.name : ""}` }))
);

const processOptions = computed(() =>
  plant.processes.map((p) => ({ id: p.id, label: p.name ? String(p.name) : `#${p.id}` }))
);

const filteredBatches = computed<Batch[]>(() => {
  const start = filters.range?.[0]?.getTime() ?? null;
  const end = filters.range?.[1]?.getTime() ?? null;
  const bid = normId(filters.batchId);
  const pid = normId(filters.processId);
  return batches.value
    .filter((b) => {
      if (bid !== null && b.id !== bid) return false;
      if (pid !== null && (b.process_id ?? -1) !== pid) return false;
      const t = tsMs(b.started_at);
      if (start !== null && (t === null || t < start)) return false;
      if (end !== null && (t === null || t > end)) return false;
      return true;
    })
    .slice()
    .sort(
      (a, b) =>
        (tsMs(b.started_at) ?? 0) - (tsMs(a.started_at) ?? 0) || b.id - a.id
    );
});

// ---------- 批次记录表（分页） ----------
const page = ref(1);
const pageSize = 8;
const listLoading = ref(false);

watch(filteredBatches, () => {
  page.value = 1;
});

const pagedBatches = computed<Batch[]>(() => {
  const start = (page.value - 1) * pageSize;
  return filteredBatches.value.slice(start, start + pageSize);
});

function yieldOf(b: Batch): number | null {
  return num(outcomeById.value.get(b.id)?.yield_percent);
}

function recipeOf(b: Batch): string {
  const name = processById.value.get(b.process_id ?? -1)?.name;
  return name ? String(name) : "--";
}

function statusInfo(b: Batch): { type: "success" | "warning" | "info"; text: string } {
  if (b.finished_at) return { type: "success", text: tr("已完成", "Completed") };
  if (b.started_at) return { type: "warning", text: tr("运行中", "Running") };
  return { type: "info", text: "--" };
}

// ---------- 选中批次与详情 ----------
const selectedId = ref<number | null>(null);
const detail = ref<BatchDetail | null>(null);
const detailLoading = ref(false);

const selectedBatch = computed<Batch | null>(
  () => batches.value.find((b) => b.id === selectedId.value) ?? detail.value?.batch ?? null
);

async function selectBatch(id: number): Promise<void> {
  if (selectedId.value === id && detail.value) {
    setTrendFromBatch(detail.value);
    return;
  }
  selectedId.value = id;
  detail.value = null;
  detailLoading.value = true;
  try {
    const d = comparisonCache.value[id] ?? (await batchApi.detail(id));
    detail.value = d;
    setTrendFromBatch(d);
  } catch (error) {
    detail.value = null;
    ElMessage.error(errorMessage(error));
  } finally {
    detailLoading.value = false;
  }
}

// ---------- 指标计算（详情卡片 & 历史对比共用） ----------
function computeMetrics(d: BatchDetail | null | undefined): DetailMetrics {
  const out: DetailMetrics = {
    yieldPct: null,
    cycleMin: null,
    maxTempC: null,
    maxPressBar: null,
    avgPh: null,
    finalConcPct: null
  };
  if (!d) return out;
  out.yieldPct = num(d.outcome?.yield_percent);
  const startT = tsMs(d.batch.started_at);
  const endT = tsMs(d.batch.finished_at);
  if (startT !== null && endT !== null && endT >= startT) {
    out.cycleMin = (endT - startT) / 60000;
  }
  const samples = d.samples ?? [];
  const temps: number[] = [];
  const presses: number[] = [];
  const phs: number[] = [];
  for (const s of samples) {
    const t = num(s.temperature_c);
    if (t !== null) temps.push(t);
    const p = num(s.pressure_mpa);
    if (p !== null) presses.push(p);
    const ph = num(s.ph);
    if (ph !== null) phs.push(ph);
  }
  if (temps.length) out.maxTempC = Math.max(...temps);
  if (presses.length) out.maxPressBar = Math.max(...presses) * 10;
  if (phs.length) out.avgPh = phs.reduce((a, b) => a + b, 0) / phs.length;
  for (let i = samples.length - 1; i >= 0; i--) {
    const c = num(samples[i]?.product_concentration_percent);
    if (c !== null) {
      out.finalConcPct = c;
      break;
    }
  }
  return out;
}

const metrics = computed<DetailMetrics>(() => computeMetrics(detail.value));

const tempLimitC = computed(() => num(plant.config?.safety?.temperature?.max_c));

const metricCards = computed(() => {
  const m = metrics.value;
  return [
    { key: "yield", en: "Yield", zh: "产率", value: fixed(m.yieldPct, 1), unit: "%", sub: "Target ≥ 95%" },
    { key: "cycle", en: "Cycle Time", zh: "周期时间", value: formatDurationMin(m.cycleMin), unit: "", sub: "" },
    {
      key: "maxT",
      en: "Max Temp",
      zh: "最高温度",
      value: fixed(m.maxTempC, 1),
      unit: "°C",
      sub: `Limit ${fixed(tempLimitC.value, 0, "°C")}`
    },
    { key: "maxP", en: "Max Pressure", zh: "最高压力", value: fixed(m.maxPressBar, 1), unit: "bar", sub: "" },
    { key: "ph", en: "Avg pH", zh: "平均pH", value: fixed(m.avgPh, 2), unit: "", sub: "Target --" },
    { key: "conc", en: "Final Conc.", zh: "终浓度", value: fixed(m.finalConcPct, 2), unit: "%", sub: "" }
  ];
});

const outcomeInfo = computed(() => {
  const y = metrics.value.yieldPct;
  if (y === null) return { cls: "", text: "--" };
  if (y >= 95) return { cls: "pass", text: tr("合格 Pass", "Pass") };
  return { cls: "review", text: tr("待复核 Review", "Review") };
});

// ---------- 趋势回放 ----------
const trendPoints = ref<ReplayPoint[]>([]);
const trendLoading = ref(false);
const trendSource = ref<{ kind: "batch"; id: number } | { kind: "history"; deviceId: string } | null>(null);

const cursor = ref(0);
const playing = ref(false);
const speed = ref(1);
const zoomed = ref(false);
let playTimer: number | null = null;

function clearPlayTimer(): void {
  if (playTimer !== null) {
    window.clearInterval(playTimer);
    playTimer = null;
  }
}

function stopReplay(): void {
  playing.value = false;
  clearPlayTimer();
}

function replayTick(): void {
  if (cursor.value >= trendPoints.value.length - 1) {
    stopReplay();
    return;
  }
  cursor.value += 1;
}

function startPlayTimer(): void {
  clearPlayTimer();
  playTimer = window.setInterval(replayTick, Math.max(60, Math.round(240 / speed.value)));
}

function toggleReplay(): void {
  if (playing.value) {
    stopReplay();
    return;
  }
  if (trendPoints.value.length < 2) return;
  if (cursor.value >= trendPoints.value.length - 1) cursor.value = 0;
  playing.value = true;
  startPlayTimer();
}

watch(speed, () => {
  if (playing.value) startPlayTimer();
});

watch(trendPoints, () => {
  cursor.value = 0;
  stopReplay();
});

onBeforeUnmount(stopReplay);

const sliderMax = computed(() => Math.max(0, trendPoints.value.length - 1));

function sliderTooltip(v: number): string {
  return shortTime(trendPoints.value[v]?.t ?? null);
}

const markTime = computed(() => trendPoints.value[cursor.value]?.t ?? null);

const trendSourceLabel = computed(() => {
  const src = trendSource.value;
  if (!src) return "";
  if (src.kind === "batch") return `${tr("数据源：批次样本", "Source: batch samples")} #${src.id}`;
  return `${tr("数据源：设备历史", "Source: device history")} · ${src.deviceId}`;
});

const series = computed<SeriesSpec[]>(() => {
  const pts = trendPoints.value;
  if (!pts.length) return [];
  return [
    { name: tr("温度 °C", "Temp °C"), color: SERIES_COLORS.temp, data: pts.map((p) => [p.t, p.temp]) },
    { name: tr("压力 bar", "Press bar"), color: SERIES_COLORS.press, data: pts.map((p) => [p.t, p.pressBar]) },
    { name: "pH", color: SERIES_COLORS.ph, data: pts.map((p) => [p.t, p.ph]) },
    { name: tr("浓度 %", "Conc %"), color: SERIES_COLORS.conc, data: pts.map((p) => [p.t, p.conc]) },
    {
      name: tr("转速 rpm", "RPM"),
      color: SERIES_COLORS.rpm,
      yAxisIndex: 1,
      data: pts.map((p) => [p.t, p.rpm])
    }
  ];
});

function setTrendFromBatch(d: BatchDetail | null): void {
  const pts: ReplayPoint[] = [];
  for (const s of d?.samples ?? []) {
    const t = tsMs(s.captured_at ?? s.created_at);
    if (t === null) continue;
    const press = num(s.pressure_mpa);
    pts.push({
      t,
      temp: num(s.temperature_c),
      pressBar: press === null ? null : press * 10,
      ph: num(s.ph),
      conc: num(s.product_concentration_percent),
      rpm: num(s.stirrer_rpm)
    });
  }
  pts.sort((a, b) => a.t - b.t);
  trendPoints.value = pts;
  trendSource.value = d ? { kind: "batch", id: d.batch.id } : null;
}

async function loadHistoryTrend(): Promise<void> {
  trendLoading.value = true;
  try {
    const end = filters.range?.[1] ?? new Date();
    const start = filters.range?.[0] ?? new Date(end.getTime() - 24 * 60 * 60 * 1000);
    const res = await realtimeApi.history(filters.deviceId, {
      startTime: start.toISOString(),
      endTime: end.toISOString(),
      pageSize: 600
    });
    const rows: HistoryRecord[] = res?.records ?? res?.items ?? [];
    const pts: ReplayPoint[] = [];
    for (const r of rows) {
      const t = tsMs(r.timestamp);
      if (t === null) continue;
      const press = num(r.data?.current_pressure);
      pts.push({
        t,
        temp: num(r.data?.current_temp),
        pressBar: press === null ? null : press * 10,
        ph: num(r.data?.ph),
        conc: num(r.data?.product_concentration),
        rpm: num(r.data?.stir_speed)
      });
    }
    pts.sort((a, b) => a.t - b.t);
    trendPoints.value = pts;
    trendSource.value = { kind: "history", deviceId: filters.deviceId };
  } catch (error) {
    ElMessage.error(errorMessage(error));
  } finally {
    trendLoading.value = false;
  }
}

// ---------- 事件回放（选中批次 detail.events） ----------
const events = computed<ControlEvent[]>(() => detail.value?.events ?? []);

function eventValue(e: ControlEvent): string {
  const parts: string[] = [];
  const t = num(e.target_temperature_c);
  if (t !== null) parts.push(fixed(t, 1, "°C"));
  const r = num(e.target_stirrer_rpm);
  if (r !== null) parts.push(fixed(r, 0, " rpm"));
  const sh = num(e.target_shake_speed_cpm);
  if (sh !== null) parts.push(fixed(sh, 0, " cpm"));
  return parts.length ? parts.join(" · ") : "--";
}

// ---------- 历史对比（最近 4 个批次） ----------
const comparisonIds = ref<number[]>([]);
const comparisonCache = ref<Record<number, BatchDetail | null>>({});
let comparisonToken = 0;

async function loadComparison(): Promise<void> {
  const ids = filteredBatches.value.slice(0, 4).map((b) => b.id);
  const token = ++comparisonToken;
  const missing = ids.filter((id) => !(id in comparisonCache.value));
  if (missing.length) {
    const loaded = await Promise.all(
      missing.map((id) => batchApi.detail(id).catch(() => null))
    );
    if (token !== comparisonToken) return;
    const next = { ...comparisonCache.value };
    missing.forEach((id, i) => {
      next[id] = loaded[i];
    });
    comparisonCache.value = next;
  }
  if (token !== comparisonToken) return;
  comparisonIds.value = ids;
}

watch(
  () => filteredBatches.value.slice(0, 4).map((b) => b.id).join(","),
  () => {
    void loadComparison();
  },
  { immediate: true }
);

const comparisonBatches = computed(() =>
  comparisonIds.value.map((id) => ({
    id,
    metrics: computeMetrics(comparisonCache.value[id] ?? null)
  }))
);

interface CompCell {
  text: string;
  state: "" | "best" | "worst";
}

interface CompRow {
  key: string;
  en: string;
  zh: string;
  cells: CompCell[];
}

const comparisonRows = computed<CompRow[]>(() => {
  const list = comparisonBatches.value;
  const defs = [
    {
      key: "yield", en: "Yield", zh: "产率", dir: 1,
      get: (m: DetailMetrics) => m.yieldPct,
      score: (v: number) => v,
      fmt: (v: number) => fixed(v, 1, "%")
    },
    {
      key: "cycle", en: "Cycle Time", zh: "周期时间", dir: -1,
      get: (m: DetailMetrics) => m.cycleMin,
      score: (v: number) => v,
      fmt: (v: number) => formatDurationMin(v)
    },
    {
      key: "maxT", en: "Max Temp", zh: "最高温度", dir: -1,
      get: (m: DetailMetrics) => m.maxTempC,
      score: (v: number) => v,
      fmt: (v: number) => fixed(v, 1, "°C")
    },
    {
      key: "maxP", en: "Max Pressure", zh: "最高压力", dir: -1,
      get: (m: DetailMetrics) => m.maxPressBar,
      score: (v: number) => v,
      fmt: (v: number) => fixed(v, 1, "bar")
    },
    {
      key: "ph", en: "Avg pH", zh: "平均pH", dir: -1,
      get: (m: DetailMetrics) => m.avgPh,
      score: (v: number) => Math.abs(v - 7),
      fmt: (v: number) => fixed(v, 2)
    },
    {
      key: "conc", en: "Final Conc.", zh: "终浓度", dir: 1,
      get: (m: DetailMetrics) => m.finalConcPct,
      score: (v: number) => v,
      fmt: (v: number) => fixed(v, 2, "%")
    }
  ];
  return defs.map((def) => {
    const raws = list.map((e) => def.get(e.metrics));
    const scored = raws
      .map((v, i) => ({ i, s: v === null ? null : def.score(v) }))
      .filter((x): x is { i: number; s: number } => x.s !== null);
    let bestI = -1;
    let worstI = -1;
    if (scored.length >= 2) {
      let best = scored[0];
      let worst = scored[0];
      for (const c of scored) {
        if (def.dir * (c.s - best.s) > 0) best = c;
        if (def.dir * (c.s - worst.s) < 0) worst = c;
      }
      if (best.i !== worst.i) {
        bestI = best.i;
        worstI = worst.i;
      }
    }
    return {
      key: def.key,
      en: def.en,
      zh: def.zh,
      cells: raws.map((v, i) => ({
        text: v === null ? "--" : def.fmt(v),
        state: i === bestI ? "best" : i === worstI ? "worst" : ""
      }))
    };
  });
});

const compGridStyle = computed(() => ({
  gridTemplateColumns: `minmax(88px, 1.1fr) repeat(${Math.max(1, comparisonBatches.value.length)}, minmax(56px, 1fr))`
}));

// ---------- 筛选动作 / 导出 ----------
function resetFilters(): void {
  filters.range = null;
  filters.batchId = null;
  filters.processId = null;
  filters.deviceId = DEVICE_ID;
  page.value = 1;
}

async function runSearch(): Promise<void> {
  page.value = 1;
  const bid = normId(filters.batchId);
  if (bid !== null && batches.value.some((b) => b.id === bid)) {
    await selectBatch(bid);
    return;
  }
  await loadHistoryTrend();
}

const exportingCsv = ref(false);
const exportingReport = ref(false);

async function exportCsv(): Promise<void> {
  exportingCsv.value = true;
  try {
    const blob = await batchApi.exportCsv();
    downloadBlob(blob, `batches_${stamp()}.csv`);
    ElMessage.success(tr("CSV 导出已开始", "CSV export started"));
  } catch (error) {
    ElMessage.error(errorMessage(error));
  } finally {
    exportingCsv.value = false;
  }
}

async function exportReport(): Promise<void> {
  if (selectedId.value === null) {
    ElMessage.warning(tr("请先在批次记录中选择一个批次", "Select a batch first"));
    return;
  }
  exportingReport.value = true;
  try {
    const blob = await batchApi.exportReport(selectedId.value);
    downloadBlob(blob, `batch_${selectedId.value}_report_${stamp()}.md`);
    ElMessage.success(tr("报表导出已开始", "Report export started"));
  } catch (error) {
    ElMessage.error(errorMessage(error));
  } finally {
    exportingReport.value = false;
  }
}

// ---------- 初始化 ----------
onMounted(async () => {
  listLoading.value = true;
  await Promise.all([
    plant.loadBatches().catch(() => undefined),
    plant.loadProcesses().catch(() => undefined)
  ]);
  listLoading.value = false;
  const first = filteredBatches.value[0];
  if (first) {
    await selectBatch(first.id);
  } else {
    await loadHistoryTrend();
  }
});
</script>

<template>
  <div class="page-stack">
    <header class="page-header">
      <div>
        <h2 class="page-title">
          History &amp; Batch Records<span class="zh">历史数据与批次记录</span>
        </h2>
        <p class="page-subtitle">
          {{ tr("批次记录、趋势回放与历史指标对比", "Batch records, replay trend and historical comparison") }}
        </p>
      </div>
    </header>

    <!-- 0) 筛选栏 -->
    <section class="panel filter-bar">
      <div class="field field-range">
        <span class="field-label">{{ tr("时间范围", "Date Range") }}</span>
        <el-date-picker
          v-model="filters.range"
          type="datetimerange"
          size="small"
          format="MM-DD HH:mm"
          :start-placeholder="tr('开始', 'Start')"
          :end-placeholder="tr('结束', 'End')"
        />
      </div>
      <div class="field">
        <span class="field-label">{{ tr("批次号", "Batch ID") }}</span>
        <el-select
          v-model="filters.batchId"
          size="small"
          filterable
          clearable
          class="w-select"
          :placeholder="tr('全部', 'All')"
        >
          <el-option v-for="o in batchOptions" :key="o.id" :value="o.id" :label="o.label" />
        </el-select>
      </div>
      <div class="field">
        <span class="field-label">{{ tr("配方", "Recipe") }}</span>
        <el-select
          v-model="filters.processId"
          size="small"
          clearable
          class="w-select"
          :placeholder="tr('全部', 'All')"
        >
          <el-option v-for="o in processOptions" :key="o.id" :value="o.id" :label="o.label" />
        </el-select>
      </div>
      <div class="field">
        <span class="field-label">{{ tr("设备", "Device") }}</span>
        <el-select v-model="filters.deviceId" size="small" class="w-select-sm">
          <el-option v-for="d in deviceOptions" :key="d" :value="d" :label="d" />
        </el-select>
      </div>
      <div class="field field-buttons">
        <el-button size="small" @click="resetFilters">
          <AppIcon name="reset" :size="13" />
          <span class="btn-gap">{{ tr("重置", "Reset") }}</span>
        </el-button>
        <el-button size="small" type="primary" @click="runSearch">
          <AppIcon name="search" :size="13" />
          <span class="btn-gap">{{ tr("查询", "Search") }}</span>
        </el-button>
      </div>
      <div class="spacer" />
      <el-button size="small" :loading="exportingCsv" @click="exportCsv">
        <AppIcon name="export" :size="13" />
        <span class="btn-gap">{{ tr("导出 CSV", "Export CSV") }}</span>
      </el-button>
      <el-button size="small" :loading="exportingReport" @click="exportReport">
        <AppIcon name="report" :size="13" />
        <span class="btn-gap">{{ tr("导出报表", "Export Report") }}</span>
      </el-button>
    </section>

    <div class="rows">
      <!-- 1) 第一行：趋势回放 + 批次详情 -->
      <div class="row row-top">
        <PanelCard en="Replay Trend" zh="趋势回放" class="replay-panel">
          <template #actions>
            <div class="replay-actions">
              <span v-if="trendSourceLabel" class="src-label">{{ trendSourceLabel }}</span>
              <el-button
                size="small"
                circle
                :type="playing ? 'warning' : 'primary'"
                :disabled="trendPoints.length < 2"
                :title="playing ? tr('暂停', 'Pause') : tr('播放', 'Play')"
                @click="toggleReplay"
              >
                <AppIcon :name="playing ? 'pause' : 'play'" :size="13" />
              </el-button>
              <el-select v-model="speed" size="small" class="speed-select" :disabled="!playing">
                <el-option :value="1" label="1x" />
                <el-option :value="2" label="2x" />
                <el-option :value="4" label="4x" />
              </el-select>
              <el-button size="small" :disabled="!trendPoints.length" @click="zoomed = true">
                <AppIcon name="search" :size="13" />
                <span class="btn-gap">{{ tr("放大", "Zoom") }}</span>
              </el-button>
            </div>
          </template>
          <div v-loading="trendLoading" class="chart-area">
            <TrendChart
              v-if="trendPoints.length"
              :series="series"
              :mark-time="markTime"
              height="100%"
            />
            <div v-else class="empty-state">
              <span class="empty-icon">📈</span>
              <span>{{ tr("暂无样本数据：调整筛选后查询，或在批次记录中选择批次", "No sample data — search with filters or select a batch") }}</span>
            </div>
          </div>
          <div class="replay-slider">
            <span class="edge mono">{{ shortTime(trendPoints[0]?.t ?? null) }}</span>
            <el-slider
              v-model="cursor"
              :min="0"
              :max="sliderMax"
              :disabled="trendPoints.length < 2"
              :format-tooltip="sliderTooltip"
            />
            <span class="edge mono">{{ shortTime(trendPoints[trendPoints.length - 1]?.t ?? null) }}</span>
          </div>
        </PanelCard>

        <PanelCard en="Batch Detail (Selected)" zh="批次详情" scrollable>
          <template #actions>
            <el-tag v-if="selectedBatch" size="small" :type="statusInfo(selectedBatch).type">
              {{ statusInfo(selectedBatch).text }}
            </el-tag>
            <el-tag v-else size="small" type="info">--</el-tag>
          </template>
          <div v-loading="detailLoading" class="detail-body">
            <template v-if="selectedBatch">
              <div class="kv-head">
                <div class="kv-cell">
                  <span class="k">Batch ID 批次号</span>
                  <span class="v mono">#{{ selectedBatch.id }}</span>
                </div>
                <div class="kv-cell">
                  <span class="k">Recipe 配方</span>
                  <span class="v">{{ recipeOf(selectedBatch) }}</span>
                </div>
                <div class="kv-cell">
                  <span class="k">Product 产品</span>
                  <span class="v">--</span>
                </div>
              </div>

              <div class="metric-grid">
                <div v-for="c in metricCards" :key="c.key" class="metric-card">
                  <span class="m-label">
                    <span class="m-en">{{ c.en }}</span>
                    <span class="m-zh">{{ c.zh }}</span>
                  </span>
                  <span class="m-value">
                    {{ c.value }}<small v-if="c.unit" class="m-unit">{{ c.unit }}</small>
                  </span>
                  <span v-if="c.sub" class="m-sub">{{ c.sub }}</span>
                </div>
              </div>

              <dl class="kv-list detail-kv">
                <dt>{{ tr("开始时间", "Start Time") }}</dt>
                <dd class="mono">{{ formatTimestamp(selectedBatch.started_at) }}</dd>
                <dt>{{ tr("结束时间", "End Time") }}</dt>
                <dd class="mono">{{ formatTimestamp(selectedBatch.finished_at) }}</dd>
                <dt>{{ tr("操作员", "Operator") }}</dt>
                <dd>--</dd>
                <dt>{{ tr("结果", "Outcome") }}</dt>
                <dd>
                  <span v-if="outcomeInfo.cls" class="outcome" :class="outcomeInfo.cls">
                    <AppIcon v-if="outcomeInfo.cls === 'pass'" name="check" :size="12" />
                    {{ outcomeInfo.text }}
                  </span>
                  <template v-else>--</template>
                </dd>
              </dl>
            </template>
            <div v-else class="empty-state">
              <span class="empty-icon">🧪</span>
              <span>{{ tr("在下方批次记录中点击行以查看详情", "Click a batch row below to view detail") }}</span>
            </div>
          </div>
        </PanelCard>
      </div>

      <!-- 2) 第二行：批次记录 + 事件回放 + 历史对比 -->
      <div class="row row-bottom">
        <PanelCard en="Batch Records" zh="批次记录" flush class="records-panel">
          <div v-loading="listLoading" class="table-wrap">
            <el-table
              :data="pagedBatches"
              size="small"
              height="100%"
              :empty-text="tr('暂无批次记录', 'No batch records')"
              :row-class-name="({ row }: { row: Batch }) => (row.id === selectedId ? 'row-selected' : '')"
              @row-click="(row: Batch) => selectBatch(row.id)"
            >
              <el-table-column prop="id" width="74">
                <template #header>
                  <span class="th-en">Batch ID</span>
                  <span class="th-zh">批次号</span>
                </template>
                <template #default="{ row }">
                  <span class="mono">#{{ row.id }}</span>
                </template>
              </el-table-column>
              <el-table-column min-width="110">
                <template #header>
                  <span class="th-en">Recipe</span>
                  <span class="th-zh">配方</span>
                </template>
                <template #default="{ row }">{{ recipeOf(row) }}</template>
              </el-table-column>
              <el-table-column min-width="140">
                <template #header>
                  <span class="th-en">Start Time</span>
                  <span class="th-zh">开始时间</span>
                </template>
                <template #default="{ row }">
                  <span class="mono cell-time">{{ formatTimestamp(row.started_at) }}</span>
                </template>
              </el-table-column>
              <el-table-column min-width="140">
                <template #header>
                  <span class="th-en">End Time</span>
                  <span class="th-zh">结束时间</span>
                </template>
                <template #default="{ row }">
                  <span class="mono cell-time">{{ formatTimestamp(row.finished_at) }}</span>
                </template>
              </el-table-column>
              <el-table-column width="80">
                <template #header>
                  <span class="th-en">Yield</span>
                  <span class="th-zh">产率</span>
                </template>
                <template #default="{ row }">
                  <span class="mono">{{ fixed(yieldOf(row), 1, "%") }}</span>
                </template>
              </el-table-column>
              <el-table-column width="92">
                <template #header>
                  <span class="th-en">Status</span>
                  <span class="th-zh">状态</span>
                </template>
                <template #default="{ row }">
                  <el-tag size="small" :type="statusInfo(row).type">{{ statusInfo(row).text }}</el-tag>
                </template>
              </el-table-column>
              <el-table-column width="84">
                <template #header>
                  <span class="th-en">Operator</span>
                  <span class="th-zh">操作员</span>
                </template>
                <template #default>
                  <span class="mono">--</span>
                </template>
              </el-table-column>
            </el-table>
          </div>
          <footer class="table-footer">
            <span class="total-text">
              {{ tr(`共 ${filteredBatches.length} 条记录`, `Total ${filteredBatches.length} records`) }}
            </span>
            <el-pagination
              v-model:current-page="page"
              :page-size="pageSize"
              :total="filteredBatches.length"
              size="small"
              layout="prev, pager, next"
              background
            />
          </footer>
        </PanelCard>

        <PanelCard en="Event Replay" zh="事件回放">
          <div class="event-list">
            <template v-if="events.length">
              <div v-for="e in events" :key="e.id" class="event-row">
                <span class="ev-time mono">{{ formatTime(e.created_at) }}</span>
                <span class="ev-tag" :title="e.event_type">{{ e.event_type }}</span>
                <span class="ev-text" :title="e.reason || e.event_type">{{ e.reason || e.event_type }}</span>
                <span class="ev-value mono">{{ eventValue(e) }}</span>
              </div>
            </template>
            <div v-else class="empty-mini">
              {{ selectedBatch
                ? tr("该批次暂无控制事件", "No control events for this batch")
                : tr("选择批次后查看事件回放", "Select a batch to replay events") }}
            </div>
          </div>
          <router-link class="audit-link" to="/audit">
            View Full Events <span class="zh">查看全部事件</span> →
          </router-link>
        </PanelCard>

        <PanelCard en="Historical Comparison" zh="历史对比 · 最近批次">
          <div class="comp-wrap">
            <template v-if="comparisonBatches.length">
              <div class="comp-row comp-head" :style="compGridStyle">
                <span class="comp-label">
                  <span class="th-en">Metric</span>
                  <span class="th-zh">指标</span>
                </span>
                <span v-for="c in comparisonBatches" :key="c.id" class="comp-cell comp-batch mono">
                  #{{ c.id }}
                </span>
              </div>
              <div v-for="row in comparisonRows" :key="row.key" class="comp-row" :style="compGridStyle">
                <span class="comp-label">
                  <span class="lbl-en">{{ row.en }}</span>
                  <span class="lbl-zh">{{ row.zh }}</span>
                </span>
                <span
                  v-for="(cell, i) in row.cells"
                  :key="comparisonBatches[i]?.id"
                  class="comp-cell mono"
                  :class="cell.state"
                >
                  {{ cell.text }}
                </span>
              </div>
            </template>
            <div v-else class="empty-mini">{{ tr("暂无可对比的批次", "No batches to compare") }}</div>
          </div>
        </PanelCard>
      </div>
    </div>

    <!-- 放大对话框 -->
    <el-dialog
      v-model="zoomed"
      :title="tr('趋势回放 · 放大', 'Replay Trend · Zoom')"
      width="86%"
      top="4vh"
      append-to-body
    >
      <div class="zoom-chart">
        <TrendChart v-if="trendPoints.length" :series="series" :mark-time="markTime" height="72vh" />
      </div>
    </el-dialog>
  </div>
</template>

<style scoped>
/* ---------- 布局骨架（整页不滚动） ---------- */
.rows {
  flex: 1 1 auto;
  min-height: 0;
  display: grid;
  grid-template-rows: 48fr 52fr;
  gap: var(--spacing);
}

.row {
  display: grid;
  gap: var(--spacing);
  min-height: 0;
}

.row > * {
  min-width: 0;
  min-height: 0;
}

.row-top {
  grid-template-columns: 6fr 4fr;
}

.row-bottom {
  grid-template-columns: 1.45fr 1fr 1.2fr;
}

@media (max-width: 1280px) {
  .row-top {
    grid-template-columns: 1fr 1fr;
  }
  .row-bottom {
    grid-template-columns: 1fr 1fr 1fr;
  }
}

/* ---------- 筛选栏 ---------- */
.filter-bar {
  flex: none;
  flex-direction: row;
  align-items: center;
  flex-wrap: wrap;
  gap: 10px 14px;
  padding: 10px 14px;
}

.field {
  display: flex;
  align-items: center;
  gap: 6px;
}

.field-label {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  white-space: nowrap;
}

.field-range {
  width: 318px;
}

.field-range :deep(.el-date-editor) {
  width: 100%;
}

.w-select {
  width: 150px;
}

.w-select-sm {
  width: 122px;
}

.field-buttons {
  gap: 8px;
}

.spacer {
  flex: 1 1 auto;
}

.btn-gap {
  margin-left: 5px;
}

/* ---------- 趋势回放 ---------- */
.replay-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.src-label {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  margin-right: 4px;
  white-space: nowrap;
}

.speed-select {
  width: 66px;
}

.chart-area {
  flex: 1 1 auto;
  min-height: 0;
  position: relative;
}

.replay-slider {
  flex: none;
  display: flex;
  align-items: center;
  gap: 10px;
  padding-top: 8px;
}

.replay-slider :deep(.el-slider) {
  flex: 1 1 auto;
}

.edge {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  white-space: nowrap;
}

.zoom-chart {
  height: 72vh;
}

/* ---------- 批次详情 ---------- */
.detail-body {
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-height: 0;
}

.kv-head {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 8px;
}

.kv-cell {
  background: var(--bg-inset);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-md);
  padding: 7px 9px;
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}

.kv-cell .k {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.kv-cell .v {
  font-size: var(--fs-sm);
  color: var(--text-primary);
  overflow-wrap: anywhere;
}

.metric-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 8px;
}

.metric-card {
  background: var(--bg-inset);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-md);
  padding: 8px 10px;
  display: flex;
  flex-direction: column;
  gap: 3px;
  min-width: 0;
}

.m-label {
  display: flex;
  align-items: baseline;
  gap: 5px;
  min-width: 0;
}

.m-en {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.m-zh {
  font-size: 10px;
  color: var(--text-tertiary);
  white-space: nowrap;
}

.m-value {
  font-family: var(--font-data);
  font-variant-numeric: tabular-nums;
  font-size: 19px;
  font-weight: 700;
  color: var(--text-primary);
  line-height: 1.15;
}

.m-unit {
  font-size: var(--fs-xs);
  font-weight: 500;
  color: var(--text-secondary);
  margin-left: 3px;
}

.m-sub {
  font-size: 10px;
  color: var(--text-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.detail-kv dt {
  white-space: normal;
}

.outcome {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  font-weight: 600;
}

.outcome.pass {
  color: var(--ind-green);
}

.outcome.review {
  color: var(--ind-amber);
}

/* ---------- 批次记录表 ---------- */
.records-panel .table-wrap {
  flex: 1 1 auto;
  min-height: 0;
  position: relative;
}

.records-panel :deep(.el-table__row) {
  cursor: pointer;
}

.records-panel :deep(.el-table__row.row-selected td.el-table__cell) {
  background: var(--accent-dim);
}

.cell-time {
  font-size: var(--fs-xs);
}

.table-footer {
  flex: none;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 7px 10px;
  border-top: 1px solid var(--border-glass);
}

.total-text {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  white-space: nowrap;
}

/* 双语表头（上下两行） */
.th-en {
  display: block;
  font-weight: 600;
  color: var(--text-secondary);
  font-size: var(--fs-xs);
  line-height: 1.25;
}

.th-zh {
  display: block;
  color: var(--text-tertiary);
  font-size: 10px;
  font-weight: 400;
  line-height: 1.25;
}

/* ---------- 事件回放 ---------- */
.event-list {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.event-row {
  display: grid;
  grid-template-columns: 60px minmax(72px, auto) minmax(0, 1fr) auto;
  gap: 8px;
  align-items: center;
  padding: 5px 6px;
  border-bottom: 1px solid var(--border-glass);
  font-size: var(--fs-sm);
}

.event-row:hover {
  background: var(--bg-hover);
}

.ev-time {
  color: var(--text-tertiary);
  font-size: var(--fs-xs);
}

.ev-tag {
  justify-self: start;
  padding: 1px 7px;
  border-radius: 999px;
  background: var(--accent-dim);
  color: var(--accent-strong);
  font-size: 10px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 110px;
}

.ev-text {
  color: var(--text-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.ev-value {
  color: var(--text-primary);
  font-size: var(--fs-xs);
  white-space: nowrap;
}

.audit-link {
  flex: none;
  margin-top: 8px;
  padding-top: 8px;
  border-top: 1px solid var(--border-glass);
  color: var(--accent);
  font-size: var(--fs-sm);
  font-weight: 600;
  text-decoration: none;
  text-align: center;
}

.audit-link:hover {
  color: var(--accent-strong);
}

.audit-link .zh {
  color: var(--text-tertiary);
  font-weight: 400;
}

.empty-mini {
  padding: 24px 10px;
  color: var(--text-tertiary);
  text-align: center;
  font-size: var(--fs-sm);
}

/* ---------- 历史对比 ---------- */
.comp-wrap {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
}

.comp-row {
  display: grid;
  gap: 4px;
  align-items: center;
}

.comp-head {
  position: sticky;
  top: 0;
  background: var(--bg-inset);
  border-bottom: 1px solid var(--border-strong);
  z-index: 1;
}

.comp-head .comp-batch {
  font-size: var(--fs-xs);
  color: var(--accent-strong);
  font-weight: 700;
}

.comp-label {
  display: flex;
  flex-direction: column;
  justify-content: center;
  padding: 5px 4px;
  min-width: 0;
}

.lbl-en {
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.lbl-zh {
  font-size: 10px;
  color: var(--text-tertiary);
  white-space: nowrap;
}

.comp-cell {
  padding: 6px 4px;
  border-bottom: 1px solid var(--border-glass);
  text-align: right;
  font-family: var(--font-data);
  font-variant-numeric: tabular-nums;
  color: var(--text-primary);
  font-size: var(--fs-sm);
  white-space: nowrap;
}

.comp-head .comp-cell {
  border-bottom: none;
}

.comp-cell.best {
  color: var(--ind-green);
  font-weight: 700;
}

.comp-cell.worst {
  color: var(--ind-red);
  font-weight: 700;
}
</style>
