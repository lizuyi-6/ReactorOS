<script setup lang="ts">
import { computed } from "vue";
import { usePlantStore } from "../stores/plant";
import { arrayAt, objectAt, textAt } from "./view-utils";

const store = usePlantStore();
const events = computed(() => arrayAt(store.audit, "events"));
const chain = computed(() => objectAt(store.audit, "chain"));
</script>

<template>
  <section class="view-stack">
    <div class="view-heading">
      <div>
        <p class="eyebrow">{{ store.tr("防篡改链", "Tamper-evident Chain") }}</p>
        <h1>{{ store.tr("审计日志", "Audit Log") }}</h1>
        <span>{{ store.tr("操作事件、哈希链和导出权限", "Operation events, hash chain, and export permissions") }}</span>
      </div>
      <el-tag :type="textAt(chain, 'valid') === 'true' ? 'success' : 'warning'">
        {{ textAt(chain, "status", store.tr("链窗口", "chain window")) }}
      </el-tag>
    </div>

    <el-table :data="events" class="data-table">
      <el-table-column prop="id" label="#" width="80" />
      <el-table-column prop="created_at" :label="store.tr('时间', 'Time')" width="190" />
      <el-table-column prop="event_type" :label="store.tr('事件', 'Event')" />
      <el-table-column prop="reason" :label="store.tr('原因', 'Reason')" />
    </el-table>
  </section>
</template>
