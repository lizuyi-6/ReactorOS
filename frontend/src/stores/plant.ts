import { computed, ref } from "vue";
import { defineStore } from "pinia";

export type ApiRecord = Record<string, unknown>;
export type UiLanguage = "zh" | "en";

type HttpMethod = "GET" | "POST" | "PUT" | "DELETE";

export interface TargetUpdatePayload {
  temperature_c: number;
  stirrer_rpm: number;
  shake_speed_cpm?: number;
}

export interface ModbusWritePayload {
  value: number;
  reason: string;
}

export interface ProductResultPayload {
  batch_id: number;
  yield_percent: number;
  product_ratio: number;
  notes?: string;
}

export interface CreateProcessPayload {
  name: string;
  description: string;
}

export interface ProcessStepPayload {
  name: string;
  target_temperature_c: number;
  ramp_rate_c_min: number;
  duration_minutes: number;
  target_stirrer_rpm: number;
  target_shake_speed_cpm: number;
  target_pressure_mpa: number;
  cooling_mode: string;
}

export interface AiControlRequest {
  dry_run: boolean;
  allow_process_start?: boolean;
  allow_process_stop?: boolean;
  allow_component_control?: boolean;
  allow_target_adjustment?: boolean;
  intent?: string;
}

export interface ComponentControlPayload {
  action: string;
  value?: string | number | boolean;
  reason?: string;
}

export interface AinasTaskPayload {
  external_task_id?: string;
  action: string;
  process_id?: number | null;
  target_temperature_c?: number | null;
  target_stirrer_rpm?: number | null;
  target_shake_speed_cpm?: number | null;
  target_pressure_mpa?: number | null;
  heat_time_s?: number | null;
  hold_time_s?: number | null;
  cool_time_s?: number | null;
  reason?: string;
}

/** PATCH-style update for PUT /api/processes/:id — all fields optional, omitted = keep. */
export interface UpdateProcessPayload {
  name?: string;
  description?: string;
  status?: string;
}

/** PUT /api/processes/:id/steps/:step_id — full step (same shape as ProcessStepPayload). */
export type UpdateProcessStepPayload = ProcessStepPayload;

/** POST /api/batches/start — at least one of these must be set (backend rejects all-absent). */
export interface StartBatchPayload {
  name?: string;
  process_id?: number | null;
  target_temperature_c?: number | null;
  target_stirrer_rpm?: number | null;
  target_shake_speed_cpm?: number | null;
  heating_minutes?: number | null;
  stirring_minutes?: number | null;
}

/** POST /api/processes/:id/stop — reason is optional. */
export interface StopProcessPayload {
  reason?: string;
}

/** v1 realtime query window for GET /api/v1/reactor/:id/history. */
export interface HistoryQueryOptions {
  startTime?: string;
  endTime?: string;
  page?: number;
  pageSize?: number;
}

interface RequestOptions {
  method?: HttpMethod;
  body?: unknown;
  auth?: boolean;
  allowFailure?: boolean;
  accept?: string;
}

interface AuditQueryOptions {
  page?: number;
  pageSize?: number;
  eventType?: string;
}

interface LoginResponse {
  token: string;
  user: {
    username: string;
    role: string;
    permissions: string[];
  };
  expires_at: string;
}

const TOKEN_KEY = "reactoros.vue.auth.token";
const USER_KEY = "reactoros.vue.auth.user";
const LANGUAGE_KEY = "reactoros.vue.language";

const rolePasswords: Record<string, string> = {
  operator: "operator123",
  engineer: "engineer123",
  admin: "admin123"
};

function readStoredUser(): LoginResponse["user"] | null {
  const raw = localStorage.getItem(USER_KEY);
  if (!raw) return null;
  try {
    return JSON.parse(raw) as LoginResponse["user"];
  } catch {
    return null;
  }
}

function unwrapData<T>(payload: unknown): T {
  if (payload && typeof payload === "object" && "data" in payload) {
    return (payload as { data: T }).data;
  }
  return payload as T;
}

function errorMessage(payload: unknown, fallback: string): string {
  if (!payload || typeof payload !== "object") return fallback;
  const record = payload as ApiRecord;
  const error = record.error;
  if (typeof error === "string") return error;
  if (error && typeof error === "object") {
    const message = (error as ApiRecord).message;
    if (typeof message === "string") return message;
  }
  const message = record.message;
  return typeof message === "string" ? message : fallback;
}

export const usePlantStore = defineStore("plant", () => {
  const token = ref(localStorage.getItem(TOKEN_KEY));
  const user = ref<LoginResponse["user"] | null>(readStoredUser());
  const language = ref<UiLanguage>(localStorage.getItem(LANGUAGE_KEY) === "en" ? "en" : "zh");
  const health = ref<ApiRecord | null>(null);
  const live = ref<ApiRecord | null>(null);
  const config = ref<ApiRecord | null>(null);
  const audit = ref<ApiRecord | null>(null);
  const modbus = ref<ApiRecord | null>(null);
  const processes = ref<ApiRecord[]>([]);
  const selectedProcess = ref<ApiRecord | null>(null);
  const batches = ref<ApiRecord | null>(null);
  const recommendation = ref<ApiRecord | null>(null);
  const demoContext = ref<ApiRecord | null>(null);
  const deviceStatus = ref<ApiRecord | null>(null);
  const deviceCapabilities = ref<ApiRecord | null>(null);
  const permissionRoles = ref<ApiRecord | null>(null);
  const ainasTasks = ref<ApiRecord[]>([]);
  const selectedAinasTask = ref<ApiRecord | null>(null);
  const runtimeFallback = ref<ApiRecord | null>(null);
  const liveStatus = ref<"fresh" | "unavailable">("unavailable");
  const liveLastUpdated = ref<string | null>(null);
  const realtimeConnected = ref(false);
  let realtimeSocket: WebSocket | null = null;
  let realtimeReconnectTimer: number | null = null;
  const loading = ref(false);
  const error = ref<string | null>(null);
  const lastUpdated = ref<string | null>(null);

  const isAuthenticated = computed(() => Boolean(token.value && user.value));
  const role = computed(() => user.value?.role ?? "guest");
  const isChinese = computed(() => language.value === "zh");

  function setLanguage(nextLanguage: UiLanguage): void {
    language.value = nextLanguage;
    localStorage.setItem(LANGUAGE_KEY, nextLanguage);
  }

  function toggleLanguage(): void {
    setLanguage(language.value === "zh" ? "en" : "zh");
  }

  function tr(zh: string, en: string): string {
    return language.value === "zh" ? zh : en;
  }

  function mergeRuntimeFallback(patch: ApiRecord): void {
    const next: ApiRecord = { ...(runtimeFallback.value ?? {}) };
    for (const [key, value] of Object.entries(patch)) {
      if (value !== undefined) next[key] = value;
    }
    runtimeFallback.value = next;
  }

  function runtimeFromLive(payload: ApiRecord | null): ApiRecord | null {
    const runtime = payload?.runtime;
    return runtime && typeof runtime === "object" ? (runtime as ApiRecord) : null;
  }

  async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
    const headers = new Headers();
    headers.set("Accept", "application/json");
    if (options.body !== undefined) headers.set("Content-Type", "application/json");
    if (options.auth !== false && token.value) headers.set("Authorization", `Bearer ${token.value}`);

    let response: Response;
    try {
      response = await fetch(path, {
        method: options.method ?? (options.body === undefined ? "GET" : "POST"),
        headers,
        body: options.body === undefined ? undefined : JSON.stringify(options.body),
        cache: "no-store"
      });
    } catch (error) {
      if (options.allowFailure) return null as T;
      throw error;
    }
    const text = await response.text();
    let payload: unknown = null;
    try {
      payload = text ? JSON.parse(text) : null;
    } catch {
      if (options.allowFailure) return null as T;
      payload = { message: text };
    }
    if (!response.ok && options.allowFailure) {
      return null as T;
    }
    if (!response.ok) {
      throw new Error(errorMessage(payload, `${response.status} ${response.statusText}`));
    }
    return unwrapData<T>(payload);
  }

  async function requestBlob(path: string, options: RequestOptions = {}): Promise<Blob> {
    const headers = new Headers();
    headers.set("Accept", options.accept ?? "application/octet-stream");
    if (options.body !== undefined) headers.set("Content-Type", "application/json");
    if (options.auth !== false && token.value) headers.set("Authorization", `Bearer ${token.value}`);

    const response = await fetch(path, {
      method: options.method ?? (options.body === undefined ? "GET" : "POST"),
      headers,
      body: options.body === undefined ? undefined : JSON.stringify(options.body),
      cache: "no-store"
    });
    if (!response.ok) {
      const text = await response.text();
      let payload: unknown = text;
      try {
        payload = text ? JSON.parse(text) : null;
      } catch {
        payload = { message: text };
      }
      throw new Error(errorMessage(payload, `${response.status} ${response.statusText}`));
    }
    return response.blob();
  }

  function auditQueryPath(basePath: string, options: AuditQueryOptions = {}): string {
    const params = new URLSearchParams();
    if (options.page !== undefined) params.set("page", String(options.page));
    if (options.pageSize !== undefined) params.set("page_size", String(options.pageSize));
    if (options.eventType?.trim()) params.set("event_type", options.eventType.trim());
    const query = params.toString();
    return query ? `${basePath}?${query}` : basePath;
  }

  async function login(nextRole = "operator", password = rolePasswords[nextRole] ?? ""): Promise<void> {
    const payload = await request<LoginResponse>("/api/auth/login", {
      method: "POST",
      auth: false,
      body: { username: nextRole, password }
    });
    token.value = payload.token;
    user.value = payload.user;
    localStorage.setItem(TOKEN_KEY, payload.token);
    localStorage.setItem(USER_KEY, JSON.stringify(payload.user));
    await refreshProtected();
  }

  function logout(): void {
    token.value = null;
    user.value = null;
    config.value = null;
    audit.value = null;
    modbus.value = null;
    processes.value = [];
    selectedProcess.value = null;
    batches.value = null;
    recommendation.value = null;
    demoContext.value = null;
    deviceStatus.value = null;
    deviceCapabilities.value = null;
    permissionRoles.value = null;
    ainasTasks.value = [];
    selectedAinasTask.value = null;
    runtimeFallback.value = null;
    liveStatus.value = "unavailable";
    liveLastUpdated.value = null;
    localStorage.removeItem(TOKEN_KEY);
    localStorage.removeItem(USER_KEY);
  }

  async function refreshPublic(): Promise<void> {
    health.value = await request<ApiRecord>("/health", { auth: false });
  }

  async function refreshLive(): Promise<void> {
    let nextLive: ApiRecord | null = null;
    try {
      nextLive = await request<ApiRecord>("/api/live?sample_limit=36&include_processes=true&include_batches=true&include_events=false", {
        auth: false,
        allowFailure: true
      });
    } catch {
      nextLive = null;
    }
    live.value = nextLive;
    const runtime = runtimeFromLive(nextLive);
    if (runtime) {
      runtimeFallback.value = runtime;
      liveStatus.value = "fresh";
      liveLastUpdated.value = new Date().toLocaleTimeString();
    } else {
      liveStatus.value = "unavailable";
    }
  }

  async function refreshProtected(): Promise<void> {
    if (!token.value) return;
    const [configPayload, auditPayload, modbusPayload, processesPayload, batchesPayload, recommendationPayload] = await Promise.all([
      request<ApiRecord>("/api/config/summary"),
      request<ApiRecord>("/api/audit/logs?page=1&page_size=8"),
      request<ApiRecord>("/api/modbus/registers"),
      request<ApiRecord[]>("/api/processes", { allowFailure: true }),
      request<ApiRecord>("/api/batches", { allowFailure: true }),
      request<ApiRecord>("/api/recommendations/latest", { allowFailure: true })
    ]);
    config.value = configPayload;
    audit.value = auditPayload;
    modbus.value = modbusPayload;
    if (Array.isArray(processesPayload)) processes.value = processesPayload;
    if (batchesPayload && typeof batchesPayload === "object") batches.value = batchesPayload;
    if (recommendationPayload && typeof recommendationPayload === "object") recommendation.value = recommendationPayload;
  }

  async function loadDemoContext(): Promise<ApiRecord> {
    const response = await request<ApiRecord>("/api/demo/context", { auth: false });
    demoContext.value = response;
    return response;
  }

  async function loadDeviceStatus(): Promise<ApiRecord> {
    const response = await request<ApiRecord>("/api/devices/status", { auth: false });
    deviceStatus.value = response;
    return response;
  }

  async function loadDeviceCapabilities(): Promise<ApiRecord> {
    const response = await request<ApiRecord>("/api/devices/capabilities", { auth: false });
    deviceCapabilities.value = response;
    return response;
  }

  async function loadPermissionRoles(): Promise<ApiRecord> {
    const response = await request<ApiRecord>("/api/permissions/roles", { auth: false });
    permissionRoles.value = response;
    return response;
  }

  async function controlDeviceComponent(
    deviceId: string,
    componentId: string,
    payload: ComponentControlPayload
  ): Promise<ApiRecord> {
    const response = await request<ApiRecord>(
      `/api/devices/${encodeURIComponent(deviceId)}/components/${encodeURIComponent(componentId)}/control`,
      {
        method: "POST",
        body: payload
      }
    );
    await loadDeviceStatus();
    await loadDeviceCapabilities();
    await refreshLive();
    return response;
  }

  async function loadAinasTasks(limit = 20): Promise<ApiRecord[]> {
    const response = await request<ApiRecord[]>(`/api/integrations/ainas/tasks?limit=${limit}`);
    ainasTasks.value = Array.isArray(response) ? response : [];
    return ainasTasks.value;
  }

  async function loadAinasTask(id: number): Promise<ApiRecord> {
    const response = await request<ApiRecord>(`/api/integrations/ainas/tasks/${id}`);
    selectedAinasTask.value = response;
    return response;
  }

  async function createAinasTask(payload: AinasTaskPayload): Promise<ApiRecord> {
    const response = await request<ApiRecord>("/api/integrations/ainas/tasks", {
      method: "POST",
      body: payload
    });
    selectedAinasTask.value = response;
    await loadAinasTasks();
    await refreshLive();
    await refreshProtected();
    return response;
  }

  async function refreshAll(): Promise<void> {
    loading.value = true;
    error.value = null;
    try {
      await refreshPublic();
      await refreshLive();
      await refreshProtected();
      lastUpdated.value = new Date().toLocaleTimeString();
    } catch (nextError) {
      error.value = nextError instanceof Error ? nextError.message : String(nextError);
    } finally {
      loading.value = false;
    }
  }

  async function updateTargets(payload: TargetUpdatePayload): Promise<ApiRecord> {
    const targets = await request<ApiRecord>("/api/control/targets", {
      method: "POST",
      body: payload
    });
    mergeRuntimeFallback({ targets });
    await refreshLive();
    await refreshProtected();
    return targets;
  }

  async function setAutoEnabled(enabled: boolean): Promise<void> {
    await request<void>("/api/control/auto", {
      method: "POST",
      body: { enabled }
    });
    mergeRuntimeFallback({ auto_enabled: enabled });
    await refreshLive();
  }

  async function setManualLocked(locked: boolean): Promise<void> {
    await request<void>("/api/control/manual-lock", {
      method: "POST",
      body: { locked }
    });
    mergeRuntimeFallback(locked ? { manual_lock: true, auto_enabled: false } : { manual_lock: false });
    await refreshLive();
  }

  async function triggerEmergencyStop(): Promise<void> {
    await request<void>("/api/control/emergency-stop", { method: "POST" });
    mergeRuntimeFallback({ emergency_stop: true, auto_enabled: false });
    await refreshLive();
  }

  async function resetEmergencyStop(): Promise<void> {
    await request<void>("/api/control/emergency-stop/reset", { method: "POST" });
    mergeRuntimeFallback({ emergency_stop: false, auto_enabled: false });
    await refreshLive();
  }

  async function resetControlFault(): Promise<void> {
    await request<void>("/api/control/fault/reset", { method: "POST" });
    mergeRuntimeFallback({ last_control_error: null, auto_enabled: false });
    await refreshLive();
  }

  async function readModbusRegister(register: string): Promise<ApiRecord> {
    return request<ApiRecord>(`/api/modbus/registers/${encodeURIComponent(register)}/read`);
  }

  async function writeModbusRegister(register: string, payload: ModbusWritePayload): Promise<ApiRecord> {
    const response = await request<ApiRecord>(`/api/modbus/registers/${encodeURIComponent(register)}/write`, {
      method: "POST",
      body: payload
    });
    const targets = response.targets;
    if (targets && typeof targets === "object") mergeRuntimeFallback({ targets: targets as ApiRecord });
    await refreshProtected();
    return response;
  }

  async function loadAudit(options: AuditQueryOptions = {}): Promise<ApiRecord> {
    const response = await request<ApiRecord>(auditQueryPath("/api/audit/logs", options));
    audit.value = response;
    return response;
  }

  async function exportAuditCsv(eventType = ""): Promise<Blob> {
    return requestBlob(auditQueryPath("/api/audit/export.csv", { eventType }));
  }

  async function generateRecommendation(): Promise<ApiRecord> {
    const response = await request<ApiRecord>("/api/recommendations/latest", { method: "POST" });
    recommendation.value = response;
    return response;
  }

  async function applyAiControl(payload: AiControlRequest): Promise<ApiRecord> {
    const response = await request<ApiRecord>("/api/ai/control", {
      method: "POST",
      body: payload
    });
    await refreshLive();
    return response;
  }

  async function loadExperimentPlan(): Promise<ApiRecord> {
    return request<ApiRecord>("/api/ai/experiment-plan");
  }

  async function saveProductResult(payload: ProductResultPayload): Promise<ApiRecord> {
    const response = await request<ApiRecord>("/api/product-results", {
      method: "POST",
      body: payload
    });
    recommendation.value = response;
    await loadBatches();
    await refreshProtected();
    return response;
  }

  async function loadBatches(): Promise<ApiRecord> {
    const response = await request<ApiRecord>("/api/batches");
    batches.value = response;
    return response;
  }

  async function loadBatchDetail(batchId: number): Promise<ApiRecord> {
    return request<ApiRecord>(`/api/batches/${batchId}`);
  }

  async function exportBatchReport(batchId: number): Promise<Blob> {
    return requestBlob(`/api/batches/${batchId}/report.md`);
  }

  async function exportBatchesCsv(): Promise<Blob> {
    return requestBlob("/api/batches/export.csv", { accept: "text/csv" });
  }

  async function exportBatchesXlsx(): Promise<Blob> {
    return requestBlob("/api/batches/export.xlsx", {
      accept: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    });
  }

  async function loadProcesses(): Promise<ApiRecord[]> {
    const response = await request<ApiRecord[]>("/api/processes");
    processes.value = response;
    return response;
  }

  async function loadProcessDetail(processId: number): Promise<ApiRecord> {
    const response = await request<ApiRecord>(`/api/processes/${processId}`);
    selectedProcess.value = response;
    return response;
  }

  async function createProcess(payload: CreateProcessPayload): Promise<ApiRecord> {
    const response = await request<ApiRecord>("/api/processes", {
      method: "POST",
      body: payload
    });
    await loadProcesses();
    selectedProcess.value = { process: response, steps: [] };
    return response;
  }

  async function addProcessStep(processId: number, payload: ProcessStepPayload): Promise<ApiRecord> {
    const response = await request<ApiRecord>(`/api/processes/${processId}/steps`, {
      method: "POST",
      body: payload
    });
    await loadProcessDetail(processId);
    await loadProcesses();
    return response;
  }

  async function startProcess(processId: number): Promise<ApiRecord> {
    const response = await request<ApiRecord>(`/api/processes/${processId}/start`, { method: "POST" });
    const targets = response.applied_targets;
    const batch = response.batch;
    if (targets && typeof targets === "object") {
      mergeRuntimeFallback({
        targets: targets as ApiRecord,
        auto_enabled: true,
        active_batch_id: batch && typeof batch === "object" ? (batch as ApiRecord).id : undefined
      });
    }
    await refreshLive();
    await refreshProtected();
    return response;
  }

  async function stopCurrentProcess(): Promise<ApiRecord> {
    const response = await request<ApiRecord>("/api/processes/current/stop", { method: "POST" });
    const targets = response.stopped_targets;
    mergeRuntimeFallback({
      targets: targets && typeof targets === "object" ? (targets as ApiRecord) : undefined,
      auto_enabled: false,
      active_batch_id: null
    });
    await refreshLive();
    await refreshProtected();
    return response;
  }

  async function updateProcess(processId: number, payload: UpdateProcessPayload): Promise<ApiRecord> {
    const response = await request<ApiRecord>(`/api/processes/${processId}`, { method: "PUT", body: payload });
    await refreshProtected();
    return response;
  }

  async function updateProcessStep(
    processId: number,
    stepId: number,
    payload: UpdateProcessStepPayload
  ): Promise<ApiRecord> {
    const response = await request<ApiRecord>(`/api/processes/${processId}/steps/${stepId}`, {
      method: "PUT",
      body: payload
    });
    await refreshProtected();
    return response;
  }

  async function applyProcess(processId: number): Promise<ApiRecord> {
    const response = await request<ApiRecord>(`/api/processes/${processId}/apply`, { method: "POST" });
    await refreshLive();
    await refreshProtected();
    return response;
  }

  async function stopProcessById(processId: number, reason?: string): Promise<ApiRecord> {
    const response = await request<ApiRecord>(`/api/processes/${processId}/stop`, {
      method: "POST",
      body: { reason: reason ?? null }
    });
    await refreshLive();
    await refreshProtected();
    return response;
  }

  async function startBatch(payload: StartBatchPayload): Promise<ApiRecord> {
    const response = await request<ApiRecord>("/api/batches/start", { method: "POST", body: payload });
    await refreshLive();
    await refreshProtected();
    return response;
  }

  async function finishBatch(batchId: number): Promise<void> {
    await request<void>(`/api/batches/${batchId}/finish`, { method: "POST" });
    await refreshLive();
    await refreshProtected();
  }

  async function loadRealtime(deviceId: string): Promise<ApiRecord> {
    return request<ApiRecord>(`/api/v1/reactor/${encodeURIComponent(deviceId)}/realtime`);
  }

  async function loadHistory(deviceId: string, options: HistoryQueryOptions = {}): Promise<ApiRecord> {
    const params = new URLSearchParams();
    if (options.startTime) params.set("start_time", options.startTime);
    if (options.endTime) params.set("end_time", options.endTime);
    if (options.page !== undefined) params.set("page", String(options.page));
    if (options.pageSize !== undefined) params.set("page_size", String(options.pageSize));
    const query = params.toString();
    const path = `/api/v1/reactor/${encodeURIComponent(deviceId)}/history${query ? `?${query}` : ""}`;
    return request<ApiRecord>(path, { allowFailure: true });
  }

  // WebSocket realtime push (/ws/v1/reactor/:id/realtime, ~1 Hz from backend).
  // The WS payload is a single-device realtime snapshot, NOT the full /api/live
  // aggregate — so on each push we trigger a lightweight refreshLive() rather
  // than replace `live` (which would drop batches/processes/recommendation).
  // Net effect: latency drops from the 5 s poll to "push → refresh".
  function connectRealtimeSocket(deviceId: string): void {
    if (typeof WebSocket === "undefined") return;
    disconnectRealtimeSocket();
    const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
    const url = `${proto}//${window.location.host}/ws/v1/reactor/${encodeURIComponent(deviceId)}/realtime`;
    try {
      realtimeSocket = new WebSocket(url);
    } catch {
      scheduleReconnect(deviceId);
      return;
    }
    realtimeSocket.onopen = () => {
      realtimeConnected.value = true;
    };
    realtimeSocket.onclose = () => {
      realtimeConnected.value = false;
      scheduleReconnect(deviceId);
    };
    realtimeSocket.onerror = () => {
      realtimeConnected.value = false;
      try {
        realtimeSocket?.close();
      } catch {
        /* ignore */
      }
    };
    realtimeSocket.onmessage = () => {
      // Any push means the pipeline is alive — refresh the aggregate. We do not
      // parse the payload into live directly (shape mismatch with /api/live);
      // a refresh keeps batches/processes/recommendation consistent.
      void refreshLive();
    };
  }

  function scheduleReconnect(deviceId: string): void {
    if (realtimeReconnectTimer !== null) return;
    realtimeReconnectTimer = window.setTimeout(() => {
      realtimeReconnectTimer = null;
      connectRealtimeSocket(deviceId);
    }, 3000);
  }

  function disconnectRealtimeSocket(): void {
    if (realtimeReconnectTimer !== null) {
      window.clearTimeout(realtimeReconnectTimer);
      realtimeReconnectTimer = null;
    }
    if (realtimeSocket) {
      const sock = realtimeSocket;
      realtimeSocket = null;
      sock.onclose = null;
      sock.onerror = null;
      sock.onmessage = null;
      sock.onopen = null;
      try {
        sock.close();
      } catch {
        /* ignore */
      }
    }
    realtimeConnected.value = false;
  }

  return {
    token,
    user,
    language,
    role,
    isChinese,
    isAuthenticated,
    health,
    live,
    config,
    audit,
    modbus,
    processes,
    selectedProcess,
    batches,
    recommendation,
    demoContext,
    deviceStatus,
    deviceCapabilities,
    permissionRoles,
    ainasTasks,
    selectedAinasTask,
    runtimeFallback,
    liveStatus,
    liveLastUpdated,
    realtimeConnected,
    loading,
    error,
    lastUpdated,
    setLanguage,
    toggleLanguage,
    tr,
    login,
    logout,
    refreshAll,
    refreshPublic,
    refreshLive,
    refreshProtected,
    updateTargets,
    setAutoEnabled,
    setManualLocked,
    triggerEmergencyStop,
    resetEmergencyStop,
    resetControlFault,
    loadDemoContext,
    loadDeviceStatus,
    loadDeviceCapabilities,
    loadPermissionRoles,
    controlDeviceComponent,
    loadAinasTasks,
    loadAinasTask,
    createAinasTask,
    readModbusRegister,
    writeModbusRegister,
    loadAudit,
    exportAuditCsv,
    generateRecommendation,
    applyAiControl,
    loadExperimentPlan,
    saveProductResult,
    loadBatches,
    loadBatchDetail,
    exportBatchesCsv,
    exportBatchesXlsx,
    exportBatchReport,
    loadProcesses,
    loadProcessDetail,
    createProcess,
    addProcessStep,
    startProcess,
    stopCurrentProcess,
    updateProcess,
    updateProcessStep,
    applyProcess,
    stopProcessById,
    startBatch,
    finishBatch,
    loadRealtime,
    loadHistory,
    connectRealtimeSocket,
    disconnectRealtimeSocket
  };
});
