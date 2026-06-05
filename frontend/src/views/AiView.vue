<script setup lang="ts">
import { computed } from "vue";
import { usePlantStore } from "../stores/plant";
import { objectAt, textAt } from "./view-utils";

const store = usePlantStore();
const localAi = computed(() => objectAt(store.config, "local_ai"));
const provider = computed(() => objectAt(store.recommendation, "provider"));
</script>

<template>
  <section class="view-stack">
    <div class="view-heading">
      <div>
        <p class="eyebrow">Qwen / LoRA Readiness</p>
        <h1>AI Decision</h1>
        <span>本地模型边界、推荐来源和人工复核状态</span>
      </div>
      <el-tag :type="textAt(localAi, 'ready_for_inference') === 'true' ? 'success' : 'warning'">
        {{ textAt(localAi, "mode", "not ready") }}
      </el-tag>
    </div>

    <section class="panel two-col">
      <el-descriptions :column="1" border>
        <el-descriptions-item label="Inference ready">{{ textAt(localAi, "ready_for_inference") }}</el-descriptions-item>
        <el-descriptions-item label="Training ready">{{ textAt(localAi, "ready_for_training") }}</el-descriptions-item>
        <el-descriptions-item label="Model path">{{ textAt(localAi, "model_path") }}</el-descriptions-item>
        <el-descriptions-item label="Adapter path">{{ textAt(localAi, "adapter_path") }}</el-descriptions-item>
      </el-descriptions>
      <div class="analysis-block">
        <h2>Latest Recommendation Provider</h2>
        <p>{{ textAt(provider, "mode", "No cached recommendation") }}</p>
        <p class="muted">{{ textAt(provider, "fallback_reason", "Waiting for AI recommendation context.") }}</p>
      </div>
    </section>
  </section>
</template>
