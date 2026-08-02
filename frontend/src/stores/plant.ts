// 业务数据 store：配置、工艺、批次、审计、Modbus、AINAS、设备。
// （旧的 950 行上帝 store 已拆分：认证→stores/auth.ts，实时→stores/live.ts，
//   http 与类型→api/*。本 store 只保留跨页面共享的业务数据与加载动作。）

import { ref } from "vue";
import { defineStore } from "pinia";
import {
  ainasApi,
  auditApi,
  authApi,
  batchApi,
  deviceApi,
  modbusApi,
  processApi,
  systemApi
} from "../api";
import type {
  AuditLogsResponse,
  BatchListResponse,
  ConfigSummary,
  DeviceCapabilitiesResponse,
  DeviceStatusSummary,
  IntegrationTask,
  ModbusRegistersResponse,
  PermissionRolesResponse,
  ProcessDefinition,
  ProcessDetail,
  DemoContext,
  AiRecommendationEnvelope
} from "../api/types";

export const usePlantStore = defineStore("plant", () => {
  const config = ref<ConfigSummary | null>(null);
  const processes = ref<ProcessDefinition[]>([]);
  const selectedProcess = ref<ProcessDetail | null>(null);
  const batches = ref<BatchListResponse | null>(null);
  const audit = ref<AuditLogsResponse | null>(null);
  const modbus = ref<ModbusRegistersResponse | null>(null);
  const deviceStatus = ref<DeviceStatusSummary | null>(null);
  const deviceCapabilities = ref<DeviceCapabilitiesResponse | null>(null);
  const permissionRoles = ref<PermissionRolesResponse | null>(null);
  const ainasTasks = ref<IntegrationTask[]>([]);
  const demoContext = ref<DemoContext | null>(null);
  const recommendation = ref<AiRecommendationEnvelope | null>(null);
  const loading = ref(false);
  const error = ref<string | null>(null);

  async function loadConfig(): Promise<void> {
    config.value = await systemApi.configSummary();
  }

  async function loadProcesses(): Promise<void> {
    const rows = await processApi.list();
    processes.value = Array.isArray(rows) ? rows : [];
  }

  async function loadProcessDetail(id: number): Promise<void> {
    selectedProcess.value = await processApi.detail(id);
  }

  async function loadBatches(): Promise<void> {
    const payload = await batchApi.list();
    if (payload && typeof payload === "object") batches.value = payload;
  }

  async function loadAudit(options: { page?: number; pageSize?: number; eventType?: string } = {}): Promise<AuditLogsResponse> {
    const payload = await auditApi.logs(options);
    audit.value = payload;
    return payload;
  }

  async function loadModbus(): Promise<void> {
    modbus.value = await modbusApi.registers();
  }

  async function loadDeviceStatus(): Promise<void> {
    deviceStatus.value = await deviceApi.status();
  }

  async function loadDeviceCapabilities(): Promise<void> {
    deviceCapabilities.value = await deviceApi.capabilities();
  }

  async function loadPermissionRoles(): Promise<void> {
    permissionRoles.value = await authApi.roles();
  }

  async function loadAinasTasks(limit = 20): Promise<void> {
    try {
      const rows = await ainasApi.list(limit);
      ainasTasks.value = Array.isArray(rows) ? rows : [];
    } catch {
      // AINAS 任务列表可能因后端存储加密未配置而 500 —— 静默降级为空列表，
      // 不让单端点故障影响整页渲染。
      ainasTasks.value = [];
    }
  }

  async function loadDemoContext(): Promise<void> {
    demoContext.value = await systemApi.demoContext();
  }

  return {
    config,
    processes,
    selectedProcess,
    batches,
    audit,
    modbus,
    deviceStatus,
    deviceCapabilities,
    permissionRoles,
    ainasTasks,
    demoContext,
    recommendation,
    loading,
    error,
    loadConfig,
    loadProcesses,
    loadProcessDetail,
    loadBatches,
    loadAudit,
    loadModbus,
    loadDeviceStatus,
    loadDeviceCapabilities,
    loadPermissionRoles,
    loadAinasTasks,
    loadDemoContext
  };
});
