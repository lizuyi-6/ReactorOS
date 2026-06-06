<script setup lang="ts">
import { computed } from "vue";
import { usePlantStore } from "../stores/plant";
import { arrayAt, objectAt, textAt } from "./view-utils";

const store = usePlantStore();
const safety = computed(() => objectAt(store.config, "safety"));
const temperature = computed(() => objectAt(safety.value, "temperature"));
const stirrer = computed(() => objectAt(safety.value, "stirrer"));
const control = computed(() => objectAt(safety.value, "control"));
const optimizer = computed(() => objectAt(safety.value, "optimizer"));
const forbidden = computed(() => arrayAt(safety.value, "forbidden_control_zones"));
const device = computed(() => objectAt(store.config, "device"));
const integrations = computed(() => objectAt(store.config, "integrations"));
const security = computed(() => objectAt(store.config, "data_security"));
const aiMemory = computed(() => objectAt(store.config, "ai_memory"));
const aiProvider = computed(() => objectAt(store.config, "ai_provider"));
const localAi = computed(() => objectAt(store.config, "local_ai"));
const permissions = computed(() => objectAt(store.config, "permissions"));
const roles = computed(() => arrayAt(permissions.value, "roles"));

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

function integrationTag(name: string, on: boolean): "success" | "info" {
  return on ? "success" : "info";
}

function boolFrom(value: unknown): boolean {
  if (value === true) return true;
  if (typeof value === "string") return value === "true";
  return false;
}

const mqttOn = computed(() => boolFrom(integrations.value?.mqtt));
const modbusRtuOn = computed(() => boolFrom(integrations.value?.modbus_rtu));
const modbusTcpOn = computed(() => boolFrom(integrations.value?.modbus_tcp));
const ainasOn = computed(() => boolFrom(integrations.value?.ainas_ready));
const restOn = computed(() => boolFrom(integrations.value?.rest_api));
const cliOn = computed(() => boolFrom(integrations.value?.cli));
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

    <section class="panel two-col">
      <el-descriptions :column="1" border>
        <el-descriptions-item :label="store.tr('设备模式', 'Device mode')">{{ textAt(store.config, "device_mode") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('设备驱动', 'Device driver')">{{ textAt(device, "mode") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('推荐 provider', 'Provider model')">{{ textAt(aiProvider, "model") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('Provider 模式', 'Provider mode')">{{ textAt(aiProvider, "mode") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('LoRA 推理就绪', 'LoRA inference ready')">{{ textAt(localAi, "ready_for_inference") }}</el-descriptions-item>
        <el-descriptions-item :label="store.tr('LoRA 训练就绪', 'LoRA training ready')">{{ textAt(localAi, "ready_for_training") }}</el-descriptions-item>
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
          <span>{{ textAt(objectAt(security.value, "storage_encryption"), "algorithm") }}</span>
        </p>
        <p>
          <strong>{{ store.tr("启用", "Enabled") }}:</strong>
          <span>{{ textAt(objectAt(security.value, "storage_encryption"), "enabled") }}</span>
        </p>
        <p>
          <strong>{{ store.tr("密钥来源", "Key source") }}:</strong>
          <span>{{ textAt(objectAt(security.value, "storage_encryption"), "key_source") }}</span>
        </p>
        <p>
          <strong>{{ store.tr("加密字段", "Encrypted fields") }}:</strong>
          <span>{{ (objectAt(security.value, "storage_encryption")?.encrypted_fields ?? []).join(", ") }}</span>
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
          <small>{{ textAt(integrations.value, "ainas_task_api") }}</small>
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

    <section class="panel two-col">
      <div class="analysis-block">
        <h2>{{ store.tr("权限矩阵", "Permission Matrix") }}</h2>
        <p class="muted">{{ textAt(permissions.value, "note") }}</p>
        <p>
          <strong>{{ store.tr("认证", "Authentication") }}:</strong>
          <span>{{ textAt(permissions.value, "authentication") }}</span>
        </p>
        <p>
          <strong>{{ store.tr("模式", "Mode") }}:</strong>
          <span>{{ textAt(permissions.value, "mode") }}</span>
        </p>
        <p>
          <strong>{{ store.tr("默认用户", "Default users") }}:</strong>
          <span>{{ (permissions.value?.default_users ?? []).join(", ") }}</span>
        </p>
      </div>
      <el-table v-if="roles.length > 0" :data="roles" class="data-table" size="small">
        <el-table-column :label="store.tr('角色', 'Role')" min-width="120">
          <template #default="{ row }">{{ textAt(row, "role") }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('能力数', 'Capabilities')" width="120">
          <template #default="{ row }">{{ (row.permissions ?? []).length }}</template>
        </el-table-column>
        <el-table-column :label="store.tr('能力', 'Capabilities')" min-width="320">
          <template #default="{ row }">{{ (row.permissions ?? []).join(", ") }}</template>
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
