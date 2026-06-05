<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import * as echarts from "echarts/core";
import { GridComponent, LegendComponent, TooltipComponent } from "echarts/components";
import { LineChart } from "echarts/charts";
import { CanvasRenderer } from "echarts/renderers";
import type { EChartsType } from "echarts/core";
import { usePlantStore } from "../stores/plant";
import { fixed, latestSample, numberAt, recentSamples, textAt } from "./view-utils";

echarts.use([GridComponent, LegendComponent, TooltipComponent, LineChart, CanvasRenderer]);

const store = usePlantStore();
const chartEl = ref<HTMLDivElement | null>(null);
let chart: EChartsType | null = null;

const sample = computed(() => latestSample(store.live));
const samples = computed(() => recentSamples(store.live));
const metrics = computed(() => [
  { label: "Temperature", zh: "温度", value: fixed(numberAt(sample.value, "temperature_c"), 1, " C") },
  { label: "Pressure", zh: "压力", value: fixed(numberAt(sample.value, "pressure_kpa"), 1, " kPa") },
  { label: "Stirrer", zh: "搅拌", value: fixed(numberAt(sample.value, "stirrer_rpm"), 0, " rpm") },
  { label: "pH", zh: "酸碱度", value: fixed(numberAt(sample.value, "ph"), 2) }
]);

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
  </section>
</template>
