<template>
  <div class="control-hmi">
    <!-- 急停区 -->
    <div class="estop-section hmi-panel">
      <div class="estop-info">
        <div class="estop-icon">⚠️</div>
        <div class="estop-text">
          <h2>{{ tr("紧急停止系统", "Emergency Stop System") }}</h2>
          <p>{{ tr("触发后将立即下发安全目标并锁定系统", "Triggers safety targets and locks the system") }}</p>
        </div>
      </div>
      <HmiButton
        v-if="!isEmergencyStopped"
        type="stop"
        class="estop-btn"
        icon="🛑"
        subLabel="EMERGENCY STOP"
        @click="doEmergencyStop"
      >{{ tr("紧急停止", "EMERGENCY STOP") }}</HmiButton>
      <HmiButton
        v-else
        type="start"
        class="estop-btn"
        icon="🔄"
        subLabel="RESET E-STOP"
        @click="doResetEmergencyStop"
      >{{ tr("复位急停", "RESET E-STOP") }}</HmiButton>
    </div>

    <!-- 操作反馈 -->
    <el-alert
      v-if="lastError"
      :title="lastError"
      type="error"
      show-icon
      :closable="true"
      @close="lastError = null"
    />
    <el-alert
      v-if="lastAction"
      :title="lastAction"
      type="success"
      show-icon
      :closable="true"
      @close="lastAction = null"
    />

    <div class="main-controls">
      <!-- 运行控制 -->
      <div class="control-group hmi-panel">
        <div class="hmi-panel-header">
          <span>{{ tr("运行控制", "Run Control") }}</span>
          <span class="status-badge" :class="modeBadgeClass">{{ modeLabel }}</span>
        </div>
        <div class="switch-grid">
          <div class="switch-row">
            <span class="switch-label">{{ tr("自动控制", "Auto Control") }}</span>
            <el-switch
              :model-value="!!runtime?.auto_enabled"
              :disabled="submitting || isEmergencyStopped"
              @change="doToggleAuto"
            />
          </div>
          <div class="switch-row">
            <span class="switch-label">{{ tr("人工锁定", "Manual Lock") }}</span>
            <el-switch
              :model-value="!!runtime?.manual_lock"
              :disabled="submitting"
              @change="doToggleLock"
            />
          </div>
        </div>
        <div v-if="faultMessage" class="fault-box">
          <span class="fault-label">{{ tr("控制故障", "Control Fault") }}</span>
          <span class="fault-text">{{ faultMessage }}</span>
        </div>
        <HmiButton
          v-if="faultMessage"
          type="warning"
          icon="⚠️"
          :disabled="submitting"
          @click="doResetFault"
        >{{ tr("故障复位", "Fault Reset") }}</HmiButton>
      </div>

      <!-- 目标覆写 -->
      <div class="control-group hmi-panel manual-panel">
        <div class="hmi-panel-header">
          <span>{{ tr("手动参数覆写", "Manual Override") }} (OVERRIDE)</span>
        </div>
        <div class="manual-content">
          <div class="input-card">
            <div class="input-header">
              <span class="data-label">{{ tr("目标温度 (°C)", "Target Temp (°C)") }}</span>
              <span class="current-value mono">{{ tr("当前", "Current") }}: {{ currentTemp }}</span>
            </div>
            <div class="input-action">
              <el-input-number
                v-model="manualTemp"
                :min="tempMin"
                :max="tempMax"
                :step="1"
                size="large"
                class="hmi-input"
                :disabled="submitting"
                @focus="editingTargets = true"
                @blur="editingTargets = false"
              />
              <HmiButton
                type="manual"
                class="apply-btn"
                icon="📥"
                :disabled="submitting"
                @click="doUpdateTargets"
              >{{ tr("写入", "Write") }}</HmiButton>
            </div>
            <div class="input-range">{{ tr("合法范围", "Valid range") }} {{ tempMin }} – {{ tempMax }} °C</div>
          </div>

          <div class="input-card">
            <div class="input-header">
              <span class="data-label">{{ tr("搅拌转速 (RPM)", "Stirrer RPM") }}</span>
              <span class="current-value mono">{{ tr("当前", "Current") }}: {{ currentRpm }}</span>
            </div>
            <div class="input-action">
              <el-input-number
                v-model="manualRpm"
                :min="rpmMin"
                :max="rpmMax"
                :step="10"
                size="large"
                class="hmi-input"
                :disabled="submitting"
                @focus="editingTargets = true"
                @blur="editingTargets = false"
              />
            </div>
            <div class="input-range">{{ tr("合法范围", "Valid range") }} {{ rpmMin }} – {{ rpmMax }} RPM</div>
          </div>
        </div>
      </div>

      <!-- 批次/工艺 -->
      <div class="control-group hmi-panel">
        <div class="hmi-panel-header">
          <span>{{ tr("批次 / 工艺", "Batch / Process") }}</span>
        </div>
        <div class="batch-info">
          <div class="info-row">
            <span class="info-label">{{ tr("活动批次", "Active Batch") }}</span>
            <span class="info-value">{{ activeBatchLabel }}</span>
          </div>
          <div class="info-row">
            <span class="info-label">{{ tr("安全状态", "Safety") }}</span>
            <span class="info-value" :class="safetyClass">{{ safetyLabel }}</span>
          </div>
        </div>
        <div class="control-grid">
          <HmiButton
            type="manual"
            icon="⏹️"
            subLabel="FINISH BATCH"
            :disabled="submitting || activeBatchId === null"
            @click="doFinishBatch"
          >{{ tr("完成批次", "Finish Batch") }}</HmiButton>
          <HmiButton
            type="stop"
            icon="🛑"
            subLabel="STOP PROCESS"
            :disabled="submitting"
            @click="doStopProcess"
          >{{ tr("停止工艺", "Stop Process") }}</HmiButton>
        </div>
      </div>
    </div>

    <!-- 底部状态反馈 -->
    <div class="feedback-bar hmi-panel">
      <div class="fb-item">
        <span class="data-label">{{ tr("控制模式", "Mode") }}</span>
        <span class="data-value" :class="modeClass">{{ modeLabel }}</span>
      </div>
      <div class="fb-item">
        <span class="data-label">{{ tr("安全联锁", "Safety") }}</span>
        <span class="data-value" :class="safetyClass">{{ safetyLabel }}</span>
      </div>
      <div class="fb-item">
        <span class="data-label">{{ tr("控制环", "Loop") }}</span>
        <span class="data-value" :class="loopClass">{{ loopLabel }}</span>
      </div>
      <div class="fb-item">
        <span class="data-label">{{ tr("活动批次", "Batch") }}</span>
        <span class="data-value">{{ activeBatchLabel }}</span>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue';
import HmiButton from '../components/HmiButton.vue';
import { useLiveStore } from '../stores/live';
import { usePlantStore } from '../stores/plant';
import { controlApi, batchApi, processApi } from '../api';
import { errorMessage } from '../api/errors';
import { useLanguage } from '../i18n';
import type { ControlTargets } from '../api/types';

const live = useLiveStore();
const plant = usePlantStore();
const { tr } = useLanguage();

const runtime = computed(() => live.runtime);
const safety = computed(() => plant.config?.safety);

const tempMin = computed(() => safety.value?.temperature?.min_c ?? 0);
const tempMax = computed(() => safety.value?.temperature?.max_c ?? 150);
const rpmMin = computed(() => safety.value?.stirrer?.min_rpm ?? 0);
const rpmMax = computed(() => safety.value?.stirrer?.max_rpm ?? 300);

const manualTemp = ref(85);
const manualRpm = ref(150);

const submitting = ref(false);
const editingTargets = ref(false);
const lastError = ref<string | null>(null);
const lastAction = ref<string | null>(null);

const isEmergencyStopped = computed(() => !!runtime.value?.emergency_stop);
const activeBatchId = computed(() => {
  const raw = runtime.value?.active_batch_id;
  if (raw === null || raw === undefined) return null;
  const n = Number(raw);
  return Number.isFinite(n) ? n : null;
});
const activeBatchLabel = computed(() =>
  activeBatchId.value === null ? tr("无", "None") : `#${activeBatchId.value}`
);

const currentTemp = computed(() => {
  const v = runtime.value?.targets?.temperature_c;
  return v === null || v === undefined ? '—' : v;
});
const currentRpm = computed(() => {
  const v = runtime.value?.targets?.stirrer_rpm;
  return v === null || v === undefined ? '—' : v;
});

const faultMessage = computed(
  () => runtime.value?.last_control_error || runtime.value?.last_sensor_error || ''
);

const modeLabel = computed(() => {
  if (isEmergencyStopped.value) return tr("急停", "E-STOP");
  if (runtime.value?.manual_lock) return tr("人工锁", "M-LOCK");
  if (runtime.value?.auto_enabled) return tr("自动", "AUTO");
  return tr("手动", "MANUAL");
});
const modeClass = computed(() => {
  if (isEmergencyStopped.value || runtime.value?.manual_lock) return 'text-red';
  if (runtime.value?.auto_enabled) return 'text-green';
  return 'text-blue';
});
const modeBadgeClass = computed(() =>
  isEmergencyStopped.value || runtime.value?.manual_lock ? 'bad' : 'ok'
);

const safetyLabel = computed(() => {
  if (runtime.value?.control_loop_terminated) return tr("控制环终止", "LOOP STOP");
  if (isEmergencyStopped.value) return tr("急停中", "E-STOP");
  if (runtime.value?.last_sensor_error) return tr("传感器故障", "SENSOR");
  if (runtime.value?.last_control_error) return tr("控制故障", "CTRL FLT");
  if (runtime.value?.manual_lock) return tr("人工锁定", "M-LOCK");
  return tr("正常", "OK");
});
const safetyClass = computed(() => {
  if (
    runtime.value?.control_loop_terminated ||
    isEmergencyStopped.value ||
    runtime.value?.last_sensor_error
  )
    return 'text-red';
  if (runtime.value?.last_control_error || runtime.value?.manual_lock) return 'text-yellow';
  return 'text-green';
});

const loopLabel = computed(() =>
  runtime.value?.control_loop_terminated ? tr("终止", "Stopped") : tr("运行中", "Running")
);
const loopClass = computed(() =>
  runtime.value?.control_loop_terminated ? 'text-red' : 'text-green'
);

watch(
  () => runtime.value?.targets as ControlTargets | undefined,
  (targets) => {
    if (!targets || editingTargets.value) return;
    if (typeof targets.temperature_c === 'number') manualTemp.value = targets.temperature_c;
    if (typeof targets.stirrer_rpm === 'number') manualRpm.value = targets.stirrer_rpm;
  },
  { immediate: true }
);

async function refresh() {
  try {
    await live.refreshLive();
  } catch {
    // refresh 错误由 store 层处理
  }
}

async function runAction(label: string, fn: () => Promise<unknown>) {
  submitting.value = true;
  lastError.value = null;
  lastAction.value = null;
  try {
    await fn();
    await refresh();
    lastAction.value = `${label}${tr(" 完成", " done")}`;
  } catch (e) {
    lastError.value = `${label}${tr(" 失败: ", " failed: ")}${errorMessage(e)}`;
  } finally {
    submitting.value = false;
  }
}

function doEmergencyStop() {
  void runAction(tr("紧急停止", "Emergency stop"), () => controlApi.emergencyStop());
}
function doResetEmergencyStop() {
  void runAction(tr("复位急停", "Reset e-stop"), () => controlApi.resetEmergencyStop());
}
function doUpdateTargets() {
  void runAction(tr("写入目标", "Write targets"), () =>
    controlApi.updateTargets({ temperature_c: manualTemp.value, stirrer_rpm: manualRpm.value })
  );
}
function doToggleAuto(enabled: boolean | string | number) {
  void runAction(enabled ? tr("启用自动", "Enable auto") : tr("关闭自动", "Disable auto"), () => controlApi.setAuto(!!enabled));
}
function doToggleLock(locked: boolean | string | number) {
  void runAction(locked ? tr("人工锁定", "Lock") : tr("解除锁定", "Unlock"), () =>
    controlApi.setManualLock(!!locked)
  );
}
function doResetFault() {
  void runAction(tr("故障复位", "Fault reset"), () => controlApi.resetFault());
}
function doFinishBatch() {
  if (activeBatchId.value === null) return;
  void runAction(tr("完成批次", "Finish batch"), () => batchApi.finish(activeBatchId.value!));
}
function doStopProcess() {
  void runAction(tr("停止工艺", "Stop process"), () => processApi.stopCurrent());
}

onMounted(() => {
  if (!plant.config) void plant.loadConfig();
});
</script>

<style scoped>
.control-hmi {
  display: flex;
  flex-direction: column;
  gap: var(--spacing);
  height: 100%;
}

/* 急停区 */
.estop-section {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 24px 32px;
  background: linear-gradient(90deg, rgba(255, 61, 0, 0.05), rgba(22, 30, 39, 0.7));
  border: 1px solid rgba(255, 61, 0, 0.2);
}
.estop-info { display: flex; align-items: center; gap: 24px; }
.estop-icon { font-size: 48px; }
.estop-text h2 { margin: 0; font-size: 24px; color: var(--ind-red); letter-spacing: 1px; }
.estop-text p { margin: 4px 0 0; color: var(--text-secondary); font-size: 14px; }
.estop-btn {
  min-width: 200px;
  min-height: 64px;
  font-size: 20px;
}

/* 主控制区 */
.main-controls {
  display: grid;
  grid-template-columns: 1fr 1.5fr 1fr;
  gap: var(--spacing);
  flex: 1;
}
@media (max-width: 900px) {
  .main-controls {
    grid-template-columns: 1fr;
  }
}

.control-group {
  padding: 16px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.hmi-panel-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: var(--fs-md);
  font-weight: 700;
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 1px;
  border-bottom: 1px solid var(--border-glass);
  padding-bottom: 8px;
}
.status-badge {
  font-size: 11px;
  padding: 4px 12px;
  border-radius: 12px;
  background: rgba(255,255,255,0.1);
  color: var(--text-secondary);
}
.status-badge.ok { background: rgba(0, 200, 83, 0.2); color: var(--ind-green); }
.status-badge.bad { background: rgba(255, 61, 0, 0.2); color: var(--ind-red); }

/* 开关 */
.switch-grid {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.switch-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 0;
}
.switch-label {
  font-size: var(--fs-md);
  font-weight: 500;
  color: var(--text-primary);
}

/* 故障 */
.fault-box {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 8px 12px;
  background: rgba(255, 61, 0, 0.08);
  border: 1px solid rgba(255, 61, 0, 0.2);
  border-radius: var(--radius-sm);
}
.fault-label {
  font-size: var(--fs-xs);
  font-weight: 700;
  color: var(--ind-red);
  text-transform: uppercase;
}
.fault-text {
  font-size: var(--fs-sm);
  color: var(--text-primary);
  word-break: break-all;
}

/* 手动覆写面板 */
.manual-content {
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.input-card {
  background: rgba(255,255,255,0.03);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-md);
  padding: 16px;
}
.input-header {
  display: flex;
  justify-content: space-between;
  margin-bottom: 12px;
}
.current-value { color: var(--text-tertiary); font-size: 13px; }
.input-action {
  display: flex;
  gap: 12px;
}
.apply-btn { min-width: 100px; min-height: 48px; }
.input-range {
  margin-top: 6px;
  font-size: 12px;
  color: var(--text-tertiary);
}

/* 批次信息 */
.batch-info {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.info-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
}
.info-label { font-size: var(--fs-sm); color: var(--text-secondary); }
.info-value {
  font-size: var(--fs-md);
  font-weight: 600;
  font-family: var(--font-mono);
  color: var(--text-primary);
}
.control-grid {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

/* 底部反馈 */
.feedback-bar {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 12px;
  padding: 16px 20px;
}
@media (max-width: 600px) {
  .feedback-bar {
    grid-template-columns: repeat(2, 1fr);
  }
}
.fb-item {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
}
.data-label {
  font-size: 11px;
  color: var(--text-tertiary);
  text-transform: uppercase;
  letter-spacing: 0.5px;
}
.data-value {
  font-size: var(--fs-md);
  font-weight: 700;
  font-family: var(--font-mono);
  color: var(--text-primary);
}

/* 语义色彩 */
.text-red { color: var(--ind-red); }
.text-green { color: var(--ind-green); }
.text-yellow { color: var(--ind-amber); }
.text-blue { color: var(--ind-blue); }
.mono { font-family: var(--font-mono); }

/* 覆盖 Element Plus 样式 */
:deep(.hmi-input) { width: 100%; }
:deep(.hmi-input .el-input__wrapper) {
  background: var(--bg-inset);
  box-shadow: none;
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-md);
  padding: 4px 12px;
}
:deep(.hmi-input .el-input__inner) {
  font-size: 24px;
  font-weight: 700;
  font-family: var(--font-data);
  color: var(--text-primary);
  height: 48px;
}
:deep(.hmi-input .el-input-number__decrease),
:deep(.hmi-input .el-input-number__increase) {
  background: rgba(255,255,255,0.05);
  border: none;
  color: var(--text-secondary);
  width: 40px;
}
</style>
