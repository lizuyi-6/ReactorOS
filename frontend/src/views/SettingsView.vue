<script setup lang="ts">
// System Settings / 系统设置 — 参考稿 6。
// 布局：页头（标题 + 未保存提示/Reset/Save&Apply）+ 卡片网格（cols-5 → cols-4 → cols-3×2）。
// 真实可写项只有语言偏好（自动持久化）；其余控制均为只读展示或"暂不支持"回弹。

import { computed, onMounted, onUnmounted, ref } from "vue";
import { ElMessage, ElMessageBox } from "element-plus";
import PanelCard from "../components/PanelCard.vue";
import SparkLine from "../components/SparkLine.vue";
import AppIcon from "../components/AppIcon.vue";
import { auditApi } from "../api";
import { downloadBlob } from "../api/http";
import { errorMessage } from "../api/errors";
import { usePlantStore } from "../stores/plant";
import { useLiveStore } from "../stores/live";
import { useLanguage } from "../i18n";
import { fixed } from "../utils/format";
import type { PermissionRoleItem } from "../api/types";

const plant = usePlantStore();
const live = useLiveStore();
const { tr, language, setLanguage } = useLanguage();

// ---------- 配置数据（只读取值） ----------
const config = computed(() => plant.config);
const roles = computed(() => plant.permissionRoles?.roles ?? []);

const deviceInfo = computed(() => {
  const device = (config.value?.device ?? null) as Record<string, unknown> | null;
  const modbus = (device?.modbus ?? null) as Record<string, unknown> | null;
  const unit = device?.unit_id ?? modbus?.slave_id ?? null;
  const name = device?.name ?? device?.device_name ?? null;
  const location = device?.location ?? device?.site ?? null;
  return {
    unit: unit === null || unit === undefined || unit === "" ? null : String(unit),
    name: name === null || name === undefined || name === "" ? null : String(name),
    location: location === null || location === undefined || location === "" ? null : String(location)
  };
});

const maxTempC = computed(() => config.value?.safety?.temperature?.max_c ?? null);
const maxStirrerRpm = computed(() => config.value?.safety?.stirrer?.max_rpm ?? null);

const aiMode = computed(() => {
  const mode = config.value?.ai_provider?.mode;
  return typeof mode === "string" && mode ? mode : "";
});
const aiModel = computed(() => {
  const model = config.value?.ai_provider?.model;
  return typeof model === "string" && model ? model : "";
});

const mqttConfigured = computed(() => {
  const mqtt = config.value?.integrations?.mqtt;
  return !!mqtt && typeof mqtt === "object" && Object.keys(mqtt).length > 0;
});

// ---------- 本地 UI 状态（仅语言真正可写） ----------
const savedLanguage = ref(language.value);
const hasChanges = computed(() => language.value !== savedLanguage.value);
const exporting = ref(false);

const timezone = ref("(UTC+08:00) Asia/Shanghai");
const timezoneOptions = [
  "(UTC+08:00) Asia/Shanghai",
  "(UTC+00:00) UTC",
  "(UTC+09:00) Asia/Tokyo",
  "(UTC+01:00) Europe/Berlin",
  "(UTC-05:00) America/New_York"
];
const dateFormat = ref("YYYY-MM-DD");
const timeFormat = ref("24-Hour");
const theme = ref("ReactorOS Dark (Blue)");
const apiKeyInput = ref("");

// ---------- 网络（Internet 用 navigator.onLine 真实状态） ----------
const online = ref(typeof navigator === "undefined" ? true : navigator.onLine);
function syncOnline(): void {
  online.value = typeof navigator === "undefined" ? true : navigator.onLine;
}

// ---------- 权限矩阵 ----------
interface PermColumn {
  key: string;
  en: string;
  zh: string;
  perms: string[];
}
const permColumns: PermColumn[] = [
  { key: "view", en: "View", zh: "查看", perms: ["view_monitor", "view_history", "view_audit"] },
  { key: "control", en: "Control", zh: "控制", perms: ["set_safe_targets", "start_stop_process", "emergency_stop"] },
  { key: "config", en: "Config", zh: "配置", perms: ["edit_process", "edit_system_config", "modbus_debug"] },
  { key: "admin", en: "Admin", zh: "管理", perms: ["manage_users", "delete_data"] }
];
const roleZhNames: Record<string, string> = { operator: "操作员", engineer: "工程师", admin: "管理员" };

function roleAllows(role: PermissionRoleItem, column: PermColumn): boolean {
  return column.perms.some((permission) => role.can?.includes(permission));
}

// ---------- 服务状态 ----------
const services = computed(() => {
  const up = !!config.value;
  return [
    { key: "core", zh: "核心服务", en: "Core Service", running: up },
    { key: "data", zh: "数据服务", en: "Data Service", running: up },
    { key: "ai", zh: "AI 服务", en: "AI Service", running: up && aiMode.value !== "" },
    { key: "alarm", zh: "告警服务", en: "Alarm Service", running: up },
    { key: "web", zh: "Web 服务", en: "Web Server", running: up },
    { key: "mqtt", zh: "MQTT 服务", en: "MQTT Service", running: mqttConfigured.value }
  ];
});

// ---------- 系统组件开关（只读状态展示，切换回弹） ----------
const componentSwitches = [
  { key: "acquisition", zh: "实时数据采集", en: "Live Data Acquisition", on: true },
  { key: "ai_engine", zh: "AI 决策引擎", en: "AI Decision Engine", on: true },
  { key: "batch", zh: "批次管理", en: "Batch Management", on: true },
  { key: "report", zh: "报表与分析", en: "Report & Analytics", on: true },
  { key: "remote", zh: "远程访问", en: "Remote Access", on: false }
];

// ---------- 边缘节点：静态装饰曲线（当前值无真实来源，显示 "--"） ----------
const sparkCpu = [22, 26, 24, 29, 27, 32, 30, 34, 31, 36, 33, 38];
const sparkMem = [41, 42, 41, 43, 42, 44, 43, 45, 44, 46, 45, 47];
const sparkDisk = [55, 55, 56, 56, 57, 57, 58, 58, 59, 59, 60, 60];

// ---------- 交互 ----------
function notSupported(): void {
  ElMessage.info(tr("该操作后端暂不支持", "Not supported by backend"));
}

function changeLanguage(value: string): void {
  setLanguage(value === "en" ? "en" : "zh");
}

function saveSettings(): void {
  // 目前只有语言偏好可持久化（setLanguage 时已写入 localStorage）。
  savedLanguage.value = language.value;
  ElMessage.success(tr("设置已保存并应用（语言偏好已持久化）", "Settings saved & applied (language preference persisted)"));
}

function resetSettings(): void {
  setLanguage("zh");
  savedLanguage.value = "zh";
  timezone.value = timezoneOptions[0];
  dateFormat.value = "YYYY-MM-DD";
  timeFormat.value = "24-Hour";
  apiKeyInput.value = "";
  ElMessage.info(tr("已重置为默认设置", "Settings reset to defaults"));
}

async function exportConfiguration(): Promise<void> {
  if (!config.value) {
    ElMessage.warning(tr("暂无配置数据", "No configuration data available"));
    return;
  }
  try {
    const blob = new Blob([JSON.stringify(config.value, null, 2)], { type: "application/json" });
    downloadBlob(blob, "reactoros-config.json");
    ElMessage.success(tr("配置已导出", "Configuration exported"));
  } catch (error) {
    ElMessage.error(errorMessage(error));
  }
}

async function exportAuditLogs(): Promise<void> {
  exporting.value = true;
  try {
    const blob = await auditApi.exportCsv();
    downloadBlob(blob, "reactoros-audit-logs.csv");
    ElMessage.success(tr("审计日志已导出", "Audit logs exported"));
  } catch (error) {
    ElMessage.error(errorMessage(error));
  } finally {
    exporting.value = false;
  }
}

function rebootSystem(): void {
  ElMessageBox.confirm(
    tr("确定要重启系统吗？重启期间控制与监控将中断。", "Reboot the system now? Control and monitoring will be interrupted."),
    tr("重启系统", "Reboot System"),
    {
      type: "warning",
      confirmButtonText: tr("重启", "Reboot"),
      cancelButtonText: tr("取消", "Cancel")
    }
  )
    .then(() => ElMessage.info(tr("该操作后端暂不支持", "Not supported by backend")))
    .catch(() => undefined);
}

// ---------- 生命周期 ----------
onMounted(async () => {
  window.addEventListener("online", syncOnline);
  window.addEventListener("offline", syncOnline);
  const results = await Promise.allSettled([plant.loadConfig(), plant.loadPermissionRoles()]);
  if (results.some((result) => result.status === "rejected")) {
    ElMessage.warning(tr("部分配置加载失败", "Some settings failed to load"));
  }
});

onUnmounted(() => {
  window.removeEventListener("online", syncOnline);
  window.removeEventListener("offline", syncOnline);
});
</script>

<template>
  <div class="page-stack">
    <!-- 页头 -->
    <header class="page-header">
      <div>
        <h1 class="page-title">System Settings <span class="zh">系统设置</span></h1>
        <p class="page-subtitle">
          {{ tr("配置系统参数、集成设置和平台偏好", "Configure system parameters, integrations, and platform preferences") }}
        </p>
      </div>
      <div class="header-actions">
        <span v-if="hasChanges" class="unsaved-badge">
          <span class="status-dot warn" />
          {{ tr("存在未保存的更改", "Unsaved changes") }}
        </span>
        <el-button size="small" @click="resetSettings">{{ tr("重置", "Reset") }}</el-button>
        <el-button size="small" type="primary" @click="saveSettings">{{ tr("保存并应用", "Save & Apply") }}</el-button>
      </div>
    </header>

    <!-- 卡片流（内部滚动，页面不滚） -->
    <div class="cards-scroll">
      <!-- 第 1 行：5 列 -->
      <div class="grid cols-5">
        <PanelCard en="Device Configuration" zh="设备配置">
          <div class="card-inner">
            <label class="field">
              <span class="k">{{ tr("设备编号", "Unit ID") }}</span>
              <el-input v-if="deviceInfo.unit !== null" :model-value="deviceInfo.unit" size="small" class="full-w mono" readonly />
              <span v-else class="v mono">--</span>
            </label>
            <label class="field">
              <span class="k">{{ tr("设备名称", "Device Name") }}</span>
              <el-input v-if="deviceInfo.name !== null" :model-value="deviceInfo.name" size="small" class="full-w mono" readonly />
              <span v-else class="v mono">--</span>
            </label>
            <label class="field">
              <span class="k">{{ tr("位置", "Location") }}</span>
              <el-input v-if="deviceInfo.location !== null" :model-value="deviceInfo.location" size="small" class="full-w mono" readonly />
              <span v-else class="v mono">--</span>
            </label>
            <label class="field">
              <span class="k">{{ tr("时区", "Time Zone") }}</span>
              <el-select v-model="timezone" size="small" class="full-w" disabled>
                <el-option v-for="tz in timezoneOptions" :key="tz" :value="tz" :label="tz" />
              </el-select>
            </label>
            <button type="button" class="link-row" @click="notSupported()">
              <span>{{ tr("更多设置", "More Settings") }}</span><span class="arrow">›</span>
            </button>
          </div>
        </PanelCard>

        <PanelCard en="API & Integration" zh="API 与集成">
          <div class="card-inner">
            <div class="kv-row">
              <span class="k">REST API</span>
              <span class="v st"><span class="status-dot ok" />{{ tr("已启用", "Enabled") }}</span>
            </div>
            <div class="kv-row">
              <span class="k">{{ tr("API 版本", "API Version") }}</span>
              <span class="v mono">--</span>
            </div>
            <div class="kv-row">
              <span class="k">WebSocket</span>
              <span class="v st">
                <span class="status-dot" :class="live.realtimeConnected ? 'ok' : 'bad'" />
                {{ live.realtimeConnected ? tr("已连接", "Connected") : tr("未连接", "Disconnected") }}
              </span>
            </div>
            <div class="kv-row">
              <span class="k">MQTT Broker</span>
              <span class="v st">
                <template v-if="mqttConfigured"><span class="status-dot ok" />{{ tr("已连接", "Connected") }}</template>
                <span v-else class="mono">--</span>
              </span>
            </div>
            <div class="kv-row">
              <span class="k">{{ tr("最后同步", "Last Sync") }}</span>
              <span class="v mono">--</span>
            </div>
            <button type="button" class="link-row" @click="notSupported()">
              <span>{{ tr("管理集成", "Manage Integrations") }}</span><span class="arrow">›</span>
            </button>
          </div>
        </PanelCard>

        <PanelCard en="AI Provider Settings" zh="AI 提供商设置">
          <div class="card-inner">
            <label class="field">
              <span class="k">{{ tr("提供商", "Provider") }}</span>
              <el-select :model-value="aiMode" size="small" class="full-w" disabled>
                <el-option value="openai" label="OpenAI" />
                <el-option value="stepfun" label="StepFun" />
                <el-option value="local" label="Local" />
              </el-select>
            </label>
            <label class="field">
              <span class="k">{{ tr("模型", "Model") }}</span>
              <el-select :model-value="aiModel" size="small" class="full-w" disabled>
                <el-option v-if="aiModel" :value="aiModel" :label="aiModel" />
              </el-select>
            </label>
            <label class="field">
              <span class="k">API Key</span>
              <el-input
                v-model="apiKeyInput"
                type="password"
                show-password
                placeholder="sk-······"
                size="small"
                class="full-w"
                disabled
              />
            </label>
            <div class="kv-row">
              <span class="k">{{ tr("状态", "Status") }}</span>
              <span class="v st">
                <template v-if="aiMode"><span class="status-dot ok" />{{ tr("已连接", "Connected") }}</template>
                <span v-else class="mono">--</span>
              </span>
            </div>
            <button type="button" class="link-row" @click="notSupported()">
              <span>{{ tr("测试连接", "Test Connection") }}</span><span class="arrow">›</span>
            </button>
          </div>
        </PanelCard>

        <PanelCard en="Safety Thresholds" zh="安全阈值配置">
          <div class="card-inner">
            <div class="threshold-row">
              <span class="k">{{ tr("高温报警", "High Temp Alarm") }}</span>
              <span class="big-val amber">{{ fixed(maxTempC, 0, "") }}<small>°C</small></span>
            </div>
            <div class="threshold-row">
              <span class="k">{{ tr("高压报警", "High Pressure") }}</span>
              <span class="big-val">--<small>bar</small></span>
            </div>
            <div class="threshold-row">
              <span class="k">{{ tr("低 pH 报警", "Low pH Alarm") }}</span>
              <span class="big-val">--</span>
            </div>
            <div class="threshold-row">
              <span class="k">{{ tr("搅拌转速上限", "Max Stirrer RPM") }}</span>
              <span class="big-val blue">{{ fixed(maxStirrerRpm, 0, "") }}<small>rpm</small></span>
            </div>
            <button type="button" class="link-row" @click="notSupported()">
              <span>{{ tr("编辑阈值", "Edit Thresholds") }}</span><span class="arrow">›</span>
            </button>
          </div>
        </PanelCard>

        <PanelCard en="User Roles & Permissions" zh="用户角色与权限">
          <div class="card-inner">
            <table v-if="roles.length > 0" class="perm-table">
              <thead>
                <tr>
                  <th class="role-col">{{ tr("角色", "Role") }}</th>
                  <th v-for="col in permColumns" :key="col.key">
                    <span class="th-en">{{ col.en }}</span>
                    <span class="th-zh">{{ col.zh }}</span>
                  </th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="role in roles" :key="role.role">
                  <td class="role-col">
                    <span class="role-name">{{ role.label || role.role }}</span>
                    <span v-if="roleZhNames[role.role]" class="role-zh">{{ roleZhNames[role.role] }}</span>
                  </td>
                  <td v-for="col in permColumns" :key="col.key" class="mark">
                    <AppIcon v-if="roleAllows(role, col)" name="check" :size="13" class="mark-yes" />
                    <span v-else class="mark-no">—</span>
                  </td>
                </tr>
              </tbody>
            </table>
            <div v-else class="empty-hint">{{ tr("暂无权限数据", "No permission data") }}</div>
            <button type="button" class="link-row" @click="notSupported()">
              <span>{{ tr("用户管理", "Manage Users") }}</span><span class="arrow">›</span>
            </button>
          </div>
        </PanelCard>
      </div>

      <!-- 第 2 行：4 列 -->
      <div class="grid cols-4">
        <PanelCard en="Deployment Info" zh="部署信息">
          <div class="card-inner">
            <div class="kv-row">
              <span class="k">{{ tr("环境", "Environment") }}</span>
              <span class="v">{{ tr("生产", "Production") }}</span>
            </div>
            <div class="kv-row">
              <span class="k">{{ tr("部署模式", "Deployment Mode") }}</span>
              <span class="v">{{ tr("边缘", "Edge") }}</span>
            </div>
            <div class="kv-row">
              <span class="k">{{ tr("实例 ID", "Instance ID") }}</span>
              <span class="v mono">--</span>
            </div>
            <div class="kv-row">
              <span class="k">{{ tr("安装时间", "Installed On") }}</span>
              <span class="v mono">--</span>
            </div>
            <div class="kv-row">
              <span class="k">{{ tr("运行时长", "Uptime") }}</span>
              <span class="v mono">--</span>
            </div>
            <button type="button" class="link-row" @click="notSupported()">
              <span>{{ tr("系统信息", "System Information") }}</span><span class="arrow">›</span>
            </button>
          </div>
        </PanelCard>

        <PanelCard en="Localization & Display" zh="语言与显示">
          <div class="card-inner">
            <label class="field">
              <span class="k">{{ tr("语言", "Language") }}</span>
              <el-select :model-value="language" size="small" class="full-w" @change="changeLanguage">
                <el-option value="zh" :label="tr('中文', 'Chinese')" />
                <el-option value="en" :label="tr('英文', 'English')" />
              </el-select>
            </label>
            <label class="field">
              <span class="k">{{ tr("日期格式", "Date Format") }}</span>
              <el-select v-model="dateFormat" size="small" class="full-w" disabled>
                <el-option value="YYYY-MM-DD" label="YYYY-MM-DD" />
                <el-option value="DD/MM/YYYY" label="DD/MM/YYYY" />
                <el-option value="MM/DD/YYYY" label="MM/DD/YYYY" />
              </el-select>
            </label>
            <label class="field">
              <span class="k">{{ tr("时间格式", "Time Format") }}</span>
              <el-select v-model="timeFormat" size="small" class="full-w" disabled>
                <el-option value="24-Hour" label="24-Hour" />
                <el-option value="12-Hour" label="12-Hour" />
              </el-select>
            </label>
            <label class="field">
              <span class="k">{{ tr("主题", "Theme") }}</span>
              <el-select v-model="theme" size="small" class="full-w" disabled>
                <el-option value="ReactorOS Dark (Blue)" label="ReactorOS Dark (Blue)" />
              </el-select>
            </label>
            <button type="button" class="link-row" @click="notSupported()">
              <span>{{ tr("自定义显示", "Customize Display") }}</span><span class="arrow">›</span>
            </button>
          </div>
        </PanelCard>

        <PanelCard en="Database & Storage" zh="数据库与存储">
          <div class="card-inner">
            <div class="kv-row">
              <span class="k">{{ tr("数据库", "Database") }}</span>
              <span class="v st"><span class="status-dot ok" />SQLite</span>
            </div>
            <div class="kv-row column">
              <span class="k">{{ tr("存储使用", "Storage Usage") }}</span>
              <div class="usage">
                <div class="usage-bar"><div class="usage-fill" style="width: 0%"></div></div>
                <span class="v mono">--</span>
              </div>
            </div>
            <div class="kv-row">
              <span class="k">{{ tr("数据保留", "Data Retention") }}</span>
              <span class="v mono">90 {{ tr("天", "days") }}</span>
            </div>
            <button type="button" class="link-row" @click="notSupported()">
              <span>{{ tr("存储设置", "Storage Settings") }}</span><span class="arrow">›</span>
            </button>
          </div>
        </PanelCard>

        <PanelCard en="Backup & Export" zh="备份与导出">
          <div class="card-inner">
            <div class="kv-row">
              <span class="k">{{ tr("上次备份", "Last Backup") }}</span>
              <span class="v mono">--</span>
            </div>
            <div class="kv-row">
              <span class="k">{{ tr("备份计划", "Backup Schedule") }}</span>
              <span class="v mono">--</span>
            </div>
            <div class="btn-stack">
              <el-button size="small" class="full-w" @click="notSupported()">{{ tr("立即备份", "Backup Now") }}</el-button>
              <el-button size="small" class="full-w" :disabled="!config" @click="exportConfiguration">
                {{ tr("导出配置", "Export Configuration") }}
              </el-button>
              <el-button size="small" class="full-w" :loading="exporting" @click="exportAuditLogs">
                {{ tr("导出审计日志", "Export Audit Logs") }}
              </el-button>
            </div>
          </div>
        </PanelCard>
      </div>

      <!-- 第 3 行：3 列 -->
      <div class="grid cols-3">
        <PanelCard en="Service Status" zh="服务状态">
          <div class="card-inner">
            <div v-for="svc in services" :key="svc.key" class="svc-row">
              <span class="status-dot" :class="svc.running ? 'ok' : ''" />
              <span class="k">{{ tr(svc.zh, svc.en) }}</span>
              <span class="v mono" :class="svc.running ? 'ok-text' : 'off-text'">
                {{ svc.running ? tr("运行中", "Running") : tr("已停止", "Stopped") }}
              </span>
            </div>
            <button type="button" class="link-row" @click="notSupported()">
              <span>{{ tr("查看日志", "View Logs") }}</span><span class="arrow">›</span>
            </button>
          </div>
        </PanelCard>

        <PanelCard en="System Components" zh="系统组件">
          <div class="card-inner">
            <div v-for="comp in componentSwitches" :key="comp.key" class="switch-row">
              <span class="k">{{ tr(comp.zh, comp.en) }}</span>
              <el-switch :model-value="comp.on" size="small" @change="notSupported()" />
            </div>
          </div>
        </PanelCard>

        <PanelCard en="Network & Connectivity" zh="网络与连接">
          <div class="card-inner">
            <div class="kv-row">
              <span class="k">{{ tr("网络模式", "Network Mode") }}</span>
              <span class="v">{{ tr("静态 IP", "Static") }}</span>
            </div>
            <div class="kv-row">
              <span class="k">IP {{ tr("地址", "Address") }}</span>
              <span class="v mono">--</span>
            </div>
            <div class="kv-row">
              <span class="k">{{ tr("子网掩码", "Subnet Mask") }}</span>
              <span class="v mono">--</span>
            </div>
            <div class="kv-row">
              <span class="k">{{ tr("网关", "Gateway") }}</span>
              <span class="v mono">--</span>
            </div>
            <div class="kv-row">
              <span class="k">DNS</span>
              <span class="v mono">--</span>
            </div>
            <div class="kv-row">
              <span class="k">{{ tr("互联网", "Internet") }}</span>
              <span class="v st">
                <span class="status-dot" :class="online ? 'ok' : 'bad'" />
                {{ online ? tr("已连接", "Connected") : tr("已离线", "Offline") }}
              </span>
            </div>
            <button type="button" class="link-row" @click="notSupported()">
              <span>{{ tr("网络设置", "Network Settings") }}</span><span class="arrow">›</span>
            </button>
          </div>
        </PanelCard>
      </div>

      <!-- 第 4 行：边缘节点（宽卡）+ 高级设置 -->
      <div class="grid cols-3">
        <PanelCard en="Edge Node Information" zh="边缘节点信息" class="span-2">
          <div class="edge-layout">
            <div class="edge-left">
              <svg viewBox="0 0 210 110" class="device-fig" role="img" aria-label="Edge node">
                <rect x="12" y="10" width="186" height="90" rx="10" fill="#0c1a2c" stroke="rgba(47,155,255,0.35)" stroke-width="1.2" />
                <rect x="24" y="22" width="162" height="26" rx="5" fill="rgba(47,155,255,0.06)" stroke="rgba(47,155,255,0.18)" stroke-width="0.8" />
                <circle cx="40" cy="35" r="4" fill="#2fd47b" />
                <circle cx="58" cy="35" r="4" fill="#f5a623" />
                <circle cx="76" cy="35" r="4" fill="#2f9bff" />
                <line x1="96" y1="28" x2="176" y2="28" stroke="rgba(157,180,207,0.25)" stroke-width="1.5" />
                <line x1="96" y1="35" x2="160" y2="35" stroke="rgba(157,180,207,0.25)" stroke-width="1.5" />
                <line x1="96" y1="42" x2="168" y2="42" stroke="rgba(157,180,207,0.25)" stroke-width="1.5" />
                <rect x="24" y="60" width="14" height="26" rx="2" fill="rgba(47,155,255,0.14)" />
                <rect x="44" y="60" width="14" height="26" rx="2" fill="rgba(47,155,255,0.10)" />
                <rect x="64" y="60" width="14" height="26" rx="2" fill="rgba(47,155,255,0.08)" />
                <text x="96" y="78" class="fig-label">RX-EDGE-01</text>
              </svg>
              <div class="kv-row">
                <span class="k">Node ID</span>
                <span class="v mono">RX-EDGE-01</span>
              </div>
              <div class="kv-row">
                <span class="k">{{ tr("硬件型号", "Hardware") }}</span>
                <span class="v mono">--</span>
              </div>
              <div class="kv-row">
                <span class="k">{{ tr("操作系统版本", "OS Version") }}</span>
                <span class="v mono">ReactorOS 2.4.1</span>
              </div>
            </div>
            <div class="edge-right">
              <div class="usage-row">
                <span class="k w-fixed">CPU</span>
                <span class="v mono">--</span>
                <SparkLine :points="sparkCpu" color="#2f9bff" :height="26" />
              </div>
              <div class="usage-row">
                <span class="k w-fixed">{{ tr("内存", "Memory") }}</span>
                <span class="v mono">--</span>
                <SparkLine :points="sparkMem" color="#b068f0" :height="26" />
              </div>
              <div class="usage-row">
                <span class="k w-fixed">{{ tr("磁盘", "Disk") }}</span>
                <span class="v mono">--</span>
                <SparkLine :points="sparkDisk" color="#f5a623" :height="26" />
              </div>
            </div>
          </div>
        </PanelCard>

        <PanelCard en="Advanced Settings" zh="高级设置">
          <div class="card-inner">
            <div class="switch-row">
              <span class="k">{{ tr("调试模式", "Debug Mode") }}</span>
              <el-switch :model-value="false" size="small" disabled />
            </div>
            <div class="kv-row">
              <span class="k">{{ tr("功能开关", "Feature Flags") }}</span>
              <button type="button" class="inline-link" @click="notSupported()">{{ tr("配置", "Configure") }} ›</button>
            </div>
            <div class="switch-row">
              <span class="k">{{ tr("维护模式", "Maintenance Mode") }}</span>
              <el-switch :model-value="false" size="small" disabled />
            </div>
            <div class="danger-zone">
              <el-button type="danger" plain class="full-w" @click="rebootSystem">{{ tr("重启系统", "Reboot System") }}</el-button>
            </div>
          </div>
        </PanelCard>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* ---------- 页头 ---------- */
.header-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
  justify-content: flex-end;
}

.unsaved-badge {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  font-size: var(--fs-sm);
  color: var(--ind-amber);
  background: rgba(245, 166, 35, 0.12);
  border: 1px solid rgba(245, 166, 35, 0.4);
  border-radius: 999px;
  padding: 4px 12px;
  white-space: nowrap;
}

/* ---------- 卡片流（页面不滚，这里滚） ---------- */
.cards-scroll {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: var(--spacing);
  padding-bottom: 4px;
}

/* 卡片按内容自然撑高：整块式布局，避免 flex/min-height:0 压缩链把内容高度塌成 0。
   关键：.grid 是滚动容器的 flex 子项，base.css 的 min-height:0 + 默认 shrink 会把它压扁，
   必须 flex:none 让网格按内容撑开、由容器滚动。 */
.cards-scroll > .grid {
  flex: none;
  min-height: auto;
}
.cards-scroll .panel {
  display: block;
  overflow: visible;
}
.cards-scroll :deep(.panel-body) {
  display: block;
  flex: none;
  overflow: visible;
}
.cards-scroll .card-inner {
  flex: none;
  min-height: 0;
}

.span-2 {
  grid-column: span 2;
}

/* ---------- 卡片内部 ---------- */
.card-inner {
  display: flex;
  flex-direction: column;
  gap: 3px;
  flex: 1;
  min-height: 0;
}

.kv-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 4px 0;
  min-height: 26px;
}

.kv-row.column {
  flex-direction: column;
  align-items: stretch;
  gap: 6px;
  padding: 6px 0;
}

.k {
  color: var(--text-tertiary);
  font-size: var(--fs-sm);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.v {
  font-family: var(--font-data);
  font-variant-numeric: tabular-nums;
  font-size: var(--fs-sm);
  color: var(--text-primary);
  text-align: right;
}

.st {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 3px 0;
}

.full-w {
  width: 100%;
}

.link-row {
  margin-top: auto;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  width: 100%;
  margin-left: 0;
  margin-right: 0;
  padding: 8px 2px 2px;
  border: none;
  border-top: 1px solid var(--border-glass);
  background: transparent;
  color: var(--accent);
  font-size: var(--fs-sm);
  font-weight: 600;
  font-family: var(--font-ui);
  cursor: pointer;
  text-align: left;
}

.link-row:hover {
  color: var(--accent-strong);
}

.link-row .arrow {
  color: var(--text-tertiary);
  font-size: var(--fs-md);
}

.inline-link {
  border: none;
  background: transparent;
  color: var(--accent);
  font-size: var(--fs-sm);
  font-weight: 600;
  font-family: var(--font-ui);
  cursor: pointer;
  padding: 0;
}

.inline-link:hover {
  color: var(--accent-strong);
}

/* ---------- 阈值大字 ---------- */
.threshold-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 5px 0;
}

.threshold-row .k {
  white-space: normal;
  line-height: 1.25;
}

.big-val {
  font-family: var(--font-data);
  font-variant-numeric: tabular-nums;
  font-size: var(--fs-lg);
  font-weight: 700;
  color: var(--text-primary);
  text-align: right;
}

.big-val small {
  font-size: var(--fs-xs);
  font-weight: 500;
  color: var(--text-tertiary);
  margin: 0 3px 0 6px;
}

.big-val.amber {
  color: var(--ind-amber);
}

.big-val.blue {
  color: var(--accent-strong);
}

/* ---------- 权限矩阵 ---------- */
.perm-table {
  width: 100%;
  border-collapse: collapse;
  table-layout: fixed;
}

.perm-table th,
.perm-table td {
  padding: 5px 2px;
  text-align: center;
  border-bottom: 1px solid var(--border-glass);
}

.perm-table th {
  vertical-align: bottom;
}

.perm-table .th-en {
  display: block;
  color: var(--text-secondary);
  font-size: var(--fs-xs);
  font-weight: 600;
}

.perm-table .th-zh {
  display: block;
  color: var(--text-tertiary);
  font-size: var(--fs-xs);
  font-weight: 400;
}

.perm-table .role-col {
  width: 34%;
  text-align: left;
  overflow: hidden;
}

.perm-table .role-name {
  display: block;
  color: var(--text-primary);
  font-size: var(--fs-xs);
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.perm-table .role-zh {
  display: block;
  color: var(--text-tertiary);
  font-size: var(--fs-xs);
}

.perm-table .mark {
  vertical-align: middle;
}

.mark-yes {
  color: var(--ind-green);
  vertical-align: middle;
}

.mark-no {
  color: var(--ind-gray);
  font-size: var(--fs-md);
}

.empty-hint {
  color: var(--text-tertiary);
  font-size: var(--fs-sm);
  padding: 10px 0;
  text-align: center;
}

/* ---------- 服务状态 / 开关行 ---------- */
.svc-row,
.switch-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 0;
  min-height: 26px;
}

.svc-row .k,
.switch-row .k {
  flex: 1;
  min-width: 0;
  text-align: left;
}

.svc-row .v {
  text-align: right;
}

.ok-text {
  color: var(--ind-green);
}

.off-text {
  color: var(--text-tertiary);
}

/* ---------- 存储使用条 ---------- */
.usage {
  display: flex;
  align-items: center;
  gap: 10px;
}

.usage-bar {
  flex: 1;
  height: 6px;
  border-radius: 3px;
  background: #22364f;
  overflow: hidden;
}

.usage-fill {
  height: 100%;
  border-radius: 3px;
  background: var(--accent);
}

/* ---------- 按钮堆叠 ---------- */
.btn-stack {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: 8px;
}

.danger-zone {
  margin-top: auto;
  padding-top: 10px;
}

/* ---------- 边缘节点 ---------- */
.edge-layout {
  display: grid;
  grid-template-columns: minmax(0, 5fr) minmax(0, 4fr);
  gap: var(--spacing);
  flex: 1;
  min-height: 0;
  align-items: start;
}

.edge-left {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.device-fig {
  width: 100%;
  max-height: 130px;
  margin-bottom: 6px;
}

.fig-label {
  font-family: var(--font-data);
  font-size: 11px;
  fill: var(--text-secondary);
}

.edge-right {
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 12px;
  min-width: 0;
}

.usage-row {
  display: flex;
  align-items: center;
  gap: 10px;
}

.usage-row .w-fixed {
  width: 48px;
  flex: none;
  text-align: left;
}

.usage-row .v {
  width: 30px;
  flex: none;
  text-align: right;
}

.usage-row :deep(.sparkline) {
  flex: 1;
  min-width: 0;
}

/* ---------- 响应式折叠 ---------- */
@media (max-width: 1650px) {
  .grid.cols-5 {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }
}

@media (max-width: 1400px) {
  .grid.cols-4 {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .grid.cols-3 {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .span-2 {
    grid-column: auto;
  }

  .edge-layout {
    grid-template-columns: minmax(0, 1fr);
  }
}

@media (max-width: 960px) {
  .grid.cols-5,
  .grid.cols-4,
  .grid.cols-3 {
    grid-template-columns: minmax(0, 1fr);
  }
}
</style>
