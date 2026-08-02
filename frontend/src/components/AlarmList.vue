<script setup lang="ts">
import type { Alarm } from "../api/types";
import { useLanguage } from "../i18n";
import { alarmLevelLabel, alarmMessage, alarmSuggestion, alarmTone, alarmTypeLabel } from "../utils/alarms";

defineProps<{
  alarms: Alarm[];
  maxItems?: number;
}>();

const { tr } = useLanguage();
</script>

<template>
  <div v-if="alarms.length === 0" class="no-alarms">
    <span class="status-dot ok"></span>
    {{ tr("当前无活动报警", "No active alarms") }}
  </div>
  <ul v-else class="alarm-list">
    <li v-for="(alarm, index) in alarms.slice(0, maxItems ?? alarms.length)" :key="index" class="alarm-item">
      <el-tag :type="alarmTone(alarm)" size="small" effect="dark" class="alarm-tag">
        {{ alarmTypeLabel(alarm, tr) }}
      </el-tag>
      <div class="alarm-body">
        <div class="alarm-message">{{ alarmMessage(alarm) }}</div>
        <div v-if="alarmSuggestion(alarm)" class="alarm-suggestion muted">{{ alarmSuggestion(alarm) }}</div>
      </div>
      <span class="alarm-level muted">{{ alarmLevelLabel(alarm, tr) }}</span>
    </li>
  </ul>
</template>

<style scoped>
.no-alarms {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  color: var(--text-secondary);
  font-size: var(--text-sm);
  padding: var(--space-2) 0;
}

.alarm-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.alarm-item {
  display: flex;
  align-items: flex-start;
  gap: var(--space-3);
  padding: var(--space-2) var(--space-3);
  border-radius: var(--radius-md);
  background: var(--bg-surface-2);
  border: 1px solid var(--border-subtle);
}

.alarm-tag {
  flex-shrink: 0;
  margin-top: 1px;
}

.alarm-body {
  flex: 1;
  min-width: 0;
}

.alarm-message {
  font-size: var(--text-sm);
  color: var(--text-primary);
  overflow-wrap: anywhere;
}

.alarm-suggestion {
  font-size: var(--text-xs);
  margin-top: 2px;
  overflow-wrap: anywhere;
}

.alarm-level {
  font-size: var(--text-xs);
  flex-shrink: 0;
}
</style>
