<script setup lang="ts">
import { computed } from "vue";
import { usePlantStore } from "../stores/plant";
import { arrayAt, textAt } from "./view-utils";

const store = usePlantStore();
const batches = computed(() => arrayAt(store.live, "recent_batches"));
</script>

<template>
  <section class="view-stack">
    <div class="view-heading">
      <div>
        <p class="eyebrow">{{ store.tr("SQLite 历史 API", "SQLite History API") }}</p>
        <h1>{{ store.tr("历史数据", "History Data") }}</h1>
        <span>{{ store.tr("批次、样本和产物结果的迁移视图", "Migration view for batches, samples, and product results") }}</span>
      </div>
      <el-tag>{{ store.tr(`${batches.length} 个批次`, `${batches.length} batches`) }}</el-tag>
    </div>

    <el-table :data="batches" class="data-table">
      <el-table-column prop="id" label="ID" width="90" />
      <el-table-column prop="name" :label="store.tr('批次', 'Batch')" />
      <el-table-column prop="status" :label="store.tr('状态', 'Status')" width="140" />
      <el-table-column :label="store.tr('开始时间', 'Started')">
        <template #default="{ row }">{{ textAt(row, "started_at") }}</template>
      </el-table-column>
    </el-table>
  </section>
</template>
