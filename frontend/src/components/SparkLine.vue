<script setup lang="ts">
// 迷你趋势线（无坐标轴），用于参数卡底部。
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import * as echarts from "echarts";

const props = withDefaults(
  defineProps<{
    points: number[];
    color?: string;
    height?: number;
  }>(),
  { color: "#2f9bff", height: 30 }
);

const el = ref<HTMLDivElement | null>(null);
let chart: echarts.ECharts | null = null;
let resizeObserver: ResizeObserver | null = null;

function render(): void {
  if (!chart) return;
  chart.setOption({
    grid: { left: 0, right: 0, top: 2, bottom: 2 },
    xAxis: { type: "category", show: false, data: props.points.map((_, i) => i) },
    yAxis: { type: "value", show: false, min: "dataMin", max: "dataMax" },
    series: [
      {
        type: "line",
        data: props.points,
        smooth: true,
        symbol: "none",
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
    ],
    animation: false
  });
}

onMounted(() => {
  if (!el.value) return;
  chart = echarts.init(el.value);
  render();
  resizeObserver = new ResizeObserver(() => chart?.resize());
  resizeObserver.observe(el.value);
});
watch(() => [props.points, props.color], render, { deep: true });
onBeforeUnmount(() => {
  resizeObserver?.disconnect();
  chart?.dispose();
  chart = null;
});
</script>

<template>
  <div ref="el" class="sparkline" :style="{ height: height + 'px' }"></div>
</template>

<style scoped>
.sparkline { width: 100%; }
</style>
