<template>
  <div class="control-page">
    <!-- ================= 第一行：批次状态 | 目标设定值 | 急停 ================= -->
    <div class="row-top">
      <PanelCard en="Current Batch Status" zh="当前批次状态" icon="batch" class="batch-panel">
        <div v-if="activeBatch" class="batch-body">
          <div class="run-badge">
            <span class="run-dot ok"></span>
            <div class="run-texts">
              <span class="run-state ok">Running</span>
              <span class="run-zh">{{ tr("运行中", "Running") }}</span>
            </div>
            <span class="run-id mono">#{{ activeBatch.id }}</span>
          </div>

          <dl class="kv-list batch-kv">
            <dt>{{ tr("批次号", "Batch ID") }}</dt>
            <dd>#{{ activeBatch.id }}</dd>
            <dt>{{ tr("配方名称", "Recipe") }}</dt>
            <dd>{{ text(recipeName) }}</dd>
            <dt>{{ tr("产品名称", "Product") }}</dt>
            <dd>{{ text(activeBatch.name) }}</dd>
            <dt>{{ tr("开始时间", "Start Time") }}</dt>
            <dd>{{ formatTimestamp(activeBatch.started_at) }}</dd>
            <dt>{{ tr("已运行", "Elapsed") }}</dt>
            <dd>{{ fmtDur(elapsedMs) }}</dd>
            <dt>{{ tr("预计剩余", "Est. Remaining") }}</dt>
            <dd>{{ fmtDur(remainingMs) }}</dd>
          </dl>

          <div class="completion">
            <div class="completion-row">
              <span class="data-label">{{ tr("完成进度", "Completion") }}</span>
              <span class="mono pct">{{ completionPct === null ? "--" : (completionPct * 100).toFixed(1) + "%" }}</span>
            </div>
            <div class="bar">
              <div class="fill" :style="{ width: (completionPct ?? 0) * 100 + '%' }"></div>
            </div>
          </div>
        </div>
        <div v-else class="empty-state batch-idle">
          <AppIcon name="batch" :size="36" />
          <span class="idle-en">IDLE</span>
          <span class="idle-zh">{{ tr("无运行批次", "No active batch") }}</span>
        </div>
      </PanelCard>

      <PanelCard en="Target Setpoints (Live)" zh="实时目标设定值" icon="control" class="sp-panel">
        <template #actions>
          <span v-if="targetsDirty" class="dirty-tag">{{ tr("未应用", "Unapplied") }}</span>
          <span class="live-tag" :class="live.liveStatus">
            {{ live.liveStatus === "fresh" ? tr("实时", "LIVE") : tr("无数据", "NO DATA") }}
          </span>
        </template>
        <div class="sp-grid">
          <div v-for="col in setpointCols" :key="col.key" class="sp-col" :class="{ readonly: !col.writable }">
            <div class="sp-head">
              <div class="sp-title">
                <span class="en">{{ col.en }} <em class="unit">{{ col.unit }}</em></span>
                <span class="zh">{{ col.zh }}</span>
              </div>
              <span v-if="!col.writable" class="ro-tag">{{ tr("只读", "Read-only") }}</span>
            </div>

            <div class="sp-stepper">
              <button
                type="button"
                class="sp-btn"
                :disabled="!col.writable || busy"
                :title="tr('减小', 'Decrease')"
                @click="bump(col, -1)"
              >▲</button>
              <span class="sp-value" :class="{ dim: !col.writable }">{{ col.display }}</span>
              <button
                type="button"
                class="sp-btn"
                :disabled="!col.writable || busy"
                :title="tr('增大', 'Increase')"
                @click="bump(col, 1)"
              >▼</button>
            </div>

            <el-slider
              :model-value="col.model"
              :min="col.min"
              :max="col.max"
              :step="col.step"
              :disabled="!col.writable || busy"
              :show-tooltip="false"
              class="sp-slider"
              @update:model-value="col.set"
            />

            <div class="sp-pv">PV <b class="mono">{{ col.pvText }}</b></div>
          </div>
        </div>
      </PanelCard>

      <EmergencyStopPanel class="estop-col" />
    </div>

    <!-- ================= 第二行：控制模式 | 快捷操作 ================= -->
    <div class="row-mid">
      <PanelCard en="Control Mode" zh="控制模式" icon="settings">
        <div class="mode-row">
          <button
            type="button"
            class="big-btn green"
            :class="{ active: !!runtime?.auto_enabled && !runtime?.manual_lock }"
            :disabled="busy"
            @click="doSetAuto(true)"
          >
            <AppIcon name="check" :size="18" />
            <span class="lbl"><span class="en">AUTO</span><span class="zh">{{ tr("自动模式", "Automatic") }}</span></span>
          </button>
          <button
            type="button"
            class="big-btn amber"
            :class="{ active: !!runtime?.manual_lock }"
            :disabled="busy"
            @click="doToggleLock"
          >
            <AppIcon name="shield" :size="18" />
            <span class="lbl"><span class="en">MANUAL LOCK</span><span class="zh">{{ tr("手动锁定", "Manual Lock") }}</span></span>
          </button>
          <button type="button" class="big-btn blue" :disabled="busy" @click="doSetAuto(false)">
            <AppIcon name="pause" :size="18" />
            <span class="lbl"><span class="en">HOLD</span><span class="zh">{{ tr("保持", "Hold") }}</span></span>
          </button>
          <button type="button" class="big-btn" :disabled="busy" @click="doResetFault">
            <AppIcon name="reset" :size="18" />
            <span class="lbl"><span class="en">RESET</span><span class="zh">{{ tr("复位", "Reset") }}</span></span>
          </button>
        </div>
      </PanelCard>

      <PanelCard en="Quick Actions" zh="快捷操作" icon="play">
        <div class="quick-row">
          <button type="button" class="big-btn green" :disabled="busy" @click="openStartDialog">
            <AppIcon name="play" :size="18" />
            <span class="lbl"><span class="en">START BATCH</span><span class="zh">{{ tr("开始批次", "Start Batch") }}</span></span>
          </button>
          <button type="button" class="big-btn amber" :disabled="busy" @click="doSetAuto(false)">
            <AppIcon name="pause" :size="18" />
            <span class="lbl"><span class="en">PAUSE</span><span class="zh">{{ tr("暂停", "Pause") }}</span></span>
          </button>
          <button type="button" class="big-btn green" :disabled="busy" @click="doSetAuto(true)">
            <AppIcon name="check" :size="18" />
            <span class="lbl"><span class="en">RESUME</span><span class="zh">{{ tr("继续", "Resume") }}</span></span>
          </button>
          <button type="button" class="big-btn red" :disabled="busy" @click="doStopProcess">
            <AppIcon name="stop" :size="18" />
            <span class="lbl"><span class="en">STOP</span><span class="zh">{{ tr("停止", "Stop") }}</span></span>
          </button>
          <button type="button" class="big-btn blue" :disabled="busy" @click="doApplyTargets">
            <AppIcon name="control" :size="18" />
            <span class="lbl">
              <span class="en">APPLY TARGETS<em v-if="targetsDirty" class="dot"></em></span>
              <span class="zh">{{ tr("应用设定值", "Apply Targets") }}</span>
            </span>
          </button>
        </div>
      </PanelCard>
    </div>

    <!-- ================= 第三行：配方 | 活动过程 | 组件 | 安全+事件 ================= -->
    <div class="row-main">
      <!-- A. 工艺配方 -->
      <PanelCard en="Process Recipe" zh="工艺配方" icon="flask" class="col-recipe">
        <template #actions>
          <span class="proc-name">{{ text(recipeLabel) }}</span>
        </template>
        <div class="recipe-wrap">
          <div class="recipe-table-wrap scrollable">
            <el-table v-if="stepRows.length" :data="stepRows" size="small" class="recipe-table">
              <el-table-column width="48">
                <template #header><span class="th-en">Step</span><span class="th-zh">{{ tr("步骤", "#") }}</span></template>
                <template #default="{ row }"><span class="mono">#{{ row.idx + 1 }}</span></template>
              </el-table-column>
              <el-table-column min-width="110">
                <template #header><span class="th-en">Phase</span><span class="th-zh">{{ tr("阶段", "Phase") }}</span></template>
                <template #default="{ row }">{{ text(row.step.name) }}</template>
              </el-table-column>
              <el-table-column min-width="128">
                <template #header><span class="th-en">Target</span><span class="th-zh">{{ tr("目标", "Target") }}</span></template>
                <template #default="{ row }"><span class="mono">{{ row.targetLabel }}</span></template>
              </el-table-column>
              <el-table-column width="76">
                <template #header><span class="th-en">Duration</span><span class="th-zh">{{ tr("时长", "Duration") }}</span></template>
                <template #default="{ row }"><span class="mono">{{ fixed(row.durMin, 0) }} min</span></template>
              </el-table-column>
              <el-table-column width="98">
                <template #header><span class="th-en">Status</span><span class="th-zh">{{ tr("状态", "Status") }}</span></template>
                <template #default="{ row }">
                  <span class="pill" :class="row.status">{{ stepStatusText(row.status) }}</span>
                </template>
              </el-table-column>
            </el-table>
            <div v-else class="empty-state">
              <AppIcon name="flask" :size="28" />
              <span>{{ tr("无工艺步骤", "No process steps") }}</span>
            </div>
          </div>

          <div class="recipe-timeline">
            <div class="tl-title">{{ tr("配方时间线", "Recipe Timeline") }}</div>
            <div v-if="stepRows.length" class="tl-track">
              <div
                v-for="row in stepRows"
                :key="row.idx"
                class="tl-seg"
                :class="row.status"
                :style="{ flexGrow: Math.max(row.durMin, 0.001) }"
                :title="tlTitle(row)"
              >
                <span class="idx">S{{ row.idx + 1 }}</span>
              </div>
              <div
                v-if="completionPct !== null && completionPct > 0"
                class="tl-marker"
                :style="{ left: completionPct * 100 + '%' }"
              ></div>
            </div>
            <div class="tl-legend">
              <span><i class="dot done"></i>{{ tr("已完成", "Completed") }}</span>
              <span><i class="dot current"></i>{{ tr("进行中", "In Progress") }}</span>
              <span><i class="dot pending"></i>{{ tr("等待中", "Pending") }}</span>
            </div>
          </div>
        </div>
      </PanelCard>

      <!-- B. 当前活动过程 -->
      <PanelCard en="Current Active Process" zh="当前活动过程" icon="clock">
        <div class="ap-wrap">
          <div class="ap-head">
            <div class="ap-icon"><AppIcon name="flask" :size="22" /></div>
            <div class="ap-title">
              <span class="ap-step mono">{{ currentRow ? "STEP " + (currentRow.idx + 1) : "IDLE" }}</span>
              <span class="ap-name">{{ currentRow?.step.name ?? tr("无进行中步骤", "No active step") }}</span>
            </div>
            <div class="ap-phase">
              <span class="data-label">{{ tr("阶段时间", "Phase Time") }}</span>
              <span class="mono ap-phase-val">
                {{ fmtDur(phaseElapsedMs) }}<em>/ {{ currentRow?.durMin ? currentRow.durMin + " min" : "--" }}</em>
              </span>
            </div>
          </div>

          <div class="ap-cards">
            <div class="tc">
              <span class="lbl">{{ tr("温度", "Temperature") }}</span>
              <span class="val mono">{{ fixed(currentRow?.step.target_temperature_c ?? null, 1) }}<em class="u">°C</em></span>
            </div>
            <div class="tc">
              <span class="lbl">{{ tr("搅拌转速", "Stirrer") }}</span>
              <span class="val mono">{{ fixed(currentRow?.step.target_stirrer_rpm ?? null, 0) }}<em class="u">rpm</em></span>
            </div>
            <div class="tc">
              <span class="lbl">{{ tr("压力", "Pressure") }}</span>
              <span class="val mono">{{ pressureBarText(currentRow?.step.target_pressure_mpa ?? null) }}<em class="u">bar</em></span>
            </div>
          </div>

          <div class="ap-trend">
            <span class="trend-label">{{ tr("关键变量趋势", "Key Variables Trend") }}</span>
            <div class="chart-fill">
              <TrendChart v-if="trendSeries.length" :series="trendSeries" :legend="true" height="100%" />
              <div v-else class="empty-state chart-empty">
                <span>{{ tr("暂无样本数据", "No sample data") }}</span>
              </div>
            </div>
          </div>
        </div>
      </PanelCard>

      <!-- C. 组件控制 -->
      <PanelCard en="Component Control" zh="组件控制" icon="valve">
        <div class="comp-wrap">
          <div v-if="primaryComponents.items.length" class="comp-list scrollable">
            <div v-for="c in primaryComponents.items" :key="compId(c)" class="comp-row">
              <span class="comp-icon"><AppIcon :name="compIcon(c)" :size="17" /></span>
              <div class="comp-main">
                <span class="comp-name">{{ text(c.label ?? compId(c)) }}</span>
                <span class="comp-state" :class="compStateClass(c.state)">{{ compStateLabel(c.state) }}</span>
              </div>
              <el-tooltip
                v-if="!c.actions || !c.actions.length"
                :content="tr('该组件无可用动作', 'No actions available')"
                placement="top"
              >
                <el-switch :model-value="compOn(c.state)" disabled size="small" />
              </el-tooltip>
              <el-switch
                v-else
                :model-value="compOn(c.state)"
                :disabled="busy"
                size="small"
                @change="onCompChange(c, $event)"
              />
            </div>
          </div>
          <div v-else class="empty-state comp-empty">
            <AppIcon name="valve" :size="28" />
            <span>{{ tr("无组件数据", "No components") }}</span>
          </div>
          <div class="comp-foot">
            <a class="comp-link" @click.prevent="goComponentsOverview">
              {{ tr("全部组件总览", "All Components Overview") }} →
            </a>
          </div>
        </div>
      </PanelCard>

      <!-- D. 安全边界 + 最近控制事件 -->
      <div class="col-safety">
        <PanelCard en="Safety Boundary" zh="安全边界" icon="shield">
          <table v-if="safetyRows.length" class="sf-table">
            <thead>
              <tr>
                <th><span class="th-en">Parameter</span><span class="th-zh">{{ tr("参数", "Parameter") }}</span></th>
                <th><span class="th-en">Lower</span><span class="th-zh">{{ tr("下限", "Lower") }}</span></th>
                <th><span class="th-en">Upper</span><span class="th-zh">{{ tr("上限", "Upper") }}</span></th>
                <th><span class="th-en">Status</span><span class="th-zh">{{ tr("状态", "Status") }}</span></th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="row in safetyRows" :key="row.key">
                <td class="sf-param">{{ row.en }} <span class="sf-zh">{{ row.zh }}</span></td>
                <td class="mono">{{ fixed(row.lower, row.digits) }}</td>
                <td class="mono">{{ fixed(row.upper, row.digits) }}</td>
                <td>
                  <span class="sf-status" :class="row.status">
                    <i class="status-dot" :class="row.status === 'ok' ? 'ok' : row.status === 'bad' ? 'bad' : ''"></i>
                    {{ row.status === "ok" ? "OK" : row.status === "bad" ? tr("越限", "ALARM") : "--" }}
                  </span>
                </td>
              </tr>
            </tbody>
          </table>
          <div v-else class="empty-state sf-empty">
            <AppIcon name="shield" :size="26" />
            <span>{{ tr("未加载安全配置", "Safety config unavailable") }}</span>
          </div>
        </PanelCard>

        <PanelCard en="Recent Control Events" zh="最近控制事件" icon="audit">
          <ul v-if="recentEvents.length" class="ev-list scrollable">
            <li v-for="ev in recentEvents" :key="ev.id" class="ev-item">
              <span class="ev-time mono">{{ formatTime(ev.created_at) }}</span>
              <div class="ev-main">
                <span class="ev-type">{{ eventTypeLabel(ev.event_type) }}</span>
                <span v-if="eventValueLabel(ev)" class="ev-val mono">{{ eventValueLabel(ev) }}</span>
              </div>
              <span class="ev-src" :class="isAutoEvent(ev.event_type) ? 'auto' : 'user'">
                {{ isAutoEvent(ev.event_type) ? "Auto" : "User" }}
              </span>
            </li>
          </ul>
          <div v-else class="empty-state ev-empty">
            <AppIcon name="audit" :size="26" />
            <span>{{ tr("暂无控制事件", "No control events") }}</span>
          </div>
        </PanelCard>
      </div>
    </div>

    <!-- 开始批次对话框 -->
    <el-dialog v-model="startVisible" :title="tr('开始批次', 'Start Batch')" width="440px">
      <div class="start-form">
        <label class="start-field">
          <span class="data-label">{{ tr("工艺", "Process") }}</span>
          <el-select
            v-model="startProcessId"
            :placeholder="tr('选择工艺', 'Select a process')"
            style="width: 100%"
            size="large"
          >
            <el-option
              v-for="p in plant.processes"
              :key="p.id"
              :value="p.id"
              :label="text(p.name, '#' + p.id) + (p.status ? ' · ' + p.status : '')"
            />
          </el-select>
        </label>
        <label class="start-field">
          <span class="data-label">{{ tr("批次名称（可选）", "Batch name (optional)") }}</span>
          <el-input
            v-model="startBatchName"
            :placeholder="tr('留空则使用工艺启动', 'Leave empty to start from process')"
            maxlength="80"
          />
        </label>
      </div>
      <template #footer>
        <el-button @click="startVisible = false">{{ tr("取消", "Cancel") }}</el-button>
        <el-button type="success" :disabled="startProcessId === null || busy" @click="doStartBatch">
          {{ tr("开始", "Start") }}
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
// Control 页：工艺控制中心。
// 布局：三行 grid（批次状态+设定值+急停 / 控制模式+快捷操作 / 配方+活动过程+组件+安全事件）。
// 压力显示统一 bar（后端 MPa × 10）；压力/流量目标后端不可写，仅做只读展示。
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { useRouter } from "vue-router";
import { ElMessage, ElMessageBox } from "element-plus";
import PanelCard from "../components/PanelCard.vue";
import TrendChart from "../components/TrendChart.vue";
import AppIcon from "../components/AppIcon.vue";
import EmergencyStopPanel from "../components/EmergencyStopPanel.vue";
import { useLiveStore } from "../stores/live";
import { usePlantStore } from "../stores/plant";
import { batchApi, controlApi, DEVICE_ID, deviceApi, processApi } from "../api";
import { errorMessage } from "../api/errors";
import { useLanguage } from "../i18n";
import { fixed, formatTime, formatTimestamp, text } from "../utils/format";
import type { ComponentAction, ControlEvent, DeviceComponentItem, ProcessStep, SensorSample } from "../api/types";

const live = useLiveStore();
const plant = usePlantStore();
const router = useRouter();
const { tr, language } = useLanguage();

const runtime = computed(() => live.runtime);
const busy = ref(false);

// ---------------------------------------------------------------- 时间基
const nowMs = ref(Date.now());
let tickTimer: number | null = null;

// ---------------------------------------------------------------- 批次状态
const activeBatchId = computed<number | null>(() => {
  const raw = runtime.value?.active_batch_id;
  if (raw === null || raw === undefined) return null;
  const n = Number(raw);
  return Number.isFinite(n) ? n : null;
});

const activeBatch = computed(() => {
  if (activeBatchId.value === null) return null;
  const list = plant.batches?.batches ?? live.live?.recent_batches ?? [];
  return list.find((b) => b.id === activeBatchId.value) ?? null;
});

const recipeName = computed(
  () => runtime.value?.active_process_name
    ?? plant.processes.find((p) => p.id === activeBatch.value?.process_id)?.name
    ?? null
);

const startedAtMs = computed<number | null>(() => {
  const raw = activeBatch.value?.started_at;
  if (!raw) return null;
  const t = Date.parse(raw);
  return Number.isNaN(t) ? null : t;
});

const elapsedMs = computed<number | null>(() =>
  startedAtMs.value === null ? null : Math.max(0, nowMs.value - startedAtMs.value)
);

// ---------------------------------------------------------------- 工艺配方 / 步骤推进
const recipeProcessId = computed<number | null>(() => {
  const active = runtime.value?.active_process_id ?? activeBatch.value?.process_id ?? null;
  if (active !== null && active !== undefined) return Number(active);
  const applied = plant.processes.find((p) => p.status === "applied");
  return applied?.id ?? plant.processes[0]?.id ?? null;
});

watch(
  recipeProcessId,
  async (id) => {
    if (id === null || id === undefined) return;
    if (plant.selectedProcess?.process.id === id) return;
    try {
      await plant.loadProcessDetail(id);
    } catch {
      // 工艺详情加载失败：保留空态，不阻塞整页。
    }
  },
  { immediate: true }
);

const recipeLabel = computed(
  () => plant.selectedProcess?.process.name ?? plant.processes.find((p) => p.id === recipeProcessId.value)?.name ?? null
);

function stepTargetLabel(step: ProcessStep): string {
  const parts: string[] = [];
  if (step.target_temperature_c !== null && step.target_temperature_c !== undefined) {
    parts.push(fixed(step.target_temperature_c, 1) + "°C");
  }
  if (step.target_stirrer_rpm !== null && step.target_stirrer_rpm !== undefined) {
    parts.push(fixed(step.target_stirrer_rpm, 0) + "rpm");
  }
  if (step.target_shake_speed_cpm !== null && step.target_shake_speed_cpm !== undefined) {
    parts.push(fixed(step.target_shake_speed_cpm, 0) + "cpm");
  }
  if (step.target_pressure_mpa !== null && step.target_pressure_mpa !== undefined) {
    parts.push(fixed(step.target_pressure_mpa * 10, 1) + "bar");
  }
  return parts.length ? parts.join(" · ") : "--";
}

interface StepRow {
  idx: number;
  step: ProcessStep;
  startMin: number;
  durMin: number;
  status: "done" | "current" | "pending";
  targetLabel: string;
}

const stepRows = computed<StepRow[]>(() => {
  const steps = plant.selectedProcess?.steps ?? [];
  const elapsed = elapsedMs.value === null ? null : elapsedMs.value / 60000;
  let cum = 0;
  return steps.map((step, idx) => {
    const durMin = step.duration_minutes ?? 0;
    const startMin = cum;
    cum += durMin;
    let status: StepRow["status"] = "pending";
    if (elapsed !== null) {
      if (elapsed >= cum) status = "done";
      else if (elapsed >= startMin) status = "current";
    }
    return { idx, step, startMin, durMin, status, targetLabel: stepTargetLabel(step) };
  });
});

const currentRow = computed<StepRow | null>(() => stepRows.value.find((r) => r.status === "current") ?? null);

const phaseElapsedMs = computed<number | null>(() =>
  currentRow.value === null || elapsedMs.value === null
    ? null
    : Math.max(0, elapsedMs.value - currentRow.value.startMin * 60000)
);

const totalExpectedMin = computed<number | null>(() => {
  if (activeBatch.value) {
    const rows = stepRows.value;
    if (rows.length) return rows.reduce((acc, r) => acc + r.durMin, 0);
    const b = activeBatch.value;
    const fallback = (b.heating_minutes ?? 0) + (b.stirring_minutes ?? 0);
    if (fallback > 0) return fallback;
  }
  return null;
});

const remainingMs = computed<number | null>(() => {
  if (totalExpectedMin.value === null || elapsedMs.value === null) return null;
  return Math.max(0, totalExpectedMin.value * 60000 - elapsedMs.value);
});

const completionPct = computed<number | null>(() => {
  if (totalExpectedMin.value === null || totalExpectedMin.value <= 0 || elapsedMs.value === null) return null;
  return Math.min(1, Math.max(0, elapsedMs.value / (totalExpectedMin.value * 60000)));
});

function stepStatusText(status: StepRow["status"]): string {
  if (status === "done") return tr("已完成", "Completed");
  if (status === "current") return tr("进行中", "In Progress");
  return tr("等待中", "Pending");
}

function tlTitle(row: StepRow): string {
  return `S${row.idx + 1} · ${stepStatusText(row.status)} · ${fixed(row.durMin, 0)} min`;
}

function fmtDur(ms: number | null): string {
  if (ms === null || !Number.isFinite(ms)) return "--";
  const total = Math.floor(ms / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
}

// ---------------------------------------------------------------- 目标设定值
const safetyTempMax = computed(() => plant.config?.safety?.temperature?.max_c ?? null);
const safetyRpmMax = computed(() => plant.config?.safety?.stirrer?.max_rpm ?? null);

const sp = reactive({ temperature: 25, stirrer: 0, shake: 0 });
const targetsDirty = ref(false);

watch(
  () => runtime.value?.targets,
  (targets) => {
    if (!targets || targetsDirty.value) return;
    if (typeof targets.temperature_c === "number") sp.temperature = targets.temperature_c;
    if (typeof targets.stirrer_rpm === "number") sp.stirrer = targets.stirrer_rpm;
    if (typeof targets.shake_speed_cpm === "number") sp.shake = targets.shake_speed_cpm;
  },
  { immediate: true }
);

function markDirty(): void {
  targetsDirty.value = true;
}

function clampNum(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}

function roundTo(v: number, digits: number): number {
  const f = Math.pow(10, digits);
  return Math.round(v * f) / f;
}

interface SpCol {
  key: string;
  en: string;
  zh: string;
  unit: string;
  writable: boolean;
  min: number;
  max: number;
  step: number;
  digits: number;
  display: string;
  model: number;
  pvText: string;
  set: (v: number) => void;
}

function pressureBarText(mpa: number | null | undefined): string {
  const v = Number(mpa);
  return mpa === null || mpa === undefined || !Number.isFinite(v) ? "--" : fixed(v * 10, 1);
}

function pvBar(): number | null {
  const raw = live.latestSample?.pressure_mpa;
  const v = Number(raw);
  return raw === null || raw === undefined || !Number.isFinite(v) ? null : v * 10;
}

const setpointCols = computed<SpCol[]>(() => {
  const sample = live.latestSample;
  const tempMax = Math.min(100, safetyTempMax.value ?? 100);
  const rpmMax = Math.min(1000, safetyRpmMax.value ?? 1000);
  const pressure = pvBar();
  return [
    {
      key: "temperature",
      en: "Target Temperature",
      zh: tr("目标温度", "Target Temperature"),
      unit: "°C",
      writable: true,
      min: 0,
      max: tempMax,
      step: 0.5,
      digits: 1,
      display: fixed(sp.temperature, 1),
      model: clampNum(sp.temperature, 0, tempMax),
      pvText: fixed(sample?.temperature_c ?? null, 1),
      set: (v: number) => {
        sp.temperature = roundTo(clampNum(v, 0, tempMax), 1);
        markDirty();
      }
    },
    {
      key: "pressure",
      en: "Target Pressure",
      zh: tr("目标压力", "Target Pressure"),
      unit: "bar",
      writable: false,
      min: 0,
      max: 5,
      step: 0.1,
      digits: 2,
      display: fixed(pressure, 2),
      model: clampNum(pressure ?? 0, 0, 5),
      pvText: fixed(pressure, 2),
      set: () => undefined
    },
    {
      key: "stirrer",
      en: "Target Stirrer",
      zh: tr("搅拌转速", "Stirrer RPM"),
      unit: "RPM",
      writable: true,
      min: 0,
      max: rpmMax,
      step: 10,
      digits: 0,
      display: fixed(sp.stirrer, 0),
      model: clampNum(sp.stirrer, 0, rpmMax),
      pvText: fixed(sample?.stirrer_rpm ?? null, 0),
      set: (v: number) => {
        sp.stirrer = Math.round(clampNum(v, 0, rpmMax));
        markDirty();
      }
    },
    {
      key: "shake",
      en: "Target Shake Speed",
      zh: tr("振荡速度", "Shake Speed"),
      unit: "rpm",
      writable: true,
      min: 0,
      max: 500,
      step: 10,
      digits: 0,
      display: fixed(sp.shake, 0),
      model: clampNum(sp.shake, 0, 500),
      pvText: fixed(sample?.shake_speed_cpm ?? null, 0),
      set: (v: number) => {
        sp.shake = Math.round(clampNum(v, 0, 500));
        markDirty();
      }
    },
    {
      key: "flow",
      en: "Target Flow Rate",
      zh: tr("目标流量", "Flow Rate"),
      unit: "L/min",
      writable: false,
      min: 0,
      max: 50,
      step: 0.5,
      digits: 2,
      display: fixed(sample?.flow_rate_l_min ?? null, 2),
      model: clampNum(Number(sample?.flow_rate_l_min ?? 0) || 0, 0, 50),
      pvText: fixed(sample?.flow_rate_l_min ?? null, 2),
      set: () => undefined
    }
  ];
});

function bump(col: SpCol, dir: 1 | -1): void {
  if (!col.writable || busy.value) return;
  const current = col.key === "temperature" ? sp.temperature : col.key === "stirrer" ? sp.stirrer : sp.shake;
  col.set(current + dir * col.step);
}

// ---------------------------------------------------------------- 关键变量趋势
interface LocalTrendSeries {
  name: string;
  data: Array<[number, number | null]>;
  color?: string;
  unit?: string;
  yAxisIndex?: number;
}

function sampleTs(r: SensorSample): number {
  const raw = r.captured_at ?? r.created_at ?? "";
  const t = Date.parse(raw);
  return Number.isNaN(t) ? 0 : t;
}

const trendSeries = computed<LocalTrendSeries[]>(() => {
  const rows = live.recentSamples;
  if (!rows.length) return [];
  return [
    {
      name: tr("温度", "Temp"),
      unit: "°C",
      color: "#2f9bff",
      yAxisIndex: 0,
      data: rows.map((r) => [sampleTs(r), r.temperature_c ?? null])
    },
    {
      name: tr("压力", "Pressure"),
      unit: "bar",
      color: "#f5a623",
      yAxisIndex: 1,
      data: rows.map((r) => {
        const v = r.pressure_mpa;
        return v === null || v === undefined || !Number.isFinite(v) ? [sampleTs(r), null] : [sampleTs(r), v * 10];
      })
    },
    {
      name: tr("转速", "RPM"),
      unit: "rpm",
      color: "#2fd47b",
      yAxisIndex: 1,
      data: rows.map((r) => [sampleTs(r), r.stirrer_rpm ?? null])
    }
  ];
});

// ---------------------------------------------------------------- 组件控制
const primaryComponents = computed<{ deviceId: string; items: DeviceComponentItem[] }>(() => {
  const device = live.primaryDevice ?? plant.deviceStatus?.devices?.[0] ?? null;
  return {
    deviceId: device?.device_id ?? DEVICE_ID,
    items: Array.isArray(device?.components) ? device!.components! : []
  };
});

function compId(c: DeviceComponentItem): string {
  return c.component_id ?? c.id ?? "";
}

function compIcon(c: DeviceComponentItem): string {
  const id = compId(c).toLowerCase() + " " + String(c.label ?? "").toLowerCase();
  if (id.includes("heat")) return "heater";
  if (id.includes("valve")) return "valve";
  if (id.includes("stir") || id.includes("motor") || id.includes("pump")) return "motor";
  if (id.includes("temp") || id.includes("sensor") || id.includes("pressure")) return "gauge";
  return "flask";
}

function compStateLabel(state?: string | null): string {
  const s = String(state ?? "").trim();
  return s ? s.toUpperCase() : "--";
}

function compStateClass(state?: string | null): string {
  const s = String(state ?? "").toLowerCase();
  if (["on", "open", "opened", "running", "heating", "active", "enabled"].includes(s)) return "on";
  if (["off", "closed", "idle", "stopped", "disabled", ""].includes(s)) return "off";
  return "mid";
}

function compOn(state?: string | null): boolean {
  return compStateClass(state) === "on";
}

function pickAction(actions: ComponentAction[], wantOn: boolean): ComponentAction {
  const onKeys = ["open", "start", "on", "enable", "run"];
  const offKeys = ["close", "stop", "off", "disable", "halt"];
  if (wantOn) {
    return actions.find((a) => onKeys.includes(String(a.action).toLowerCase())) ?? actions[0];
  }
  return actions.find((a) => offKeys.includes(String(a.action).toLowerCase())) ?? actions[actions.length - 1];
}

function onCompChange(c: DeviceComponentItem, value: string | number | boolean | undefined): void {
  doComponentControl(c, !!value);
}

function doComponentControl(c: DeviceComponentItem, wantOn: boolean): void {
  const actions = c.actions ?? [];
  if (!actions.length) return;
  const action = pickAction(actions, wantOn);
  const label = `${tr("组件控制", "Component control")} ${text(c.label ?? compId(c))}`;
  void runAction(label, () =>
    deviceApi.controlComponent(primaryComponents.value.deviceId, compId(c), { action: action.action })
  );
}

function goComponentsOverview(): void {
  void router.push("/monitor");
}

// ---------------------------------------------------------------- 安全边界
interface SafetyRow {
  key: string;
  en: string;
  zh: string;
  lower: number | null;
  upper: number | null;
  digits: number;
  status: "ok" | "bad" | "unknown";
}

function boundStatus(pv: number | null | undefined, lo: number | null, hi: number | null): SafetyRow["status"] {
  if (pv === null || pv === undefined || !Number.isFinite(pv)) return "unknown";
  if (lo !== null && pv < lo) return "bad";
  if (hi !== null && pv > hi) return "bad";
  return "ok";
}

const safetyRows = computed<SafetyRow[]>(() => {
  const s = plant.config?.safety;
  const sample = live.latestSample;
  const rows: SafetyRow[] = [];
  if (s?.temperature) {
    rows.push({
      key: "temperature",
      en: "Temperature",
      zh: tr("温度", "Temperature"),
      lower: s.temperature.min_c ?? null,
      upper: s.temperature.max_c ?? null,
      digits: 1,
      status: boundStatus(sample?.temperature_c ?? null, s.temperature.min_c ?? null, s.temperature.max_c ?? null)
    });
  }
  if (s?.stirrer) {
    rows.push({
      key: "stirrer",
      en: "Stirrer",
      zh: tr("搅拌器", "Stirrer"),
      lower: s.stirrer.min_rpm ?? null,
      upper: s.stirrer.max_rpm ?? null,
      digits: 0,
      status: boundStatus(sample?.stirrer_rpm ?? null, s.stirrer.min_rpm ?? null, s.stirrer.max_rpm ?? null)
    });
  }
  return rows;
});

// ---------------------------------------------------------------- 最近控制事件
const EVENT_TYPE_LABELS: Record<string, [string, string]> = {
  batch_started: ["Batch Started", "批次开始"],
  batch_finished: ["Batch Finished", "批次结束"],
  process_started: ["Process Started", "工艺已开始"],
  process_stopped: ["Process Stopped", "工艺已停止"],
  process_applied: ["Process Applied", "工艺已应用"],
  process_start_failed: ["Process Start Failed", "工艺启动失败"],
  auto_enabled: ["Auto Enabled", "自动已启用"],
  auto_disabled: ["Auto Disabled", "自动已停用"],
  manual_lock_on: ["Manual Lock On", "手动锁定"],
  manual_lock_off: ["Manual Lock Off", "锁定解除"],
  manual_unlock_refused: ["Unlock Refused", "解锁被拒绝"],
  emergency_stop: ["Emergency Stop", "紧急停止"],
  emergency_stop_reset: ["E-Stop Reset", "急停复位"],
  component_control: ["Component Control", "组件控制"],
  ai_targets_updated: ["AI Targets Updated", "AI 目标更新"],
  ai_process_started: ["AI Process Start", "AI 启动工艺"],
  ai_process_stopped: ["AI Process Stop", "AI 停止工艺"],
  ai_component_control: ["AI Component Control", "AI 组件控制"],
  ainas_process_started: ["AINAS Process Start", "AINAS 启动工艺"],
  ainas_process_stopped: ["AINAS Process Stop", "AINAS 停止工艺"]
};

function eventTypeLabel(type: string): string {
  const hit = EVENT_TYPE_LABELS[type];
  if (!hit) return type;
  return language.value === "zh" ? hit[1] : hit[0];
}

function isAutoEvent(type: string): boolean {
  const t = type.toLowerCase();
  return t.startsWith("ai_") || t.startsWith("ainas_") || t.startsWith("auto_");
}

function eventValueLabel(ev: ControlEvent): string {
  const parts: string[] = [];
  if (ev.target_temperature_c !== null && ev.target_temperature_c !== undefined) {
    parts.push(fixed(ev.target_temperature_c, 1) + "°C");
  }
  if (ev.target_stirrer_rpm !== null && ev.target_stirrer_rpm !== undefined) {
    parts.push(fixed(ev.target_stirrer_rpm, 0) + "rpm");
  }
  if (ev.target_shake_speed_cpm !== null && ev.target_shake_speed_cpm !== undefined) {
    parts.push(fixed(ev.target_shake_speed_cpm, 0) + "cpm");
  }
  return parts.join(" · ");
}

const recentEvents = computed<ControlEvent[]>(() => (plant.audit?.events ?? []).slice(0, 9));

// ---------------------------------------------------------------- 写操作
async function runAction(label: string, fn: () => Promise<unknown>): Promise<void> {
  if (busy.value) return;
  busy.value = true;
  let ok = false;
  try {
    await fn();
    ok = true;
    ElMessage.success(`${label} ${tr("成功", "OK")}`);
  } catch (e) {
    ElMessage.error(`${label}${tr("失败：", " failed: ")}${errorMessage(e)}`);
  } finally {
    busy.value = false;
    void live.refreshLive();
    void plant.loadAudit({ page: 1, pageSize: 30 }).catch(() => undefined);
    if (ok) void plant.loadBatches().catch(() => undefined);
  }
}

function doSetAuto(enabled: boolean): void {
  void runAction(
    enabled ? tr("启用自动控制", "Enable auto control") : tr("暂停自动控制", "Disable auto control"),
    () => controlApi.setAuto(enabled)
  );
}

function doToggleLock(): void {
  const locked = !!runtime.value?.manual_lock;
  void runAction(
    locked ? tr("解除手动锁定", "Release manual lock") : tr("手动锁定", "Engage manual lock"),
    () => controlApi.setManualLock(!locked)
  );
}

function doResetFault(): void {
  void runAction(tr("故障复位", "Fault reset"), () => controlApi.resetFault());
}

function doApplyTargets(): void {
  const tempMax = safetyTempMax.value ?? 100;
  const rpmMax = safetyRpmMax.value ?? 1000;
  if (sp.temperature < 0 || sp.temperature > tempMax || sp.stirrer < 0 || sp.stirrer > rpmMax) {
    ElMessage.warning(
      tr(
        `目标值超出安全范围（温度 0–${tempMax}°C，转速 0–${rpmMax}rpm）`,
        `Targets out of safety range (temp 0–${tempMax}°C, RPM 0–${rpmMax})`
      )
    );
    return;
  }
  void runAction(tr("应用设定值", "Apply targets"), async () => {
    await controlApi.updateTargets({
      temperature_c: sp.temperature,
      stirrer_rpm: sp.stirrer,
      shake_speed_cpm: sp.shake
    });
    targetsDirty.value = false;
  });
}

// 开始批次
const startVisible = ref(false);
const startProcessId = ref<number | null>(null);
const startBatchName = ref("");

function openStartDialog(): void {
  startProcessId.value = recipeProcessId.value;
  startBatchName.value = "";
  startVisible.value = true;
}

function doStartBatch(): void {
  const pid = startProcessId.value;
  if (pid === null) return;
  const name = startBatchName.value.trim();
  void runAction(tr("开始批次", "Start batch"), async () => {
    if (name) {
      await batchApi.start({ name, process_id: pid });
    } else {
      await processApi.start(pid);
    }
  });
  startVisible.value = false;
}

async function doStopProcess(): Promise<void> {
  try {
    await ElMessageBox.confirm(
      tr("确认停止当前工艺？进行中的批次将被结束。", "Stop the current process? The active batch will be finished."),
      tr("停止工艺", "Stop Process"),
      {
        confirmButtonText: tr("停止", "Stop"),
        cancelButtonText: tr("取消", "Cancel"),
        type: "warning",
        confirmButtonClass: "el-button--danger"
      }
    );
  } catch {
    return;
  }
  void runAction(tr("停止工艺", "Stop process"), () => processApi.stopCurrent());
}

// ---------------------------------------------------------------- 生命周期
onMounted(() => {
  tickTimer = window.setInterval(() => {
    nowMs.value = Date.now();
  }, 1000);

  void live.refreshLive();
  plant.loadProcesses().catch(() => undefined);
  plant.loadBatches().catch(() => undefined);
  plant.loadConfig().catch(() => undefined);
  plant.loadDeviceStatus().catch(() => undefined);
  plant.loadAudit({ page: 1, pageSize: 30 }).catch(() => undefined);
});

onBeforeUnmount(() => {
  if (tickTimer !== null) {
    window.clearInterval(tickTimer);
    tickTimer = null;
  }
});
</script>

<style scoped>
/* ============ 页面骨架：三行 grid，禁止整页滚动 ============ */
.control-page {
  height: 100%;
  min-height: 0;
  display: grid;
  grid-template-rows: minmax(170px, 30fr) minmax(124px, 13fr) minmax(230px, 57fr);
  gap: var(--spacing);
  overflow: hidden;
}

.row-top {
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(280px, 330px) minmax(0, 1fr) 200px;
  gap: var(--spacing);
}

.row-mid {
  min-height: 0;
  display: grid;
  grid-template-columns: 44fr 56fr;
  gap: var(--spacing);
}

.row-main {
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(0, 32fr) minmax(0, 23fr) minmax(0, 21fr) minmax(0, 24fr);
  gap: var(--spacing);
}

.col-safety {
  min-height: 0;
  display: grid;
  grid-template-rows: minmax(0, 42fr) minmax(0, 58fr);
  gap: var(--spacing);
}

/* ============ 批次状态 ============ */
.batch-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.run-badge {
  display: flex;
  align-items: center;
  gap: 10px;
}

.run-dot {
  width: 11px;
  height: 11px;
  border-radius: 50%;
  background: var(--ind-gray);
  flex: none;
}

.run-dot.ok {
  background: var(--ind-green);
  box-shadow: 0 0 10px var(--ind-green-glow, rgba(47, 212, 123, 0.6));
  animation: breathe 1.7s ease-in-out infinite;
}

@keyframes breathe {
  0%, 100% { opacity: 1; transform: scale(1); }
  50% { opacity: 0.45; transform: scale(0.78); }
}

.run-texts {
  display: flex;
  flex-direction: column;
  line-height: 1.1;
  flex: 1;
  min-width: 0;
}

.run-state {
  font-size: var(--fs-xl);
  font-weight: 800;
  letter-spacing: 0.5px;
}

.run-state.ok { color: var(--ind-green); }

.run-zh {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
}

.run-id {
  font-size: var(--fs-md);
  color: var(--text-secondary);
  font-weight: 700;
}

.batch-kv dt { white-space: normal; }

.completion {
  display: flex;
  flex-direction: column;
  gap: 5px;
  flex: none;
}

.completion-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
}

.completion .pct {
  font-size: var(--fs-md);
  color: var(--accent-strong);
  font-weight: 700;
}

.bar {
  height: 9px;
  border-radius: 5px;
  background: #22364f;
  overflow: hidden;
}

.bar .fill {
  height: 100%;
  border-radius: 5px;
  background: linear-gradient(90deg, var(--accent), var(--accent-cyan));
  box-shadow: 0 0 8px rgba(47, 155, 255, 0.5);
  transition: width 0.6s ease;
}

.batch-idle .idle-en {
  font-size: var(--fs-2xl);
  font-weight: 800;
  letter-spacing: 2px;
  color: var(--text-tertiary);
}

.batch-idle .idle-zh {
  font-size: var(--fs-sm);
  color: var(--text-tertiary);
}

.batch-idle svg { opacity: 0.4; }

/* ============ 目标设定值 ============ */
.dirty-tag {
  font-size: var(--fs-xs);
  color: var(--ind-amber);
  border: 1px solid rgba(245, 166, 35, 0.5);
  background: rgba(245, 166, 35, 0.12);
  border-radius: var(--radius-sm);
  padding: 2px 8px;
  font-weight: 600;
}

.live-tag {
  font-size: var(--fs-xs);
  padding: 2px 8px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border-glass);
  color: var(--text-tertiary);
  font-weight: 600;
}

.live-tag.fresh {
  color: var(--ind-green);
  border-color: rgba(47, 212, 123, 0.5);
  background: rgba(47, 212, 123, 0.1);
}

.sp-grid {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: repeat(5, minmax(0, 1fr));
  gap: 12px;
}

.sp-col {
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 7px;
  background: var(--bg-inset);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-md);
  padding: 10px 12px;
}

.sp-col.readonly { opacity: 0.88; }

.sp-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 6px;
  min-height: 32px;
}

.sp-title {
  display: flex;
  flex-direction: column;
  line-height: 1.25;
  min-width: 0;
}

.sp-title .en {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-primary);
  white-space: normal;
}

.sp-title .en .unit {
  font-style: normal;
  font-size: var(--fs-xs);
  color: var(--accent-cyan);
  margin-left: 3px;
}

.sp-title .zh {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
}

.ro-tag {
  flex: none;
  font-size: 10px;
  color: var(--text-tertiary);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-sm);
  padding: 1px 5px;
  white-space: nowrap;
}

.sp-stepper {
  display: flex;
  align-items: center;
  gap: 6px;
}

.sp-value {
  flex: 1;
  min-width: 0;
  text-align: center;
  font-family: var(--font-data);
  font-variant-numeric: tabular-nums;
  font-size: var(--fs-2xl);
  font-weight: 700;
  color: var(--text-primary);
  line-height: 1;
}

.sp-value.dim {
  color: var(--text-secondary);
  font-size: var(--fs-xl);
}

.sp-btn {
  width: 24px;
  height: 42px;
  flex: none;
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-sm);
  background: rgba(120, 180, 240, 0.06);
  color: var(--text-secondary);
  font-size: 10px;
  cursor: pointer;
  transition: color 0.15s, border-color 0.15s;
}

.sp-btn:hover:not(:disabled) {
  color: var(--accent);
  border-color: var(--accent);
}

.sp-btn:disabled {
  opacity: 0.35;
  cursor: not-allowed;
}

.sp-slider { margin: 0 2px; }

.sp-pv {
  text-align: center;
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  white-space: nowrap;
}

.sp-pv b { color: var(--text-secondary); font-weight: 600; }

/* ============ 控制模式 / 快捷操作大按钮 ============ */
.mode-row,
.quick-row {
  flex: 1;
  min-height: 0;
  display: grid;
  gap: 10px;
  align-items: stretch;
}

.mode-row { grid-template-columns: repeat(4, minmax(0, 1fr)); }
.quick-row { grid-template-columns: repeat(5, minmax(0, 1fr)); }

.big-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 9px;
  min-height: 44px;
  padding: 6px 8px;
  background: var(--bg-inset);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-md);
  color: var(--text-secondary);
  font: inherit;
  cursor: pointer;
  transition: border-color 0.15s, color 0.15s, background 0.15s;
  overflow: hidden;
}

.big-btn:hover:not(:disabled) {
  border-color: var(--border-strong);
  color: var(--text-primary);
}

.big-btn:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.big-btn .lbl {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  line-height: 1.2;
  min-width: 0;
}

.big-btn .lbl .en {
  font-size: var(--fs-sm);
  font-weight: 700;
  letter-spacing: 0.4px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 100%;
}

.big-btn .lbl .zh {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  white-space: nowrap;
}

.big-btn.green { border-color: rgba(47, 212, 123, 0.32); }
.big-btn.green.active,
.big-btn.green:hover:not(:disabled) {
  border-color: var(--ind-green);
  color: var(--ind-green);
  background: rgba(47, 212, 123, 0.1);
}

.big-btn.amber { border-color: rgba(245, 166, 35, 0.32); }
.big-btn.amber.active,
.big-btn.amber:hover:not(:disabled) {
  border-color: var(--ind-amber);
  color: var(--ind-amber);
  background: rgba(245, 166, 35, 0.1);
}

.big-btn.red { border-color: rgba(255, 82, 82, 0.32); }
.big-btn.red:hover:not(:disabled) {
  border-color: var(--ind-red);
  color: var(--ind-red);
  background: rgba(255, 82, 82, 0.1);
}

.big-btn.blue { border-color: rgba(47, 155, 255, 0.32); }
.big-btn.blue:hover:not(:disabled) {
  border-color: var(--accent);
  color: var(--accent-strong);
  background: var(--accent-dim);
}

.big-btn .dot {
  display: inline-block;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--ind-amber);
  margin-left: 5px;
  vertical-align: 2px;
}

/* ============ 工艺配方 ============ */
.recipe-wrap {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.recipe-table-wrap {
  flex: 1.3;
  min-height: 0;
}

.recipe-table { width: 100%; }

.th-en {
  display: block;
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-secondary);
  line-height: 1.2;
}

.th-zh {
  display: block;
  font-size: 10px;
  color: var(--text-tertiary);
  line-height: 1.2;
}

.pill {
  display: inline-block;
  font-size: 10px;
  font-weight: 600;
  padding: 2px 8px;
  border-radius: 9px;
  white-space: nowrap;
}

.pill.done {
  color: var(--ind-green);
  background: rgba(47, 212, 123, 0.14);
  border: 1px solid rgba(47, 212, 123, 0.45);
}

.pill.current {
  color: var(--accent-strong);
  background: var(--accent-dim);
  border: 1px solid rgba(47, 155, 255, 0.5);
}

.pill.pending {
  color: var(--text-tertiary);
  background: rgba(90, 115, 150, 0.16);
  border: 1px solid var(--ind-gray);
}

.proc-name {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  max-width: 200px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.recipe-timeline {
  flex: none;
  display: flex;
  flex-direction: column;
  gap: 7px;
  padding-top: 10px;
  border-top: 1px dashed var(--border-glass);
}

.tl-title {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  font-weight: 600;
}

.tl-track {
  position: relative;
  display: flex;
  gap: 3px;
  height: 32px;
}

.tl-seg {
  position: relative;
  min-width: 10px;
  border-radius: 4px;
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
}

.tl-seg.done { background: rgba(47, 212, 123, 0.45); }

.tl-seg.current {
  background: linear-gradient(180deg, var(--accent-strong), var(--accent));
  box-shadow: 0 0 12px rgba(47, 155, 255, 0.45);
  animation: segpulse 1.8s ease-in-out infinite;
}

@keyframes segpulse {
  0%, 100% { filter: brightness(1); }
  50% { filter: brightness(1.45); }
}

.tl-seg.pending { background: #26374e; }

.tl-seg .idx {
  font-size: 10px;
  font-weight: 700;
  color: rgba(232, 241, 251, 0.88);
  letter-spacing: 0.5px;
}

.tl-marker {
  position: absolute;
  top: -4px;
  bottom: -4px;
  width: 2px;
  background: #fff;
  box-shadow: 0 0 7px rgba(255, 255, 255, 0.85);
  border-radius: 1px;
}

.tl-legend {
  display: flex;
  gap: 14px;
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
}

.tl-legend .dot {
  display: inline-block;
  width: 9px;
  height: 9px;
  border-radius: 2px;
  margin-right: 5px;
  vertical-align: -1px;
}

.tl-legend .dot.done { background: rgba(47, 212, 123, 0.45); }
.tl-legend .dot.current { background: var(--accent); }
.tl-legend .dot.pending { background: #26374e; }

/* ============ 当前活动过程 ============ */
.ap-wrap {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.ap-head {
  flex: none;
  display: flex;
  align-items: center;
  gap: 10px;
}

.ap-icon {
  width: 40px;
  height: 40px;
  flex: none;
  border-radius: var(--radius-md);
  background: rgba(47, 155, 255, 0.12);
  border: 1px solid rgba(47, 155, 255, 0.35);
  color: var(--accent);
  display: flex;
  align-items: center;
  justify-content: center;
}

.ap-title {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  line-height: 1.25;
}

.ap-step {
  font-size: var(--fs-xs);
  font-weight: 700;
  letter-spacing: 1px;
  color: var(--accent-cyan);
}

.ap-name {
  font-size: var(--fs-md);
  font-weight: 600;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.ap-phase {
  flex: none;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  line-height: 1.3;
}

.ap-phase-val {
  font-size: var(--fs-md);
  font-weight: 700;
  color: var(--text-primary);
}

.ap-phase-val em {
  font-style: normal;
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  font-weight: 400;
  margin-left: 3px;
}

.ap-cards {
  flex: none;
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 8px;
}

.tc {
  min-width: 0;
  background: var(--bg-inset);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-md);
  padding: 7px 9px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.tc .lbl {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.tc .val {
  font-size: var(--fs-lg);
  font-weight: 700;
  color: var(--text-primary);
  white-space: nowrap;
}

.tc .val .u {
  font-style: normal;
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  font-weight: 400;
  margin-left: 3px;
}

.ap-trend {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.trend-label {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
  font-weight: 600;
}

.chart-fill {
  flex: 1;
  min-height: 70px;
  position: relative;
}

.chart-empty { padding: 12px; }

/* ============ 组件控制 ============ */
.comp-wrap {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}

.comp-list {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding-right: 2px;
}

.comp-row {
  flex: none;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 10px;
  background: var(--bg-inset);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-md);
}

.comp-icon {
  width: 34px;
  height: 34px;
  flex: none;
  border-radius: var(--radius-sm);
  background: rgba(47, 155, 255, 0.12);
  color: var(--accent);
  display: flex;
  align-items: center;
  justify-content: center;
}

.comp-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  line-height: 1.3;
}

.comp-name {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.comp-state {
  font-family: var(--font-data);
  font-size: var(--fs-xs);
  font-weight: 700;
  letter-spacing: 0.6px;
}

.comp-state.on { color: var(--ind-green); }
.comp-state.off { color: var(--text-tertiary); }
.comp-state.mid { color: var(--ind-amber); }

.comp-empty { padding: 16px; }

.comp-foot {
  flex: none;
  padding-top: 9px;
  border-top: 1px solid var(--border-glass);
  text-align: center;
}

.comp-link {
  color: var(--accent);
  font-size: var(--fs-sm);
  cursor: pointer;
  text-decoration: none;
  font-weight: 600;
}

.comp-link:hover { color: var(--accent-strong); }

/* ============ 安全边界 ============ */
.sf-table {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--fs-sm);
}

.sf-table th {
  text-align: left;
  padding: 3px 6px;
  border-bottom: 1px solid var(--border-glass);
  vertical-align: bottom;
}

.sf-table td {
  padding: 7px 6px;
  border-bottom: 1px solid rgba(74, 127, 184, 0.12);
  color: var(--text-primary);
}

.sf-param { white-space: normal; }

.sf-param .sf-zh {
  display: block;
  font-size: 10px;
  color: var(--text-tertiary);
}

.sf-status {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-xs);
  font-weight: 600;
  color: var(--text-secondary);
  white-space: nowrap;
}

.sf-empty { padding: 14px; }

/* ============ 最近控制事件 ============ */
.ev-list {
  flex: 1;
  min-height: 0;
  margin: 0;
  padding: 0 2px 0 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.ev-item {
  display: grid;
  grid-template-columns: 54px minmax(0, 1fr) auto;
  gap: 8px;
  align-items: center;
  padding: 5px 6px;
  border-radius: var(--radius-sm);
}

.ev-item:hover { background: var(--bg-hover); }

.ev-time {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
}

.ev-main {
  min-width: 0;
  display: flex;
  flex-direction: column;
  line-height: 1.25;
}

.ev-type {
  font-size: var(--fs-sm);
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.ev-val {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
}

.ev-src {
  font-size: 10px;
  font-weight: 600;
  padding: 1px 7px;
  border-radius: 9px;
  white-space: nowrap;
}

.ev-src.user {
  color: var(--accent-strong);
  border: 1px solid rgba(47, 155, 255, 0.45);
  background: var(--accent-dim);
}

.ev-src.auto {
  color: var(--ind-purple);
  border: 1px solid rgba(176, 104, 240, 0.45);
  background: rgba(176, 104, 240, 0.12);
}

.ev-empty { padding: 14px; }

/* ============ 开始批次对话框 ============ */
.start-form {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.start-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

/* ============ 急停列 ============ */
.estop-col {
  min-height: 0;
  min-width: 0;
}

/* ============ 窄屏保护：仍不允许页面滚动 ============ */
@media (max-width: 1360px) {
  .row-top { grid-template-columns: minmax(260px, 300px) minmax(0, 1fr) 180px; }
  .sp-title .en { font-size: var(--fs-xs); }
  .sp-value { font-size: var(--fs-xl); }
}
</style>
