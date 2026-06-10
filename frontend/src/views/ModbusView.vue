<script setup lang="ts">
import { computed, reactive, ref, watch } from "vue";
import { usePlantStore } from "../stores/plant";
import type { ApiRecord } from "../stores/plant";
import { arrayAt, objectAt, textAt } from "./view-utils";

const store = usePlantStore();
const readRegisters = computed(() => arrayAt(store.modbus, "read_registers"));
const writeRegisters = computed(() => arrayAt(store.modbus, "write_registers"));
const registers = computed(() => [...readRegisters.value, ...writeRegisters.value]);
const coils = computed(() => arrayAt(store.modbus, "coils"));
const integrations = computed(() => objectAt(store.config, "integrations"));
const mqttStatus = computed(() => objectAt(integrations.value, "mqtt_status"));
const modbusTcpStatus = computed(() => objectAt(integrations.value, "modbus_tcp_status"));
const localAi = computed(() => objectAt(store.config, "local_ai"));
const liveAlarms = computed(() => arrayAt<ApiRecord>(store.live, "alarms"));
const liveUnavailable = computed(() => store.liveStatus !== "fresh");
const submitting = ref(false);
const actionMessage = ref("");
const readResult = ref<ApiRecord | null>(null);
const writeResult = ref<ApiRecord | null>(null);
const debugForm = reactive({
  readRegister: "target_temperature_c",
  writeRegister: "target_temperature_c",
  value: 65,
  reason: "Vue Modbus debug acceptance"
});

const writeOptions = computed(() =>
  writeRegisters.value.map((register) => ({
    label: `${textAt(register, "name")} @${textAt(register, "address")}`,
    value: textAt(register, "name", "")
  }))
);

const unfinishedBatchRecoveryAlarm = computed(
  () => liveAlarms.value.find((alarm) => textAt(alarm, "type", "") === "unfinished_batch_recovery") ?? null
);
const batchRecoveryBlocked = computed(() => Boolean(unfinishedBatchRecoveryAlarm.value));
const writeBlocked = computed(
  () => store.role !== "admin" || submitting.value || !debugForm.reason.trim() || liveUnavailable.value || batchRecoveryBlocked.value
);
const writeBlockReason = computed(() => {
  if (liveUnavailable.value) {
    return store.tr(
      "实时现场状态不可用，Modbus 调试写入已锁定；只允许读寄存器。",
      "Live field state is unavailable; Modbus debug writes are locked and reads remain available."
    );
  }
  if (batchRecoveryBlocked.value) {
    const ids = textAt(unfinishedBatchRecoveryAlarm.value, "unfinished_batch_ids", "");
    return store.tr(
      `未完成批次恢复中，Modbus 调试写入已锁定。${ids}`,
      `Unfinished batch recovery is active; Modbus debug writes are locked. ${ids}`
    ).trim();
  }
  return "";
});

const valueTranslations: Record<string, { zh: string; en: string }> = {
  true: { zh: "是", en: "Yes" },
  false: { zh: "否", en: "No" },
  prd_lora_ready: { zh: "PRD LoRA/RK 闭环", en: "PRD LoRA/RK ready" },
  lora_inference_ready: { zh: "LoRA 推理入口就绪", en: "LoRA inference ready" },
  base_inference_only: { zh: "仅基础模型入口", en: "Base model only" },
  configured_not_ready: { zh: "配置未就绪", en: "Configured, not ready" },
  disabled: { zh: "未启用", en: "Disabled" }
};

const readOptions = computed(() =>
  registers.value.map((register) => ({
    label: `${textAt(register, "name")} @${textAt(register, "address")}`,
    value: textAt(register, "name", "")
  }))
);

watch(
  writeOptions,
  (options) => {
    if (options.length && !options.some((option) => option.value === debugForm.writeRegister)) {
      debugForm.writeRegister = options[0].value;
    }
  },
  { immediate: true }
);

watch(
  readOptions,
  (options) => {
    if (options.length && !options.some((option) => option.value === debugForm.readRegister)) {
      debugForm.readRegister = options[0].value;
    }
  },
  { immediate: true }
);

function resultRows(result: ApiRecord | null): { label: string; value: string }[] {
  if (!result) return [];
  return [
    { label: store.tr("寄存器", "Register"), value: textAt(result, "register") },
    { label: store.tr("地址", "Address"), value: textAt(result, "address") },
    { label: store.tr("数值", "Value"), value: textAt(result, "value", textAt(result, "applied_value")) },
    { label: store.tr("原始值", "Raw"), value: textAt(result, "raw") },
    { label: store.tr("来源", "Source"), value: textAt(result, "source", textAt(result, "requested_value")) }
  ];
}

function boolText(value: unknown): string {
  if (value === true || value === "true") return store.tr("是", "Yes");
  if (value === false || value === "false") return store.tr("否", "No");
  return value === null || value === undefined || value === "" ? "--" : String(value);
}

function localizedValue(value: string): string {
  const translation = valueTranslations[value];
  return translation ? store.tr(translation.zh, translation.en) : value;
}

function displayAt(source: unknown, key: string, fallback = "--"): string {
  if (!source || typeof source !== "object") return fallback;
  const value = (source as ApiRecord)[key];
  if (typeof value === "boolean") return boolText(value);
  if (Array.isArray(value)) return value.length ? value.join(", ") : fallback;
  if (value && typeof value === "object") return JSON.stringify(value);
  return value === null || value === undefined || value === "" ? fallback : localizedValue(String(value));
}

async function runModbusAction(action: () => Promise<void>, successMessage: string): Promise<void> {
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

async function readSelectedRegister(): Promise<void> {
  await runModbusAction(
    async () => {
      readResult.value = await store.readModbusRegister(debugForm.readRegister);
    },
    store.tr("寄存器已读回", "Register read completed")
  );
}

async function writeSelectedRegister(): Promise<void> {
  await runModbusAction(
    async () => {
      writeResult.value = await store.writeModbusRegister(debugForm.writeRegister, {
        value: debugForm.value,
        reason: debugForm.reason
      });
      readResult.value = await store.readModbusRegister(debugForm.writeRegister);
    },
    store.tr("写入已通过安全链路和审计原因", "Write passed the safety path with audit reason")
  );
}
</script>

<template>
  <section class="view-stack">
    <div class="view-heading">
      <div>
        <p class="eyebrow">tokio-modbus Migration Target</p>
        <h1>{{ store.tr("Modbus 调试", "Modbus Debug") }}</h1>
        <span>{{ store.tr("寄存器映射、调试权限和第三方接口验收", "Register map, debug permissions, and third-party interface acceptance") }}</span>
      </div>
      <el-tag :type="store.role === 'admin' ? 'success' : 'danger'">
        {{ store.role === "admin" ? store.tr("管理员写入已解锁", "Admin writes unlocked") : store.tr("仅管理员可写", "Admin writes only") }}
      </el-tag>
    </div>

    <section class="panel modbus-debug-panel">
      <div>
        <h2>{{ store.tr("寄存器调试", "Register Debug") }}</h2>
        <p>{{ store.tr("读操作可直接执行；写操作仅允许管理员，并且必须填写非空审计原因。", "Reads are available directly; writes require an admin session and a non-empty audit reason.") }}</p>
      </div>
      <el-form label-position="top" class="modbus-form">
        <el-form-item :label="store.tr('读寄存器', 'Read register')">
          <el-select v-model="debugForm.readRegister" filterable>
            <el-option v-for="option in readOptions" :key="option.value" :label="option.label" :value="option.value" />
          </el-select>
        </el-form-item>
        <el-form-item :label="store.tr('写寄存器', 'Write register')">
          <el-select v-model="debugForm.writeRegister" filterable>
            <el-option v-for="option in writeOptions" :key="option.value" :label="option.label" :value="option.value" />
          </el-select>
        </el-form-item>
        <el-form-item :label="store.tr('写入值', 'Write value')">
          <el-input-number v-model="debugForm.value" :step="1" controls-position="right" />
        </el-form-item>
        <el-form-item :label="store.tr('审计原因', 'Audit reason')" class="reason-field">
          <el-input v-model="debugForm.reason" maxlength="160" show-word-limit />
        </el-form-item>
        <div class="control-actions">
          <el-button :loading="submitting" @click="readSelectedRegister">{{ store.tr("读取寄存器", "Read Register") }}</el-button>
          <el-button
            type="danger"
            :loading="submitting"
            :disabled="writeBlocked"
            @click="writeSelectedRegister"
          >
            {{ store.tr("写入并读回", "Write and Read Back") }}
          </el-button>
        </div>
      </el-form>
      <el-alert
        v-if="writeBlockReason"
        class="control-alert"
        type="error"
        :closable="false"
        show-icon
        :title="store.tr('Modbus 写入安全锁定', 'Modbus write safety lock')"
        :description="writeBlockReason"
      />
    </section>

    <section class="panel two-col">
      <div>
        <h2>{{ store.tr("调试结果", "Debug Result") }}</h2>
        <p>{{ actionMessage || store.tr("写入结果展示 requested/applied/raw，读回结果展示 runtime 或传感器来源。", "Write results show requested/applied/raw values; read results show runtime or sensor source.") }}</p>
      </div>
      <div class="target-summary">
        <div v-for="row in resultRows(writeResult)" :key="`write-${row.label}`">
          <span>{{ row.label }}</span>
          <strong>{{ row.value }}</strong>
          <small>{{ store.tr("写入结果", "Write result") }}</small>
        </div>
        <div v-for="row in resultRows(readResult)" :key="`read-${row.label}`">
          <span>{{ row.label }}</span>
          <strong>{{ row.value }}</strong>
          <small>{{ store.tr("读回结果", "Read result") }}</small>
        </div>
      </div>
    </section>

    <section class="panel">
      <div class="panel-title">
        <h2>{{ store.tr("集成接口状态", "Integration Surface") }}</h2>
        <span>{{ store.tr("Modbus / MQTT / AINAS / LoRA 边界", "Modbus / MQTT / AINAS / LoRA boundary") }}</span>
      </div>
      <div class="target-summary integration-grid">
        <div>
          <span>Modbus TCP</span>
          <strong>{{ boolText(integrations?.modbus_tcp) }}</strong>
          <small>{{ displayAt(modbusTcpStatus, "bind") }}</small>
        </div>
        <div>
          <span>MQTT</span>
          <strong>{{ boolText(integrations?.mqtt) }}</strong>
          <small>{{ displayAt(mqttStatus, "broker") }}</small>
        </div>
        <div>
          <span>AINAS</span>
          <strong>{{ boolText(integrations?.ainas_ready) }}</strong>
          <small>{{ displayAt(integrations, "ainas_task_api") }}</small>
        </div>
        <div>
          <span>{{ store.tr("基础模型入口", "Base inference") }}</span>
          <strong>{{ displayAt(localAi, "ready_for_base_inference") }}</strong>
          <small>{{ displayAt(localAi, "inference_endpoint", displayAt(localAi, "model_path")) }}</small>
        </div>
        <div>
          <span>{{ store.tr("LoRA 推理闭环", "LoRA inference") }}</span>
          <strong>{{ displayAt(localAi, "ready_for_lora_inference") }}</strong>
          <small>{{ displayAt(localAi, "adapter_path") }}</small>
        </div>
        <div>
          <span>{{ store.tr("PRD LoRA/RK 闭环", "PRD LoRA/RK") }}</span>
          <strong>{{ displayAt(localAi, "ready_for_prd_lora") }}</strong>
          <small>{{ displayAt(localAi, "mode") }}</small>
        </div>
      </div>
    </section>

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
        <el-table-column :label="store.tr('当前值', 'Current value')" width="130">
          <template #default="{ row }">{{ textAt(row, "value") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('原始值', 'Raw')" width="110">
          <template #default="{ row }">{{ textAt(row, "raw") }}</template>
        </el-table-column>
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
