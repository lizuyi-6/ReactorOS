<script setup lang="ts">
import { computed } from "vue";
import { usePlantStore } from "../stores/plant";
import { objectAt, textAt } from "./view-utils";

const store = usePlantStore();
const safety = computed(() => objectAt(store.config, "safety"));
const temperature = computed(() => objectAt(safety.value, "temperature"));
const stirrer = computed(() => objectAt(safety.value, "stirrer"));
</script>

<template>
  <section class="view-stack">
    <div class="view-heading">
      <div>
        <p class="eyebrow">{{ store.tr("Element Plus 表单", "Element Plus Forms") }}</p>
        <h1>{{ store.tr("参数配置", "Process Control") }}</h1>
        <span>{{ store.tr("参数配置、安全限幅和执行前复核", "Parameter setup, safety limits, and pre-execution review") }}</span>
      </div>
      <el-tag type="info">{{ store.tr("只读迁移切片", "Read-only migration slice") }}</el-tag>
    </div>

    <section class="panel two-col">
      <div>
        <h2>{{ store.tr("安全边界", "Safety Envelope") }}</h2>
        <p>{{ store.tr("Vue 首版先展示后端安全边界，后续再接入 set_targets 表单和操作员确认流。", "This first Vue slice exposes the backend safety envelope; set_targets forms and operator confirmation will be wired in next.") }}</p>
      </div>
      <el-descriptions :column="1" border>
        <el-descriptions-item :label="store.tr('温度上限', 'Temperature max')">{{ textAt(temperature, "max_c") }} C</el-descriptions-item>
        <el-descriptions-item :label="store.tr('温度下限', 'Temperature min')">{{ textAt(temperature, "min_c") }} C</el-descriptions-item>
        <el-descriptions-item :label="store.tr('搅拌上限', 'Stirrer max')">{{ textAt(stirrer, "max_rpm") }} rpm</el-descriptions-item>
        <el-descriptions-item :label="store.tr('当前角色', 'Current role')">{{ store.role }}</el-descriptions-item>
      </el-descriptions>
    </section>
  </section>
</template>
