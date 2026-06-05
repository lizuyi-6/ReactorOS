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
        <p class="eyebrow">SQLite History API</p>
        <h1>History Data</h1>
        <span>批次、样本和产物结果的迁移视图</span>
      </div>
      <el-tag>{{ batches.length }} batches</el-tag>
    </div>

    <el-table :data="batches" class="data-table">
      <el-table-column prop="id" label="ID" width="90" />
      <el-table-column prop="name" label="Batch" />
      <el-table-column prop="status" label="Status" width="140" />
      <el-table-column label="Started">
        <template #default="{ row }">{{ textAt(row, "started_at") }}</template>
      </el-table-column>
    </el-table>
  </section>
</template>
