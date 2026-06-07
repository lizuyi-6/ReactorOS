<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import * as echarts from "echarts/core";
import { GridComponent, LegendComponent, TooltipComponent } from "echarts/components";
import { LineChart } from "echarts/charts";
import { CanvasRenderer } from "echarts/renderers";
import type { EChartsType } from "echarts/core";
import { usePlantStore } from "../stores/plant";
import { fixed, latestSample, numberAt, recentSamples, textAt } from "./view-utils";
import type { ApiRecord } from "../stores/plant";

echarts.use([GridComponent, LegendComponent, TooltipComponent, LineChart, CanvasRenderer]);

const store = usePlantStore();
const chartEl = ref<HTMLDivElement | null>(null);
let chart: EChartsType | null = null;

const sample = computed(() => latestSample(store.live));
const samples = computed(() => recentSamples(store.live));
const alarms = computed(() => (Array.isArray(store.live?.alarms) ? (store.live!.alarms as Record<string, unknown>[]) : []));
const metrics = computed(() => [
  { label: "Temperature", zh: "温度", value: fixed(numberAt(sample.value, "temperature_c"), 1, " C") },
  { label: "Pressure", zh: "压力", value: fixed(numberAt(sample.value, "pressure_kpa"), 1, " kPa") },
  { label: "Stirrer", zh: "搅拌", value: fixed(numberAt(sample.value, "stirrer_rpm"), 0, " rpm") },
  { label: "pH", zh: "酸碱度", value: fixed(numberAt(sample.value, "ph"), 2) }
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
  chart.setOption({
    animation: false,
    tooltip: { trigger: "axis" },
    legend: { textStyle: { color: "#cbd5e1" } },
    grid: { left: 42, right: 20, top: 36, bottom: 28 },
    xAxis: {
      type: "category",
      data: rows.map((row) => textAt(row, "created_at", "")),
      axisLabel: { color: "#94a3b8", hideOverlap: true },
      axisLine: { lineStyle: { color: "#334155" } }
    },
    yAxis: {
      type: "value",
      axisLabel: { color: "#94a3b8" },
      splitLine: { lineStyle: { color: "#1f2937" } }
    },
    series: [
      {
        name: store.tr("温度 C", "Temperature C"),
        type: "line",
        smooth: true,
        data: rows.map((row) => numberAt(row, "temperature_c")),
        connectNulls: true
      },
      {
        name: store.tr("压力 kPa", "Pressure kPa"),
        type: "line",
        smooth: true,
        data: rows.map((row) => numberAt(row, "pressure_kpa")),
        connectNulls: true
      }
    ]
  });
}

watch(samples, () => void nextTick(drawChart), { deep: true });
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
  <section class="view-stack">
    <div class="view-heading">
      <div>
        <p class="eyebrow">Vue + ECharts</p>
        <h1>{{ store.tr("实时监控", "Realtime Monitor") }}</h1>
        <span>{{ store.tr("实时传感器、报警和样本曲线", "Live sensors, alarms, and sample trends") }}</span>
      </div>
      <div class="heading-actions">
        <el-tag :type="sample ? 'success' : 'warning'">
          {{ sample ? store.tr("采集链路在线", "Pipeline online") : store.tr("等待样本", "Waiting for samples") }}
        </el-tag>
        <el-button size="small" @click="store.refreshLive()">{{ store.tr("加载实时数据", "Load live data") }}</el-button>
      </div>
    </div>

    <div class="metric-grid">
      <article v-for="metric in metrics" :key="metric.label" class="metric">
        <span>{{ store.tr(metric.zh, metric.label) }}</span>
        <strong>{{ metric.value }}</strong>
        <small>{{ store.tr("当前读数", "Current reading") }}</small>
      </article>
    </div>

    <section class="panel">
      <div class="panel-title">
        <h2>{{ store.tr("实时趋势", "Live Trend") }}</h2>
        <span>{{ store.tr(`${samples.length} 条样本`, `${samples.length} samples`) }}</span>
      </div>
      <div ref="chartEl" class="chart"></div>
    </section>

    <section class="panel">
      <div class="panel-title">
        <h2>{{ store.tr("报警中心", "Alarm Center") }}</h2>
        <el-tag :type="alarms.length > 0 ? 'warning' : 'success'">
          {{ store.tr(`${alarms.length} 条`, `${alarms.length} active`) }}
        </el-tag>
      </div>
      <el-table v-if="alarms.length > 0" :data="alarms" class="data-table" size="small">
        <el-table-column :label="store.tr('级别', 'Level')" width="100">
          <template #default="{ row }">
            <el-tag :type="alarmTagType(row)" size="small">
              {{ alarmLevelText(row) }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="store.tr('类型', 'Type')" min-width="180">
          <template #default="{ row }">{{ alarmType(row) }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('说明', 'Message')" min-width="240">
          <template #default="{ row }">{{ alarmText(row, "message") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('当前/限值', 'Current / Limit')" width="140">
          <template #default="{ row }">{{ alarmValue(row) }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('建议', 'Suggestion')" min-width="220">
          <template #default="{ row }">{{ alarmText(row, "suggestion") }}</template>
        </el-table-column>
      </el-table>
      <p v-else class="muted">{{ store.tr("当前无报警。", "No active alarms.") }}</p>
    </section>
  </section>
</template>
