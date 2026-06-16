<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from "vue";
import { usePlantStore } from "../stores/plant";
import type { AinasTaskPayload, ApiRecord } from "../stores/plant";
import { arrayAt, numberAt, objectAt, textAt } from "./view-utils";

const store = usePlantStore();
const safety = computed(() => objectAt(store.config, "safety"));
const temperature = computed(() => objectAt(safety.value, "temperature"));
const stirrer = computed(() => objectAt(safety.value, "stirrer"));
const control = computed(() => objectAt(safety.value, "control"));
const optimizer = computed(() => objectAt(safety.value, "optimizer"));
const forbidden = computed(() => arrayAt(safety.value, "forbidden_control_zones"));
// runtimeInfo: live runtime fields (control_loop_terminated / last_sensor_error /
// last_control_error) surfaced via /api/live so the safety/isolation panel can
// show the real fail-safe state, not just static config.
const runtimeInfo = computed(() => objectAt(store.live, "runtime") ?? store.runtimeFallback);
const device = computed(() => objectAt(store.config, "device"));
const integrations = computed(() => objectAt(store.config, "integrations"));
const security = computed(() => objectAt(store.config, "data_security"));
const storageEncryption = computed(() => objectAt(security.value, "storage_encryption"));
const encryptedFields = computed(() => arrayAt<string>(storageEncryption.value, "encrypted_fields"));
const aiMemory = computed(() => objectAt(store.config, "ai_memory"));
const aiProvider = computed(() => objectAt(store.config, "ai_provider"));
const localAi = computed(() => objectAt(store.config, "local_ai"));
const permissions = computed(() => objectAt(store.config, "permissions"));
const configFieldScenario = computed(() => objectAt(store.config, "field_scenario"));
const liveFieldScenario = computed(() => objectAt(store.live, "field_scenario"));
const fieldScenario = computed(() => liveFieldScenario.value ?? configFieldScenario.value);
const fieldScenarioSignals = computed(() => arrayAt<string>(fieldScenario.value, "signals"));
const fieldScenarioActions = computed(() => arrayAt<string>(fieldScenario.value, "actions"));
const fieldScenarioNotes = computed(() => arrayAt<string>(fieldScenario.value, "notes"));
const configProductionLine = computed(() => objectAt(store.config, "production_line"));
const liveProductionLine = computed(() => objectAt(store.live, "production_line"));
const productionLine = computed(() => liveProductionLine.value ?? configProductionLine.value);
const productionLineSignals = computed(() => arrayAt<string>(productionLine.value, "signals"));
const productionLineActions = computed(() => arrayAt<string>(productionLine.value, "actions"));
const productionLineNotes = computed(() => arrayAt<string>(productionLine.value, "notes"));
const roles = computed(() => arrayAt(permissions.value, "roles"));
const defaultUsers = computed(() => arrayAt<Record<string, unknown> | string>(permissions.value, "default_users"));
const defaultUserLabels = computed(() => defaultUsers.value.map(defaultUserLabel));

const endpointGroups = [
  {
    name: { zh: "REST 公共", en: "REST public" },
    items: [
      "GET /health",
      "GET /api/live",
      "POST /api/auth/login"
    ]
  },
  {
    name: { zh: "REST 控制", en: "REST control" },
    items: [
      "POST /api/control/targets",
      "POST /api/control/auto",
      "POST /api/control/manual-lock",
      "POST /api/control/fault/reset",
      "POST /api/control/emergency-stop",
      "POST /api/control/emergency-stop/reset"
    ]
  },
  {
    name: { zh: "工艺 / 批次", en: "Process / batch" },
    items: [
      "GET  /api/processes",
      "POST /api/processes",
      "GET  /api/processes/:id",
      "POST /api/processes/:id/steps",
      "POST /api/processes/:id/start",
      "POST /api/processes/current/stop",
      "GET  /api/batches",
      "GET  /api/batches/:id",
      "GET  /api/batches/export.csv",
      "GET  /api/batches/:id/report.md"
    ]
  },
  {
    name: { zh: "AI / 推荐", en: "AI / recommendation" },
    items: [
      "GET  /api/ai/experiment-plan",
      "POST /api/ai/control",
      "GET  /api/recommendations/latest",
      "POST /api/recommendations/latest"
    ]
  },
  {
    name: { zh: "审计 / Modbus", en: "Audit / Modbus" },
    items: [
      "GET  /api/audit/logs",
      "GET  /api/audit/export.csv",
      "GET  /api/modbus/registers",
      "GET  /api/modbus/registers/:name/read",
      "POST /api/modbus/registers/:name/write"
    ]
  },
  {
    name: { zh: "集成", en: "Integrations" },
    items: [
      "POST /api/integrations/ainas/tasks",
      "GET  /api/integrations/ainas/tasks",
      "GET  /api/integrations/ainas/tasks/:id"
    ]
  },
  {
    name: { zh: "配置 / 权限", en: "Config / permissions" },
    items: [
      "GET /api/config/summary",
      "GET /api/permissions/roles"
    ]
  }
];

const mqttStatus = computed(() => objectAt(integrations.value, "mqtt_status"));
const modbusTcpStatus = computed(() => objectAt(integrations.value, "modbus_tcp_status"));

const valueTranslations: Record<string, { zh: string; en: string }> = {
  true: { zh: "是", en: "Yes" },
  false: { zh: "否", en: "No" },
  pipeline: { zh: "管线模式", en: "Pipeline" },
  local_optimizer: { zh: "本地优化器", en: "Local optimizer" },
  local_role_policy: { zh: "本地角色策略", en: "Local role policy" },
  bearer_session_enforced: { zh: "Bearer 会话认证", en: "Bearer session enforced" },
  operator: { zh: "操作员", en: "Operator" },
  engineer: { zh: "工程师", en: "Engineer" },
  admin: { zh: "管理员", en: "Admin" },
  prd_lora_ready: { zh: "PRD LoRA/RK 闭环", en: "PRD LoRA/RK ready" },
  lora_inference_ready: { zh: "LoRA 推理入口就绪", en: "LoRA inference ready" },
  base_inference_only: { zh: "仅基础模型入口", en: "Base model only" },
  configured_not_ready: { zh: "配置未就绪", en: "Configured, not ready" },
  disabled: { zh: "未启用", en: "Disabled" }
};

const permissionTranslations: Record<string, { zh: string; en: string }> = {
  view_monitor: { zh: "查看监控", en: "View monitor" },
  view_history: { zh: "查看历史", en: "View history" },
  view_audit: { zh: "查看审计", en: "View audit" },
  export_reports: { zh: "导出报告", en: "Export reports" },
  edit_process: { zh: "编辑工艺", en: "Edit process" },
  start_stop_process: { zh: "启停工艺", en: "Start/stop process" },
  set_safe_targets: { zh: "写入安全目标", en: "Set safe targets" },
  apply_ai_suggestion: { zh: "应用 AI 建议", en: "Apply AI suggestion" },
  emergency_stop: { zh: "急停控制", en: "Emergency stop" },
  modbus_debug: { zh: "Modbus 调试", en: "Modbus debug" },
  edit_system_config: { zh: "编辑系统配置", en: "Edit system config" },
  delete_data: { zh: "删除数据", en: "Delete data" },
  manage_users: { zh: "管理用户", en: "Manage users" },
  apply_integration_task: { zh: "执行集成任务", en: "Apply integration task" }
};

function integrationTag(name: string, on: boolean): "success" | "info" {
  return on ? "success" : "info";
}

function boolFrom(value: unknown): boolean {
  if (value === true) return true;
  if (typeof value === "string") return value === "true";
  return false;
}

function localizedValue(value: string): string {
  const hit = valueTranslations[value];
  if (!hit) return value;
  return store.isChinese ? hit.zh : hit.en;
}

function displayAt(source: unknown, key: string, fallback = "--"): string {
  const value = textAt(source, key, fallback);
  return value === fallback ? fallback : localizedValue(value);
}

function stringListAt(row: Record<string, unknown>, key: string): string[] {
  const value = row[key];
  return Array.isArray(value) ? value.map(String) : [];
}

function permissionLabel(permission: string): string {
  const hit = permissionTranslations[permission];
  if (!hit) return permission;
  return store.isChinese ? hit.zh : hit.en;
}

function permissionList(row: Record<string, unknown>): string[] {
  const can = stringListAt(row, "can");
  return (can.length > 0 ? can : stringListAt(row, "permissions")).map(permissionLabel);
}

function defaultUserLabel(row: Record<string, unknown> | string): string {
  if (!row || typeof row !== "object") return localizedValue(String(row || "--"));
  const username = textAt(row, "username", "");
  const role = textAt(row, "role", "");
  if (username && role) return `${username} (${localizedValue(role)})`;
  return username || localizedValue(role) || "--";
}

function permissionNote(): string {
  return store.isChinese
    ? "本地用户名/密码登录会签发 Bearer 会话；写入和导出操作会按角色权限校验。"
    : textAt(permissions.value, "note");
}

const mqttOn = computed(() => boolFrom(integrations.value?.mqtt));
const modbusRtuOn = computed(() => boolFrom(integrations.value?.modbus_rtu));
const modbusTcpOn = computed(() => boolFrom(integrations.value?.modbus_tcp));
const ainasOn = computed(() => boolFrom(integrations.value?.ainas_ready));
const restOn = computed(() => boolFrom(integrations.value?.rest_api));
const cliOn = computed(() => boolFrom(integrations.value?.cli));
const fieldScenarioTagType = computed(() => {
  if (textAt(fieldScenario.value, "kind") === "offline_demo") return "info";
  return "success";
});
const productionLineTagType = computed(() => {
  if (textAt(productionLine.value, "special_handling_required", "false") === "true") return "warning";
  return "success";
});

function fieldScenarioSourceLabel(source: string): string {
  if (source === "environment_override") return store.tr("环境覆盖", "Environment override");
  return store.tr("自动判断", "Auto detected");
}

function fieldScenarioListLabel(value: string): string {
  const labels: Record<string, { zh: string; en: string }> = {
    lab_research: { zh: "实验室研发", en: "Lab research" },
    pilot_scale: { zh: "中试放大", en: "Pilot scale" },
    legacy_retrofit: { zh: "旧线改造", en: "Legacy retrofit" },
    offline_demo: { zh: "离线演示", en: "Offline demo" }
  };
  const hit = labels[value];
  return hit ? (store.isChinese ? hit.zh : hit.en) : value;
}

function productionLineListLabel(value: string): string {
  const labels: Record<string, { zh: string; en: string }> = {
    general_chemistry: { zh: "通用化学", en: "General chemistry" },
    petrochemical_refining: { zh: "石油炼化", en: "Petrochemical refining" },
    biopharmaceutical: { zh: "生物制药", en: "Biopharmaceutical" },
    fine_chemical: { zh: "精细化工", en: "Fine chemical" },
    material_synthesis: { zh: "材料合成", en: "Material synthesis" }
  };
  const hit = labels[value];
  return hit ? (store.isChinese ? hit.zh : hit.en) : value;
}

const backendSurfaceLoading = ref(false);
const backendSurfaceError = ref("");
const componentSubmitting = ref(false);
const componentControlResult = ref<ApiRecord | null>(null);
const ainasSubmitting = ref(false);
const ainasMessage = ref("");

const devices = computed(() => arrayAt<ApiRecord>(store.deviceStatus, "devices"));
const capabilityDevices = computed(() => arrayAt<ApiRecord>(store.deviceCapabilities, "devices"));
const selectedCapabilityDevice = computed(() => capabilityDevices.value[0] ?? null);
const deviceSensors = computed(() => arrayAt<ApiRecord>(selectedCapabilityDevice.value, "sensors"));
const deviceComponents = computed(() => arrayAt<ApiRecord>(selectedCapabilityDevice.value, "components"));
const demoAlarms = computed(() => arrayAt<ApiRecord>(store.demoContext, "demo_alarms"));
const demoProcesses = computed(() => arrayAt<ApiRecord>(store.demoContext, "processes"));
const demoOutcomes = computed(() => arrayAt<ApiRecord>(store.demoContext, "recent_outcomes"));
const endpointRoleRows = computed(() => arrayAt<ApiRecord>(store.permissionRoles ?? permissions.value, "roles"));
const canCreateAinasTask = computed(() => store.role === "engineer" || store.role === "admin");

const componentForm = reactive({
  device_id: "",
  component_id: "",
  action: "",
  value: "",
  reason: "Vue component control acceptance"
});

const ainasForm = reactive({
  external_task_id: "",
  action: "set_targets",
  process_id: null as number | null,
  target_temperature_c: 65,
  target_stirrer_rpm: 320,
  target_shake_speed_cpm: 30,
  target_pressure_mpa: 0.5,
  heat_time_s: 120,
  hold_time_s: 60,
  cool_time_s: 60,
  reason: "Vue AINAS task acceptance"
});

const selectedComponent = computed(
  () => deviceComponents.value.find((component) => textAt(component, "component_id", "") === componentForm.component_id) ?? null
);
const componentActions = computed(() => arrayAt<ApiRecord>(selectedComponent.value, "actions"));
const selectedComponentAction = computed(
  () => componentActions.value.find((action) => textAt(action, "action", "") === componentForm.action) ?? null
);
const componentControlDisabled = computed(
  () =>
    !store.isAuthenticated ||
    componentSubmitting.value ||
    !componentForm.device_id ||
    !componentForm.component_id ||
    !componentForm.action ||
    !componentForm.reason.trim()
);

watch(
  capabilityDevices,
  (rows) => {
    if (!componentForm.device_id && rows.length > 0) componentForm.device_id = textAt(rows[0], "device_id", "");
  },
  { immediate: true }
);

watch(
  deviceComponents,
  (rows) => {
    if (rows.length > 0 && !rows.some((component) => textAt(component, "component_id", "") === componentForm.component_id)) {
      componentForm.component_id = textAt(rows[0], "component_id", "");
    }
  },
  { immediate: true }
);

watch(
  componentActions,
  (rows) => {
    if (rows.length > 0 && !rows.some((action) => textAt(action, "action", "") === componentForm.action)) {
      componentForm.action = textAt(rows[0], "action", "");
    }
  },
  { immediate: true }
);

async function loadBackendSurfaces(): Promise<void> {
  backendSurfaceLoading.value = true;
  backendSurfaceError.value = "";
  try {
    const tasks: Promise<unknown>[] = [
      store.loadDeviceStatus(),
      store.loadDeviceCapabilities(),
      store.loadDemoContext(),
      store.loadPermissionRoles()
    ];
    if (store.isAuthenticated) tasks.push(store.loadAinasTasks(20));
    const results = await Promise.allSettled(tasks);
    const rejected = results.find((result) => result.status === "rejected");
    if (rejected && rejected.status === "rejected") {
      backendSurfaceError.value = rejected.reason instanceof Error ? rejected.reason.message : String(rejected.reason);
    }
  } finally {
    backendSurfaceLoading.value = false;
  }
}

function actionValue(): string | number | boolean | undefined {
  const valueType = textAt(selectedComponentAction.value, "value_type", "").toLowerCase();
  if (!valueType || valueType === "none" || valueType === "void") return undefined;
  if (valueType === "bool" || valueType === "boolean") {
    return ["true", "1", "on", "yes"].includes(componentForm.value.trim().toLowerCase());
  }
  if (valueType === "number" || valueType === "float" || valueType === "integer" || valueType === "int") {
    const parsed = Number(componentForm.value);
    return Number.isFinite(parsed) ? parsed : 0;
  }
  return componentForm.value;
}

async function submitComponentControl(): Promise<void> {
  componentSubmitting.value = true;
  componentControlResult.value = null;
  store.error = null;
  try {
    const value = actionValue();
    componentControlResult.value = await store.controlDeviceComponent(componentForm.device_id, componentForm.component_id, {
      action: componentForm.action,
      value,
      reason: componentForm.reason
    });
  } catch (error) {
    store.error = error instanceof Error ? error.message : String(error);
  } finally {
    componentSubmitting.value = false;
  }
}

function cleanPayload<T extends Record<string, unknown>>(payload: T): T {
  return Object.fromEntries(
    Object.entries(payload).filter(([, value]) => value !== undefined && value !== "" && value !== null)
  ) as T;
}

function buildAinasPayload(): AinasTaskPayload {
  if (ainasForm.action === "set_targets") {
    return cleanPayload({
      external_task_id: ainasForm.external_task_id,
      action: ainasForm.action,
      target_temperature_c: ainasForm.target_temperature_c,
      target_stirrer_rpm: ainasForm.target_stirrer_rpm,
      target_shake_speed_cpm: ainasForm.target_shake_speed_cpm,
      target_pressure_mpa: ainasForm.target_pressure_mpa,
      heat_time_s: ainasForm.heat_time_s,
      hold_time_s: ainasForm.hold_time_s,
      cool_time_s: ainasForm.cool_time_s,
      reason: ainasForm.reason
    });
  }
  return cleanPayload({
    external_task_id: ainasForm.external_task_id,
    action: ainasForm.action,
    process_id: ainasForm.process_id,
    reason: ainasForm.reason
  });
}

async function createAinasTaskFromForm(): Promise<void> {
  ainasSubmitting.value = true;
  ainasMessage.value = "";
  store.error = null;
  try {
    const task = await store.createAinasTask(buildAinasPayload());
    ainasMessage.value = `Task #${textAt(task, "id")} ${textAt(task, "status")}`;
  } catch (error) {
    store.error = error instanceof Error ? error.message : String(error);
  } finally {
    ainasSubmitting.value = false;
  }
}

async function refreshAinasTasks(): Promise<void> {
  ainasSubmitting.value = true;
  try {
    await store.loadAinasTasks(20);
  } catch (error) {
    store.error = error instanceof Error ? error.message : String(error);
  } finally {
    ainasSubmitting.value = false;
  }
}

function tagForStatus(status: string): "success" | "warning" | "info" | "danger" {
  if (status === "executed" || status === "online" || status === "running") return "success";
  if (status === "failed" || status === "rejected" || status === "error" || status === "offline") return "danger";
  if (status === "executing" || status === "received" || status === "stale") return "warning";
  return "info";
}

function compactJson(value: unknown): string {
  if (value === null || value === undefined || value === "") return "--";
  const text = typeof value === "string" ? value : JSON.stringify(value);
  return text.length > 96 ? `${text.slice(0, 96)}...` : text;
}

onMounted(() => {
  void loadBackendSurfaces();
});
</script>

<template>
  <section class="view-stack">
    <div class="view-heading">
      <div>
        <p class="eyebrow">{{ store.tr("系统配置", "System Configuration") }}</p>
        <h1>{{ store.tr("系统配置", "System Settings") }}</h1>
        <span>{{ store.tr("设备、安全、AI、集成、存储、权限和端点矩阵", "Device, safety, AI, integration, storage, permissions, and endpoint matrix") }}</span>
      </div>
      <div class="heading-actions">
        <el-tag>{{ store.tr("角色", "Role") }}: {{ store.role }}</el-tag>
        <el-tag :type="store.isAuthenticated ? 'success' : 'info'">
          {{ store.isAuthenticated ? store.tr("已登录", "Signed in") : store.tr("未登录", "Signed out") }}
        </el-tag>
      </div>
    </div>

    <section class="panel">
      <div class="panel-title">
        <div>
          <h2>{{ store.tr("设备与组件", "Devices & Components") }}</h2>
          <p>{{ store.tr("展示 /api/devices/status 与 /api/devices/capabilities，并提供组件级控制入口。", "Displays /api/devices/status and /api/devices/capabilities, with component-level control.") }}</p>
        </div>
        <div class="heading-actions">
          <el-tag :type="devices.length > 0 ? 'success' : 'warning'">{{ devices.length }} devices</el-tag>
          <el-button size="small" :loading="backendSurfaceLoading" @click="loadBackendSurfaces">
            {{ store.tr("刷新后端组件", "Refresh Surfaces") }}
          </el-button>
        </div>
      </div>
      <el-alert
        v-if="backendSurfaceError"
        class="control-alert"
        type="warning"
        :closable="false"
        show-icon
        :title="backendSurfaceError"
      />
      <div class="target-summary integration-grid">
        <div>
          <span>{{ store.tr("设备总数", "Total devices") }}</span>
          <strong>{{ textAt(store.deviceStatus, "total_count") }}</strong>
          <small>{{ store.tr("在线", "Online") }} {{ textAt(store.deviceStatus, "online_count") }}</small>
        </div>
        <div>
          <span>{{ store.tr("能力设备", "Capability devices") }}</span>
          <strong>{{ textAt(store.deviceCapabilities, "total_count") }}</strong>
          <small>{{ store.tr("模式", "Mode") }} {{ textAt(selectedCapabilityDevice, "mode") }}</small>
        </div>
        <div>
          <span>{{ store.tr("传感器", "Sensors") }}</span>
          <strong>{{ deviceSensors.length }}</strong>
          <small>{{ store.tr("来自能力接口", "from capabilities") }}</small>
        </div>
        <div>
          <span>{{ store.tr("执行组件", "Components") }}</span>
          <strong>{{ deviceComponents.length }}</strong>
          <small>{{ store.tr("可控", "controllable") }} {{ deviceComponents.filter((item) => textAt(item, "controllable") === "true").length }}</small>
        </div>
      </div>
      <div class="backend-surface-grid">
        <el-table :data="devices" class="data-table" size="small">
          <el-table-column :label="store.tr('设备', 'Device')" min-width="130">
            <template #default="{ row }">{{ textAt(row, "device_id") }}</template>
          </el-table-column>
          <el-table-column :label="store.tr('状态', 'Status')" width="100">
            <template #default="{ row }">
              <el-tag :type="tagForStatus(textAt(row, 'status'))" size="small">{{ textAt(row, "status") }}</el-tag>
            </template>
          </el-table-column>
          <el-table-column :label="store.tr('在线', 'Online')" width="80">
            <template #default="{ row }">{{ textAt(row, "online") }}</template>
          </el-table-column>
          <el-table-column :label="store.tr('帧校验', 'Frame ok')" width="90">
            <template #default="{ row }">
              <el-tag :type="textAt(row, 'last_frame_ok') === 'true' ? 'success' : 'danger'" size="small">
                {{ textAt(row, "last_frame_ok") === "true" ? store.tr("正常", "ok") : store.tr("异常", "bad") }}
              </el-tag>
            </template>
          </el-table-column>
          <el-table-column :label="store.tr('命令成功', 'Cmd ok')" width="90">
            <template #default="{ row }">
              <el-tag :type="textAt(row, 'last_command_ok') === 'true' ? 'success' : 'danger'" size="small">
                {{ textAt(row, "last_command_ok") === "true" ? store.tr("成功", "ok") : store.tr("失败", "fail") }}
              </el-tag>
            </template>
          </el-table-column>
          <el-table-column :label="store.tr('命令错误', 'Command error')" min-width="160">
            <template #default="{ row }">
              <span v-if="textAt(row, 'last_command_error')" class="muted">{{ textAt(row, "last_command_error") }}</span>
              <span v-else class="muted">--</span>
            </template>
          </el-table-column>
          <el-table-column :label="store.tr('最近命令', 'Last command')" min-width="160">
            <template #default="{ row }">{{ textAt(row, "last_command_request_id", "--") }}</template>
          </el-table-column>
        </el-table>

        <el-form label-position="top" class="component-action-form">
          <el-form-item :label="store.tr('设备 ID', 'Device ID')">
            <el-input v-model="componentForm.device_id" />
          </el-form-item>
          <el-form-item :label="store.tr('组件', 'Component')">
            <el-select v-model="componentForm.component_id" filterable>
              <el-option
                v-for="component in deviceComponents"
                :key="textAt(component, 'component_id')"
                :label="`${textAt(component, 'label')} / ${textAt(component, 'component_id')}`"
                :value="textAt(component, 'component_id')"
              />
            </el-select>
          </el-form-item>
          <el-form-item :label="store.tr('动作', 'Action')">
            <el-select v-model="componentForm.action" filterable>
              <el-option
                v-for="action in componentActions"
                :key="textAt(action, 'action')"
                :label="`${textAt(action, 'label')} / ${textAt(action, 'action')}`"
                :value="textAt(action, 'action')"
              />
            </el-select>
          </el-form-item>
          <el-form-item :label="`${store.tr('值', 'Value')} (${textAt(selectedComponentAction, 'value_type', 'none')})`">
            <el-input v-model="componentForm.value" :placeholder="store.tr('无值动作可留空', 'Leave empty for no-value actions')" />
          </el-form-item>
          <el-form-item :label="store.tr('审计原因', 'Audit reason')" class="reason-field">
            <el-input v-model="componentForm.reason" maxlength="200" show-word-limit />
          </el-form-item>
          <div class="control-actions">
            <el-button type="primary" :loading="componentSubmitting" :disabled="componentControlDisabled" @click="submitComponentControl">
              {{ store.tr("执行组件控制", "Run Component Control") }}
            </el-button>
            <span class="muted">{{ store.tr("需要登录和 SetSafeTargets 权限", "Requires login and SetSafeTargets permission") }}</span>
          </div>
        </el-form>
      </div>
      <el-table :data="deviceComponents" class="data-table component-table" size="small">
        <el-table-column :label="store.tr('组件', 'Component')" min-width="190">
          <template #default="{ row }">{{ textAt(row, "label") }} / {{ textAt(row, "component_id") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('状态', 'Status')" width="120">
          <template #default="{ row }">
            <el-tag :type="tagForStatus(textAt(row, 'status'))" size="small">{{ textAt(row, "status") }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="store.tr('动作', 'Actions')" min-width="260">
          <template #default="{ row }">
            {{ arrayAt(row, "actions").map((action) => textAt(action, "action")).join(", ") || "--" }}
          </template>
        </el-table-column>
        <el-table-column :label="store.tr('状态数据', 'State')" min-width="260">
          <template #default="{ row }">{{ compactJson(row.state) }}</template>
        </el-table-column>
      </el-table>
      <el-alert
        v-if="componentControlResult"
        class="control-alert"
        type="success"
        :closable="false"
        show-icon
        :title="store.tr('组件控制完成', 'Component control completed')"
        :description="compactJson(componentControlResult)"
      />
    </section>

    <section class="panel">
      <div class="panel-title">
        <div>
          <h2>{{ store.tr("AINAS 任务中心", "AINAS Task Center") }}</h2>
          <p>{{ store.tr("实际接入 /api/integrations/ainas/tasks，可查看任务并发起受控 set_targets/start/stop。", "Uses /api/integrations/ainas/tasks to list tasks and submit controlled set_targets/start/stop actions.") }}</p>
        </div>
        <div class="heading-actions">
          <el-tag :type="canCreateAinasTask ? 'success' : 'info'">{{ store.role }}</el-tag>
          <el-button size="small" :loading="ainasSubmitting" :disabled="!store.isAuthenticated" @click="refreshAinasTasks">
            {{ store.tr("刷新任务", "Refresh Tasks") }}
          </el-button>
        </div>
      </div>
      <div class="backend-surface-grid">
        <el-form label-position="top" class="component-action-form">
          <el-form-item :label="store.tr('外部任务 ID', 'External task ID')">
            <el-input v-model="ainasForm.external_task_id" placeholder="optional" />
          </el-form-item>
          <el-form-item :label="store.tr('动作', 'Action')">
            <el-select v-model="ainasForm.action">
              <el-option label="set_targets" value="set_targets" />
              <el-option label="start_process" value="start_process" />
              <el-option label="stop_process" value="stop_process" />
            </el-select>
          </el-form-item>
          <el-form-item v-if="ainasForm.action !== 'set_targets'" :label="store.tr('工艺 ID', 'Process ID')">
            <el-input-number v-model="ainasForm.process_id" :min="1" controls-position="right" />
          </el-form-item>
          <template v-if="ainasForm.action === 'set_targets'">
            <el-form-item :label="store.tr('目标温度 C', 'Target temp C')">
              <el-input-number v-model="ainasForm.target_temperature_c" :min="0" :max="220" controls-position="right" />
            </el-form-item>
            <el-form-item :label="store.tr('目标转速 RPM', 'Target RPM')">
              <el-input-number v-model="ainasForm.target_stirrer_rpm" :min="0" :max="1800" controls-position="right" />
            </el-form-item>
            <el-form-item :label="store.tr('摇摆 CPM', 'Shake CPM')">
              <el-input-number v-model="ainasForm.target_shake_speed_cpm" :min="0" :max="60" controls-position="right" />
            </el-form-item>
            <el-form-item :label="store.tr('压力 MPa', 'Pressure MPa')">
              <el-input-number v-model="ainasForm.target_pressure_mpa" :min="0" :max="10" :step="0.1" controls-position="right" />
            </el-form-item>
          </template>
          <el-form-item :label="store.tr('原因', 'Reason')" class="reason-field">
            <el-input v-model="ainasForm.reason" maxlength="220" show-word-limit />
          </el-form-item>
          <div class="control-actions">
            <el-button type="danger" :loading="ainasSubmitting" :disabled="!canCreateAinasTask || ainasSubmitting" @click="createAinasTaskFromForm">
              {{ store.tr("提交 AINAS 任务", "Submit AINAS Task") }}
            </el-button>
            <span class="muted">{{ ainasMessage || store.tr("engineer/admin 才可提交集成任务", "engineer/admin can submit integration tasks") }}</span>
          </div>
        </el-form>
        <el-table :data="store.ainasTasks" class="data-table" size="small">
          <el-table-column label="ID" width="70">
            <template #default="{ row }">{{ textAt(row, "id") }}</template>
          </el-table-column>
          <el-table-column :label="store.tr('动作', 'Action')" width="130">
            <template #default="{ row }">{{ textAt(row, "action") }}</template>
          </el-table-column>
          <el-table-column :label="store.tr('状态', 'Status')" width="120">
            <template #default="{ row }">
              <el-tag :type="tagForStatus(textAt(row, 'status'))" size="small">{{ textAt(row, "status") }}</el-tag>
            </template>
          </el-table-column>
          <el-table-column :label="store.tr('外部 ID', 'External ID')" min-width="150">
            <template #default="{ row }">{{ textAt(row, "external_task_id", "--") }}</template>
          </el-table-column>
          <el-table-column :label="store.tr('响应', 'Response')" min-width="260">
            <template #default="{ row }">{{ compactJson(row.response) }}</template>
          </el-table-column>
        </el-table>
      </div>
    </section>

    <section class="panel two-col">
      <div>
        <h2>{{ store.tr("演示上下文", "Demo Context") }}</h2>
        <p>{{ store.tr("接入 /api/demo/context，展示不伪造传感器数据的演示上下文、demo alarms 和参考批次。", "Uses /api/demo/context to show demo context, demo alarms, and reference batches without fabricating sensor data.") }}</p>
        <div class="target-summary">
          <div>
            <span>{{ store.tr("演示策略", "Demo policy") }}</span>
            <strong>{{ textAt(store.demoContext, "demo") }}</strong>
            <small>{{ textAt(store.demoContext, "sensor_data_policy") }}</small>
          </div>
          <div>
            <span>{{ store.tr("工艺", "Processes") }}</span>
            <strong>{{ demoProcesses.length }}</strong>
            <small>{{ store.tr("来自 demo context", "from demo context") }}</small>
          </div>
          <div>
            <span>{{ store.tr("结果", "Outcomes") }}</span>
            <strong>{{ demoOutcomes.length }}</strong>
            <small>{{ store.tr("参考批次", "reference batches") }}</small>
          </div>
          <div>
            <span>{{ store.tr("Demo alarms", "Demo alarms") }}</span>
            <strong>{{ demoAlarms.length }}</strong>
            <small>{{ textAt(objectAt(store.demoContext, "ai_memory"), "profile_name") }}</small>
          </div>
        </div>
      </div>
      <div>
        <h2>{{ store.tr("权限角色接口", "Permission Roles Endpoint") }}</h2>
        <p>{{ store.tr("接入 /api/permissions/roles，而不是只展示 config summary 内嵌权限。", "Uses /api/permissions/roles instead of only the embedded config summary permissions.") }}</p>
        <el-table :data="endpointRoleRows" class="data-table" size="small">
          <el-table-column :label="store.tr('角色', 'Role')" width="120">
            <template #default="{ row }">{{ displayAt(row, "role") }}</template>
          </el-table-column>
          <el-table-column :label="store.tr('能力', 'Capabilities')" min-width="260">
            <template #default="{ row }">{{ permissionList(row).join(", ") || "--" }}</template>
          </el-table-column>
        </el-table>
      </div>
    </section>

    <section class="panel two-col">
      <el-descriptions :column="1" border>
        <el-descriptions-item :label="store.tr('应用场景', 'Deployment scenario')">
          <el-tag :type="fieldScenarioTagType" size="small">
            {{ fieldScenarioListLabel(textAt(fieldScenario, "kind")) }}
          </el-tag>
        </el-descriptions-item>
        <el-descriptions-item :label="store.tr('适宜产线', 'Production line')">
          <el-tag :type="productionLineTagType" size="small">
            {{ productionLineListLabel(textAt(productionLine, "kind")) }}
          </el-tag>
        </el-descriptions-item>
        <el-descriptions-item :label="store.tr('判定来源', 'Detection source')">
          {{ fieldScenarioSourceLabel(textAt(fieldScenario, "source")) }}
        </el-descriptions-item>
        <el-descriptions-item :label="store.tr('产线来源', 'Line source')">
          {{ fieldScenarioSourceLabel(textAt(productionLine, "source")) }}
        </el-descriptions-item>
        <el-descriptions-item :label="store.tr('设备模式', 'Device mode')">
          {{ textAt(fieldScenario, "device_mode") }}
        </el-descriptions-item>
        <el-descriptions-item :label="store.tr('站点标识', 'Site label')">
          {{ textAt(fieldScenario, "site_label") }}
        </el-descriptions-item>
        <el-descriptions-item :label="store.tr('置信度', 'Confidence')">
          {{ textAt(fieldScenario, "confidence") }}
        </el-descriptions-item>
        <el-descriptions-item :label="store.tr('产线置信度', 'Line confidence')">
          {{ textAt(productionLine, "confidence") }}
        </el-descriptions-item>
        <el-descriptions-item :label="store.tr('专项处理', 'Special handling')">
          <el-tag :type="textAt(productionLine, 'special_handling_required') === 'true' ? 'warning' : 'success'" size="small">
            {{ textAt(productionLine, "special_handling_required") === "true" ? store.tr("需要复核", "Review required") : store.tr("常规", "Normal") }}
          </el-tag>
        </el-descriptions-item>
        <el-descriptions-item :label="store.tr('石油炼化处理', 'Petrochemical handling')">
          <el-tag :type="textAt(productionLine, 'petrochemical_handling_required') === 'true' ? 'warning' : 'success'" size="small">
            {{ textAt(productionLine, "petrochemical_handling_required") === "true" ? store.tr("需要复核", "Review required") : store.tr("常规", "Normal") }}
          </el-tag>
        </el-descriptions-item>
      </el-descriptions>
      <div class="analysis-block">
        <h2>{{ store.tr("场景动作", "Scenario Actions") }}</h2>
        <p class="muted">{{ fieldScenarioActions.join("; ") || "--" }}</p>
        <h2>{{ store.tr("识别信号", "Detection Signals") }}</h2>
        <p class="muted">{{ fieldScenarioSignals.join(", ") || "--" }}</p>
        <p v-if="fieldScenarioNotes.length > 0" class="muted">{{ fieldScenarioNotes.join(" ") }}</p>
        <h2>{{ store.tr("产线动作", "Production Line Actions") }}</h2>
        <p class="muted">{{ productionLineActions.join("; ") || "--" }}</p>
        <h2>{{ store.tr("产线信号", "Production Line Signals") }}</h2>
        <p class="muted">{{ productionLineSignals.join(", ") || "--" }}</p>
        <p v-if="productionLineNotes.length > 0" class="muted">{{ productionLineNotes.join(" ") }}</p>
      </div>
    </section>

    <section class="panel two-col">
      <el-descriptions :column="1" border>
        <el-descriptions-item :label="store.tr('设备模式', 'Device mode')">{{ displayAt(store.config, "device_mode") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('设备驱动', 'Device driver')">{{ displayAt(device, "mode") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('推荐 provider', 'Provider model')">{{ textAt(aiProvider, "model") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('Provider 模式', 'Provider mode')">{{ displayAt(aiProvider, "mode") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('AI 模式', 'AI mode')">{{ displayAt(localAi, "mode") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('基础模型入口', 'Base inference')">{{ displayAt(localAi, "ready_for_base_inference") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('LoRA 推理闭环', 'LoRA inference')">{{ displayAt(localAi, "ready_for_lora_inference") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('LoRA 训练就绪', 'LoRA training ready')">{{ displayAt(localAi, "ready_for_training") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('PRD LoRA/RK 闭环', 'PRD LoRA/RK')">{{ displayAt(localAi, "ready_for_prd_lora") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('AI 记忆', 'AI memory')">
          {{ textAt(aiMemory, "profile_name") }} / {{ textAt(aiMemory, "profile_version") }}
        </el-descriptions-item>
        <el-descriptions-item :label="store.tr('参考批次', 'Reference batches')">{{ textAt(aiMemory, "reference_batch_count") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('禁区数量', 'Forbidden zones')">{{ textAt(aiMemory, "forbidden_zone_count") }}</el-descriptions-item>
      </el-descriptions>
      <div class="analysis-block">
        <h2>{{ store.tr("存储安全", "Storage Security") }}</h2>
        <p>
          <strong>{{ store.tr("算法", "Algorithm") }}:</strong>
          <span>{{ textAt(storageEncryption, "algorithm") }}</span>
        </p>
        <p>
          <strong>{{ store.tr("启用", "Enabled") }}:</strong>
          <span>{{ displayAt(storageEncryption, "enabled") }}</span>
        </p>
        <p>
          <strong>{{ store.tr("密钥来源", "Key source") }}:</strong>
          <span>{{ textAt(storageEncryption, "key_source") }}</span>
        </p>
        <p>
          <strong>{{ store.tr("加密字段", "Encrypted fields") }}:</strong>
          <span>{{ encryptedFields.join(", ") || "--" }}</span>
        </p>
      </div>
    </section>

    <section class="panel two-col">
      <el-descriptions :column="1" border>
        <el-descriptions-item :label="store.tr('温度上限', 'Temperature max')">{{ textAt(temperature, "max_c") }} C</el-descriptions-item>
        <el-descriptions-item :label="store.tr('温度下限', 'Temperature min')">{{ textAt(temperature, "min_c") }} C</el-descriptions-item>
        <el-descriptions-item :label="store.tr('温度步长', 'Temperature step')">{{ textAt(temperature, "max_step_c") }} C / call</el-descriptions-item>
        <el-descriptions-item :label="store.tr('搅拌上限', 'Stirrer max')">{{ textAt(stirrer, "max_rpm") }} rpm</el-descriptions-item>
        <el-descriptions-item :label="store.tr('搅拌步长', 'Stirrer step')">{{ textAt(stirrer, "max_step_rpm") }} rpm / call</el-descriptions-item>
        <el-descriptions-item :label="store.tr('控制间隔', 'Control interval')">{{ textAt(control, "control_interval_ms") }} ms</el-descriptions-item>
        <el-descriptions-item :label="store.tr('传感器超时', 'Sensor timeout')">{{ textAt(control, "sensor_timeout_ms") }} ms</el-descriptions-item>
        <el-descriptions-item :label="store.tr('安全进程超时', 'Safety guard timeout')">{{ textAt(control, "safety_guard_timeout_ms") }} ms</el-descriptions-item>
        <el-descriptions-item :label="store.tr('优化器温度区间', 'Optimizer temperature range')">{{ textAt(optimizer, "min_temperature_c") }} - {{ textAt(optimizer, "max_temperature_c") }} C</el-descriptions-item>
        <el-descriptions-item :label="store.tr('优化器转速区间', 'Optimizer stirrer range')">{{ textAt(optimizer, "min_stirrer_rpm") }} - {{ textAt(optimizer, "max_stirrer_rpm") }} rpm</el-descriptions-item>
      </el-descriptions>
      <div class="analysis-block">
        <h2>{{ store.tr('禁区', 'Forbidden Zones') }}</h2>
        <el-table v-if="forbidden.length > 0" :data="forbidden" class="data-table" size="small">
          <el-table-column :label="store.tr('名称', 'Name')" min-width="160">
            <template #default="{ row }">{{ textAt(row, "name") }}</template>
          </el-table-column>
          <el-table-column :label="store.tr('温度区间', 'Temp C')" min-width="140">
            <template #default="{ row }">{{ textAt(row, "min_temperature_c") }} - {{ textAt(row, "max_temperature_c") }}</template>
          </el-table-column>
          <el-table-column :label="store.tr('转速区间', 'RPM')" min-width="140">
            <template #default="{ row }">{{ textAt(row, "min_stirrer_rpm") }} - {{ textAt(row, "max_stirrer_rpm") }}</template>
          </el-table-column>
          <el-table-column :label="store.tr('说明', 'Reason')" min-width="220">
            <template #default="{ row }">{{ textAt(row, "reason") }}</template>
          </el-table-column>
        </el-table>
        <p v-else class="muted">{{ store.tr("未配置禁区。", "No forbidden zones configured.") }}</p>
      </div>
    </section>

    <section class="panel two-col">
      <div class="target-summary integration-grid">
        <div>
          <span>MQTT</span>
          <strong>
            <el-tag :type="integrationTag('mqtt', mqttOn)">
              {{ mqttOn ? store.tr("已启用", "Enabled") : store.tr("未启用", "Disabled") }}
            </el-tag>
          </strong>
          <small>{{ textAt(mqttStatus, "broker") }}</small>
        </div>
        <div>
          <span>Modbus RTU</span>
          <strong>
            <el-tag :type="integrationTag('modbus_rtu', modbusRtuOn)">
              {{ modbusRtuOn ? store.tr("已启用", "Enabled") : store.tr("未启用", "Disabled") }}
            </el-tag>
          </strong>
          <small>{{ textAt(mqttStatus, "use_tls") === "true" ? store.tr("已配置 TLS", "TLS configured") : "" }}</small>
        </div>
        <div>
          <span>Modbus TCP</span>
          <strong>
            <el-tag :type="integrationTag('modbus_tcp', modbusTcpOn)">
              {{ modbusTcpOn ? store.tr("已启用", "Enabled") : store.tr("未启用", "Disabled") }}
            </el-tag>
          </strong>
          <small>{{ textAt(modbusTcpStatus, "bind") }}</small>
        </div>
        <div>
          <span>AINAS</span>
          <strong>
            <el-tag :type="integrationTag('ainas', ainasOn)">
              {{ ainasOn ? store.tr("已接入", "Connected") : store.tr("未接入", "Not connected") }}
            </el-tag>
          </strong>
          <small>{{ displayAt(integrations, "ainas_task_api") }}</small>
        </div>
        <div>
          <span>REST API</span>
          <strong>
            <el-tag :type="integrationTag('rest', restOn)">
              {{ restOn ? store.tr("已上线", "Online") : store.tr("未上线", "Offline") }}
            </el-tag>
          </strong>
          <small>--</small>
        </div>
        <div>
          <span>CLI</span>
          <strong>
            <el-tag :type="integrationTag('cli', cliOn)">
              {{ cliOn ? store.tr("已发布", "Released") : store.tr("未发布", "Not released") }}
            </el-tag>
          </strong>
          <small>xingshu</small>
        </div>
      </div>
      <div class="analysis-block">
        <h2>{{ store.tr("集成状态", "Integration Status") }}</h2>
        <p v-if="textAt(mqttStatus, 'last_error')">
          <strong>MQTT:</strong> {{ textAt(mqttStatus, "last_error") }}
        </p>
        <p v-else class="muted">{{ store.tr("MQTT 暂无错误。", "MQTT has no current error.") }}</p>
        <p v-if="textAt(modbusTcpStatus, 'last_error')">
          <strong>Modbus TCP:</strong> {{ textAt(modbusTcpStatus, "last_error") }}
        </p>
        <p v-else class="muted">{{ store.tr("Modbus TCP 暂无错误。", "Modbus TCP has no current error.") }}</p>
      </div>
    </section>

    <section class="panel">
      <div class="panel-title">
        <h2>{{ store.tr("MQTT 连接详情", "MQTT Connection Detail") }}</h2>
        <el-tag :type="textAt(mqttStatus, 'connected') === 'true' ? 'success' : 'danger'" size="small">
          {{ textAt(mqttStatus, "connected") === "true" ? store.tr("已连接", "Connected") : store.tr("未连接", "Disconnected") }}
        </el-tag>
      </div>
      <el-descriptions :column="2" border>
        <el-descriptions-item :label="store.tr('连接状态', 'Connected')">{{ textAt(mqttStatus, "connected") === "true" ? store.tr("已连接", "Connected") : store.tr("未连接", "Disconnected") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('Broker', 'Broker')">{{ textAt(mqttStatus, "broker") || "--" }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('任务主题', 'Task topic')">{{ textAt(mqttStatus, "task_topic") || "--" }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('回执主题', 'Receipt topic')">{{ textAt(mqttStatus, "receipt_topic") || "--" }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('状态主题', 'Status topic')">{{ textAt(mqttStatus, "status_topic") || "--" }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('告警主题', 'Alert topic')">{{ textAt(mqttStatus, "alert_topic") || "--" }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('CA 证书', 'CA cert configured')">{{ textAt(mqttStatus, "ca_cert_configured") === "true" ? store.tr("已配置", "Configured") : store.tr("未配置", "Not configured") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('客户端证书', 'Client cert configured')">{{ textAt(mqttStatus, "client_cert_configured") === "true" ? store.tr("已配置", "Configured") : store.tr("未配置", "Not configured") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('最近任务', 'Last task id')">{{ textAt(mqttStatus, "last_task_id") || "--" }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('最近告警', 'Last alert at')">{{ textAt(mqttStatus, "last_alert_at") || "--" }}</el-descriptions-item>
      </el-descriptions>
      <p v-if="textAt(mqttStatus, 'last_error')" class="muted">
        <strong>{{ store.tr("最近错误", "Last error") }}:</strong> {{ textAt(mqttStatus, "last_error") }}
      </p>
    </section>

    <section class="panel">
      <div class="panel-title">
        <h2>{{ store.tr("安全控制与隔离", "Safety Control & Isolation") }}</h2>
        <span class="muted">{{ store.tr("fail-safe 监督 / 隔离子进程 / 限幅", "Fail-safe supervisor / isolation subprocess / clamping") }}</span>
      </div>
      <el-descriptions :column="2" border>
        <el-descriptions-item :label="store.tr('安全进程超时', 'Safety guard timeout')">{{ textAt(control, "safety_guard_timeout_ms") }} ms</el-descriptions-item>
        <el-descriptions-item :label="store.tr('控制环监督', 'Control loop supervisor')">
          <el-tag :type="textAt(runtimeInfo, 'control_loop_terminated') === 'true' ? 'danger' : 'success'" size="small">
            {{ textAt(runtimeInfo, "control_loop_terminated") === "true" ? store.tr("已终止（需重启）", "Terminated (restart req)") : store.tr("运行中", "Running") }}
          </el-tag>
        </el-descriptions-item>
        <el-descriptions-item :label="store.tr('传感器故障', 'Sensor fault (fail-closed)')">
          <el-tag :type="textAt(runtimeInfo, 'last_sensor_error') ? 'warning' : 'success'" size="small">
            {{ textAt(runtimeInfo, "last_sensor_error") ? store.tr("有故障", "Fault") : store.tr("正常", "Ok") }}
          </el-tag>
        </el-descriptions-item>
        <el-descriptions-item :label="store.tr('控制写入故障', 'Control write fault')">
          <el-tag :type="textAt(runtimeInfo, 'last_control_error') ? 'warning' : 'success'" size="small">
            {{ textAt(runtimeInfo, "last_control_error") ? store.tr("已锁存", "Latched") : store.tr("正常", "Ok") }}
          </el-tag>
        </el-descriptions-item>
        <el-descriptions-item v-if="textAt(runtimeInfo, 'last_sensor_error')" :label="store.tr('传感器故障详情', 'Sensor fault detail')" :span="2">
          {{ textAt(runtimeInfo, "last_sensor_error") }}
        </el-descriptions-item>
      </el-descriptions>
      <p class="muted">{{ store.tr("安全决策经独立 safety-guard 子进程隔离判定；监督任务死亡时自动控制被禁用且只能重启恢复。", "Safety decisions are isolated through an independent safety-guard subprocess; supervisor task death disables auto-control and only a restart can recover it.") }}</p>
    </section>

    <section class="panel two-col">
      <div class="analysis-block">
        <h2>{{ store.tr("权限矩阵", "Permission Matrix") }}</h2>
        <p class="muted">{{ permissionNote() }}</p>
        <p>
          <strong>{{ store.tr("认证", "Authentication") }}:</strong>
          <span>{{ displayAt(permissions, "authentication") }}</span>
        </p>
        <p>
          <strong>{{ store.tr("模式", "Mode") }}:</strong>
          <span>{{ displayAt(permissions, "mode") }}</span>
        </p>
        <p>
          <strong>{{ store.tr("默认用户", "Default users") }}:</strong>
          <span>{{ defaultUserLabels.join(", ") || "--" }}</span>
        </p>
      </div>
      <el-table v-if="roles.length > 0" :data="roles" class="data-table" size="small">
        <el-table-column :label="store.tr('角色', 'Role')" min-width="120">
          <template #default="{ row }">{{ displayAt(row, "role") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('能力数', 'Capabilities')" width="120">
          <template #default="{ row }">{{ permissionList(row).length }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('能力', 'Capabilities')" min-width="320">
          <template #default="{ row }">{{ permissionList(row).join(", ") || "--" }}</template>
        </el-table-column>
      </el-table>
    </section>

    <section class="panel">
      <div class="panel-title">
        <h2>{{ store.tr("端点矩阵", "Endpoint Matrix") }}</h2>
        <el-tag>{{ store.tr("Bearer Token + RBAC", "Bearer Token + RBAC") }}</el-tag>
      </div>
      <div class="endpoint-grid">
        <div v-for="group in endpointGroups" :key="group.name.en" class="endpoint-card">
          <h3>{{ store.tr(group.name.zh, group.name.en) }}</h3>
          <ul>
            <li v-for="item in group.items" :key="item">{{ item }}</li>
          </ul>
        </div>
      </div>
    </section>
  </section>
</template>
