<script setup lang="ts">
// 多序列趋势图（ECharts 暗色封装）。
// 用法：
// <TrendChart
//   :series="[{ name: 'Temp', unit: '°C', color: '#2f9bff', data: [[ts, v], ...] }]"
//   :y-axes="[{ name: '°C' }]"
//   height="100%"
// />
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import echarts from "../lib/echarts";
import type { EChartsType } from "../lib/echarts";

export interface TrendSeries {
  /** 跨更新保持稳定，用于 ECharts 按序列局部合并；未提供时回退到稳定索引。 */
  id?: string;
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
let chart: EChartsType | null = null;
let resizeObserver: ResizeObserver | null = null;
let renderPending = false;
let needsVisibleRender = false;
let initialized = false;

function axisLabel(v: number): string {
  const d = new Date(v);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return hh + ":" + mm;
}

function axisLabelSec(v: number): string {
  const d = new Date(v);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

function axisLabelDay(v: number): string {
  const d = new Date(v);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

// X 轴标签按数据时间跨度选粒度：分钟级跨度用 HH:mm（同分钟刻度会重复），
// 秒级跨度补秒，跨天补日期，避免一排相同的 "16:49"。
function pickAxisLabel(): (v: number) => string {
  let lo = Infinity;
  let hi = -Infinity;
  for (const s of props.series) {
    for (const [t] of s.data) {
      if (typeof t === "number" && Number.isFinite(t)) {
        if (t < lo) lo = t;
        if (t > hi) hi = t;
      }
    }
  }
  if (!(lo <= hi)) return axisLabel;
  const span = hi - lo;
  if (span < 10 * 60_000) return axisLabelSec;
  if (span >= 36 * 3_600_000) return axisLabelDay;
  return axisLabel;
}

// Y 轴噪声护栏：恒定信号（如搅拌 300rpm）在 scale:true 下轴会贴死
// dataMin/dataMax，把亚单位噪声放大成剧烈波动；数据跨度小于量级 2% 时
// 显式把轴撑到该最小跨度。
function paddedAxisExtent(idx: number): { min: number; max: number } | undefined {
  let lo = Infinity;
  let hi = -Infinity;
  for (const s of props.series) {
    if ((s.yAxisIndex ?? 0) !== idx) continue;
    for (const [, v] of s.data) {
      if (typeof v === "number" && Number.isFinite(v)) {
        if (v < lo) lo = v;
        if (v > hi) hi = v;
      }
    }
  }
  if (!(lo <= hi)) return undefined;
  const center = (lo + hi) / 2;
  const minSpan = Math.max(Math.abs(center) * 0.02, 1e-3);
  if (hi - lo >= minSpan) return undefined;
  // 轴端点对齐到量级的整步长，避免 "297.015" 这类毛刺刻度
  const step = Math.pow(10, Math.floor(Math.log10(minSpan)));
  const prec = Math.max(0, -Math.floor(Math.log10(step)));
  const align = (v: number, dir: 1 | -1) =>
    Number(((dir < 0 ? Math.floor(v / step) : Math.ceil(v / step)) * step).toFixed(prec));
  return { min: align(center - minSpan / 2, -1), max: align(center + minSpan / 2, 1) };
}

function seriesId(series: TrendSeries, index: number): string {
  return series.id ?? `trend-${index}`;
}

function render(): void {
  if (!chart) return;
  if (document.hidden) {
    needsVisibleRender = true;
    return;
  }
  needsVisibleRender = false;
  const yAxisNames = Array.from(new Set(props.series.map((s) => s.yAxisIndex ?? 0)));
  const option: echarts.EChartsCoreOption = {
    animation: false,
    grid: { left: 44, right: yAxisNames.length > 1 ? 44 : 12, top: props.legend ? 30 : 12, bottom: 24 },
    legend: props.legend
      ? {
          show: true,
          top: 0,
          data: props.series.map((series) => series.name),
          textStyle: { color: "#9db4cf", fontSize: 11 },
          icon: "roundRect",
          itemWidth: 12,
          itemHeight: 3
        }
      : { show: false },
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
      axisLabel: { color: "#5f7a9c", fontSize: 10, formatter: pickAxisLabel(), hideOverlap: true },
      splitLine: { show: false }
    },
    yAxis: yAxisNames.map((idx) => ({
      id: `trend-axis-${idx}`,
      type: "value",
      position: idx === 0 ? "left" : "right",
      scale: true,
      axisLabel: { color: "#5f7a9c", fontSize: 10, hideOverlap: true },
      splitLine: idx === 0 ? { lineStyle: { color: "rgba(74,127,184,0.12)" } } : { show: false },
      axisLine: { show: false },
      ...paddedAxisExtent(idx)
    })) as echarts.EChartsCoreOption["yAxis"],
    series: props.series.map((s, i) => ({
      id: seriesId(s, i),
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
          : { data: [] }
    }))
  };
  chart.setOption(option, {
    notMerge: !initialized,
    replaceMerge: initialized ? ["series"] : undefined
  });
  initialized = true;
}

function queueRender(): void {
  if (renderPending) return;
  renderPending = true;
  queueMicrotask(() => {
    renderPending = false;
    render();
  });
}

function handleVisibilityChange(): void {
  if (!document.hidden && needsVisibleRender) {
    chart?.resize();
    queueRender();
  }
}

onMounted(() => {
  if (!el.value) return;
  chart = echarts.init(el.value);
  render();
  resizeObserver = new ResizeObserver(() => {
    if (!document.hidden) chart?.resize();
  });
  resizeObserver.observe(el.value);
  document.addEventListener("visibilitychange", handleVisibilityChange);
});
watch(() => props.series, queueRender);
watch(() => props.markTime, queueRender);
watch(() => props.legend, queueRender);
onBeforeUnmount(() => {
  document.removeEventListener("visibilitychange", handleVisibilityChange);
  resizeObserver?.disconnect();
  chart?.dispose();
  chart = null;
  initialized = false;
});
</script>

<template>
  <div ref="el" :style="{ width: '100%', height }"></div>
</template>
