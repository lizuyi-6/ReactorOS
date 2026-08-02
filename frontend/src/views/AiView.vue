<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { ElMessage } from "element-plus";
import PageHeader from "../components/PageHeader.vue";
import EmptyState from "../components/EmptyState.vue";
import HmiButton from "../components/HmiButton.vue";
import { aiApi } from "../api";
import { errorMessage } from "../api/errors";
import { useAuthStore } from "../stores/auth";
import { useLiveStore } from "../stores/live";
import { usePlantStore } from "../stores/plant";
import { useLanguage } from "../i18n";
import { boolText, fixed, text } from "../utils/format";
import type { AiControlResponse, ExperimentPlanResponse } from "../api/types";

const auth = useAuthStore();
const live = useLiveStore();
const plant = usePlantStore();
const { tr } = useLanguage();

const submitting = ref(false);
const loadingPlan = ref(false);
const controlResult = ref<AiControlResponse | null>(null);
const plan = ref<ExperimentPlanResponse | null>(null);

const localAi = computed(() => plant.config?.local_ai ?? null);
const aiProvider = computed(() => live.live?.ai_provider ?? plant.config?.ai_provider ?? null);
const recommendation = computed(() => live.recommendation ?? plant.recommendation);

const aiForm = reactive({
  intent: "optimize_and_control",
  dry_run: true,
  allow_process_start: false,
  allow_process_stop: false,
  allow_component_control: false,
  allow_target_adjustment: true
});

const executeBlocked = computed(() => {
  const rt = live.runtime;
  return (
    !auth.isAuthenticated ||
    submitting.value ||
    live.liveStatus !== "fresh" ||
    boolText(rt?.emergency_stop) ||
    boolText(rt?.manual_lock) ||
    live.alarms.some((a) => String(a.type ?? "") === "unfinished_batch_recovery")
  );
});

const localAiMode = computed(() => text(localAi.value?.mode, "--"));
const localAiMissing = computed(() => (Array.isArray(localAi.value?.missing) ? localAi.value!.missing! : []));

function formatConfigKey(key: string): string {
  const map: Record<string, string> = {
    XINGSHU_LOCAL_AI_ENABLED: tr("本地 AI 启用", "Local AI enabled"),
    XINGSHU_LOCAL_AI_BIN: tr("本地 AI 二进制路径", "Local AI binary path"),
    XINGSHU_LOCAL_AI_GGUF: tr("本地 AI 模型文件", "Local AI model file"),
    XINGSHU_LOCAL_AI_LORA: tr("本地 AI LoRA 权重", "Local AI LoRA weights"),
    XINGSHU_LOCAL_AI_TRAIN_SCRIPT: tr("本地 AI 训练脚本", "Local AI training script"),
    XINGSHU_LOCAL_AI_CONVERT_SCRIPT: tr("本地 AI 转换脚本", "Local AI convert script"),
    XINGSHU_LOCAL_AI_RK_REPORT: tr("本地 AI 报告输出", "Local AI report output"),
  };
  return map[key] ?? key;
}

async function regenerate(): Promise<void> {
  submitting.value = true;
  try {
    const rec = await aiApi.regenerateRecommendation();
    plant.recommendation = rec;
    ElMessage.success(tr("推荐已刷新", "Recommendation regenerated"));
  } catch (error) {
    ElMessage.error(errorMessage(error));
  } finally {
    submitting.value = false;
  }
}

async function runControl(dryRun: boolean): Promise<void> {
  submitting.value = true;
  try {
    const body: Record<string, unknown> = {
      intent: aiForm.intent,
      dry_run: dryRun,
      allow_target_adjustment: aiForm.allow_target_adjustment
    };
    if (aiForm.allow_process_start) body.allow_process_start = true;
    if (aiForm.allow_process_stop) body.allow_process_stop = true;
    if (aiForm.allow_component_control) body.allow_component_control = true;
    controlResult.value = await aiApi.control(body as never);
    ElMessage.success(dryRun ? tr("预演完成", "Dry-run complete") : tr("已执行", "Executed"));
    if (!dryRun) await live.refreshLive();
  } catch (error) {
    ElMessage.error(errorMessage(error));
  } finally {
    submitting.value = false;
  }
}

async function loadPlan(): Promise<void> {
  loadingPlan.value = true;
  try {
    plan.value = await aiApi.experimentPlan();
  } catch (error) {
    ElMessage.error(errorMessage(error));
  } finally {
    loadingPlan.value = false;
  }
}

function actionStatusType(status: string): "success" | "warning" | "info" | "danger" {
  if (status === "executed") return "success";
  if (status === "planned") return "info";
  if (status === "skipped") return "warning";
  if (status === "blocked") return "danger";
  return "info";
}

onMounted(async () => {
  await Promise.allSettled([plant.loadConfig()]);
});
</script>

<template>
  <div class="page-stack">
    <PageHeader :title="tr('AI 决策', 'AI Decision')" :subtitle="tr('推荐、AI 主控预演与实验 SOP 草案', 'Recommendations, AI master control dry-run and SOP drafts')">
      <template #actions>
        <HmiButton type="manual" :disabled="!auth.isAuthenticated" @click="regenerate">
          {{ tr("刷新推荐", "Regenerate") }}
        </HmiButton>
      </template>
    </PageHeader>

    <div class="ai-grid">
      <!-- 本地 AI 状态 -->
      <section class="hmi-panel">
        <div class="hmi-panel-header">
          <span>{{ tr("本地 AI 状态", "Local AI status") }}</span>
          <el-tag size="small" :type="localAiMissing.length === 0 ? 'success' : 'warning'">{{ localAiMode }}</el-tag>
        </div>
        <div class="hmi-panel-body">
          <dl class="kv-list">
            <dt>{{ tr("Provider", "Provider") }}</dt>
            <dd>{{ text(aiProvider?.mode) }}</dd>
            <dt>{{ tr("模型", "Model") }}</dt>
            <dd>{{ text(aiProvider?.model) }}</dd>
            <dt>{{ tr("回退原因", "Fallback") }}</dt>
            <dd>{{ text(aiProvider?.fallback_reason, tr("无", "none")) }}</dd>
          </dl>
          <div v-if="localAiMissing.length > 0" class="missing-list">
            <small class="muted">{{ tr("缺失配置项", "Missing configuration") }}</small>
            <div class="config-status">
              <div v-for="item in localAiMissing" :key="item" class="config-item">
                <span class="config-name">{{ formatConfigKey(item) }}</span>
                <el-tag size="small" type="warning">{{ tr("未配置", "Not configured") }}</el-tag>
              </div>
            </div>
          </div>
        </div>
      </section>

      <!-- 推荐详情 -->
      <section class="hmi-panel">
        <div class="hmi-panel-header">
          <span>{{ tr("最新推荐", "Latest recommendation") }}</span>
          <span class="muted">{{ tr("基于批次数", "Based on") }}: {{ text(recommendation?.based_on_batch_count) }}</span>
        </div>
        <div class="hmi-panel-body">
          <template v-if="recommendation">
            <p class="rationale">{{ text(recommendation.rationale) }}</p>
            <dl class="kv-list">
              <dt>{{ tr("目标温度", "Target temp") }}</dt>
              <dd>{{ fixed(recommendation.target_temperature_c ?? null, 1) }} °C</dd>
              <dt>{{ tr("目标转速", "Target RPM") }}</dt>
              <dd>{{ fixed(recommendation.target_stirrer_rpm ?? null, 0) }} rpm</dd>
              <dt>{{ tr("加热时长", "Heating") }}</dt>
              <dd>{{ fixed(recommendation.heating_minutes ?? null, 1) }} min</dd>
              <dt>{{ tr("搅拌时长", "Stirring") }}</dt>
              <dd>{{ fixed(recommendation.stirring_minutes ?? null, 1) }} min</dd>
              <dt>{{ tr("预期分数", "Expected score") }}</dt>
              <dd>{{ fixed(recommendation.expected_score ?? null, 1) }}</dd>
            </dl>
          </template>
          <EmptyState v-else icon="AI" :title="tr('暂无推荐', 'No recommendation')" :description="tr('录入产物结果或点击刷新推荐生成。', 'Record a product result or regenerate.')" />
        </div>
      </section>
    </div>

    <!-- AI 主控 -->
    <section class="hmi-panel">
      <div class="hmi-panel-header">
        <span>{{ tr("AI 主控", "AI master control") }}</span>
        <span class="muted">{{ tr("先 dry-run 复核，再执行", "Dry-run first, then execute") }}</span>
      </div>
      <div class="hmi-panel-body">
        <div class="ai-control-form">
          <el-form-item :label="tr('意图', 'Intent')">
            <el-select v-model="aiForm.intent" class="intent-select">
              <el-option value="optimize_and_control" :label="tr('优化并控制', 'Optimize & control')" />
              <el-option value="hold_only" :label="tr('仅保持', 'Hold only')" />
              <el-option value="cool_down" :label="tr('降温', 'Cool down')" />
            </el-select>
          </el-form-item>
          <div class="allow-switches">
            <el-checkbox v-model="aiForm.allow_target_adjustment">{{ tr("允许调目标", "Allow target adjust") }}</el-checkbox>
            <el-checkbox v-model="aiForm.allow_process_start">{{ tr("允许启动工艺", "Allow process start") }}</el-checkbox>
            <el-checkbox v-model="aiForm.allow_process_stop">{{ tr("允许停止工艺", "Allow process stop") }}</el-checkbox>
            <el-checkbox v-model="aiForm.allow_component_control">{{ tr("允许组件控制", "Allow component control") }}</el-checkbox>
          </div>
          <div class="form-actions">
            <HmiButton type="manual" :disabled="!auth.isAuthenticated" @click="runControl(true)">
              {{ tr("预演（dry-run）", "Dry-run") }}
            </HmiButton>
            <HmiButton type="start" :disabled="executeBlocked" @click="runControl(false)">
              {{ tr("执行", "Execute") }}
            </HmiButton>
          </div>
          <el-alert
            v-if="executeBlocked && auth.isAuthenticated"
            type="warning"
            :closable="false"
            show-icon
            :title="tr('执行已被安全门控禁用', 'Execute disabled by safety gate')"
            :description="tr('实时不可用 / 急停 / 人工锁 / 批次恢复中。', 'Live unavailable / E-stop / manual lock / batch recovery.')"
          />
        </div>

        <!-- 结果复核 -->
        <div v-if="controlResult" class="control-result">
          <dl class="kv-list">
            <dt>{{ tr("模式", "Mode") }}</dt>
            <dd>{{ text(controlResult.mode) }}</dd>
            <dt>{{ tr("决策", "Decision") }}</dt>
            <dd>{{ text(controlResult.decision) }}</dd>
            <dt>{{ tr("Dry-run", "Dry-run") }}</dt>
            <dd>{{ boolText(controlResult.dry_run) ? "true" : "false" }}</dd>
          </dl>
          <p class="rationale">{{ text(controlResult.rationale) }}</p>
          <el-table v-if="(controlResult.actions ?? []).length > 0" :data="controlResult.actions" size="small">
            <el-table-column prop="action_type" :label="tr('动作', 'Action')" min-width="140" />
            <el-table-column :label="tr('状态', 'Status')" width="100">
              <template #default="{ row }">
                <el-tag :type="actionStatusType(String(row.status ?? ''))" size="small">{{ row.status }}</el-tag>
              </template>
            </el-table-column>
            <el-table-column prop="target" :label="tr('目标', 'Target')" min-width="120">
              <template #default="{ row }">{{ text(row.target) }}</template>
            </el-table-column>
            <el-table-column prop="message" :label="tr('说明', 'Message')" min-width="200">
              <template #default="{ row }">{{ text(row.message) }}</template>
            </el-table-column>
          </el-table>
          <el-collapse class="raw-json">
            <el-collapse-item :title="tr('原始响应 JSON', 'Raw response JSON')">
              <pre class="mono">{{ JSON.stringify(controlResult, null, 2) }}</pre>
            </el-collapse-item>
          </el-collapse>
        </div>
      </div>
    </section>

    <!-- SOP 草案 -->
    <section class="hmi-panel">
      <div class="hmi-panel-header">
        <span>{{ tr("实验 SOP 草案", "Experiment SOP draft") }}</span>
        <HmiButton type="manual" :disabled="loadingPlan" @click="loadPlan">{{ tr("生成草案", "Generate") }}</HmiButton>
      </div>
      <div class="hmi-panel-body">
        <template v-if="plan">
          <div class="plan-head">
            <h3>{{ text(plan.title, tr("未命名计划", "Untitled plan")) }}</h3>
            <el-tag size="small" type="warning">{{ text(plan.status) }}</el-tag>
          </div>
          <p class="rationale">{{ text(plan.objective) }}</p>
          <p class="muted">{{ text(plan.sop_summary) }}</p>
          <el-table v-if="(plan.steps ?? []).length > 0" :data="plan.steps" size="small" class="plan-steps">
            <el-table-column prop="step_no" label="#" width="50" />
            <el-table-column :label="tr('步骤', 'Step')" min-width="120">
              <template #default="{ row }">{{ text(row.name) }}</template>
            </el-table-column>
            <el-table-column :label="tr('目标', 'Targets')" min-width="200">
              <template #default="{ row }">
                {{ fixed(row.target_temperature_c ?? null, 0) }}°C / {{ fixed(row.target_stirrer_rpm ?? null, 0) }}rpm /
                {{ fixed(row.target_shake_speed_cpm ?? null, 0) }}cpm / {{ fixed(row.duration_minutes ?? null, 0) }}min
              </template>
            </el-table-column>
            <el-table-column :label="tr('操作', 'Operator action')" min-width="180">
              <template #default="{ row }">{{ text(row.operator_action) }}</template>
            </el-table-column>
            <el-table-column :label="tr('安全检查', 'Safety check')" min-width="180">
              <template #default="{ row }">{{ text(row.safety_check) }}</template>
            </el-table-column>
          </el-table>
          <div v-if="(plan.safety_notes ?? []).length > 0" class="plan-notes">
            <strong>{{ tr("安全说明", "Safety notes") }}</strong>
            <ul><li v-for="(note, i) in plan.safety_notes" :key="i">{{ note }}</li></ul>
          </div>
          <el-collapse class="raw-json">
            <el-collapse-item :title="tr('原始计划 JSON', 'Raw plan JSON')">
              <pre class="mono">{{ JSON.stringify(plan, null, 2) }}</pre>
            </el-collapse-item>
          </el-collapse>
        </template>
        <EmptyState
          v-else
          icon="▦"
          :title="tr('尚未生成 SOP 草案', 'No SOP draft yet')"
          :description="tr('点击「生成草案」由后端基于历史批次生成（需要已有产物结果）。', 'Click Generate; the backend drafts a plan from batch history (requires product results).')"
        />
      </div>
    </section>
  </div>
</template>

<style scoped>
.ai-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: var(--spacing);
  align-items: start;
}

.rationale {
  font-size: var(--fs-sm);
  color: var(--text-secondary);
  line-height: 1.6;
  margin-bottom: var(--spacing);
  overflow-wrap: anywhere;
}

.missing-list {
  margin-top: var(--spacing);
}

.config-status {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: 8px;
}

.config-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  background: var(--bg-inset);
  border-radius: var(--radius-md);
  border: 1px solid var(--border-glass);
}

.config-name {
  font-size: var(--fs-sm);
  color: var(--text-secondary);
}

.ai-control-form {
  display: flex;
  flex-direction: column;
  gap: var(--spacing);
  margin-bottom: var(--spacing);
}

.intent-select {
  width: 260px;
}

.allow-switches {
  display: flex;
  gap: var(--spacing);
  flex-wrap: wrap;
}

.control-result {
  border-top: 1px solid var(--border-glass);
  padding-top: var(--spacing);
  display: flex;
  flex-direction: column;
  gap: var(--spacing);
}

.raw-json pre {
  max-height: 320px;
  overflow: auto;
  font-size: var(--fs-xs);
  background: var(--bg-inset);
  padding: var(--spacing);
  border-radius: var(--radius-md);
}

.plan-head {
  display: flex;
  align-items: center;
  gap: var(--spacing);
  margin-bottom: var(--spacing);
}

.plan-steps {
  margin: var(--spacing) 0;
}

.plan-notes {
  margin-top: var(--spacing);
}

.plan-notes ul {
  margin: 8px 0 0;
  padding-left: var(--spacing);
  color: var(--text-secondary);
  font-size: var(--fs-sm);
}

@media (max-width: 1000px) {
  .ai-grid {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
