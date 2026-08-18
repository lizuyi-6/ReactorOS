<script setup lang="ts">
// 多序列趋势图（ECharts 暗色封装）。
// 用法：
// <TrendChart
//   :series="[{ name: 'Temp', unit: '°C', color: '#2f9bff', data: [[ts, v], ...] }]"
//   :y-axes="[{ name: '°C' }]"
//   height="100%"
// />
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import * as echarts from "echarts";

export interface TrendSeries {
  name: string;
  data: Array<[number, number | null]>;
  color?: string;
  unit?: string;
  yAxisIndex?: number;
  smooth?: boolean;
  dashed?: boolean;
}

const props = withDefaults(
  defineProps<{
    series: TrendSeries[];
    legend?: boolean;
    height?: string;
    markTime?: number | null;
  }>(),
  { legend: true, height: "100%", markTime: null }
);

const palette = ["#2f9bff", "#2fd47b", "#f5a623", "#ff5252", "#b068f0", "#38c8f2", "#ff8a65"];

const el = ref<HTMLDivElement | null>(null);
let chart: echarts.ECharts | null = null;
let resizeObserver: ResizeObserver | null = null;

function axisLabel(v: number): string {
  const d = new Date(v);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return hh + ":" + mm;
}

function render(): void {
  if (!chart) return;
  const yAxisNames = Array.from(new Set(props.series.map((s) => s.yAxisIndex ?? 0)));
  const option: echarts.EChartsOption = {
    animation: false,
    grid: { left: 44, right: yAxisNames.length > 1 ? 44 : 12, top: props.legend ? 30 : 12, bottom: 24 },
    legend: props.legend
      ? {
          top: 0,
          textStyle: { color: "#9db4cf", fontSize: 11 },
          icon: "roundRect",
          itemWidth: 12,
          itemHeight: 3
        }
      : undefined,
    tooltip: {
      trigger: "axis",
      backgroundColor: "#16283f",
      borderColor: "rgba(74,127,184,0.3)",
      textStyle: { color: "#e8f1fb", fontSize: 12 },
      valueFormatter: (v) => (v === null || v === undefined ? "—" : String(v))
    },
    xAxis: {
      type: "time",
      axisLine: { lineStyle: { color: "rgba(74,127,184,0.3)" } },
      axisLabel: { color: "#5f7a9c", fontSize: 10, formatter: axisLabel },
      splitLine: { show: false }
    },
    yAxis: yAxisNames.map((idx) => ({
      type: "value",
      position: idx === 0 ? "left" : "right",
      scale: true,
      axisLabel: { color: "#5f7a9c", fontSize: 10 },
      splitLine: idx === 0 ? { lineStyle: { color: "rgba(74,127,184,0.12)" } } : { show: false },
      axisLine: { show: false }
    })) as echarts.EChartsOption["yAxis"],
    series: props.series.map((s, i) => ({
      name: s.name,
      type: "line",
      data: s.data,
      yAxisIndex: s.yAxisIndex ?? 0,
      smooth: s.smooth ?? true,
      symbol: "none",
      connectNulls: true,
      lineStyle: {
        width: 1.6,
        color: s.color ?? palette[i % palette.length],
        type: s.dashed ? "dashed" : "solid"
      },
      markLine:
        props.markTime && (s.yAxisIndex ?? 0) === 0 && i === 0
          ? {
              symbol: "none",
              label: { color: "#9db4cf", fontSize: 10, formatter: axisLabel(props.markTime) },
              lineStyle: { color: "#9db4cf", type: "dashed" },
              data: [{ xAxis: props.markTime }]
            }
          : undefined
    }))
  };
  chart.setOption(option, { notMerge: true });
}

onMounted(() => {
  if (!el.value) return;
  chart = echarts.init(el.value);
  render();
  resizeObserver = new ResizeObserver(() => chart?.resize());
  resizeObserver.observe(el.value);
});
watch(() => [props.series, props.markTime], render, { deep: true });
onBeforeUnmount(() => {
  resizeObserver?.disconnect();
  chart?.dispose();
  chart = null;
});
</script>

<template>
  <div ref="el" :style="{ width: '100%', height }"></div>
</template>
