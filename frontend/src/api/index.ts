// 按域组织的 API 模块。所有接口路径与后端 src/api.rs 对齐，device_id 固定 reactor_001。

import { request, requestBlob } from "./http";
import type {
  AiControlResponse,
  AuditLogsResponse,
  AuthUser,
  Batch,
  BatchDetail,
  BatchListResponse,
  ConfigSummary,
  DemoContext,
  DeviceCapabilitiesResponse,
  DeviceStatusSummary,
  ExperimentPlanResponse,
  HistoryResponse,
  IntegrationTask,
  LiveResponse,
  LoginResponse,
  ModbusRegistersResponse,
  PermissionRolesResponse,
  ProcessApplyResponse,
  ProcessDefinition,
  ProcessDetail,
  ProcessStopResponse,
  ProcessStep,
  RealtimePayload,
  AiRecommendationEnvelope,
  ControlTargets,
  HealthResponse
} from "./types";

export const DEVICE_ID = "reactor_001";
// Covers the default provider budget (3 x 20s plus backoff); no client retries.
export const AI_REQUEST_TIMEOUT_MS = 90_000;

// ---------- auth ----------
export const authApi = {
  login(username: string, password: string) {
    return request<LoginResponse>("/api/auth/login", { method: "POST", auth: false, body: { username, password } });
  },
  me() {
    return request<AuthUser>("/api/auth/me");
  },
  roles() {
    return request<PermissionRolesResponse>("/api/permissions/roles", { auth: false });
  },
  changePassword(oldPassword: string, newPassword: string) {
    return request<{ username: string; changed: boolean }>("/api/auth/change-password", {
      method: "POST",
      body: { old_password: oldPassword, new_password: newPassword }
    });
  }
};

// ---------- health / live / demo ----------
export const systemApi = {
  health() {
    return request<HealthResponse>("/health", { auth: false });
  },
  live(sampleLimit = 24) {
    return request<LiveResponse>(
      `/api/live?sample_limit=${sampleLimit}&include_processes=true&include_batches=true&include_events=false`,
      { auth: false }
    );
  },
  configSummary() {
    return request<ConfigSummary>("/api/config/summary", { auth: false });
  },
  demoContext() {
    return request<DemoContext>("/api/demo/context", { auth: false });
  }
};

// ---------- devices / components ----------
export const deviceApi = {
  status() {
    return request<DeviceStatusSummary>("/api/devices/status", { auth: false });
  },
  capabilities() {
    return request<DeviceCapabilitiesResponse>("/api/devices/capabilities", { auth: false });
  },
  controlComponent(deviceId: string, componentId: string, body: { action: string; value?: number; reason?: string }) {
    return request<{ device_id: string; component: unknown; outcome?: unknown }>(
      `/api/devices/${encodeURIComponent(deviceId)}/components/${encodeURIComponent(componentId)}/control`,
      { method: "POST", body }
    );
  }
};

// ---------- manual control ----------
export const controlApi = {
  updateTargets(body: { temperature_c: number; stirrer_rpm: number; shake_speed_cpm?: number }) {
    return request<ControlTargets>("/api/control/targets", { method: "POST", body });
  },
  setAuto(enabled: boolean) {
    return request<void>("/api/control/auto", { method: "POST", body: { enabled } });
  },
  setManualLock(locked: boolean) {
    return request<void>("/api/control/manual-lock", { method: "POST", body: { locked } });
  },
  resetFault() {
    return request<void>("/api/control/fault/reset", { method: "POST" });
  },
  emergencyStop() {
    return request<void>("/api/control/emergency-stop", { method: "POST" });
  },
  resetEmergencyStop() {
    return request<void>("/api/control/emergency-stop/reset", { method: "POST" });
  }
};

// ---------- processes ----------
export const processApi = {
  list() {
    return request<ProcessDefinition[]>("/api/processes", { auth: false, allowFailure: true });
  },
  detail(id: number) {
    return request<ProcessDetail>(`/api/processes/${id}`, { auth: false });
  },
  create(body: { name?: string; description?: string }) {
    return request<ProcessDefinition>("/api/processes", { method: "POST", body });
  },
  update(id: number, body: { name?: string; description?: string; status?: string }) {
    return request<ProcessDefinition>(`/api/processes/${id}`, { method: "PUT", body });
  },
  addStep(id: number, body: Partial<ProcessStep>) {
    return request<ProcessStep>(`/api/processes/${id}/steps`, { method: "POST", body });
  },
  updateStep(id: number, stepId: number, body: Partial<ProcessStep>) {
    return request<ProcessStep>(`/api/processes/${id}/steps/${stepId}`, { method: "PUT", body });
  },
  apply(id: number) {
    return request<ProcessApplyResponse>(`/api/processes/${id}/apply`, { method: "POST" });
  },
  start(id: number, name?: string) {
    const trimmed = name?.trim();
    return request<ProcessApplyResponse>(`/api/processes/${id}/start`, {
      method: "POST",
      body: trimmed ? { name: trimmed } : {}
    });
  },
  stop(id: number, reason?: string) {
    return request<ProcessStopResponse>(`/api/processes/${id}/stop`, { method: "POST", body: reason ? { reason } : {} });
  },
  stopCurrent(reason?: string) {
    return request<ProcessStopResponse>("/api/processes/current/stop", { method: "POST", body: reason ? { reason } : {} });
  }
};

// ---------- batches / product results ----------
export const batchApi = {
  list() {
    return request<BatchListResponse>("/api/batches", { auth: false, allowFailure: true });
  },
  detail(id: number) {
    return request<BatchDetail>(`/api/batches/${id}`, { auth: false });
  },
  start(body: {
    name?: string;
    process_id?: number;
    target_temperature_c?: number;
    target_stirrer_rpm?: number;
    target_shake_speed_cpm?: number;
    heating_minutes?: number;
    stirring_minutes?: number;
  }) {
    return request<Batch>("/api/batches/start", { method: "POST", body });
  },
  finish(id: number) {
    return request<void>(`/api/batches/${id}/finish`, { method: "POST" });
  },
  saveProductResult(body: { batch_id: number; yield_percent: number; product_ratio: number; notes?: string }) {
    return request<AiRecommendationEnvelope>("/api/product-results", { method: "POST", body, timeoutMs: AI_REQUEST_TIMEOUT_MS });
  },
  exportCsv() {
    return requestBlob("/api/batches/export.csv", { accept: "text/csv" });
  },
  exportXlsx() {
    return requestBlob("/api/batches/export.xlsx", {
      accept: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    });
  },
  exportReport(id: number) {
    return requestBlob(`/api/batches/${id}/report.md`, { accept: "text/markdown" });
  }
};

// ---------- audit ----------
export const auditApi = {
  logs(options: { page?: number; pageSize?: number; eventType?: string } = {}) {
    const params = new URLSearchParams();
    params.set("page", String(options.page ?? 1));
    params.set("page_size", String(options.pageSize ?? 50));
    if (options.eventType?.trim()) params.set("event_type", options.eventType.trim());
    return request<AuditLogsResponse>(`/api/audit/logs?${params.toString()}`);
  },
  exportCsv(eventType = "") {
    const params = new URLSearchParams();
    if (eventType.trim()) params.set("event_type", eventType.trim());
    const query = params.toString();
    return requestBlob(`/api/audit/export.csv${query ? `?${query}` : ""}`, { accept: "text/csv" });
  }
};

// ---------- AI ----------
export const aiApi = {
  latestRecommendation() {
    return request<AiRecommendationEnvelope | null>("/api/recommendations/latest", { auth: false, allowFailure: true });
  },
  regenerateRecommendation() {
    return request<AiRecommendationEnvelope>("/api/recommendations/latest", { method: "POST", timeoutMs: AI_REQUEST_TIMEOUT_MS });
  },
  control(body: {
    intent?: string;
    dry_run?: boolean;
    allow_process_start?: boolean;
    allow_process_stop?: boolean;
    allow_component_control?: boolean;
    allow_target_adjustment?: boolean;
  }) {
    return request<AiControlResponse>("/api/ai/control", { method: "POST", body, timeoutMs: AI_REQUEST_TIMEOUT_MS });
  },
  experimentPlan() {
    return request<ExperimentPlanResponse>("/api/ai/experiment-plan", { auth: false, timeoutMs: AI_REQUEST_TIMEOUT_MS });
  }
};

// ---------- modbus ----------
export const modbusApi = {
  registers() {
    return request<ModbusRegistersResponse>("/api/modbus/registers", { auth: false });
  },
  read(register: string) {
    return request<Record<string, unknown>>(`/api/modbus/registers/${encodeURIComponent(register)}/read`, { auth: false });
  },
  write(register: string, body: { value: number; reason: string }) {
    return request<Record<string, unknown>>(`/api/modbus/registers/${encodeURIComponent(register)}/write`, {
      method: "POST",
      body
    });
  }
};

// ---------- AINAS integration tasks ----------
export const ainasApi = {
  list(limit = 20) {
    return request<IntegrationTask[]>(`/api/integrations/ainas/tasks?limit=${limit}`);
  },
  detail(id: number) {
    return request<IntegrationTask>(`/api/integrations/ainas/tasks/${id}`);
  },
  create(body: {
    external_task_id?: string;
    action: string;
    process_id?: number;
    target_temperature_c?: number;
    target_stirrer_rpm?: number;
    target_shake_speed_cpm?: number;
    target_pressure_mpa?: number;
    heat_time_s?: number;
    hold_time_s?: number;
    cool_time_s?: number;
    reason?: string;
  }) {
    return request<IntegrationTask>("/api/integrations/ainas/tasks", { method: "POST", body });
  }
};

// ---------- v1 realtime / history ----------
export const realtimeApi = {
  realtime(deviceId: string = DEVICE_ID) {
    return request<RealtimePayload>(`/api/v1/reactor/${encodeURIComponent(deviceId)}/realtime`);
  },
  history(
    deviceId: string = DEVICE_ID,
    options: { startTime?: string; endTime?: string; page?: number; pageSize?: number } = {}
  ) {
    const endTime = options.endTime ?? new Date().toISOString();
    const startTime = options.startTime ?? new Date(Date.now() - 24 * 60 * 60 * 1000).toISOString();
    const params = new URLSearchParams();
    params.set("start_time", startTime);
    params.set("end_time", endTime);
    params.set("page", String(options.page ?? 1));
    params.set("page_size", String(options.pageSize ?? 100));
    return request<HistoryResponse>(`/api/v1/reactor/${encodeURIComponent(deviceId)}/history?${params.toString()}`, {
      auth: false,
      allowFailure: true
    });
  }
};

export function realtimeSocketUrl(deviceId: string = DEVICE_ID, token?: string | null): string {
  const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
  const base = `${proto}//${window.location.host}/ws/v1/reactor/${encodeURIComponent(deviceId)}/realtime`;
  return token ? `${base}?token=${encodeURIComponent(token)}` : base;
}
