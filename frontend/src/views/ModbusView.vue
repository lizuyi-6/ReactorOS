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
        <h1>{{ store.tr("Modbus 调试", "Modbus Debug") }}</h1>
        <span>{{ store.tr("寄存器映射、调试权限和第三方接口验收", "Register map, debug permissions, and third-party interface acceptance") }}</span>
      </div>
      <el-tag type="danger">{{ store.tr("仅管理员可写", "Admin writes only") }}</el-tag>
    </div>

    <section class="panel">
      <div class="panel-title">
        <h2>{{ store.tr("保持/输入寄存器", "Holding / Input Registers") }}</h2>
        <span>{{ store.tr(`${registers.length} 个映射`, `${registers.length} mapped`) }}</span>
      </div>
      <el-table :data="registers" class="data-table">
        <el-table-column :label="store.tr('地址', 'Address')" width="110">
          <template #default="{ row }">{{ textAt(row, "address") }}</template>
        </el-table-column>
        <el-table-column prop="name" :label="store.tr('名称', 'Name')" />
        <el-table-column prop="access" :label="store.tr('访问', 'Access')" width="120" />
        <el-table-column prop="unit" :label="store.tr('单位', 'Unit')" width="110" />
      </el-table>
    </section>

    <section class="panel">
      <div class="panel-title">
        <h2>{{ store.tr("线圈", "Coils") }}</h2>
        <span>{{ store.tr(`${coils.length} 个映射`, `${coils.length} mapped`) }}</span>
      </div>
      <el-table :data="coils" class="data-table">
        <el-table-column prop="address" :label="store.tr('地址', 'Address')" width="110" />
        <el-table-column prop="name" :label="store.tr('名称', 'Name')" />
        <el-table-column prop="access" :label="store.tr('访问', 'Access')" width="120" />
      </el-table>
    </section>
  </section>
</template>
