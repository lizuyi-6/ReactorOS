<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import * as echarts from "echarts/core";
import { GridComponent, LegendComponent, TooltipComponent } from "echarts/components";
import { LineChart } from "echarts/charts";
import { CanvasRenderer } from "echarts/renderers";
import type { EChartsType } from "echarts/core";
import { usePlantStore } from "../stores/plant";
import { arrayAt, fixed, latestSample, numberAt, objectAt, recentSamples, textAt } from "./view-utils";
import type { ApiRecord } from "../stores/plant";

echarts.use([GridComponent, LegendComponent, TooltipComponent, LineChart, CanvasRenderer]);

const store = usePlantStore();
const chartEl = ref<HTMLDivElement | null>(null);
let chart: EChartsType | null = null;

const sample = computed(() => latestSample(store.live));
const samples = computed(() => recentSamples(store.live));
const alarms = computed(() => (Array.isArray(store.live?.alarms) ? (store.live!.alarms as Record<string, unknown>[]) : []));
const runtime = computed(() => objectAt(store.live, "runtime") ?? store.runtimeFallback);
const targets = computed(() => objectAt(runtime.value, "targets"));
const activeBatchId = computed(() => numberAt(runtime.value, "active_batch_id"));
const recommendation = computed(() => store.recommendation ?? objectAt(store.live, "latest_recommendation"));
const batchOutcomes = computed(() => arrayAt<ApiRecord>(store.batches, "outcomes"));
const auditEvents = computed(() => arrayAt<ApiRecord>(store.audit, "events"));
const metrics = computed(() => [
  { label: "Temperature", zh: "温度", value: fixed(numberAt(sample.value, "temperature_c"), 1, " C") },
  { label: "Pressure", zh: "压力", value: fixed(numberAt(sample.value, "pressure_mpa"), 2, " MPa") },
  { label: "Stirrer", zh: "搅拌", value: fixed(numberAt(sample.value, "stirrer_rpm"), 0, " rpm") },
  { label: "pH", zh: "酸碱度", value: fixed(numberAt(sample.value, "ph"), 2) }
]);

function clampPercent(value: number | null, min: number, max: number): number {
  if (value === null || max <= min) return 6;
  return Math.min(100, Math.max(6, ((value - min) / (max - min)) * 100));
}

const sensorRows = computed(() => [
  {
    code: "T",
    label: store.tr("反应釜温度", "Reactor temperature"),
    value: fixed(numberAt(sample.value, "temperature_c"), 1),
    unit: "C",
    percent: clampPercent(numberAt(sample.value, "temperature_c"), 0, 160)
  },
  {
    code: "P",
    label: store.tr("釜内压力", "Vessel pressure"),
    value: fixed(numberAt(sample.value, "pressure_mpa"), 2),
    unit: "MPa",
    percent: clampPercent(numberAt(sample.value, "pressure_mpa"), 0, 2)
  },
  {
    code: "R",
    label: store.tr("搅拌转速", "Stirrer speed"),
    value: fixed(numberAt(sample.value, "stirrer_rpm"), 0),
    unit: "RPM",
    percent: clampPercent(numberAt(sample.value, "stirrer_rpm"), 0, 1200)
  },
  {
    code: "C",
    label: store.tr("产物浓度", "Product concentration"),
    value: fixed(numberAt(sample.value, "product_concentration_percent"), 1),
    unit: "%",
    percent: clampPercent(numberAt(sample.value, "product_concentration_percent"), 0, 100)
  },
  {
    code: "pH",
    label: store.tr("pH 值", "pH value"),
    value: fixed(numberAt(sample.value, "ph"), 2),
    unit: "",
    percent: clampPercent(numberAt(sample.value, "ph"), 0, 14)
  }
]);

const parameterRows = computed(() => [
  { label: store.tr("目标温度", "Target temperature"), value: textAt(targets.value, "temperature_c", "60"), unit: "C" },
  { label: store.tr("加热时长", "Heating time"), value: textAt(targets.value, "heat_time_s", "120"), unit: "s" },
  { label: store.tr("搅拌时长", "Stirring time"), value: textAt(targets.value, "hold_time_s", "60"), unit: "s" },
  { label: store.tr("搅拌转速", "Stirrer speed"), value: textAt(targets.value, "stirrer_rpm", "300"), unit: "rpm" },
  { label: store.tr("摇摆速度", "Shake speed"), value: textAt(targets.value, "shake_speed_cpm", "30"), unit: "cpm" },
  { label: store.tr("冷却方式", "Cooling mode"), value: textAt(targets.value, "cooling_mode", store.tr("自然冷却", "Natural")), unit: "" }
]);

const aiScore = computed(() => numberAt(recommendation.value, "expected_score"));
const aiProgress = computed(() => clampPercent(aiScore.value, 0, 100));
const recommendationRows = computed(() => [
  {
    label: store.tr("目标温度", "Target temperature"),
    value: fixed(numberAt(recommendation.value, "target_temperature_c"), 1),
    unit: "C"
  },
  {
    label: store.tr("加热时长", "Heating time"),
    value: fixed(numberAt(recommendation.value, "heating_minutes"), 1),
    unit: "min"
  },
  {
    label: store.tr("搅拌时长", "Stirring time"),
    value: fixed(numberAt(recommendation.value, "stirring_minutes"), 1),
    unit: "min"
  },
  {
    label: store.tr("预期产率", "Expected yield"),
    value: aiScore.value === null ? store.tr("待学习", "Learning") : fixed(aiScore.value, 1),
    unit: aiScore.value === null ? "" : "%"
  }
]);
const historyRows = computed(() => batchOutcomes.value.slice(-3).reverse());
const eventRows = computed(() => auditEvents.value.slice(0, 6));
const latestAlarm = computed(() => alarms.value[0] ?? null);
const heroReadouts = computed(() => {
  const meta = sampleFresh.value ? store.tr("数据新鲜", "FRESH") : freshnessText.value;
  return [
    {
      key: "temperature",
      label: "TEMPERATURE",
      value: fixed(numberAt(sample.value, "temperature_c"), 1),
      unit: "degC",
      icon: "♨",
      tone: "amber",
      meta
    },
    {
      key: "pressure",
      label: "PRESSURE",
      value: fixed(numberAt(sample.value, "pressure_mpa"), 2),
      unit: "MPa",
      icon: "↕",
      tone: "blue",
      meta
    },
    {
      key: "rpm",
      label: "STIRRER",
      value: fixed(numberAt(sample.value, "stirrer_rpm"), 0),
      unit: "RPM",
      icon: "◎",
      tone: "green",
      meta: latestAlarm.value ? alarmType(latestAlarm.value) : store.tr("电机就绪", "MOTOR READY")
    },
    {
      key: "flow",
      label: "FLOW",
      value: fixed(flowRate.value, 2),
      unit: "L/min",
      icon: "≈",
      tone: "cyan",
      meta
    }
  ];
});
const currentBatchRows = computed(() => {
  // Phase + progress are real backend fields surfaced via /api/v1/.../realtime
  // (phase_for / progress_for). Active process name comes from runtime when a
  // process is actually running; no fabricated elapsed/remaining — those backend
  // fields do not exist, so we show only what the backend actually carries.
  const activeProcessName = textAt(runtime.value, "active_process_name", "");
  const rows: { label: string; value: string }[] = [];
  if (activeProcessName) rows.push({ label: store.tr("当前工艺", "Active process"), value: activeProcessName });
  if (activeBatchId.value !== null) rows.push({ label: store.tr("活动批次", "Active batch"), value: String(activeBatchId.value) });
  return rows;
});
const currentBatchPanelRows = computed(() => {
  if (currentBatchRows.value.length > 0) return currentBatchRows.value;
  const latestEvent = eventRows.value[0];
  return [
    { label: store.tr("活动批次", "Active batch"), value: store.tr("无", "None") },
    { label: store.tr("样本窗口", "Sample window"), value: String(samples.value.length) },
    { label: store.tr("报警数", "Alarm count"), value: String(alarms.value.length) },
    { label: store.tr("AI 评分", "AI score"), value: aiScore.value === null ? "--" : fixed(aiScore.value, 1, "%") },
    { label: store.tr("最新事件", "Latest event"), value: textAt(latestEvent, "event_type", "--") },
    { label: store.tr("产物记录", "Outcomes"), value: String(batchOutcomes.value.length) }
  ];
});
// Real per-sample freshness from backend device_status (last_seen_age_ms /
// stale_after_ms / status). No more "PIPELINE ONLINE" boolean masquerade — a
// stale-but-200 sample now reads "STALE" honestly.
const deviceStatusItem = computed(() => {
  const ds = store.deviceStatus;
  const devices = arrayAt<ApiRecord>(ds, "devices");
  return devices[0] ?? null;
});
const sampleAgeMs = computed(() => numberAt(deviceStatusItem.value, "last_seen_age_ms"));
const staleAfterMs = computed(() => numberAt(deviceStatusItem.value, "stale_after_ms"));
const deviceStatusCode = computed(() => textAt(deviceStatusItem.value, "status", "").toLowerCase());
const sampleFresh = computed(() => {
  if (store.liveStatus !== "fresh") return false;
  const status = deviceStatusCode.value;
  if (status === "offline" || status === "stale" || status === "error") return false;
  const age = sampleAgeMs.value;
  const limit = staleAfterMs.value;
  if (age !== null && limit !== null && age > limit) return false;
  return true;
});
const freshnessText = computed(() => {
  if (store.liveStatus !== "fresh") return store.tr("无数据", "NO DATA");
  const status = deviceStatusCode.value;
  if (status === "offline") return store.tr("离线", "OFFLINE");
  if (status === "stale") return store.tr("数据过期", "STALE");
  if (status === "error") return store.tr("设备异常", "ERROR");
  const age = sampleAgeMs.value;
  const limit = staleAfterMs.value;
  if (age !== null && limit !== null && age > limit) return store.tr("数据过期", "STALE");
  return store.tr("数据新鲜", "FRESH");
});
const freshnessTone = computed<"good" | "warn" | "bad">(() => {
  if (sampleFresh.value) return "good";
  const status = deviceStatusCode.value;
  if (status === "offline" || status === "error") return "bad";
  return "warn";
});
const ageSeconds = computed(() => (sampleAgeMs.value !== null ? Math.round(sampleAgeMs.value / 1000) : null));
// control_loop_terminated: backend fail-safe (state.rs:135) — supervisor task
// died; the only recovery is a process restart. Surface it as an unmistakable
// banner so the operator does NOT believe the system is healthy.
const controlLoopTerminated = computed(
  () => textAt(runtime.value, "control_loop_terminated", "false") === "true"
);
const lastSensorError = computed(() => textAt(runtime.value, "last_sensor_error", ""));
// flow_rate_l_min: real backend sample field (SensorSnapshot), previously omitted.
const flowRate = computed(() => numberAt(sample.value, "flow_rate_l_min"));

const detectorRows = computed(() => [
  { label: "TEMP", value: fixed(numberAt(sample.value, "temperature_c"), 1), unit: "degC", range: "Range 50.00-240.00 degC", percent: clampPercent(numberAt(sample.value, "temperature_c"), 0, 240) },
  { label: "PRESS", value: fixed(numberAt(sample.value, "pressure_mpa"), 2), unit: "MPa", range: "Range 0.05-0.90 MPa", percent: clampPercent(numberAt(sample.value, "pressure_mpa"), 0, 1.2) },
  { label: "RPM", value: fixed(numberAt(sample.value, "stirrer_rpm"), 0), unit: "RPM", range: "Range 0-1200 RPM", percent: clampPercent(numberAt(sample.value, "stirrer_rpm"), 0, 1200) }
]);

type Translation = { zh: string; en: string };

const alarmLevelLabels: Record<string, Translation> = {
  high: { zh: "高", en: "High" },
  medium: { zh: "中", en: "Medium" },
  warning: { zh: "预警", en: "Warning" },
  low: { zh: "低", en: "Low" }
};

const alarmTypeLabels: Record<string, Translation> = {
  emergency_stop: { zh: "急停", en: "Emergency stop" },
  communication_error: { zh: "通信错误", en: "Communication error" },
  sensor_error: { zh: "传感器错误", en: "Sensor error" },
  temperature_limit: { zh: "温度越限", en: "Temperature limit" },
  pressure_limit: { zh: "压力越限", en: "Pressure limit" },
  stirrer_limit: { zh: "搅拌越限", en: "Stirrer limit" },
  shake_speed_limit: { zh: "摇摆速度越限", en: "Shake speed limit" },
  tilt_angle_limit: { zh: "倾角越限", en: "Tilt angle limit" },
  flow_rate_limit: { zh: "流量越限", en: "Flow rate limit" },
  product_concentration_limit: { zh: "产物浓度越限", en: "Product concentration limit" },
  ph_limit: { zh: "pH 越限", en: "pH limit" }
};

const alarmMessageLabels: Record<string, Translation> = {
  "manual emergency stop is active": { zh: "人工急停已触发", en: "Manual emergency stop is active" },
  "confirm field safety before resetting emergency stop": {
    zh: "复位急停前确认现场安全",
    en: "Confirm field safety before resetting emergency stop"
  }
};

const sensorAlarmLabels: Record<string, Translation> = {
  temperature_limit: { zh: "反应温度", en: "Reactor temperature" },
  pressure_limit: { zh: "反应压力", en: "Reactor pressure" },
  stirrer_limit: { zh: "搅拌转速", en: "Stirrer speed" },
  shake_speed_limit: { zh: "摇摆速度", en: "Shake speed" },
  tilt_angle_limit: { zh: "倾角", en: "Tilt angle" },
  flow_rate_limit: { zh: "冷却流量", en: "Coolant flow" },
  product_concentration_limit: { zh: "产物浓度", en: "Product concentration" },
  ph_limit: { zh: "pH", en: "pH" }
};

const alarmSuggestionLabels: Record<string, Translation> = {
  "Stop heating, keep stirring if safe, and check cooling loop and temperature probe.": {
    zh: "停止加热，在安全条件下保持搅拌，并检查冷却回路和温度探头。",
    en: "Stop heating, keep stirring if safe, and check cooling loop and temperature probe."
  },
  "Vent through the validated relief path and inspect pressure sensor and exhaust line.": {
    zh: "通过已验证的泄压路径泄压，并检查压力传感器和排气管路。",
    en: "Vent through the validated relief path and inspect pressure sensor and exhaust line."
  },
  "Reduce stirrer target and inspect mechanical coupling.": {
    zh: "降低搅拌目标并检查机械联轴器。",
    en: "Reduce stirrer target and inspect mechanical coupling."
  },
  "Reduce shake speed and verify vessel fixation.": {
    zh: "降低摇摆速度并确认反应容器固定可靠。",
    en: "Reduce shake speed and verify vessel fixation."
  },
  "Stop or reduce shake motion and inspect the vessel clamp, stepper linkage, and tilt sensor mounting.": {
    zh: "停止或降低摇摆动作，并检查容器夹具、步进电机连杆和倾角传感器安装。",
    en: "Stop or reduce shake motion and inspect the vessel clamp, stepper linkage, and tilt sensor mounting."
  },
  "Check coolant pump, valve position, and blocked tubing.": {
    zh: "检查冷却泵、阀门位置和管路堵塞。",
    en: "Check coolant pump, valve position, and blocked tubing."
  },
  "Confirm online concentration probe calibration before using the value for optimization.": {
    zh: "用于优化前先确认在线浓度探头校准状态。",
    en: "Confirm online concentration probe calibration before using the value for optimization."
  },
  "Confirm pH probe calibration and pause automatic optimization if the chemistry is outside the validated range.": {
    zh: "确认 pH 探头校准；若反应体系超出验证范围，暂停自动优化。",
    en: "Confirm pH probe calibration and pause automatic optimization if the chemistry is outside the validated range."
  }
};

function rawAt(row: unknown, key: string): unknown {
  if (!row || typeof row !== "object") return undefined;
  return (row as ApiRecord)[key];
}

function translatedFrom(map: Record<string, Translation>, value: unknown): string {
  const text = value === null || value === undefined || value === "" ? "--" : String(value);
  const label = map[text];
  return label ? store.tr(label.zh, label.en) : text;
}

function alarmLevel(row: ApiRecord): string {
  const level = rawAt(row, "level") ?? rawAt(row, "severity");
  return level === null || level === undefined || level === "" ? "--" : String(level);
}

function alarmType(row: ApiRecord): string {
  const type = rawAt(row, "type") ?? rawAt(row, "code");
  return translatedFrom(alarmTypeLabels, type);
}

function alarmLevelText(row: ApiRecord): string {
  return translatedFrom(alarmLevelLabels, alarmLevel(row));
}

function alarmTagType(row: ApiRecord): "danger" | "warning" | "info" {
  const level = alarmLevel(row);
  if (level === "high") return "danger";
  if (level === "medium" || level === "warning") return "warning";
  return "info";
}

function alarmText(row: ApiRecord, key: string): string {
  const value = rawAt(row, key);
  if (key === "message") {
    const current = rawAt(row, "current_value");
    const limit = rawAt(row, "limit_value");
    const type = String(rawAt(row, "type") ?? "");
    const sensor = sensorAlarmLabels[type];
    if (sensor && current !== null && current !== undefined && limit !== null && limit !== undefined) {
      const base = store.tr(sensor.zh, sensor.en);
      const level = alarmLevel(row);
      const limitText = level === "high" ? store.tr("硬限值", "hard limit") : store.tr("正常范围", "normal range");
      return store.tr(
        `${base}越限：当前 ${current}，限值 ${limit}`,
        `${base} outside ${limitText}: current ${current}, limit ${limit}`
      );
    }
  }
  if (key === "suggestion") return translatedFrom(alarmSuggestionLabels, value);
  return translatedFrom(alarmMessageLabels, value);
}

function alarmValue(row: ApiRecord): string {
  const current = rawAt(row, "current_value") ?? rawAt(row, "value");
  const limit = rawAt(row, "limit_value");
  const currentText = current === null || current === undefined || current === "" ? "--" : String(current);
  if (limit === null || limit === undefined || limit === "") return currentText;
  return `${currentText} / ${limit}`;
}

function drawChart(): void {
  if (!chartEl.value) return;
  if (!chart) chart = echarts.init(chartEl.value);
  const rows = samples.value;
  const targetTemperature = numberAt(targets.value, "temperature_c");
  const aiTemperature = numberAt(recommendation.value, "target_temperature_c");
  chart.setOption({
    animation: false,
    color: ["#4cae9d", "#aab5bc", "#5b8def"],
    tooltip: {
      trigger: "axis",
      backgroundColor: "rgba(18, 23, 27, 0.96)",
      borderColor: "#3d4a53",
      borderWidth: 1,
      textStyle: { color: "#edf2f4" }
    },
    legend: { right: 8, top: 0, textStyle: { color: "#aab5bc" } },
    grid: { left: 42, right: 20, top: 42, bottom: 28 },
    xAxis: {
      type: "category",
      data: rows.map((row) => textAt(row, "created_at", "")),
      axisLabel: { color: "#78858e", hideOverlap: true },
      axisLine: { lineStyle: { color: "#344049" } }
    },
    yAxis: {
      type: "value",
      axisLabel: { color: "#78858e" },
      splitLine: { lineStyle: { color: "#273139" } }
    },
    series: [
      {
        name: store.tr("实测温度", "Measured temp"),
        type: "line",
        smooth: true,
        showSymbol: false,
        data: rows.map((row) => numberAt(row, "temperature_c")),
        connectNulls: true
      },
      {
        name: store.tr("目标温度", "Target temp"),
        type: "line",
        smooth: true,
        showSymbol: false,
        lineStyle: { type: "dashed", width: 2 },
        data: rows.map(() => targetTemperature),
        connectNulls: true,
        emphasis: { disabled: true }
      },
      {
        name: store.tr("AI 推荐曲线", "AI curve"),
        type: "line",
        smooth: true,
        showSymbol: false,
        lineStyle: { type: "dotted", width: 2 },
        data: rows.map(() => aiTemperature),
        connectNulls: true,
        emphasis: { disabled: true }
      }
    ]
  });
}

watch(samples, () => void nextTick(drawChart), { deep: true });
watch(targets, () => void nextTick(drawChart), { deep: true });
watch(recommendation, () => void nextTick(drawChart), { deep: true });
watch(() => store.language, () => void nextTick(drawChart));

onMounted(() => {
  drawChart();
  window.addEventListener("resize", drawChart);
});

onBeforeUnmount(() => {
  window.removeEventListener("resize", drawChart);
  chart?.dispose();
});
</script>

<template>
  <section class="view-stack monitor-workbench dark-origin-monitor">
    <!-- Fail-safe banner: control_loop_terminated (state.rs:135) means the
         supervisor task died and the ONLY recovery is a process restart.
         This must be unmissable — it overrides every "fresh/ok" readout below. -->
    <div v-if="controlLoopTerminated" class="fatal-status-banner">
      <span class="status-dot"></span>
      <strong>{{ store.tr("控制环监督已终止", "CONTROL LOOP SUPERVISOR TERMINATED") }}</strong>
      <span>{{ store.tr("自动控制已禁用，且只能通过重启进程恢复。API 复归/启动将被后端拒绝。", "Automatic control is disabled and can ONLY be cleared by a process restart. API reset/start will be rejected by the backend.") }}</span>
    </div>

    <div v-else-if="lastSensorError" class="sensor-fault-banner">
      <span class="status-dot"></span>
      <strong>{{ store.tr("传感器故障 (fail-closed)", "Sensor fault (fail-closed)") }}</strong>
      <span>{{ lastSensorError }}</span>
    </div>

    <!-- Real telemetry status strip: every value below comes from a backend
         field (device_status / runtime). No fabricated CPU/OEE/throughput. -->
    <div class="telemetry-status-strip">
      <article class="ts-cell" :class="freshnessTone">
        <span>{{ store.tr("数据新鲜度", "FRESHNESS") }}</span>
        <strong>{{ freshnessText }}</strong>
        <small v-if="ageSeconds !== null">{{ store.tr("采样于", "sampled") }} {{ ageSeconds }}s {{ store.tr("前", "ago") }}</small>
        <small v-else-if="staleAfterMs !== null">{{ store.tr("超时阈值", "stale after") }} {{ Math.round(staleAfterMs / 1000) }}s</small>
        <small v-else>{{ store.tr("无设备状态", "no device status") }}</small>
      </article>
      <article class="ts-cell">
        <span>{{ store.tr("设备状态", "DEVICE") }}</span>
        <strong>{{ deviceStatusCode ? deviceStatusCode.toUpperCase() : store.tr("未知", "UNKNOWN") }}</strong>
        <small>{{ store.liveStatus === "fresh" ? store.tr("链路可达", "link reachable") : store.tr("链路不可达", "link unreachable") }}</small>
      </article>
      <article class="ts-cell">
        <span>{{ store.tr("采样计数", "SAMPLES") }}</span>
        <strong>{{ samples.length }}</strong>
        <small>{{ store.tr("本窗口", "in window") }}</small>
      </article>
      <article class="ts-cell" v-if="activeBatchId !== null">
        <span>{{ store.tr("活动批次", "ACTIVE BATCH") }}</span>
        <strong>#{{ activeBatchId }}</strong>
        <small>{{ store.tr("运行中", "running") }}</small>
      </article>
    </div>

    <div class="dark-monitor-grid">
      <main class="dark-monitor-main">
        <div class="enterprise-component-grid">
          <section class="origin-panel process-line-panel">
            <div class="component-head">
              <strong>{{ store.tr("工艺线概览", "Process line") }}</strong>
              <span>ESP32 → Pi → HMI</span>
            </div>
            <div class="reactor-vessel">
              <div class="feed-label">FEED A/B/C</div>
              <div class="cond-label">COND. FLOW</div>
              <div class="vessel-body"><span></span></div>
              <strong>REACTOR V-01</strong>
            </div>
          </section>

          <section class="origin-panel detector-panel">
            <div class="component-head">
              <strong>{{ store.tr("关键传感器", "Core sensors") }}</strong>
              <span class="active-alarm">
                {{ alarms.length }} {{ store.tr("条报警", alarms.length === 1 ? "alarm" : "alarms") }}
              </span>
            </div>
            <div class="detector-list">
              <article v-for="row in detectorRows" :key="row.label">
                <div class="detector-title">
                  <span>{{ row.label }}</span>
                  <em :class="{ stale: !sampleFresh }">
                    {{ sampleFresh ? store.tr("在线", "LIVE") : freshnessText }}
                  </em>
                </div>
                <strong>{{ row.value }} <small>{{ row.unit }}</small></strong>
                <span>{{ row.range }}</span>
                <div class="detector-bar"><i :style="{ width: `${row.percent}%` }"></i></div>
              </article>
            </div>
          </section>

          <section class="origin-panel predictive-panel">
            <div class="component-head">
              <strong>{{ store.tr("数据与模型证据", "Data & model evidence") }}</strong>
              <span>{{ store.tr("仅显示后端实值", "Backend values only") }}</span>
            </div>
            <div class="predictive-columns">
              <div>
                <h3>{{ store.tr("采样窗口", "Sample window") }}</h3>
                <div class="signal-summary">
                  <strong>{{ samples.length }}</strong>
                  <span>{{ store.tr("条真实样本", "real samples") }}</span>
                </div>
                <small>{{ freshnessText }}</small>
              </div>
              <div>
                <h3>{{ store.tr("建议来源", "Recommendation source") }}</h3>
                <div class="signal-summary">
                  <strong>{{ textAt(recommendation, "provider", "--") }}</strong>
                  <span>{{ recommendation ? store.tr("已有建议", "available") : store.tr("暂无建议", "not available") }}</span>
                </div>
                <small>{{ store.tr("详情需进入 AI 页面复核", "Review details on the AI page") }}</small>
              </div>
            </div>
          </section>
        </div>

        <section class="origin-panel operator-control-panel">
          <div class="origin-panel-head">
            <h2>{{ store.tr("运行参数与温度趋势", "Parameters & temperature trend") }}</h2>
            <span>{{ freshnessText }}</span>
          </div>
          <div class="operator-fields">
            <label v-for="row in parameterRows" :key="row.label">
              <span>{{ row.label }}</span>
              <strong>{{ row.value }}</strong>
              <small>{{ row.unit }}</small>
            </label>
          </div>
          <div ref="chartEl" class="chart origin-chart compact"></div>
        </section>
      </main>

      <aside class="dark-right-stack">
        <section class="origin-panel ai-command-panel">
          <div class="ai-command-head">
            <h2>{{ store.tr("AI 参数建议", "AI recommendation") }}</h2>
            <span>{{ store.tr("模型", "Model") }}: {{ textAt(recommendation, "provider", "--") }}</span>
          </div>
          <div class="ai-command-body">
            <p class="ai-command-note">{{ textAt(recommendation, "rationale", store.tr("暂无 AI 建议", "No AI recommendation")) }}</p>
            <div class="ai-target-box">
              <div>
                <span>{{ store.tr("建议温度", "Suggested temp") }}</span>
                <strong>{{ fixed(numberAt(recommendation, "target_temperature_c"), 1) }} degC</strong>
              </div>
              <div>
                <span>{{ store.tr("建议转速", "Suggested RPM") }}</span>
                <strong>{{ fixed(numberAt(recommendation, "target_stirrer_rpm"), 0) }}</strong>
              </div>
            </div>
            <RouterLink class="apply-ai-button" to="/ai">
              {{ store.tr("进入 AI 页面复核建议", "Review recommendation") }}
            </RouterLink>
          </div>
        </section>

        <section class="origin-panel current-batch-panel">
          <div class="current-batch-head">
            <h2>{{ store.tr("当前批次", "Current batch") }}</h2>
            <span>ID: {{ textAt(runtime, "active_batch_id", "--") }}</span>
          </div>
          <div class="batch-started">{{ store.tr("状态来自后端实时数据", "Live backend state") }}</div>
          <dl>
            <template v-for="row in currentBatchPanelRows" :key="row.label">
              <dt>{{ row.label }}</dt>
              <dd>{{ row.value }}</dd>
            </template>
          </dl>
          <div class="batch-actions">
            <RouterLink to="/history">{{ store.tr("查看批次", "View batches") }}</RouterLink>
            <RouterLink to="/control">{{ store.tr("进入控制", "Open control") }}</RouterLink>
          </div>
        </section>
      </aside>
    </div>
  </section>
</template>
