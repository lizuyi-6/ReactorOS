import { computed, ref } from "vue";
import { defineStore } from "pinia";

export type ApiRecord = Record<string, unknown>;
export type UiLanguage = "zh" | "en";

type HttpMethod = "GET" | "POST" | "PUT" | "DELETE";

interface RequestOptions {
  method?: HttpMethod;
  body?: unknown;
  auth?: boolean;
  allowFailure?: boolean;
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
  const recommendation = ref<ApiRecord | null>(null);
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

  async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
    const headers = new Headers();
    headers.set("Accept", "application/json");
    if (options.body !== undefined) headers.set("Content-Type", "application/json");
    if (options.auth !== false && token.value) headers.set("Authorization", `Bearer ${token.value}`);

    const response = await fetch(path, {
      method: options.method ?? (options.body === undefined ? "GET" : "POST"),
      headers,
      body: options.body === undefined ? undefined : JSON.stringify(options.body),
      cache: "no-store"
    });
    const text = await response.text();
    const payload = text ? JSON.parse(text) : null;
    if (!response.ok && options.allowFailure) {
      return null as T;
    }
    if (!response.ok) {
      throw new Error(errorMessage(payload, `${response.status} ${response.statusText}`));
    }
    return unwrapData<T>(payload);
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
    recommendation.value = null;
    localStorage.removeItem(TOKEN_KEY);
    localStorage.removeItem(USER_KEY);
  }

  async function refreshPublic(): Promise<void> {
    health.value = await request<ApiRecord>("/health", { auth: false });
  }

  async function refreshLive(): Promise<void> {
    live.value = await request<ApiRecord>("/api/live?sample_limit=36&include_processes=true&include_batches=true&include_events=false", {
      auth: false,
      allowFailure: true
    });
  }

  async function refreshProtected(): Promise<void> {
    if (!token.value) return;
    const [configPayload, auditPayload, modbusPayload, recommendationPayload] = await Promise.all([
      request<ApiRecord>("/api/config/summary"),
      request<ApiRecord>("/api/audit/logs?limit=8"),
      request<ApiRecord>("/api/modbus/registers"),
      request<ApiRecord>("/api/recommendations/latest")
    ]);
    config.value = configPayload;
    audit.value = auditPayload;
    modbus.value = modbusPayload;
    recommendation.value = recommendationPayload;
  }

  async function refreshAll(): Promise<void> {
    loading.value = true;
    error.value = null;
    try {
      await refreshPublic();
      await refreshProtected();
      lastUpdated.value = new Date().toLocaleTimeString();
    } catch (nextError) {
      error.value = nextError instanceof Error ? nextError.message : String(nextError);
    } finally {
      loading.value = false;
    }
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
    recommendation,
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
    refreshProtected
  };
});
