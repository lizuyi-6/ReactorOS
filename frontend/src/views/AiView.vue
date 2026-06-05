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
        <h1>{{ store.tr("AI 决策", "AI Decision") }}</h1>
        <span>{{ store.tr("本地模型边界、推荐来源和人工复核状态", "Local model readiness, recommendation source, and manual review state") }}</span>
      </div>
      <el-tag :type="textAt(localAi, 'ready_for_inference') === 'true' ? 'success' : 'warning'">
        {{ textAt(localAi, "mode", store.tr("未就绪", "not ready")) }}
      </el-tag>
    </div>

    <section class="panel two-col">
      <el-descriptions :column="1" border>
        <el-descriptions-item :label="store.tr('推理就绪', 'Inference ready')">{{ textAt(localAi, "ready_for_inference") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('训练就绪', 'Training ready')">{{ textAt(localAi, "ready_for_training") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('模型路径', 'Model path')">{{ textAt(localAi, "model_path") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('适配器路径', 'Adapter path')">{{ textAt(localAi, "adapter_path") }}</el-descriptions-item>
      </el-descriptions>
      <div class="analysis-block">
        <h2>{{ store.tr("最新推荐来源", "Latest Recommendation Provider") }}</h2>
        <p>{{ textAt(provider, "mode", store.tr("暂无缓存推荐", "No cached recommendation")) }}</p>
        <p class="muted">{{ textAt(provider, "fallback_reason", store.tr("等待 AI 推荐上下文。", "Waiting for AI recommendation context.")) }}</p>
      </div>
    </section>
  </section>
</template>
