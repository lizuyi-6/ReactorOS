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
        <p class="eyebrow">Tamper-evident Chain</p>
        <h1>Audit Log</h1>
        <span>操作事件、哈希链和导出权限</span>
      </div>
      <el-tag :type="textAt(chain, 'valid') === 'true' ? 'success' : 'warning'">
        {{ textAt(chain, "status", "chain window") }}
      </el-tag>
    </div>

    <el-table :data="events" class="data-table">
      <el-table-column prop="id" label="#" width="80" />
      <el-table-column prop="created_at" label="Time" width="190" />
      <el-table-column prop="event_type" label="Event" />
      <el-table-column prop="reason" label="Reason" />
    </el-table>
  </section>
</template>
