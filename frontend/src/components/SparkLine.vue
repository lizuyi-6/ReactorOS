<script setup lang="ts">
// 迷你趋势线（无坐标轴），用于参数卡底部。
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import echarts from "../lib/echarts";
import type { EChartsType } from "../lib/echarts";

const props = withDefaults(
  defineProps<{
    points: number[];
    color?: string;
    /** 像素高度；传 0 表示由父布局（flex）决定，随卡片剩余空间伸缩 */
    height?: number;
  }>(),
  { color: "#2f9bff", height: 30 }
);

const el = ref<HTMLDivElement | null>(null);
let chart: EChartsType | null = null;
let resizeObserver: ResizeObserver | null = null;
let renderPending = false;
let needsVisibleRender = false;

function render(): void {
  if (!chart) return;
  if (document.hidden) {
    needsVisibleRender = true;
    return;
  }
  needsVisibleRender = false;
  chart.setOption({
    xAxis: { data: props.points.map((_, index) => index) },
    series: [
      {
        id: "sparkline",
        data: props.points,
        lineStyle: { width: 1.6, color: props.color },
        areaStyle: {
          color: {
            type: "linear", x: 0, y: 0, x2: 0, y2: 1,
            colorStops: [
              { offset: 0, color: props.color + "44" },
              { offset: 1, color: props.color + "00" }
            ]
          }
        }
      }
    ]
  });
}

function initializeChart(): void {
  if (!chart && el.value && el.value.clientWidth > 0 && el.value.clientHeight > 0) {
    chart = echarts.init(el.value);
    chart.setOption({
      grid: { left: 0, right: 0, top: 2, bottom: 2 },
      xAxis: { type: "category", show: false, data: [] },
      yAxis: { type: "value", show: false, min: "dataMin", max: "dataMax" },
      series: [
        {
          id: "sparkline",
          type: "line",
          smooth: true,
          symbol: "none"
        }
      ],
      animation: false
    });
    render();
  }
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
    initializeChart();
    chart?.resize();
    queueRender();
  }
}

onMounted(() => {
  if (!el.value) return;
  // 卡片偏矮时容器高度会被 flex 压到 0：等有实际尺寸再初始化。
  initializeChart();
  resizeObserver = new ResizeObserver(() => {
    initializeChart();
    if (!document.hidden) chart?.resize();
  });
  resizeObserver.observe(el.value);
  document.addEventListener("visibilitychange", handleVisibilityChange);
});
watch(() => props.points, queueRender);
watch(() => props.color, queueRender);
onBeforeUnmount(() => {
  document.removeEventListener("visibilitychange", handleVisibilityChange);
  resizeObserver?.disconnect();
  chart?.dispose();
  chart = null;
});
</script>

<template>
  <div ref="el" class="sparkline" :style="height > 0 ? { height: height + 'px' } : undefined"></div>
</template>

<style scoped>
.sparkline { width: 100%; }
</style>
