<script setup lang="ts">
import { computed, reactive, ref, watch } from "vue";
import { usePlantStore } from "../stores/plant";
import { numberAt, objectAt, textAt } from "./view-utils";

const store = usePlantStore();
const safety = computed(() => objectAt(store.config, "safety"));
const temperature = computed(() => objectAt(safety.value, "temperature"));
const stirrer = computed(() => objectAt(safety.value, "stirrer"));
const runtime = computed(() => objectAt(store.live, "runtime") ?? store.runtimeFallback);
const targets = computed(() => objectAt(runtime.value, "targets"));
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
  emergency_stop: textAt(runtime.value, "emergency_stop", "false") === "true"
}));

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
    Boolean(value) ? store.tr("人工锁定已启用", "Manual lock enabled") : store.tr("人工锁定已关闭", "Manual lock disabled")
  );
}
</script>

<template>
  <section class="view-stack">
    <div class="view-heading">
      <div>
        <p class="eyebrow">{{ store.tr("Element Plus 表单", "Element Plus Forms") }}</p>
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
          <el-button :loading="submitting" type="primary" :disabled="!store.isAuthenticated" @click="submitTargets">
            {{ store.tr("安全写入目标", "Write Safe Targets") }}
          </el-button>
          <el-button :disabled="!store.isAuthenticated || submitting" @click="syncFormFromTargets">
            {{ store.tr("使用当前目标", "Use Current Targets") }}
          </el-button>
        </div>
      </el-form>
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
            :disabled="!store.isAuthenticated || submitting"
            @change="changeAutoEnabled"
          />
        </label>
        <label>
          <span>{{ store.tr("人工锁定", "Manual lock") }}</span>
          <el-switch
            :model-value="runtimeFlags.manual_lock"
            :disabled="!store.isAuthenticated || submitting"
            @change="changeManualLock"
          />
        </label>
        <div class="emergency-actions">
          <el-button type="danger" :disabled="!store.isAuthenticated || submitting" @click="runControlAction(store.triggerEmergencyStop, store.tr('急停已触发', 'Emergency stop triggered'))">
            {{ store.tr("触发急停", "Emergency Stop") }}
          </el-button>
          <el-button plain :disabled="!store.isAuthenticated || submitting" @click="runControlAction(store.resetEmergencyStop, store.tr('急停已复位', 'Emergency stop reset'))">
            {{ store.tr("复位急停", "Reset Stop") }}
          </el-button>
        </div>
      </div>
    </section>

    <section class="panel two-col">
      <div>
        <h2>{{ store.tr("安全边界", "Safety Envelope") }}</h2>
        <p>{{ actionMessage || store.tr("当前运行目标与配置限幅用于操作员复核。", "Current runtime targets and configured limits are shown for operator review.") }}</p>
      </div>
      <div class="target-summary">
        <div v-for="row in targetRows" :key="row.label">
          <span>{{ row.label }}</span>
          <strong>{{ row.value }}</strong>
          <small>{{ row.unit }}</small>
        </div>
      </div>
      <el-descriptions :column="1" border>
        <el-descriptions-item :label="store.tr('温度上限', 'Temperature max')">{{ textAt(temperature, "max_c") }} C</el-descriptions-item>
        <el-descriptions-item :label="store.tr('温度下限', 'Temperature min')">{{ textAt(temperature, "min_c") }} C</el-descriptions-item>
        <el-descriptions-item :label="store.tr('搅拌上限', 'Stirrer max')">{{ textAt(stirrer, "max_rpm") }} rpm</el-descriptions-item>
        <el-descriptions-item :label="store.tr('当前角色', 'Current role')">{{ store.role }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('急停状态', 'Emergency state')">{{ runtimeFlags.emergency_stop ? store.tr("已触发", "Triggered") : store.tr("正常", "Normal") }}</el-descriptions-item>
      </el-descriptions>
    </section>
  </section>
</template>
