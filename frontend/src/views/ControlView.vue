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
        <p class="eyebrow">Element Plus Forms</p>
        <h1>Process Control</h1>
        <span>参数配置、安全限幅和执行前复核</span>
      </div>
      <el-tag type="info">Read-only migration slice</el-tag>
    </div>

    <section class="panel two-col">
      <div>
        <h2>Safety Envelope</h2>
        <p>Vue 首版先展示后端安全边界，后续再接入 set_targets 表单和操作员确认流。</p>
      </div>
      <el-descriptions :column="1" border>
        <el-descriptions-item label="Temperature max">{{ textAt(temperature, "max_c") }} C</el-descriptions-item>
        <el-descriptions-item label="Temperature min">{{ textAt(temperature, "min_c") }} C</el-descriptions-item>
        <el-descriptions-item label="Stirrer max">{{ textAt(stirrer, "max_rpm") }} rpm</el-descriptions-item>
        <el-descriptions-item label="Current role">{{ store.role }}</el-descriptions-item>
      </el-descriptions>
    </section>
  </section>
</template>
