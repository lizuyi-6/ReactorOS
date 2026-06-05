<script setup lang="ts">
import { computed } from "vue";
import { usePlantStore } from "../stores/plant";
import { arrayAt, textAt } from "./view-utils";

const store = usePlantStore();
const registers = computed(() => arrayAt(store.modbus, "registers"));
const coils = computed(() => arrayAt(store.modbus, "coils"));
</script>

<template>
  <section class="view-stack">
    <div class="view-heading">
      <div>
        <p class="eyebrow">tokio-modbus Migration Target</p>
        <h1>Modbus Debug</h1>
        <span>寄存器映射、调试权限和第三方接口验收</span>
      </div>
      <el-tag type="danger">Admin writes only</el-tag>
    </div>

    <section class="panel">
      <div class="panel-title">
        <h2>Holding / Input Registers</h2>
        <span>{{ registers.length }} mapped</span>
      </div>
      <el-table :data="registers" class="data-table">
        <el-table-column label="Address" width="110">
          <template #default="{ row }">{{ textAt(row, "address") }}</template>
        </el-table-column>
        <el-table-column prop="name" label="Name" />
        <el-table-column prop="access" label="Access" width="120" />
        <el-table-column prop="unit" label="Unit" width="110" />
      </el-table>
    </section>

    <section class="panel">
      <div class="panel-title">
        <h2>Coils</h2>
        <span>{{ coils.length }} mapped</span>
      </div>
      <el-table :data="coils" class="data-table">
        <el-table-column prop="address" label="Address" width="110" />
        <el-table-column prop="name" label="Name" />
        <el-table-column prop="access" label="Access" width="120" />
      </el-table>
    </section>
  </section>
</template>
