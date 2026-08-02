<template>
  <div class="control-hmi">
    <!-- 紧急停止区域：悬浮式巨大按钮 -->
    <div class="estop-section hmi-panel">
      <div class="estop-info">
        <div class="estop-icon">⚠️</div>
        <div class="estop-text">
          <h2>紧急停止系统</h2>
          <p>触发后将立即切断所有动力输出并锁定系统</p>
        </div>
      </div>
      <HmiButton type="stop" class="estop-btn" icon="🛑">
        紧急停止
      </HmiButton>
    </div>

    <div class="main-controls">
      <!-- 批次控制 -->
      <div class="control-group hmi-panel">
        <div class="hmi-panel-header">
          <span>批次生命周期</span>
          <span class="status-badge ok">自动模式</span>
        </div>
        <div class="control-grid">
          <HmiButton type="start" icon="▶️" subLabel="START BATCH">启动批次</HmiButton>
          <HmiButton type="warning" icon="⏸️" subLabel="HOLD BATCH">暂停保持</HmiButton>
          <HmiButton type="manual" icon="⏹️" subLabel="FINISH BATCH">完成批次</HmiButton>
        </div>
      </div>

      <!-- 工艺控制 -->
      <div class="control-group hmi-panel">
        <div class="hmi-panel-header">
          <span>工艺执行</span>
          <span class="status-badge">手动干预可用</span>
        </div>
        <div class="control-grid">
          <HmiButton type="manual" icon="📋" subLabel="APPLY RECIPE">应用工艺</HmiButton>
          <HmiButton type="start" icon="🔥" subLabel="RUN PROCESS">开始加热</HmiButton>
          <HmiButton type="stop" icon="🛑" subLabel="STOP PROCESS">停止工艺</HmiButton>
        </div>
      </div>

      <!-- 手动干预 -->
      <div class="control-group hmi-panel manual-panel">
        <div class="hmi-panel-header">
          <span>手动参数覆写 (OVERRIDE)</span>
        </div>
        <div class="manual-content">
          <div class="input-card">
            <div class="input-header">
              <span class="data-label">目标温度 (°C)</span>
              <span class="current-value mono">当前: 85.0</span>
            </div>
            <div class="input-action">
              <el-input-number v-model="manualTemp" :min="0" :max="150" size="large" class="hmi-input" />
              <HmiButton type="manual" class="apply-btn">写入</HmiButton>
            </div>
          </div>
          
          <div class="input-card">
            <div class="input-header">
              <span class="data-label">搅拌转速 (RPM)</span>
              <span class="current-value mono">当前: 150</span>
            </div>
            <div class="input-action">
              <el-input-number v-model="manualRpm" :min="0" :max="300" size="large" class="hmi-input" />
              <HmiButton type="manual" class="apply-btn">写入</HmiButton>
            </div>
          </div>
        </div>
      </div>
    </div>

    <!-- 底部状态反馈 -->
    <div class="feedback-bar hmi-panel">
      <div class="fb-item">
        <span class="data-label">控制模式</span>
        <span class="data-value text-blue">AUTO</span>
      </div>
      <div class="fb-item">
        <span class="data-label">阀门状态</span>
        <span class="data-value text-green">开启</span>
      </div>
      <div class="fb-item">
        <span class="data-label">加热器功率</span>
        <span class="data-value">45%</span>
      </div>
      <div class="fb-item">
        <span class="data-label">上次操作</span>
        <span class="data-value text-dim">09:42:11 - 启动批次</span>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import HmiButton from '../components/HmiButton.vue'

const manualTemp = ref(85)
const manualRpm = ref(150)
</script>

<style scoped>
.control-hmi {
  display: flex;
  flex-direction: column;
  gap: var(--spacing);
  height: 100%;
}

/* 紧急停止区域 */
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
  width: 320px;
  height: 80px;
  font-size: 24px;
  border: 2px solid var(--ind-red);
  box-shadow: 0 0 30px rgba(255, 61, 0, 0.2), var(--shadow-btn);
}

/* 主控制区 */
.main-controls {
  display: grid;
  grid-template-columns: 1fr 1fr 1.5fr;
  gap: var(--spacing);
  flex: 1;
}

.control-grid {
  display: grid;
  grid-template-columns: 1fr;
  gap: 16px;
  padding: 20px;
}

.status-badge {
  font-size: 11px;
  padding: 4px 12px;
  border-radius: 12px;
  background: rgba(255,255,255,0.1);
  color: var(--text-secondary);
}
.status-badge.ok { background: rgba(0, 200, 83, 0.2); color: var(--ind-green); }

/* 手动覆写面板 */
.manual-content {
  padding: 20px;
  display: flex;
  flex-direction: column;
  gap: 20px;
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
.apply-btn { min-width: 100px; height: 48px; font-size: 14px; }

/* 底部反馈 */
.feedback-bar {
  display: flex;
  justify-content: space-around;
  padding: 20px;
}
.fb-item { display: flex; flex-direction: column; align-items: center; gap: 8px; }
.text-blue { color: var(--ind-blue); }
.text-green { color: var(--ind-green); }
.text-dim { color: var(--text-tertiary); font-size: 14px; }

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
