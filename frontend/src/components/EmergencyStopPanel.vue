<script setup lang="ts">
// 急停面板：按住 1.5s 触发急停；急停状态下点击弹出复位确认。
import { computed, onBeforeUnmount, ref } from "vue";
import { ElMessage, ElMessageBox } from "element-plus";
import { controlApi } from "../api";
import { useLiveStore } from "../stores/live";
import { useLanguage } from "../i18n";
import { boolText } from "../utils/format";

const live = useLiveStore();
const { tr } = useLanguage();

const HOLD_MS = 1500;
const holding = ref(false);
const progress = ref(0);
const busy = ref(false);
let holdStart = 0;
let rafId: number | null = null;

const engaged = computed(() => boolText(live.runtime?.emergency_stop));

function tick(): void {
  const elapsed = Date.now() - holdStart;
  progress.value = Math.min(1, elapsed / HOLD_MS);
  if (progress.value >= 1) {
    cancelHold();
    void triggerStop();
    return;
  }
  rafId = requestAnimationFrame(tick);
}

function startHold(): void {
  if (engaged.value || busy.value) return;
  holding.value = true;
  holdStart = Date.now();
  progress.value = 0;
  rafId = requestAnimationFrame(tick);
}

function cancelHold(): void {
  holding.value = false;
  progress.value = 0;
  if (rafId !== null) cancelAnimationFrame(rafId);
  rafId = null;
}

async function triggerStop(): Promise<void> {
  busy.value = true;
  try {
    await controlApi.emergencyStop();
    ElMessage.error(tr("紧急停止已触发", "Emergency stop engaged"));
    await live.refreshLive();
  } catch (e) {
    ElMessage.error(tr("急停触发失败：", "E-stop failed: ") + String(e));
  } finally {
    busy.value = false;
  }
}

async function handleClick(): Promise<void> {
  if (!engaged.value || busy.value) return;
  try {
    await ElMessageBox.confirm(
      tr("确认复位紧急停止？复位后需人工确认现场安全。", "Reset emergency stop? Verify field safety first."),
      tr("复位急停", "Reset E-Stop"),
      { confirmButtonText: tr("复位", "Reset"), cancelButtonText: tr("取消", "Cancel"), type: "warning" }
    );
  } catch {
    return;
  }
  busy.value = true;
  try {
    await controlApi.resetEmergencyStop();
    ElMessage.success(tr("急停已复位", "E-stop reset"));
    await live.refreshLive();
  } catch (e) {
    ElMessage.error(tr("复位失败：", "Reset failed: ") + String(e));
  } finally {
    busy.value = false;
  }
}

onBeforeUnmount(cancelHold);
</script>

<template>
  <section class="estop-panel" :class="{ engaged }">
    <header class="estop-title">
      <span class="en">EMERGENCY STOP</span>
      <span class="zh">{{ tr("紧急停止", "E-STOP") }}</span>
    </header>

    <button
      class="estop-button"
      :class="{ holding, engaged }"
      :disabled="busy"
      @pointerdown="startHold"
      @pointerup="cancelHold"
      @pointerleave="cancelHold"
      @click="handleClick"
    >
      <svg class="estop-ring" viewBox="0 0 100 100">
        <circle cx="50" cy="50" r="44" class="ring-bg" />
        <circle
          cx="50" cy="50" r="44"
          class="ring-fg"
          :stroke-dasharray="String(progress * 276.5) + ' 276.5'"
        />
      </svg>
      <svg class="estop-symbol" viewBox="0 0 24 24" width="34" height="34" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round">
        <path d="M18.4 6.6a9 9 0 1 1-12.8 0" />
        <path d="M12 2v9" />
      </svg>
    </button>

    <p class="estop-hint">
      {{ engaged
        ? tr("急停已触发 · 点击复位", "Engaged · click to reset")
        : tr("按住以立即停止所有反应釜操作", "Press and hold to immediately stop all reactor operations") }}
    </p>
  </section>
</template>

<style scoped>
.estop-panel {
  background: linear-gradient(160deg, rgba(198, 47, 59, 0.22), rgba(120, 20, 30, 0.3));
  border: 1px solid rgba(255, 82, 82, 0.45);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-panel), inset 0 0 40px rgba(255, 82, 82, 0.08);
  padding: 14px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  min-height: 0;
}
.estop-panel.engaged {
  border-color: var(--ind-red);
  box-shadow: var(--shadow-panel), 0 0 24px rgba(255, 82, 82, 0.35);
}
.estop-title {
  text-align: center;
  line-height: 1.3;
}
.estop-title .en {
  display: block;
  color: var(--ind-red);
  font-weight: 800;
  font-size: 15px;
  letter-spacing: 1.5px;
}
.estop-title .zh {
  color: #ff9d9d;
  font-size: 13px;
  font-weight: 600;
}
.estop-button {
  position: relative;
  width: 108px;
  height: 108px;
  border-radius: 50%;
  border: 3px solid #ff6b6b;
  background: radial-gradient(circle at 35% 30%, #e5484d, #8f1d26 75%);
  color: #fff;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 6px 22px rgba(198, 47, 59, 0.5), inset 0 2px 6px rgba(255, 255, 255, 0.25);
  transition: transform 0.1s;
  flex: none;
}
.estop-button:active { transform: scale(0.96); }
.estop-button.engaged { animation: estop-blink 1.2s infinite; }
@keyframes estop-blink {
  0%, 100% { box-shadow: 0 6px 22px rgba(198, 47, 59, 0.5), 0 0 0 0 rgba(255, 82, 82, 0.5); }
  50% { box-shadow: 0 6px 22px rgba(198, 47, 59, 0.5), 0 0 0 12px rgba(255, 82, 82, 0); }
}
/* V30：进度环收入按钮盒内（原 inset:-9px 造成 18px 外溢被检出） */
.estop-ring { position: absolute; inset: 0; width: 100%; height: 100%; transform: rotate(-90deg); }
.ring-bg { fill: none; stroke: rgba(255, 255, 255, 0.15); stroke-width: 4; }
.ring-fg { fill: none; stroke: #ffd166; stroke-width: 4; stroke-linecap: round; }
.estop-symbol { position: relative; }
.estop-hint {
  margin: 0;
  text-align: center;
  font-size: 11px;
  color: #ffb3b3;
  line-height: 1.5;
}
</style>
