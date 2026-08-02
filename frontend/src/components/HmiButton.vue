<template>
  <button
    class="hmi-btn"
    :class="[type, { 'is-disabled': disabled }]"
    :disabled="disabled"
    @click="$emit('click')"
  >
    <div class="btn-inner">
      <span v-if="icon" class="btn-icon">{{ icon }}</span>
      <div class="btn-text">
        <span class="btn-label"><slot /></span>
        <span v-if="subLabel" class="btn-sub">{{ subLabel }}</span>
      </div>
    </div>
    <!-- 工业状态光效 -->
    <div class="btn-glow"></div>
  </button>
</template>

<script setup lang="ts">
defineProps<{
  type?: 'start' | 'stop' | 'warning' | 'manual' | 'default'
  disabled?: boolean
  icon?: string
  subLabel?: string
}>()

defineEmits(['click'])
</script>

<style scoped>
.hmi-btn {
  position: relative;
  min-height: var(--touch-target);
  min-width: 140px;
  padding: 12px 24px;
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-md);
  font-family: var(--font-ui);
  font-size: var(--fs-lg);
  font-weight: 600;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
  transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
  box-shadow: var(--shadow-btn);
  text-transform: uppercase;
  letter-spacing: 1px;
  color: var(--text-primary);
  background: var(--bg-panel-solid);
}

/* 物理按压感 */
.hmi-btn:active {
  transform: translateY(2px);
  box-shadow: var(--shadow-btn-active);
}

.hmi-btn.is-disabled {
  opacity: 0.4;
  cursor: not-allowed;
  filter: grayscale(0.8);
  box-shadow: none;
}

.btn-inner {
  display: flex;
  align-items: center;
  gap: 16px;
  z-index: 2;
}

.btn-text {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  text-align: left;
}

.btn-icon { font-size: 28px; }
.btn-sub {
  font-size: 10px;
  font-weight: 400;
  opacity: 0.7;
  letter-spacing: 0.5px;
  margin-top: 2px;
}

/* 语义色彩与光效 */
.hmi-btn.start {
  border-color: rgba(0, 200, 83, 0.3);
  background: linear-gradient(135deg, rgba(0, 200, 83, 0.1), rgba(0, 200, 83, 0.2));
  color: var(--ind-green);
}
.hmi-btn.start:hover { border-color: var(--ind-green); box-shadow: 0 0 20px var(--ind-green-glow), var(--shadow-btn); }
.hmi-btn.start .btn-glow { background: var(--ind-green); }

.hmi-btn.stop {
  border-color: rgba(255, 61, 0, 0.3);
  background: linear-gradient(135deg, rgba(255, 61, 0, 0.1), rgba(255, 61, 0, 0.2));
  color: var(--ind-red);
}
.hmi-btn.stop:hover { border-color: var(--ind-red); box-shadow: 0 0 20px var(--ind-red-glow), var(--shadow-btn); }
.hmi-btn.stop .btn-glow { background: var(--ind-red); }

.hmi-btn.warning {
  border-color: rgba(255, 171, 0, 0.3);
  background: linear-gradient(135deg, rgba(255, 171, 0, 0.1), rgba(255, 171, 0, 0.2));
  color: var(--ind-amber);
}
.hmi-btn.warning:hover { border-color: var(--ind-amber); box-shadow: 0 0 20px var(--ind-amber-glow), var(--shadow-btn); }
.hmi-btn.warning .btn-glow { background: var(--ind-amber); }

.hmi-btn.manual {
  border-color: rgba(41, 121, 255, 0.3);
  background: linear-gradient(135deg, rgba(41, 121, 255, 0.1), rgba(41, 121, 255, 0.2));
  color: var(--ind-blue);
}
.hmi-btn.manual:hover { border-color: var(--ind-blue); box-shadow: 0 0 20px var(--ind-blue-glow), var(--shadow-btn); }
.hmi-btn.manual .btn-glow { background: var(--ind-blue); }

/* 内部呼吸光效 */
.btn-glow {
  position: absolute;
  top: -50%; left: -50%; width: 200%; height: 200%;
  background: radial-gradient(circle, currentColor 0%, transparent 70%);
  opacity: 0.05;
  pointer-events: none;
  transition: opacity 0.3s;
}
.hmi-btn:hover .btn-glow { opacity: 0.15; }
</style>
