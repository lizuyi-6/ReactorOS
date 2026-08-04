<template>
  <div class="monitor-hmi">
    <!-- 顶部全局状态栏 -->
    <div class="status-bar hmi-panel">
      <div class="status-item">
        <div class="status-icon-wrapper" :class="systemTone">
          <span class="status-light" :class="systemTone"></span>
        </div>
        <div class="status-info">
          <span class="data-label">{{ tr("系统状态", "System") }}</span>
          <span class="data-value" :class="systemTextClass">{{ modeLabel }}</span>
        </div>
      </div>
      <div class="status-divider"></div>
      <div class="status-item">
        <div class="status-info">
          <span class="data-label">{{ tr("当前批次", "Batch") }}</span>
          <span class="data-value mono">{{ activeBatchLabel }}</span>
        </div>
      </div>
      <div class="status-divider"></div>
      <div class="status-item">
        <div class="status-info">
          <span class="data-label">{{ tr("安全联锁", "Safety") }}</span>
          <span class="data-value" :class="safetyClass">{{ safetyLabel }}</span>
        </div>
      </div>
      <div class="status-divider"></div>
      <div class="status-item time">
        <div class="status-info">
          <span class="data-label">{{ tr("系统时间", "Time") }}</span>
          <span class="data-value mono">{{ currentTime }}</span>
        </div>
      </div>
    </div>

    <!-- 核心传感器矩阵：Bento Grid 风格，大字体 -->
    <div class="sensor-matrix">
      <div v-for="card in sensorCards" :key="card.key" class="sensor-box hmi-panel">
        <div class="sensor-header">
          <span class="data-label">{{ card.label }}</span>
          <span class="freshness" :class="{ stale: !isFresh }">
            {{ isFresh ? '● LIVE' : '○ STALE' }}
          </span>
        </div>
        <div class="sensor-main">
          <span class="sensor-value data-value">{{ card.value }}</span>
          <span class="sensor-unit">{{ card.unit }}</span>
        </div>
        <!-- 装饰性背景光效 -->
        <div class="sensor-glow" :class="card.key"></div>
      </div>
    </div>

    <!-- 趋势与报警：现代分栏 -->
    <div class="bottom-section">
      <div class="trend-container hmi-panel">
        <div class="hmi-panel-header">
          <span>{{ tr("过程趋势分析", "Trend Analysis") }} (TEMP / PRESSURE)</span>
          <div class="trend-legend">
            <span class="legend-item temp">{{ tr("温度", "Temp") }}</span>
            <span class="legend-item press">{{ tr("压力", "Press") }}</span>
          </div>
        </div>
        <div ref="chartRef" class="chart-box"></div>
      </div>
      <div class="alarm-container hmi-panel">
        <div class="hmi-panel-header">{{ tr("实时报警中心", "Alarm Center") }}</div>
        <div class="alarm-list">
          <div v-for="(alarm, idx) in displayAlarms" :key="alarm.code || idx" class="alarm-item" :class="alarm.level">
            <div class="alarm-icon">{{ alarm.level === 'warn' ? '⚠️' : '🚨' }}</div>
            <div class="alarm-content">
              <span class="alarm-msg">{{ alarm.message }}</span>
              <span class="alarm-time mono">{{ alarm.detail }}</span>
            </div>
          </div>
          <div v-if="displayAlarms.length === 0" class="no-alarm">
            <div class="no-alarm-icon">🛡️</div>
            <span>{{ tr("系统运行正常，无活动报警", "All normal, no active alarms") }}</span>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import * as echarts from 'echarts/core'
import { LineChart } from 'echarts/charts'
import { GridComponent } from 'echarts/components'
import { CanvasRenderer } from 'echarts/renderers'

// Tree-shake ECharts: this view only uses a time-axis line chart, so register
// just LineChart + GridComponent + the canvas renderer instead of pulling in
// the full `echarts` bundle (~1 MB -> ~250 KB). Missing a component would log
// to the console and is caught by the e2e `assertNoVueConsoleErrors` check.
echarts.use([LineChart, GridComponent, CanvasRenderer])
import { useLiveStore } from '../stores/live'
import { useLanguage } from '../i18n'
import { storeToRefs } from 'pinia'

const liveStore = useLiveStore()
const { latestSample, runtime, liveStatus, alarms } = storeToRefs(liveStore)
const { tr } = useLanguage()

const currentTime = ref(new Date().toLocaleTimeString())
const chartRef = ref<HTMLElement>()
let chart: echarts.ECharts | null = null

const isFresh = computed(() => liveStatus.value === 'fresh')

const activeBatchId = computed(() => {
  const id = runtime.value?.active_batch_id
  if (id === null || id === undefined) return null
  const n = Number(id)
  return Number.isFinite(n) ? n : null
})
const activeBatchLabel = computed(() =>
  activeBatchId.value === null ? tr("待机", "Idle") : `#${activeBatchId.value}`
)

const modeLabel = computed(() => {
  if (runtime.value?.emergency_stop) return tr("急停", "E-STOP")
  if (runtime.value?.manual_lock) return tr("人工锁", "M-LOCK")
  if (runtime.value?.auto_enabled) return tr("自动运行中", "AUTO")
  return tr("手动", "MANUAL")
})
const systemTone = computed<'ok' | 'warn' | 'bad'>(() => {
  if (runtime.value?.emergency_stop || runtime.value?.manual_lock || runtime.value?.control_loop_terminated) return 'bad'
  if (runtime.value?.last_control_error || runtime.value?.last_sensor_error) return 'warn'
  return 'ok'
})
const systemTextClass = computed(() => `text-${systemTone.value === 'bad' ? 'red' : systemTone.value === 'warn' ? 'yellow' : 'green'}`)

const safetyLabel = computed(() => {
  if (runtime.value?.control_loop_terminated) return tr("控制环终止", "LOOP STOP")
  if (runtime.value?.emergency_stop) return tr("急停中", "E-STOP")
  if (runtime.value?.last_sensor_error) return tr("传感器故障", "SENSOR")
  if (runtime.value?.last_control_error) return tr("控制故障", "CTRL FLT")
  if (runtime.value?.manual_lock) return tr("人工锁定", "M-LOCK")
  return tr("正常", "OK")
})
const safetyClass = computed(() => {
  if (runtime.value?.control_loop_terminated || runtime.value?.emergency_stop || runtime.value?.last_sensor_error) return 'text-red'
  if (runtime.value?.last_control_error || runtime.value?.manual_lock) return 'text-yellow'
  return 'text-green'
})

const displayAlarms = computed(() =>
  (alarms.value ?? []).map((a) => ({
    code: a.code ?? a.type ?? '',
    message: a.message ?? a.type ?? tr("报警", "Alarm"),
    level: a.level === 'error' || a.severity === 'critical' ? 'error' : 'warn',
    detail: a.current_value != null
      ? `${tr("当前值", "Current")}: ${a.current_value}${a.limit_value != null ? ' / ' + tr("限值", "Limit") + ': ' + a.limit_value : ''}`
      : ''
  }))
)

const sensorCards = computed(() => {
  const s = latestSample.value
  if (!s) return []
  return [
    { key: 'temp', label: tr("釜内温度", "Temperature"), value: s.temperature_c?.toFixed(1) ?? '--', unit: '°C' },
    { key: 'press', label: tr("釜内压力", "Pressure"), value: s.pressure_mpa?.toFixed(3) ?? '--', unit: 'MPa' },
    { key: 'rpm', label: tr("搅拌转速", "Stirrer"), value: s.stirrer_rpm?.toFixed(0) ?? '--', unit: 'RPM' },
    { key: 'conc', label: tr("产物浓度", "Concentration"), value: s.product_concentration_percent?.toFixed(1) ?? '--', unit: '%' },
    { key: 'flow', label: tr("进料流量", "Flow Rate"), value: s.flow_rate_l_min?.toFixed(2) ?? '--', unit: 'L/min' },
    { key: 'ph', label: tr("pH 值", "pH"), value: s.ph?.toFixed(2) ?? '--', unit: '' },
  ]
})

const timer = setInterval(() => {
  currentTime.value = new Date().toLocaleTimeString()
}, 1000)

onMounted(() => {
  if (chartRef.value) {
    chart = echarts.init(chartRef.value)
    chart.setOption({
      grid: { top: 30, right: 20, bottom: 20, left: 40 },
      xAxis: { type: 'time', splitLine: { show: false }, axisLine: { lineStyle: { color: 'rgba(255,255,255,0.1)' } } },
      yAxis: [
        { type: 'value', name: 'T', position: 'left', splitLine: { lineStyle: { color: 'rgba(255,255,255,0.05)' } }, axisLabel: { color: '#78909c' } },
        { type: 'value', name: 'P', position: 'right', splitLine: { show: false }, axisLabel: { color: '#78909c' } }
      ],
      series: [
        { name: 'Temp', type: 'line', showSymbol: false, data: [], lineStyle: { color: '#ff3d00', width: 3 }, areaStyle: { color: 'rgba(255,61,0,0.1)' } },
        { name: 'Press', type: 'line', yAxisIndex: 1, showSymbol: false, data: [], lineStyle: { color: '#2979ff', width: 3 }, areaStyle: { color: 'rgba(41,121,255,0.1)' } }
      ],
      textStyle: { color: '#78909c', fontFamily: 'JetBrains Mono' }
    })
  }
})

watch(latestSample, (s) => {
  if (!s || !chart) return
  const opt = chart.getOption()
  const now = new Date()
  const tempData = (opt.series[0].data as any[]).concat([[now, s.temperature_c]]).slice(-50)
  const pressData = (opt.series[1].data as any[]).concat([[now, s.pressure_mpa]]).slice(-50)
  chart.setOption({
    series: [{ data: tempData }, { data: pressData }]
  })
})

onUnmounted(() => {
  clearInterval(timer)
  chart?.dispose()
})
</script>

<style scoped>
.monitor-hmi {
  display: flex;
  flex-direction: column;
  gap: var(--spacing);
  height: 100%;
  overflow: hidden;
}

/* 顶部状态栏 */
.status-bar {
  display: flex;
  align-items: center;
  padding: 16px 24px;
  gap: 32px;
}
.status-item { display: flex; align-items: center; gap: 16px; }
.status-item.time { margin-left: auto; }
.status-divider { width: 1px; height: 32px; background: var(--border-glass); }
.status-info { display: flex; flex-direction: column; gap: 4px; }
.text-ok { color: var(--ind-green); }
.text-green { color: var(--ind-green); }
.text-red { color: var(--ind-red); }
.text-yellow { color: var(--ind-amber); }

/* 状态灯动态色彩 */
.status-icon-wrapper.ok, .status-light.ok { color: var(--ind-green); }
.status-light.ok { background: var(--ind-green); box-shadow: 0 0 8px var(--ind-green-glow); }
.status-icon-wrapper.warn, .status-light.warn { color: var(--ind-amber); }
.status-light.warn { background: var(--ind-amber); box-shadow: 0 0 8px var(--ind-amber-glow); }
.status-icon-wrapper.bad, .status-light.bad { color: var(--ind-red); }
.status-light.bad { background: var(--ind-red); box-shadow: 0 0 8px var(--ind-red-glow); }

/* Bento Grid 传感器矩阵 */
.sensor-matrix {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  grid-template-rows: repeat(2, 1fr);
  gap: var(--spacing);
  flex: 1;
}

.sensor-box {
  position: relative;
  padding: 24px;
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  transition: transform 0.2s, box-shadow 0.2s;
}
.sensor-box:hover {
  transform: translateY(-2px);
  box-shadow: 0 12px 40px rgba(0,0,0,0.5), 0 0 20px rgba(255,255,255,0.05);
}

.sensor-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
}
.freshness { font-size: 11px; font-weight: 600; color: var(--ind-green); letter-spacing: 1px; }
.freshness.stale { color: var(--ind-amber); }

.sensor-main {
  display: flex;
  align-items: baseline;
  gap: 8px;
  margin-top: 16px;
}
.sensor-value { font-size: 48px; line-height: 1; }
.sensor-unit { font-size: 18px; color: var(--text-tertiary); font-weight: 500; }

/* 装饰性光效 */
.sensor-glow {
  position: absolute;
  bottom: -20%; right: -20%;
  width: 150px; height: 150px;
  border-radius: 50%;
  filter: blur(40px);
  opacity: 0.15;
  pointer-events: none;
}
.sensor-glow.temp { background: var(--ind-red); }
.sensor-glow.press { background: var(--ind-blue); }
.sensor-glow.rpm { background: var(--ind-green); }

/* 底部区域 */
.bottom-section {
  display: grid;
  grid-template-columns: 2fr 1fr;
  gap: var(--spacing);
  height: 280px;
}

.trend-legend { display: flex; gap: 16px; }
.legend-item { font-size: 12px; display: flex; align-items: center; gap: 6px; }
.legend-item::before { content: ""; display: block; width: 12px; height: 4px; border-radius: 2px; }
.legend-item.temp::before { background: var(--ind-red); }
.legend-item.press::before { background: var(--ind-blue); }

.chart-box { flex: 1; width: 100%; margin-top: 8px; }

.alarm-list { flex: 1; overflow: hidden; display: flex; flex-direction: column; gap: 12px; margin-top: 8px; }
.alarm-item {
  display: flex; align-items: center; gap: 16px;
  padding: 16px; border-radius: var(--radius-md);
  background: rgba(255,255,255,0.03);
  border: 1px solid var(--border-glass);
}
.alarm-item.warn { border-left: 4px solid var(--ind-amber); }
.alarm-item.error { border-left: 4px solid var(--ind-red); }
.alarm-icon { font-size: 24px; }
.alarm-content { display: flex; flex-direction: column; gap: 4px; flex: 1; }
.alarm-msg { font-size: 14px; font-weight: 600; color: var(--text-primary); }
.alarm-time { font-size: 12px; color: var(--text-tertiary); }

.no-alarm {
  flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center;
  color: var(--text-tertiary); gap: 16px;
}
.no-alarm-icon { font-size: 48px; opacity: 0.5; }

/* 响应式 */
@media (max-width: 1200px) {
  .sensor-matrix { grid-template-columns: repeat(2, 1fr); grid-template-rows: repeat(3, 1fr); }
  .sensor-value { font-size: 36px; }
  .bottom-section { grid-template-columns: 1fr; height: 320px; }
}
</style>
