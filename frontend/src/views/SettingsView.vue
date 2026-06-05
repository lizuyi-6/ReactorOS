<script setup lang="ts">
import { computed } from "vue";
import { usePlantStore } from "../stores/plant";
import { objectAt, textAt } from "./view-utils";

const store = usePlantStore();
const device = computed(() => objectAt(store.config, "device"));
const integrations = computed(() => objectAt(store.config, "integrations"));
const security = computed(() => objectAt(store.config, "data_security"));
</script>

<template>
  <section class="view-stack">
    <div class="view-heading">
      <div>
        <p class="eyebrow">System Configuration</p>
        <h1>System Settings</h1>
        <span>设备、接口、安全和权限策略</span>
      </div>
      <el-tag>{{ store.role }}</el-tag>
    </div>

    <section class="panel two-col">
      <el-descriptions :column="1" border>
        <el-descriptions-item label="Device mode">{{ textAt(store.config, "device_mode") }}</el-descriptions-item>
        <el-descriptions-item label="Device driver">{{ textAt(device, "mode") }}</el-descriptions-item>
        <el-descriptions-item label="MQTT">{{ textAt(integrations, "mqtt") }}</el-descriptions-item>
        <el-descriptions-item label="Storage security">{{ textAt(objectAt(security, "storage_encryption"), "algorithm") }}</el-descriptions-item>
      </el-descriptions>
      <div class="analysis-block">
        <h2>PRD Stack Cutover</h2>
        <p>当前页面已由 Vue 3、Element Plus、Pinia 和 Vue Router 驱动；后续会把生产静态 HMI 替换为该构建产物。</p>
      </div>
    </section>
  </section>
</template>
