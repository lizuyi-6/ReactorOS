<script setup lang="ts">
import { computed, onMounted, reactive, ref } from "vue";
import { ElMessage } from "element-plus";
import PageHeader from "../components/PageHeader.vue";
import EmptyState from "../components/EmptyState.vue";
import HmiButton from "../components/HmiButton.vue";
import { ainasApi, deviceApi } from "../api";
import { errorMessage } from "../api/errors";
import { useAuthStore } from "../stores/auth";
import { usePlantStore } from "../stores/plant";
import { useLanguage } from "../i18n";
import { formatTimestamp, text } from "../utils/format";
import type { ComponentAction, DeviceComponentItem, IntegrationTask } from "../api/types";

const auth = useAuthStore();
const plant = usePlantStore();
const { tr } = useLanguage();

const loading = ref(false);
const submitting = ref(false);

const config = computed(() => plant.config);
const devices = computed(() => plant.deviceStatus?.devices ?? []);
const capabilityDevices = computed(() => plant.deviceCapabilities?.devices ?? []);
const roles = computed(() => plant.permissionRoles?.roles ?? []);
const tasks = computed(() => plant.ainasTasks);
const demo = computed(() => plant.demoContext);
const scenario = computed(() => config.value?.field_scenario ?? null);
const productionLine = computed(() => config.value?.production_line ?? null);
const encryption = computed(() => config.value?.data_security?.storage_encryption ?? null);
const integrations = computed(() => config.value?.integrations ?? null);
const safety = computed(() => config.value?.safety ?? null);
const localAi = computed(() => config.value?.local_ai ?? null);

// ---- 组件控制 ----
const componentForm = reactive({ deviceId: "reactor_001", componentId: "", action: "", value: 0, reason: "" });

const selectedComponent = computed<DeviceComponentItem | null>(() => {
  for (const device of capabilityDevices.value) {
    const hit = (device.components ?? []).find((c) => (c.component_id ?? c.id) === componentForm.componentId);
    if (hit) return hit;
  }
  return null;
});

const availableActions = computed<ComponentAction[]>(() => selectedComponent.value?.actions ?? []);
const selectedAction = computed<ComponentAction | null>(
  () => availableActions.value.find((a) => a.action === componentForm.action) ?? null
);

const allComponents = computed(() => {
  const rows: { deviceId: string; componentId: string; label: string }[] = [];
  for (const device of capabilityDevices.value) {
    for (const component of device.components ?? []) {
      const id = component.component_id ?? component.id ?? "";
      rows.push({ deviceId: device.device_id, componentId: id, label: component.label ?? id });
    }
  }
  return rows;
});

async function controlComponent(): Promise<void> {
  submitting.value = true;
  try {
    const body: { action: string; value?: number; reason?: string } = { action: componentForm.action };
    if (selectedAction.value?.value_type === "number") body.value = componentForm.value;
    if (componentForm.reason.trim()) body.reason = componentForm.reason.trim();
    await deviceApi.controlComponent(componentForm.deviceId, componentForm.componentId, body);
    ElMessage.success(tr("组件控制已执行", "Component control executed"));
    await Promise.all([plant.loadDeviceStatus(), plant.loadDeviceCapabilities()]);
  } catch (error) {
    ElMessage.error(errorMessage(error));
  } finally {
    submitting.value = false;
  }
}

// ---- AINAS 任务 ----
const ainasForm = reactive({
  action: "set_targets",
  process_id: undefined as number | undefined,
  target_temperature_c: 60,
  target_stirrer_rpm: 300,
  reason: ""
});

async function createTask(): Promise<void> {
  submitting.value = true;
  try {
    const body: Record<string, unknown> = { action: ainasForm.action };
    if (ainasForm.action === "set_targets") {
      body.target_temperature_c = ainasForm.target_temperature_c;
      body.target_stirrer_rpm = ainasForm.target_stirrer_rpm;
    } else if (ainasForm.process_id !== undefined) {
      body.process_id = ainasForm.process_id;
    }
    if (ainasForm.reason.trim()) body.reason = ainasForm.reason.trim();
    await ainasApi.create(body as never);
    ElMessage.success(tr("任务已创建并执行", "Task created and executed"));
    await plant.loadAinasTasks();
  } catch (error) {
    ElMessage.error(errorMessage(error));
  } finally {
    submitting.value = false;
  }
}

function taskStatusType(status: string): "success" | "warning" | "info" | "danger" {
  if (status === "executed") return "success";
  if (status === "failed" || status === "rejected") return "danger";
  if (status === "executing") return "warning";
  return "info";
}

onMounted(async () => {
  loading.value = true;
  await Promise.allSettled([
    plant.loadConfig(),
    plant.loadDeviceStatus(),
    plant.loadDeviceCapabilities(),
    plant.loadPermissionRoles(),
    plant.loadDemoContext(),
    auth.isAuthenticated ? plant.loadAinasTasks() : Promise.resolve()
  ]);
  loading.value = false;
  if (allComponents.value.length > 0) {
    componentForm.componentId = allComponents.value[0].componentId;
    const first = availableActions.value[0];
    if (first) componentForm.action = first.action;
  }
});
</script>

<template>
  <div class="page-stack">
    <PageHeader :title="tr('系统配置', 'System Settings')" :subtitle="tr('设备、集成、权限与安全配置总览', 'Devices, integrations, permissions and safety configuration')" />

    <!-- 场景与产线 -->
    <div class="settings-grid cols-2">
      <section class="hmi-panel">
        <div class="hmi-panel-header">{{ tr("应用场景", "Field scenario") }}</div>
        <div class="hmi-panel-body">
          <dl class="kv-list">
            <dt>{{ tr("类型", "Kind") }}</dt>
            <dd>{{ text(scenario?.kind) }}</dd>
            <dt>{{ tr("标签", "Label") }}</dt>
            <dd>{{ text(scenario?.label) }}</dd>
            <dt>{{ tr("来源", "Source") }}</dt>
            <dd>{{ text(scenario?.source) }}</dd>
            <dt>{{ tr("站点", "Site") }}</dt>
            <dd>{{ text(scenario?.site_label) }}</dd>
            <dt>{{ tr("置信度", "Confidence") }}</dt>
            <dd>{{ text(scenario?.confidence) }}</dd>
          </dl>
        </div>
      </section>

      <section class="hmi-panel">
        <div class="hmi-panel-header">{{ tr("产线识别", "Production line") }}</div>
        <div class="hmi-panel-body">
          <dl class="kv-list">
            <dt>{{ tr("类型", "Kind") }}</dt>
            <dd>{{ text(productionLine?.kind) }}</dd>
            <dt>{{ tr("标签", "Label") }}</dt>
            <dd>{{ text(productionLine?.label) }}</dd>
            <dt>{{ tr("需问询", "Inquiry") }}</dt>
            <dd>{{ text(productionLine?.requires_operator_inquiry) }}</dd>
            <dt>{{ tr("专项处理", "Special handling") }}</dt>
            <dd>{{ text(productionLine?.special_handling_required) }}</dd>
          </dl>
        </div>
      </section>
    </div>

    <!-- 设备与组件控制 -->
    <section class="hmi-panel">
      <div class="hmi-panel-header">
        <span>{{ tr("设备与组件控制", "Devices & component control") }}</span>
        <span class="muted">{{ devices.length }} {{ tr("台设备", "devices") }}</span>
      </div>
      <div class="hmi-panel-body device-layout">
        <div>
          <el-table v-if="devices.length > 0" :data="devices" size="small">
            <el-table-column prop="device_id" :label="tr('设备', 'Device')" min-width="120">
              <template #default="{ row }"><span class="mono">{{ row.device_id }}</span></template>
            </el-table-column>
            <el-table-column :label="tr('状态', 'Status')" width="100">
              <template #default="{ row }">
                <el-tag size="small" :type="row.online ? 'success' : 'info'">{{ text(row.status) }}</el-tag>
              </template>
            </el-table-column>
            <el-table-column :label="tr('在线', 'Online')" width="80">
              <template #default="{ row }">{{ row.online ? tr("是", "yes") : tr("否", "no") }}</template>
            </el-table-column>
            <el-table-column :label="tr('活动批次', 'Batch')" width="90">
              <template #default="{ row }">{{ text(row.active_batch_id) }}</template>
            </el-table-column>
            <el-table-column :label="tr('命令回执', 'Last command')" min-width="140">
              <template #default="{ row }">
                <span v-if="row.last_command_error" class="mono" style="color: var(--ind-red)">{{ row.last_command_error }}</span>
                <span v-else class="mono">{{ text(row.last_command_request_id) }}</span>
              </template>
            </el-table-column>
          </el-table>
          <EmptyState v-else icon="⚙" :title="tr('无设备数据', 'No device data')" />
        </div>

        <div class="component-control">
          <strong>{{ tr("组件控制", "Component control") }}</strong>
          <el-select v-model="componentForm.componentId" :placeholder="tr('组件', 'Component')" class="full-width" @change="componentForm.action = availableActions[0]?.action ?? ''">
            <el-option v-for="c in allComponents" :key="c.componentId" :value="c.componentId" :label="`${c.label} (${c.deviceId})`" />
          </el-select>
          <el-select v-model="componentForm.action" :placeholder="tr('动作', 'Action')" class="full-width">
            <el-option v-for="a in availableActions" :key="a.action" :value="a.action" :label="a.label" />
          </el-select>
          <el-input-number
            v-if="selectedAction?.value_type === 'number'"
            v-model="componentForm.value"
            :min="selectedAction.min"
            :max="selectedAction.max"
            controls-position="right"
            class="full-width"
          />
          <el-input v-model="componentForm.reason" :placeholder="tr('审计原因（可选）', 'Audit reason (optional)')" maxlength="120" />
          <HmiButton
            type="manual"
            :disabled="!auth.isAuthenticated || !componentForm.componentId || !componentForm.action"
            @click="controlComponent"
          >
            {{ tr("执行", "Execute") }}
          </HmiButton>
        </div>
      </div>
    </section>

    <!-- AINAS 任务 -->
    <section class="hmi-panel">
      <div class="hmi-panel-header">
        <span>{{ tr("AINAS 集成任务", "AINAS integration tasks") }}</span>
        <span class="muted">{{ tasks.length }}</span>
      </div>
      <div class="hmi-panel-body ainas-layout">
        <div class="ainas-form">
          <strong>{{ tr("创建任务", "Create task") }}</strong>
          <el-select v-model="ainasForm.action" class="full-width">
            <el-option value="set_targets" :label="tr('写目标 set_targets', 'set_targets')" />
            <el-option value="start_process" :label="tr('启动工艺 start_process', 'start_process')" />
            <el-option value="stop_process" :label="tr('停止工艺 stop_process', 'stop_process')" />
          </el-select>
          <template v-if="ainasForm.action === 'set_targets'">
            <el-input-number v-model="ainasForm.target_temperature_c" :min="0" :max="220" controls-position="right" class="full-width" />
            <el-input-number v-model="ainasForm.target_stirrer_rpm" :min="0" :max="1800" controls-position="right" class="full-width" />
          </template>
          <el-input-number v-else v-model="ainasForm.process_id" :min="1" controls-position="right" class="full-width" :placeholder="tr('工艺 ID', 'Process ID')" />
          <el-input v-model="ainasForm.reason" :placeholder="tr('审计原因（可选）', 'Reason (optional)')" maxlength="120" />
          <HmiButton type="manual" :disabled="!auth.isEngineerOrAdmin" @click="createTask">
            {{ tr("创建并执行", "Create & execute") }}
          </HmiButton>
          <small v-if="!auth.isEngineerOrAdmin" class="muted">{{ tr("需 engineer/admin", "Requires engineer/admin") }}</small>
        </div>

        <div>
          <el-table v-if="tasks.length > 0" :data="tasks" size="small" max-height="360">
            <el-table-column prop="id" label="ID" width="60" />
            <el-table-column prop="action" :label="tr('动作', 'Action')" min-width="120">
              <template #default="{ row }"><span class="mono">{{ row.action }}</span></template>
            </el-table-column>
            <el-table-column :label="tr('状态', 'Status')" width="110">
              <template #default="{ row }">
                <el-tag size="small" :type="taskStatusType(String(row.status ?? ''))">{{ row.status }}</el-tag>
              </template>
            </el-table-column>
            <el-table-column prop="source" :label="tr('来源', 'Source')" width="90" />
            <el-table-column :label="tr('时间', 'Time')" min-width="150">
              <template #default="{ row }">{{ formatTimestamp(row.created_at) }}</template>
            </el-table-column>
          </el-table>
          <EmptyState v-else icon="◌" :title="tr('暂无任务', 'No tasks')" :description="auth.isAuthenticated ? '' : tr('登录后查看', 'Sign in to view')" />
        </div>
      </div>
    </section>

    <!-- 权限矩阵 -->
    <section class="hmi-panel">
      <div class="hmi-panel-header">{{ tr("角色权限矩阵", "Role permissions") }}</div>
      <div class="hmi-panel-body flush">
        <el-table v-if="roles.length > 0" :data="roles" size="small">
          <el-table-column prop="role" :label="tr('角色', 'Role')" width="110">
            <template #default="{ row }"><span class="mono">{{ row.role }}</span></template>
          </el-table-column>
          <el-table-column prop="label" :label="tr('名称', 'Label')" width="140" />
          <el-table-column :label="tr('允许', 'Can')" min-width="320">
            <template #default="{ row }">
              <div class="perm-tags">
                <el-tag v-for="perm in row.can" :key="perm" size="small" type="success" effect="plain">{{ perm }}</el-tag>
              </div>
            </template>
          </el-table-column>
          <el-table-column :label="tr('禁止', 'Blocked')" min-width="200">
            <template #default="{ row }">
              <div class="perm-tags">
                <el-tag v-for="perm in row.blocked" :key="perm" size="small" type="danger" effect="plain">{{ perm }}</el-tag>
              </div>
            </template>
          </el-table-column>
        </el-table>
      </div>
    </section>

    <!-- 配置摘要 -->
    <div class="settings-grid cols-3">
      <section class="hmi-panel">
        <div class="hmi-panel-header">{{ tr("集成状态", "Integrations") }}</div>
        <div class="hmi-panel-body">
          <dl class="kv-list">
            <dt>MQTT</dt>
            <dd>{{ text((integrations?.mqtt as Record<string, unknown> | undefined)?.enabled) }}</dd>
            <dt>Modbus TCP</dt>
            <dd>{{ text((integrations?.modbus_tcp as Record<string, unknown> | undefined)?.enabled) }}</dd>
            <dt>AINAS</dt>
            <dd>{{ text(integrations?.ainas_ready) }}</dd>
            <dt>REST API</dt>
            <dd>{{ text(integrations?.rest_api) }}</dd>
          </dl>
        </div>
      </section>

      <section class="hmi-panel">
        <div class="hmi-panel-header">{{ tr("本地 AI", "Local AI") }}</div>
        <div class="hmi-panel-body">
          <dl class="kv-list">
            <dt>{{ tr("模式", "Mode") }}</dt>
            <dd>{{ text(localAi?.mode) }}</dd>
            <dt>{{ tr("缺失资产", "Missing") }}</dt>
            <dd>{{ (localAi?.missing ?? []).length }}</dd>
          </dl>
        </div>
      </section>

      <section class="hmi-panel">
        <div class="hmi-panel-header">{{ tr("存储加密", "Storage encryption") }}</div>
        <div class="hmi-panel-body">
          <dl class="kv-list">
            <dt>{{ tr("启用", "Enabled") }}</dt>
            <dd>{{ text(encryption?.enabled) }}</dd>
            <dt>{{ tr("算法", "Algorithm") }}</dt>
            <dd>{{ text(encryption?.algorithm) }}</dd>
            <dt>{{ tr("密钥来源", "Key source") }}</dt>
            <dd>{{ text(encryption?.key_source) }}</dd>
          </dl>
        </div>
      </section>
    </div>

    <!-- 安全限幅 -->
    <section class="hmi-panel">
      <div class="hmi-panel-header">{{ tr("安全限幅", "Safety limits") }}</div>
      <div class="hmi-panel-body">
        <dl class="kv-list">
          <dt>{{ tr("温度范围", "Temperature") }}</dt>
          <dd>{{ text(safety?.temperature?.min) }} – {{ text(safety?.temperature?.max) }} °C（step {{ text(safety?.temperature?.max_step) }}）</dd>
          <dt>{{ tr("搅拌范围", "Stirrer") }}</dt>
          <dd>{{ text(safety?.stirrer?.min) }} – {{ text(safety?.stirrer?.max) }} rpm（step {{ text(safety?.stirrer?.max_step) }}）</dd>
          <dt>{{ tr("控制间隔", "Control interval") }}</dt>
          <dd>{{ text((safety?.control as Record<string, unknown> | undefined)?.control_interval_ms) }} ms</dd>
          <dt>{{ tr("传感器超时", "Sensor timeout") }}</dt>
          <dd>{{ text((safety?.control as Record<string, unknown> | undefined)?.sensor_timeout_ms) }} ms</dd>
        </dl>
      </div>
    </section>

    <!-- 演示上下文 -->
    <section class="hmi-panel">
      <div class="hmi-panel-header">
        <span>{{ tr("演示上下文", "Demo context") }}</span>
        <el-tag size="small" :type="demo?.demo ? 'warning' : 'info'">{{ demo?.demo ? "demo" : "off" }}</el-tag>
      </div>
      <div class="hmi-panel-body">
        <dl class="kv-list">
          <dt>{{ tr("数据策略", "Policy") }}</dt>
          <dd>{{ text(demo?.sensor_data_policy) }}</dd>
          <dt>{{ tr("工艺数", "Processes") }}</dt>
          <dd>{{ demo?.processes?.length ?? 0 }}</dd>
          <dt>{{ tr("批次数", "Batches") }}</dt>
          <dd>{{ demo?.recent_batches?.length ?? 0 }}</dd>
          <dt>{{ tr("演示告警", "Demo alarms") }}</dt>
          <dd>{{ demo?.demo_alarms?.length ?? 0 }}</dd>
        </dl>
      </div>
    </section>
  </div>
</template>

<style scoped>
.settings-grid {
  display: grid;
  gap: var(--spacing);
  align-items: start;
}

.settings-grid.cols-2 {
  grid-template-columns: repeat(2, minmax(0, 1fr));
}

.settings-grid.cols-3 {
  grid-template-columns: repeat(3, minmax(0, 1fr));
}

.device-layout {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 280px;
  gap: var(--spacing);
}

.component-control,
.ainas-form {
  display: flex;
  flex-direction: column;
  gap: var(--spacing);
}

.ainas-layout {
  display: grid;
  grid-template-columns: 280px minmax(0, 1fr);
  gap: var(--spacing);
}

.full-width {
  width: 100%;
}

.perm-tags {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

@media (max-width: 1100px) {
  .settings-grid.cols-2,
  .settings-grid.cols-3,
  .device-layout,
  .ainas-layout {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
