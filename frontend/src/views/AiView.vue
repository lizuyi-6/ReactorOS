<script setup lang="ts">
import { computed, reactive, ref } from "vue";
import { usePlantStore } from "../stores/plant";
import { arrayAt, numberAt, objectAt, textAt } from "./view-utils";
import type { AiControlRequest } from "../stores/plant";

const store = usePlantStore();
const localAi = computed(() => objectAt(store.config, "local_ai"));
const provider = computed(() => objectAt(store.recommendation, "provider"));
const recommendation = computed(() => store.recommendation);
const targets = computed(() => objectAt(recommendation.value, "targets"));
const rationale = computed(() => textAt(recommendation.value, "rationale"));
const reasons = computed(() => arrayAt(recommendation.value, "reasons"));
const alternatives = computed(() => arrayAt(recommendation.value, "alternatives"));
const basedOn = computed(() => numberAt(recommendation.value, "based_on_batch_count"));
const updatedAt = computed(() => textAt(recommendation.value, "updated_at"));
const dryRunPlan = ref<Record<string, unknown> | null>(null);
const executeResult = ref<Record<string, unknown> | null>(null);
const experimentPlan = ref<Record<string, unknown> | null>(null);
const submitting = ref(false);
const actionMessage = ref("");

const intentOptions = [
  { value: "optimize_and_control", label: { zh: "寻优并控制", en: "Optimize and control" } },
  { value: "hold_only", label: { zh: "仅保持现状", en: "Hold only" } },
  { value: "cool_down", label: { zh: "降温收敛", en: "Cool down" } }
];

const dryRunForm = reactive<AiControlRequest>({
  dry_run: true,
  allow_process_start: true,
  allow_process_stop: true,
  allow_component_control: true,
  allow_target_adjustment: true,
  intent: "optimize_and_control"
});

async function withAction(label: string, action: () => Promise<void>): Promise<void> {
  submitting.value = true;
  actionMessage.value = "";
  store.error = null;
  try {
    await action();
    actionMessage.value = label;
  } catch (error) {
    store.error = error instanceof Error ? error.message : String(error);
  } finally {
    submitting.value = false;
  }
}

async function refreshRecommendation(): Promise<void> {
  await withAction(
    store.tr("推荐已刷新", "Recommendation refreshed"),
    async () => {
      await store.generateRecommendation();
    }
  );
}

async function runDryRun(): Promise<void> {
  await withAction(
    store.tr("AI 主控 dry-run 已生成", "AI master-control dry-run generated"),
    async () => {
      const result = await store.applyAiControl({ ...dryRunForm, dry_run: true });
      dryRunPlan.value = result;
    }
  );
}

async function executeAi(): Promise<void> {
  await withAction(
    store.tr("AI 主控已执行", "AI master-control executed"),
    async () => {
      const result = await store.applyAiControl({ ...dryRunForm, dry_run: false });
      executeResult.value = result;
      await store.refreshProtected();
    }
  );
}

async function loadPlan(): Promise<void> {
  await withAction(
    store.tr("实验方案已加载", "Experiment plan loaded"),
    async () => {
      experimentPlan.value = await store.loadExperimentPlan();
    }
  );
}

const safetyBlock = computed(() => {
  const runtime = objectAt(store.live, "runtime") ?? store.runtimeFallback;
  return {
    emergency_stop: textAt(runtime, "emergency_stop", "false") === "true",
    manual_lock: textAt(runtime, "manual_lock", "false") === "true"
  };
});
</script>

<template>
  <section class="view-stack">
    <div class="view-heading">
      <div>
        <p class="eyebrow">{{ store.tr("Qwen / LoRA 边界", "Qwen / LoRA Readiness") }}</p>
        <h1>{{ store.tr("AI 决策", "AI Decision") }}</h1>
        <span>{{ store.tr("本地模型边界、推荐来源、AI 主控 dry-run/execute 和 SOP 草案", "Local model readiness, recommendation source, AI master-control dry-run/execute, and SOP draft") }}</span>
      </div>
      <div class="heading-actions">
        <el-tag :type="textAt(localAi, 'ready_for_inference') === 'true' ? 'success' : 'warning'">
          {{ textAt(localAi, "mode", store.tr("未就绪", "not ready")) }}
        </el-tag>
        <el-button :loading="submitting" :disabled="!store.isAuthenticated" @click="refreshRecommendation">
          {{ store.tr("刷新推荐", "Refresh Recommendation") }}
        </el-button>
      </div>
    </div>

    <section class="panel two-col">
      <el-descriptions :column="1" border>
        <el-descriptions-item :label="store.tr('推理就绪', 'Inference ready')">{{ textAt(localAi, "ready_for_inference") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('训练就绪', 'Training ready')">{{ textAt(localAi, "ready_for_training") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('模型路径', 'Model path')">{{ textAt(localAi, "model_path") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('适配器路径', 'Adapter path')">{{ textAt(localAi, "adapter_path") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('本地优化器', 'Local optimizer')">{{ store.tr("已激活", "Active") }}</el-descriptions-item>
      </el-descriptions>
      <div class="analysis-block">
        <h2>{{ store.tr("最新推荐来源", "Latest Recommendation Provider") }}</h2>
        <p>{{ textAt(provider, "mode", store.tr("暂无缓存推荐", "No cached recommendation")) }}</p>
        <p class="muted">{{ textAt(provider, "fallback_reason", store.tr("等待 AI 推荐上下文。", "Waiting for AI recommendation context.")) }}</p>
        <p>
          <strong>{{ store.tr("更新于", "Updated at") }}:</strong>
          <span>{{ updatedAt || "--" }}</span>
        </p>
        <p>
          <strong>{{ store.tr("参考批次", "Reference batches") }}:</strong>
          <span>{{ basedOn ?? "--" }}</span>
        </p>
      </div>
    </section>

    <section class="panel control-panel">
      <div>
        <h2>{{ store.tr("推荐内容", "Recommendation Detail") }}</h2>
        <p>{{ rationale || store.tr("尚无可读推荐。点 “刷新推荐” 触发一次生成。", "No rationale yet. Press Refresh Recommendation to generate one.") }}</p>
        <div v-if="reasons.length > 0" class="ai-reasons">
          <span v-for="(reason, index) in reasons" :key="index" class="ai-reason">{{ textAt(reason, "label") }}</span>
        </div>
      </div>
      <div class="target-summary">
        <div>
          <span>{{ store.tr("目标温度 C", "Target temperature C") }}</span>
          <strong>{{ textAt(targets, "temperature_c") }}</strong>
          <small>C</small>
        </div>
        <div>
          <span>{{ store.tr("搅拌 rpm", "Stirrer rpm") }}</span>
          <strong>{{ textAt(targets, "stirrer_rpm") }}</strong>
          <small>rpm</small>
        </div>
        <div>
          <span>{{ store.tr("摇速 cpm", "Shake speed cpm") }}</span>
          <strong>{{ textAt(targets, "shake_speed_cpm") }}</strong>
          <small>cpm</small>
        </div>
        <div>
          <span>{{ store.tr("压力 MPa", "Pressure MPa") }}</span>
          <strong>{{ textAt(targets, "target_pressure_mpa") }}</strong>
          <small>MPa</small>
        </div>
      </div>
      <el-table v-if="alternatives.length > 0" :data="alternatives" class="data-table" size="small">
        <el-table-column :label="store.tr('候选', 'Candidate')" min-width="160">
          <template #default="{ row }">{{ textAt(row, "label") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('温度', 'Temp')" width="80">
          <template #default="{ row }">{{ textAt(row, "temperature_c") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('转速', 'RPM')" width="80">
          <template #default="{ row }">{{ textAt(row, "stirrer_rpm") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('摇速', 'Shake')" width="80">
          <template #default="{ row }">{{ textAt(row, "shake_speed_cpm") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('理由', 'Rationale')" min-width="200">
          <template #default="{ row }">{{ textAt(row, "rationale") }}</template>
        </el-table-column>
      </el-table>
    </section>

    <section class="panel control-panel">
      <div>
        <h2>{{ store.tr("AI 主控", "AI Master Control") }}</h2>
        <p>{{ store.tr("默认仅 dry-run，operator 复核后再以 execute 提交；执行会受 RBAC、急停、人工锁定、传感器新鲜度共同约束。", "Dry-run is the default. Operators review then submit execute; the action is gated by RBAC, emergency stop, manual lock, and sensor freshness.") }}</p>
        <p v-if="safetyBlock.emergency_stop" class="muted">
          {{ store.tr("急停已触发：AI 主控被阻断。", "Emergency stop is active: AI master control is blocked.") }}
        </p>
        <p v-else-if="safetyBlock.manual_lock" class="muted">
          {{ store.tr("人工锁定已开启：AI 主控被阻断。", "Manual lock is active: AI master control is blocked.") }}
        </p>
      </div>
      <el-form label-position="top" class="control-form">
        <el-form-item :label="store.tr('意图', 'Intent')">
          <el-select v-model="dryRunForm.intent">
            <el-option v-for="option in intentOptions" :key="option.value" :label="store.tr(option.label.zh, option.label.en)" :value="option.value" />
          </el-select>
        </el-form-item>
        <el-form-item :label="store.tr('允许工艺启动', 'Allow process start')">
          <el-switch v-model="dryRunForm.allow_process_start" />
        </el-form-item>
        <el-form-item :label="store.tr('允许工艺停止', 'Allow process stop')">
          <el-switch v-model="dryRunForm.allow_process_stop" />
        </el-form-item>
        <el-form-item :label="store.tr('允许部件控制', 'Allow component control')">
          <el-switch v-model="dryRunForm.allow_component_control" />
        </el-form-item>
        <el-form-item :label="store.tr('允许目标调整', 'Allow target adjustment')">
          <el-switch v-model="dryRunForm.allow_target_adjustment" />
        </el-form-item>
        <div class="control-actions">
          <el-button :loading="submitting" :disabled="!store.isAuthenticated" @click="runDryRun">
            {{ store.tr("Dry-run", "Dry-run") }}
          </el-button>
          <el-button type="primary" :loading="submitting" :disabled="!store.isAuthenticated || safetyBlock.emergency_stop || safetyBlock.manual_lock" @click="executeAi">
            {{ store.tr("Execute (受 RBAC 约束)", "Execute (gated by RBAC)") }}
          </el-button>
          <span v-if="actionMessage" class="muted">{{ actionMessage }}</span>
        </div>
      </el-form>
      <div v-if="dryRunPlan || executeResult" class="ai-result">
        <h3>{{ store.tr("执行计划", "Execution Plan") }}</h3>
        <pre>{{ JSON.stringify(dryRunPlan ?? executeResult, null, 2) }}</pre>
      </div>
    </section>

    <section class="panel control-panel">
      <div>
        <h2>{{ store.tr("实验方案 / SOP 草案", "Experiment Plan / SOP Draft") }}</h2>
        <p>{{ store.tr("只读草案，来源于历史批次结果 + 当前安全/优化器边界 + 本地 LoRA readiness。", "Read-only draft sourced from batch history, current safety/optimizer bounds, and local LoRA readiness.") }}</p>
      </div>
      <div class="control-actions">
        <el-button :loading="submitting" @click="loadPlan">
          {{ store.tr("加载 SOP 草案", "Load SOP Draft") }}
        </el-button>
      </div>
      <pre v-if="experimentPlan" class="ai-result">{{ JSON.stringify(experimentPlan, null, 2) }}</pre>
      <p v-else class="muted">{{ store.tr("尚未加载实验方案。", "No experiment plan loaded yet.") }}</p>
    </section>
  </section>
</template>
