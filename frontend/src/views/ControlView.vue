<script setup lang="ts">
import { computed, reactive, ref, watch } from "vue";
import { usePlantStore } from "../stores/plant";
import { arrayAt, numberAt, objectAt, textAt } from "./view-utils";
import type { ApiRecord, CreateProcessPayload, ProcessStepPayload } from "../stores/plant";

const store = usePlantStore();
const safety = computed(() => objectAt(store.config, "safety"));
const temperature = computed(() => objectAt(safety.value, "temperature"));
const stirrer = computed(() => objectAt(safety.value, "stirrer"));
const runtime = computed(() => objectAt(store.live, "runtime") ?? store.runtimeFallback);
const targets = computed(() => objectAt(runtime.value, "targets"));
const liveAlarms = computed(() => arrayAt<ApiRecord>(store.live, "alarms"));
const liveUnavailable = computed(() => store.liveStatus !== "fresh");
const submitting = ref(false);
const actionMessage = ref("");
const targetForm = reactive({
  temperature_c: 60,
  stirrer_rpm: 300,
  shake_speed_cpm: 30
});

const runtimeFlags = computed(() => ({
  auto_enabled: textAt(runtime.value, "auto_enabled", "false") === "true",
  manual_lock: textAt(runtime.value, "manual_lock", "false") === "true",
  emergency_stop: textAt(runtime.value, "emergency_stop", "false") === "true",
  last_control_error: textAt(runtime.value, "last_control_error", ""),
  last_sensor_error: textAt(runtime.value, "last_sensor_error", ""),
  // control_loop_terminated (state.rs:135): supervisor task died; the ONLY
  // recovery is a process restart. When true, the reset-fault button must be
  // disabled — backend will 409 any reset anyway.
  control_loop_terminated: textAt(runtime.value, "control_loop_terminated", "false") === "true",
  active_batch_id: textAt(runtime.value, "active_batch_id", "")
}));

const unfinishedBatchRecoveryAlarm = computed(
  () => liveAlarms.value.find((alarm) => textAt(alarm, "type", "") === "unfinished_batch_recovery") ?? null
);
const batchRecoveryBlocked = computed(() => Boolean(unfinishedBatchRecoveryAlarm.value));
const batchRecoveryReason = computed(() => {
  const alarm = unfinishedBatchRecoveryAlarm.value;
  if (!alarm) return "";
  const message = textAt(
    alarm,
    "message",
    store.tr(
      "数据库仍有未完成批次，需先核对现场并修复批次账。",
      "The database still has unfinished batch state; verify the field and repair batch records first."
    )
  );
  const ids = textAt(alarm, "unfinished_batch_ids", "");
  return ids ? `${message} (${ids})` : message;
});
const riskIncreasingDisabled = computed(
  () => !store.isAuthenticated || submitting.value || batchRecoveryBlocked.value || liveUnavailable.value
);
const productionBasisWriteDisabled = computed(
  () =>
    !isEngineer.value ||
    submitting.value ||
    liveUnavailable.value ||
    batchRecoveryBlocked.value ||
    runtimeFlags.value.active_batch_id !== "" ||
    runtimeFlags.value.emergency_stop ||
    runtimeFlags.value.manual_lock
);
const productionBasisWriteReason = computed(() => {
  if (!isEngineer.value) {
    return store.tr("需要 engineer/admin 角色修改工艺依据。", "Engineer/admin role is required to edit production recipes.");
  }
  if (liveUnavailable.value) {
    return store.tr(
      "实时现场状态不可用，工艺修改已锁定，避免在未知现场状态下改变后续生产依据。",
      "Live field state is unavailable; recipe edits are locked to avoid changing future production basis from an unknown state."
    );
  }
  if (batchRecoveryBlocked.value) return batchRecoveryReason.value;
  if (runtimeFlags.value.active_batch_id !== "") {
    return store.tr(
      `当前批次 #${runtimeFlags.value.active_batch_id} 仍在运行，结束并确认后再修改工艺。`,
      `Batch #${runtimeFlags.value.active_batch_id} is still running; finish and verify before editing recipes.`
    );
  }
  if (runtimeFlags.value.emergency_stop) {
    return store.tr("急停未复位前不修改生产依据。", "Do not edit production basis while emergency stop is active.");
  }
  if (runtimeFlags.value.manual_lock) {
    return store.tr("人工锁定未解除前不修改生产依据。", "Do not edit production basis while manual lock is active.");
  }
  return "";
});

const targetRows = computed(() => [
  { label: store.tr("目标温度", "Target temperature"), value: textAt(targets.value, "temperature_c"), unit: "C" },
  { label: store.tr("搅拌转速", "Stirrer speed"), value: textAt(targets.value, "stirrer_rpm"), unit: "rpm" },
  { label: store.tr("摇速", "Shake speed"), value: textAt(targets.value, "shake_speed_cpm"), unit: "cpm" },
  { label: store.tr("目标压力", "Target pressure"), value: textAt(targets.value, "target_pressure_mpa"), unit: "MPa" }
]);

function syncFormFromTargets(): void {
  targetForm.temperature_c = numberAt(targets.value, "temperature_c") ?? targetForm.temperature_c;
  targetForm.stirrer_rpm = numberAt(targets.value, "stirrer_rpm") ?? targetForm.stirrer_rpm;
  targetForm.shake_speed_cpm = numberAt(targets.value, "shake_speed_cpm") ?? targetForm.shake_speed_cpm;
}

watch(targets, syncFormFromTargets, { immediate: true });

async function runControlAction(action: () => Promise<void>, successMessage: string): Promise<void> {
  submitting.value = true;
  actionMessage.value = "";
  store.error = null;
  try {
    await action();
    actionMessage.value = successMessage;
  } catch (error) {
    store.error = error instanceof Error ? error.message : String(error);
  } finally {
    submitting.value = false;
  }
}

async function submitTargets(): Promise<void> {
  await runControlAction(
    async () => {
      await store.updateTargets({
        temperature_c: targetForm.temperature_c,
        stirrer_rpm: targetForm.stirrer_rpm,
        shake_speed_cpm: targetForm.shake_speed_cpm
      });
    },
    store.tr("目标已通过安全限幅写入", "Targets were written through the safety gate")
  );
}

function changeAutoEnabled(value: string | number | boolean): Promise<void> {
  return runControlAction(
    () => store.setAutoEnabled(Boolean(value)),
    Boolean(value) ? store.tr("自动控制已启用", "Automatic control enabled") : store.tr("自动控制已关闭", "Automatic control disabled")
  );
}

function changeManualLock(value: string | number | boolean): Promise<void> {
  return runControlAction(
    () => store.setManualLocked(Boolean(value)),
    Boolean(value)
      ? store.tr("人工锁定已启用，自动控制已关闭", "Manual lock enabled; automatic control disabled")
      : store.tr("人工锁定已关闭，自动控制仍需单独开启", "Manual lock disabled; automatic control must be enabled separately")
  );
}

function resetControlFault(): Promise<void> {
  return runControlAction(
    () => store.resetControlFault(),
    store.tr("控制写入故障已复归，自动控制保持关闭", "Control write fault reset; automatic control remains disabled")
  );
}

// --- Process / batch lifecycle -------------------------------------------------

const processForm = reactive<CreateProcessPayload>({
  name: "Vue acceptance process",
  description: "Created from Vue HMI"
});

const stepForm = reactive<ProcessStepPayload>({
  name: "Heat",
  target_temperature_c: 65,
  ramp_rate_c_min: 2,
  duration_minutes: 20,
  target_stirrer_rpm: 320,
  target_shake_speed_cpm: 30,
  target_pressure_mpa: 0.5,
  cooling_mode: "natural"
});

const selectedProcessId = ref<number | null>(null);
const selectedProcessInfo = computed(() => objectAt(store.selectedProcess, "process"));
const selectedSteps = computed(() => arrayAt(store.selectedProcess, "steps"));
const batchRows = computed(() => arrayAt(store.batches, "batches"));
const batchOutcomes = computed(() => arrayAt(store.batches, "outcomes"));
const isEngineer = computed(() => store.role === "engineer" || store.role === "admin");

const processList = computed(() => store.processes);
const activeProcessId = computed(() => numberAt(runtime.value, "active_process_id"));

async function loadProcessList(): Promise<void> {
  try {
    await store.loadProcesses();
  } catch (error) {
    store.error = error instanceof Error ? error.message : String(error);
  }
}

async function selectProcessFromList(id: number | null): Promise<void> {
  selectedProcessId.value = id;
  if (id === null) {
    return;
  }
  try {
    await store.loadProcessDetail(id);
  } catch (error) {
    store.error = error instanceof Error ? error.message : String(error);
  }
}

async function createProcessFromForm(): Promise<void> {
  await runControlAction(
    async () => {
      const created = await store.createProcess({
        name: processForm.name.trim() || "Untitled process",
        description: processForm.description.trim()
      });
      const newId = numberAt(created, "id");
      if (newId !== null) await selectProcessFromList(newId);
    },
    store.tr("工艺已创建", "Process created")
  );
}

async function addStepToSelected(): Promise<void> {
  const id = selectedProcessId.value;
  if (id === null) return;
  await runControlAction(
    async () => {
      await store.addProcessStep(id, { ...stepForm });
    },
    store.tr("步骤已添加", "Step added")
  );
}

async function startProcessFromList(id: number): Promise<void> {
  await runControlAction(
    async () => {
      await store.startProcess(id);
    },
    store.tr("工艺已启动", "Process started")
  );
}

async function stopCurrent(): Promise<void> {
  await runControlAction(
    async () => {
      await store.stopCurrentProcess();
    },
    store.tr("当前工艺已停止", "Current process stopped")
  );
}

// --- Process lifecycle gaps (backend endpoints previously unwired) ---

async function applyProcessFromList(id: number): Promise<void> {
  await runControlAction(
    async () => {
      await store.applyProcess(id);
    },
    store.tr("工艺已应用（applied）", "Process applied")
  );
}

async function stopProcessFromList(id: number): Promise<void> {
  await runControlAction(
    async () => {
      await store.stopProcessById(id, store.tr("操作员按 id 停止", "operator stop by id"));
    },
    store.tr("指定工艺已停止", "Process stopped")
  );
}

async function updateSelectedProcessMeta(): Promise<void> {
  const id = selectedProcessId.value;
  if (id === null) return;
  await runControlAction(
    async () => {
      await store.updateProcess(id, {
        name: processForm.name.trim() || undefined,
        description: processForm.description.trim() || undefined
      });
    },
    store.tr("工艺已更新", "Process updated")
  );
}

async function editStepFromSelected(stepId: number): Promise<void> {
  const id = selectedProcessId.value;
  if (id === null) return;
  await runControlAction(
    async () => {
      await store.updateProcessStep(id, stepId, { ...stepForm });
    },
    store.tr("步骤已更新", "Step updated")
  );
}

function statusTagType(status: string): "success" | "warning" | "info" | "danger" {
  if (status === "running" || status === "applied") return "success";
  if (status === "failed" || status === "rejected") return "danger";
  if (status === "draft" || status === "pending") return "warning";
  return "info";
}

function processStatusText(status: string): string {
  if (store.isChinese) {
    if (status === "draft") return "草稿";
    if (status === "applied") return "已应用";
    if (status === "archived") return "已归档";
    if (status === "running") return "运行中";
    if (status === "completed") return "已完成";
    if (status === "failed") return "失败";
    return status || "未知";
  }
  if (status === "draft") return "Draft";
  if (status === "applied") return "Applied";
  if (status === "archived") return "Archived";
  if (status === "running") return "Running";
  if (status === "completed") return "Completed";
  if (status === "failed") return "Failed";
  return status || "Unknown";
}

function stepSummary(step: Record<string, unknown>): string {
  const temp = textAt(step, "target_temperature_c");
  const rpm = textAt(step, "target_stirrer_rpm");
  const dur = textAt(step, "duration_minutes");
  return store.isChinese
    ? `${temp} C / ${rpm} rpm / ${dur} min`
    : `${temp} C / ${rpm} rpm / ${dur} min`;
}

function outcomeForBatch(batchId: number | null): string {
  if (batchId === null) return "--";
  const hit = batchOutcomes.value.find((row) => numberAt(row, "batch_id") === batchId);
  if (!hit) return "--";
  const product = textAt(hit, "product", "");
  const yieldValue = textAt(hit, "yield_percent", "");
  if (!product && !yieldValue) return "--";
  return store.isChinese
    ? `${product || "未命名"} · ${yieldValue || "--"}%`
    : `${product || "unnamed"} · ${yieldValue || "--"}%`;
}
</script>

<template>
  <section class="view-stack">
    <div class="view-heading">
      <div>
        <p class="eyebrow">{{ store.tr("目标与联锁", "Targets & interlocks") }}</p>
        <h1>{{ store.tr("参数配置", "Process Control") }}</h1>
        <span>{{ store.tr("参数配置、安全限幅和执行前复核", "Parameter setup, safety limits, and pre-execution review") }}</span>
      </div>
      <el-tag :type="store.isAuthenticated ? 'success' : 'warning'">
        {{ store.isAuthenticated ? store.tr("安全写入已接入", "Safety writes wired") : store.tr("登录后写入", "Sign in to write") }}
      </el-tag>
    </div>

    <section class="panel control-panel">
      <div>
        <h2>{{ store.tr("目标写入", "Target Write") }}</h2>
        <p>{{ store.tr("通过后端 safety gate 写入目标温度、搅拌和摇速；后端会按配置限幅并写入审计链。", "Write target temperature, stirrer speed, and shake speed through the backend safety gate; the backend clamps values and records the audit chain.") }}</p>
      </div>
      <el-form label-position="top" class="control-form">
        <el-form-item :label="store.tr('目标温度 C', 'Target temperature C')">
          <el-input-number v-model="targetForm.temperature_c" :min="0" :max="220" :step="1" controls-position="right" />
        </el-form-item>
        <el-form-item :label="store.tr('搅拌转速 rpm', 'Stirrer rpm')">
          <el-input-number v-model="targetForm.stirrer_rpm" :min="0" :max="1800" :step="10" controls-position="right" />
        </el-form-item>
        <el-form-item :label="store.tr('摇速 cpm', 'Shake speed cpm')">
          <el-input-number v-model="targetForm.shake_speed_cpm" :min="0" :max="80" :step="1" controls-position="right" />
        </el-form-item>
        <div class="control-actions">
          <el-button :loading="submitting" type="primary" :disabled="riskIncreasingDisabled" @click="submitTargets">
            {{ store.tr("安全写入目标", "Write Safe Targets") }}
          </el-button>
          <el-button :disabled="!store.isAuthenticated || submitting" @click="syncFormFromTargets">
            {{ store.tr("使用当前目标", "Use Current Targets") }}
          </el-button>
        </div>
      </el-form>
      <el-alert
        v-if="liveUnavailable"
        class="control-alert"
        type="error"
        :closable="false"
        show-icon
        :title="store.tr('实时现场状态不可用，升风险操作已锁定', 'Live field state is unavailable; risk-increasing actions are locked')"
        :description="store.tr('只保留急停、人工锁定和停止当前工艺等降风险动作。', 'Only risk-reducing actions such as emergency stop, manual lock, and stopping the current process remain available.')"
      />
    </section>

    <section class="panel control-panel">
      <div>
        <h2>{{ store.tr("运行开关", "Runtime Switches") }}</h2>
        <p>{{ store.tr("自动控制、人工锁定和急停都复用后端 RBAC、审计和安全状态。", "Automatic control, manual lock, and emergency stop reuse backend RBAC, audit, and safety state.") }}</p>
      </div>
      <div class="switch-grid">
        <label>
          <span>{{ store.tr("自动控制", "Automatic control") }}</span>
          <el-switch
            :model-value="runtimeFlags.auto_enabled"
            :disabled="riskIncreasingDisabled"
            @change="changeAutoEnabled"
          />
        </label>
        <label>
          <span>{{ store.tr("人工锁定", "Manual lock") }}</span>
          <el-switch
            :model-value="runtimeFlags.manual_lock"
            :disabled="!store.isAuthenticated || submitting || (!runtimeFlags.manual_lock && batchRecoveryBlocked)"
            @change="changeManualLock"
          />
        </label>
        <div class="emergency-actions">
          <el-button type="danger" :disabled="!store.isAuthenticated || submitting" @click="runControlAction(store.triggerEmergencyStop, store.tr('急停已触发', 'Emergency stop triggered'))">
            {{ store.tr("触发急停", "Emergency Stop") }}
          </el-button>
          <el-button plain :disabled="riskIncreasingDisabled" @click="runControlAction(store.resetEmergencyStop, store.tr('急停已复位', 'Emergency stop reset'))">
            {{ store.tr("复位急停", "Reset Stop") }}
          </el-button>
          <el-button
            plain
            :disabled="riskIncreasingDisabled || runtimeFlags.control_loop_terminated || !runtimeFlags.last_control_error"
            @click="resetControlFault"
          >
            {{ store.tr("复归控制故障", "Reset Control Fault") }}
          </el-button>
        </div>
      </div>
      <el-alert
        v-if="runtimeFlags.control_loop_terminated"
        class="control-alert"
        type="error"
        :closable="false"
        show-icon
        :title="store.tr('控制环监督已终止 — 必须重启进程', 'Control loop supervisor terminated — process restart required')"
        :description="store.tr('监督任务已退出，自动控制被禁用，且只能通过重启进程恢复。复归/启动操作将被后端拒绝（409）。', 'The supervisor task has exited; automatic control is disabled and can ONLY be cleared by a process restart. Reset/start will be rejected (409) by the backend.')"
      />
      <el-alert
        v-if="runtimeFlags.last_sensor_error"
        class="control-alert"
        type="warning"
        :closable="false"
        show-icon
        :title="store.tr('传感器故障 (fail-closed) — 自动控制已关闭', 'Sensor fault (fail-closed) — automatic control disabled')"
        :description="runtimeFlags.last_sensor_error"
      />
      <el-alert
        v-if="batchRecoveryBlocked"
        class="control-alert"
        type="error"
        :closable="false"
        show-icon
        :title="store.tr('未完成批次恢复中，升风险操作已锁定', 'Unfinished batch recovery is active; risk-increasing actions are locked')"
        :description="batchRecoveryReason"
      />
      <el-alert
        v-if="runtimeFlags.last_control_error"
        class="control-alert"
        type="warning"
        :closable="false"
        show-icon
        :title="store.tr('设备控制写入故障已锁存，自动控制已关闭', 'Device control write fault is latched; automatic control is disabled')"
        :description="runtimeFlags.last_control_error"
      />
    </section>

    <section class="panel process-panel">
      <div class="panel-title">
        <div>
          <h2>{{ store.tr("工艺管理", "Process Recipes") }}</h2>
          <p>{{ store.tr("列出已有工艺、查看步骤、创建新工艺并通过 engineer/admin 角色写入。", "List existing processes, inspect steps, create new recipes, and write through engineer/admin roles.") }}</p>
        </div>
        <el-tag :type="isEngineer ? 'success' : 'info'">
          {{ store.tr("当前角色", "Current role") }}: {{ store.role }}
        </el-tag>
      </div>

      <div class="process-grid">
        <el-form label-position="top" class="process-form">
          <el-form-item :label="store.tr('工艺名称', 'Process name')">
            <el-input v-model="processForm.name" maxlength="80" show-word-limit :placeholder="store.tr('例如：聚合反应 A', 'e.g. Polymerization A')" />
          </el-form-item>
          <el-form-item :label="store.tr('工艺说明', 'Description')">
            <el-input v-model="processForm.description" type="textarea" :rows="2" maxlength="240" show-word-limit />
          </el-form-item>
          <div class="control-actions">
            <el-button type="primary" :loading="submitting" :disabled="productionBasisWriteDisabled" @click="createProcessFromForm">
              {{ store.tr("创建工艺", "Create Process") }}
            </el-button>
            <el-button :disabled="submitting" @click="loadProcessList">
              {{ store.tr("刷新工艺列表", "Refresh Recipes") }}
            </el-button>
          </div>
        </el-form>
        <el-alert
          v-if="productionBasisWriteReason"
          class="inline-alert"
          type="warning"
          :closable="false"
          show-icon
          :title="store.tr('工艺编辑已锁定', 'Recipe editing locked')"
          :description="productionBasisWriteReason"
        />

        <div class="process-list">
          <div class="process-list-head">
            <strong>{{ store.tr("工艺列表", "Recipe List") }}</strong>
            <small>{{ store.tr("共", "Total") }} {{ processList.length }}</small>
          </div>
          <div v-if="processList.length === 0" class="process-empty">
            {{ store.tr("暂无工艺。请先创建或登录。", "No recipes yet. Create one or sign in first.") }}
          </div>
          <el-table v-else :data="processList" class="data-table" size="small" @row-click="(row) => selectProcessFromList(numberAt(row, 'id'))">
            <el-table-column :label="store.tr('ID', 'ID')" width="64">
              <template #default="{ row }">{{ textAt(row, "id") }}</template>
            </el-table-column>
            <el-table-column :label="store.tr('名称', 'Name')" min-width="160">
              <template #default="{ row }">{{ textAt(row, "name") }}</template>
            </el-table-column>
            <el-table-column :label="store.tr('状态', 'Status')" width="110">
              <template #default="{ row }">
                <el-tag :type="statusTagType(textAt(row, 'status'))" size="small">
                  {{ processStatusText(textAt(row, 'status')) }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column :label="store.tr('步骤', 'Steps')" width="72">
              <template #default="{ row }">{{ textAt(row, "step_count") }}</template>
            </el-table-column>
            <el-table-column :label="store.tr('操作', 'Actions')" width="290" align="right">
              <template #default="{ row }">
                <el-button size="small" :disabled="!store.isAuthenticated || submitting" @click.stop="selectProcessFromList(numberAt(row, 'id'))">
                  {{ store.tr("查看", "View") }}
                </el-button>
                <el-button
                  size="small"
                  :disabled="riskIncreasingDisabled || productionBasisWriteDisabled"
                  @click.stop="applyProcessFromList(numberAt(row, 'id') ?? 0)"
                >
                  {{ store.tr("应用", "Apply") }}
                </el-button>
                <el-button
                  size="small"
                  type="primary"
                  :disabled="riskIncreasingDisabled || runtimeFlags.active_batch_id !== '' || runtimeFlags.emergency_stop || runtimeFlags.manual_lock"
                  @click.stop="startProcessFromList(numberAt(row, 'id') ?? 0)"
                >
                  {{ store.tr("启动", "Start") }}
                </el-button>
                <el-button
                  size="small"
                  type="danger"
                  plain
                  :disabled="riskIncreasingDisabled || runtimeFlags.control_loop_terminated"
                  @click.stop="stopProcessFromList(numberAt(row, 'id') ?? 0)"
                >
                  {{ store.tr("停止", "Stop") }}
                </el-button>
              </template>
            </el-table-column>
          </el-table>
        </div>
      </div>
    </section>

    <section class="panel process-panel">
      <div class="panel-title">
        <div>
          <h2>{{ store.tr("工艺详情", "Process Detail") }}</h2>
          <p>{{ store.tr("查看选中工艺的步骤并通过 engineer/admin 添加新步骤。", "Inspect the selected process and add new steps when role allows.") }}</p>
        </div>
        <el-tag v-if="selectedProcessId === null" type="info">
          {{ store.tr("未选择", "Not selected") }}
        </el-tag>
        <el-tag v-else type="success">
          {{ store.tr("已选 ID", "Selected ID") }}: {{ selectedProcessId }}
        </el-tag>
      </div>

      <div v-if="selectedProcessId === null" class="process-empty">
        {{ store.tr("先在工艺列表中点击查看，再返回此处添加步骤。", "Pick a recipe from the list to view it and add steps here.") }}
      </div>
      <div v-else class="process-detail-grid">
        <el-descriptions :column="1" border>
          <el-descriptions-item :label="store.tr('工艺名称', 'Process name')">
            {{ textAt(selectedProcessInfo, "name") }}
          </el-descriptions-item>
          <el-descriptions-item :label="store.tr('说明', 'Description')">
            {{ textAt(selectedProcessInfo, "description") || store.tr("无", "n/a") }}
          </el-descriptions-item>
          <el-descriptions-item :label="store.tr('状态', 'Status')">
            <el-tag :type="statusTagType(textAt(selectedProcessInfo, 'status'))" size="small">
              {{ processStatusText(textAt(selectedProcessInfo, 'status')) }}
            </el-tag>
          </el-descriptions-item>
          <el-descriptions-item :label="store.tr('步骤数', 'Step count')">
            {{ textAt(selectedProcessInfo, "step_count") }}
          </el-descriptions-item>
          <el-descriptions-item :label="store.tr('版本', 'Version')">
            {{ textAt(selectedProcessInfo, "version") }}
          </el-descriptions-item>
          <el-descriptions-item :label="store.tr('已应用时间', 'Applied at')">
            {{ textAt(selectedProcessInfo, "applied_at") || "--" }}
          </el-descriptions-item>
        </el-descriptions>

        <div class="process-steps">
          <div class="process-list-head">
            <strong>{{ store.tr("步骤", "Steps") }}</strong>
            <small>{{ store.tr("共", "Total") }} {{ selectedSteps.length }}</small>
          </div>
          <el-table v-if="selectedSteps.length > 0" :data="selectedSteps" class="data-table" size="small">
            <el-table-column :label="store.tr('序号', '#')" width="60">
              <template #default="{ row }">{{ textAt(row, "step_index") }}</template>
            </el-table-column>
            <el-table-column :label="store.tr('名称', 'Name')" min-width="120">
              <template #default="{ row }">{{ textAt(row, "name") }}</template>
            </el-table-column>
            <el-table-column :label="store.tr('设定参数', 'Targets')" min-width="220">
              <template #default="{ row }">{{ stepSummary(row) }}</template>
            </el-table-column>
            <el-table-column :label="store.tr('降温', 'Cooling')" width="100">
              <template #default="{ row }">{{ textAt(row, "cooling_mode") }}</template>
            </el-table-column>
            <el-table-column :label="store.tr('操作', 'Actions')" width="90" align="right">
              <template #default="{ row }">
                <el-button
                  size="small"
                  plain
                  :disabled="riskIncreasingDisabled || productionBasisWriteDisabled"
                  @click="editStepFromSelected(numberAt(row, 'id') ?? 0)"
                >
                  {{ store.tr("编辑", "Edit") }}
                </el-button>
              </template>
            </el-table-column>
          </el-table>
          <div v-else class="process-empty">
            {{ store.tr("此工艺暂无步骤。", "No steps yet for this recipe.") }}
          </div>

          <el-form label-position="top" class="process-form">
            <el-form-item :label="store.tr('步骤名称', 'Step name')">
              <el-input v-model="stepForm.name" maxlength="80" />
            </el-form-item>
            <el-form-item :label="store.tr('目标温度 C', 'Target temperature C')">
              <el-input-number v-model="stepForm.target_temperature_c" :min="0" :max="220" :step="1" controls-position="right" />
            </el-form-item>
            <el-form-item :label="store.tr('升温速率 C/min', 'Ramp rate C/min')">
              <el-input-number v-model="stepForm.ramp_rate_c_min" :min="0" :max="20" :step="0.5" controls-position="right" />
            </el-form-item>
            <el-form-item :label="store.tr('时长 min', 'Duration min')">
              <el-input-number v-model="stepForm.duration_minutes" :min="1" :max="600" :step="1" controls-position="right" />
            </el-form-item>
            <el-form-item :label="store.tr('搅拌 rpm', 'Stirrer rpm')">
              <el-input-number v-model="stepForm.target_stirrer_rpm" :min="0" :max="1800" :step="10" controls-position="right" />
            </el-form-item>
            <el-form-item :label="store.tr('摇速 cpm', 'Shake speed cpm')">
              <el-input-number v-model="stepForm.target_shake_speed_cpm" :min="0" :max="60" :step="1" controls-position="right" />
            </el-form-item>
            <el-form-item :label="store.tr('压力 MPa', 'Pressure MPa')">
              <el-input-number v-model="stepForm.target_pressure_mpa" :min="0" :max="10" :step="0.1" controls-position="right" />
            </el-form-item>
            <el-form-item :label="store.tr('降温模式', 'Cooling mode')">
              <el-input v-model="stepForm.cooling_mode" maxlength="40" />
            </el-form-item>
            <div class="control-actions">
              <el-button type="primary" :loading="submitting" :disabled="productionBasisWriteDisabled" @click="addStepToSelected">
                {{ store.tr("添加步骤", "Add Step") }}
              </el-button>
            </div>
          </el-form>
        </div>
      </div>
    </section>

    <section class="panel two-col">
      <div>
        <h2>{{ store.tr("当前运行", "Current Run") }}</h2>
        <p>{{ store.tr("显示当前活动批次、自动控制状态和最新目标。", "Shows the active batch, automatic control state, and latest targets.") }}</p>
        <el-tag v-if="runtimeFlags.active_batch_id" type="success">
          {{ store.tr("活动批次", "Active batch") }}: {{ runtimeFlags.active_batch_id }}
        </el-tag>
        <el-tag v-else type="info">
          {{ store.tr("无活动批次", "No active batch") }}
        </el-tag>
        <el-tag :type="runtimeFlags.auto_enabled ? 'success' : 'info'">
          {{ runtimeFlags.auto_enabled ? store.tr("自动控制已启用", "Auto enabled") : store.tr("自动控制已关闭", "Auto disabled") }}
        </el-tag>
        <el-tag v-if="batchRecoveryBlocked" type="danger">
          {{ store.tr("批次恢复中", "Batch recovery") }}
        </el-tag>
      </div>
      <div>
        <div class="target-summary">
          <div v-for="row in targetRows" :key="row.label">
            <span>{{ row.label }}</span>
            <strong>{{ row.value }}</strong>
            <small>{{ row.unit }}</small>
          </div>
        </div>
        <div class="control-actions">
          <el-button
            type="danger"
            plain
            :disabled="!store.isAuthenticated || submitting || !runtimeFlags.active_batch_id"
            @click="stopCurrent"
          >
            {{ store.tr("停止当前工艺", "Stop Current Process") }}
          </el-button>
        </div>
      </div>
    </section>

    <section class="panel two-col">
      <div>
        <h2>{{ store.tr("安全边界", "Safety Envelope") }}</h2>
        <p>{{ actionMessage || store.tr("当前运行目标与配置限幅用于操作员复核。", "Current runtime targets and configured limits are shown for operator review.") }}</p>
      </div>
      <el-descriptions :column="1" border>
        <el-descriptions-item :label="store.tr('温度上限', 'Temperature max')">{{ textAt(temperature, "max_c") }} C</el-descriptions-item>
        <el-descriptions-item :label="store.tr('温度下限', 'Temperature min')">{{ textAt(temperature, "min_c") }} C</el-descriptions-item>
        <el-descriptions-item :label="store.tr('搅拌上限', 'Stirrer max')">{{ textAt(stirrer, "max_rpm") }} rpm</el-descriptions-item>
        <el-descriptions-item :label="store.tr('当前角色', 'Current role')">{{ store.role }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('急停状态', 'Emergency state')">{{ runtimeFlags.emergency_stop ? store.tr("已触发", "Triggered") : store.tr("正常", "Normal") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('控制故障', 'Control fault')">{{ runtimeFlags.last_control_error || store.tr("无", "None") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('批次恢复', 'Batch recovery')">{{ batchRecoveryReason || store.tr("无", "None") }}</el-descriptions-item>
      </el-descriptions>
    </section>

    <section class="panel">
      <div class="panel-title">
        <div>
          <h2>{{ store.tr("最近批次", "Recent Batches") }}</h2>
          <p>{{ store.tr("按时间倒序显示最近 8 个批次及其结果。", "Latest 8 batches and outcomes, newest first.") }}</p>
        </div>
        <el-tag>{{ batchRows.length }} {{ store.tr("条", "rows") }}</el-tag>
      </div>
      <el-table v-if="batchRows.length > 0" :data="batchRows" class="data-table" size="small">
        <el-table-column :label="store.tr('ID', 'ID')" width="64">
          <template #default="{ row }">{{ textAt(row, "id") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('名称', 'Name')" min-width="160">
          <template #default="{ row }">{{ textAt(row, "name") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('工艺', 'Process')" width="80">
          <template #default="{ row }">{{ textAt(row, "process_id") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('状态', 'Status')" width="110">
          <template #default="{ row }">
            <el-tag :type="statusTagType(textAt(row, 'status'))" size="small">
              {{ processStatusText(textAt(row, 'status')) }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="store.tr('开始', 'Started')" min-width="160">
          <template #default="{ row }">{{ textAt(row, "started_at") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('结束', 'Finished')" min-width="160">
          <template #default="{ row }">{{ textAt(row, "finished_at") || "--" }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('产品结果', 'Outcome')" min-width="180">
          <template #default="{ row }">{{ outcomeForBatch(numberAt(row, "id")) }}</template>
        </el-table-column>
      </el-table>
      <div v-else class="process-empty">
        {{ store.tr("暂无批次记录。", "No batches recorded yet.") }}
      </div>
    </section>
  </section>
</template>
