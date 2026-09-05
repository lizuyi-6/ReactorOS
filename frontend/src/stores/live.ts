// 实时数据 store：/api/live 轮询 + WebSocket 1Hz 推送合并。
// 关键点（审计 §2.1/§3.2）：
// - /api/live 在无新鲜样本时返回 503，这是 fail-closed 常态，视为"数据不可用"而非错误。
// - WS 推送单设备快照（RealtimePayload），需映射进 live 结构并保持样本窗口。

import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { DEVICE_ID, realtimeApi, realtimeSocketUrl, systemApi } from "../api";
import type {
  AiRecommendationEnvelope,
  DeviceStatusItem,
  DeviceStatusSummary,
  LiveResponse,
  RealtimePayload,
  RuntimeState,
  SensorSample
} from "../api/types";

function readPositiveInt(value: unknown, fallback: number, min: number, max: number): number {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) return fallback;
  return Math.min(max, Math.max(min, Math.trunc(parsed)));
}

export const HMI_REFRESH_INTERVAL_MS = readPositiveInt(import.meta.env.XINGSHU_VITE_REFRESH_MS, 15_000, 5_000, 60_000);
export const LIVE_CALIBRATION_INTERVAL_MS = 60_000;
export const LIVE_SAMPLE_LIMIT = readPositiveInt(import.meta.env.XINGSHU_VITE_LIVE_SAMPLE_LIMIT, 24, 1, 120);

export const useLiveStore = defineStore("live", () => {
  const live = ref<LiveResponse | null>(null);
  /** live 不可用（503）时保留的最近一次 runtime，供控制页降级显示。 */
  const runtimeFallback = ref<RuntimeState | null>(null);
  const liveStatus = ref<"fresh" | "unavailable">("unavailable");
  const liveLastUpdated = ref<string | null>(null);
  const liveLastRefreshAt = ref<number | null>(null);
  const realtimeConnected = ref(false);

  let socket: WebSocket | null = null;
  let reconnectTimer: number | null = null;
  let refreshPromise: Promise<void> | null = null;
  let getToken: () => string | null = () => null;

  const runtime = computed<RuntimeState | null>(() => live.value?.runtime ?? runtimeFallback.value);
  const latestSample = computed<SensorSample | null>(() => runtime.value?.latest_sample ?? null);
  const recentSamples = computed<SensorSample[]>(() =>
    Array.isArray(live.value?.recent_samples) ? (live.value!.recent_samples as SensorSample[]) : []
  );
  const alarms = computed(() => (Array.isArray(live.value?.alarms) ? live.value!.alarms! : []));
  const recommendation = computed<AiRecommendationEnvelope | null>(
    () => live.value?.latest_recommendation ?? null
  );
  const primaryDevice = computed<DeviceStatusItem | null>(() => live.value?.device_status?.devices?.[0] ?? null);
  const refreshIntervalMs = computed(() =>
    realtimeConnected.value ? LIVE_CALIBRATION_INTERVAL_MS : HMI_REFRESH_INTERVAL_MS
  );
  const nextRefreshDelayMs = computed(() => {
    const last = liveLastRefreshAt.value;
    return last === null ? 0 : Math.max(0, last + refreshIntervalMs.value - Date.now());
  });

  function markUnavailable(): void {
    liveStatus.value = "unavailable";
  }

  function applyLive(payload: LiveResponse | null): void {
    live.value = payload;
    if (payload?.runtime) {
      runtimeFallback.value = payload.runtime;
      liveStatus.value = "fresh";
      liveLastUpdated.value = new Date().toLocaleTimeString();
    } else {
      markUnavailable();
    }
  }

  function refreshLive(): Promise<void> {
    if (refreshPromise) return refreshPromise;
    refreshPromise = (async () => {
      try {
        const payload = await systemApi.live(LIVE_SAMPLE_LIMIT);
        applyLive(payload);
      } catch {
        // 503（样本缺失/过期）是常态降级，静默进入 unavailable。
        applyLive(null);
      } finally {
        liveLastRefreshAt.value = Date.now();
        refreshPromise = null;
      }
    })();
    return refreshPromise;
  }

  function realtimeSampleFromPayload(payload: RealtimePayload, previous: SensorSample | null): SensorSample | null {
    const data = payload.data;
    if (!data) return null;
    const timestamp = payload.timestamp ?? new Date().toISOString();
    return {
      ...(previous ?? {}),
      captured_at: timestamp,
      created_at: timestamp,
      temperature_c: data.current_temp ?? null,
      pressure_mpa: data.current_pressure ?? null,
      stirrer_rpm: data.stir_speed ?? null,
      shake_speed_cpm: data.shake_speed ?? null,
      tilt_state: data.tilt_state ?? null,
      tilt_angle_deg: data.tilt_angle ?? null,
      flow_rate_l_min: data.flow_rate ?? null,
      product_concentration_percent: data.product_concentration_percent ?? previous?.product_concentration_percent ?? null,
      ph: data.ph ?? previous?.ph ?? null
    };
  }

  function applyRealtimePayload(payload: RealtimePayload): void {
    // 首帧可能是错误信封（{code,message,data:{error}}）——识别并视为不可用。
    if ("code" in payload && typeof (payload as { code?: unknown }).code === "number") {
      markUnavailable();
      return;
    }
    const previousLive = live.value ?? {};
    const previousRuntime = previousLive.runtime ?? runtimeFallback.value ?? undefined;
    const previousSample = previousRuntime?.latest_sample ?? null;
    const sample = realtimeSampleFromPayload(payload, previousSample);
    if (!sample) {
      markUnavailable();
      return;
    }

    const previousSamples = Array.isArray(previousLive.recent_samples) ? previousLive.recent_samples : [];
    const device = payload.device_status ?? null;
    const nextDeviceStatus: DeviceStatusSummary | undefined = device
      ? {
          total_count: 1,
          online_count: payload.device_online === false ? 0 : 1,
          devices: [device],
          sensors: Array.isArray(device.sensors) ? device.sensors : [],
          components: Array.isArray(device.components) ? device.components : []
        }
      : previousLive.device_status;

    const nextRuntime: RuntimeState = { ...(previousRuntime ?? {}), latest_sample: sample };
    runtimeFallback.value = nextRuntime;
    live.value = {
      ...previousLive,
      runtime: nextRuntime,
      device_status: nextDeviceStatus,
      alarms: Array.isArray(payload.alarms) ? payload.alarms : previousLive.alarms,
      recent_samples: [...previousSamples, sample].slice(-LIVE_SAMPLE_LIMIT)
    };
    liveStatus.value = "fresh";
    liveLastUpdated.value = new Date().toLocaleTimeString();
  }

  function connectRealtimeSocket(): void {
    const token = getToken();
    if (typeof WebSocket === "undefined" || !token) {
      realtimeConnected.value = false;
      return;
    }
    disconnectRealtimeSocket();
    let sock: WebSocket;
    try {
      sock = new WebSocket(realtimeSocketUrl(DEVICE_ID, token));
    } catch {
      scheduleReconnect();
      return;
    }
    socket = sock;
    sock.onopen = () => {
      realtimeConnected.value = true;
    };
    sock.onclose = () => {
      realtimeConnected.value = false;
      scheduleReconnect();
    };
    sock.onerror = () => {
      realtimeConnected.value = false;
      try {
        sock.close();
      } catch {
        /* ignore */
      }
    };
    sock.onmessage = (event) => {
      try {
        applyRealtimePayload(JSON.parse(String(event.data)) as RealtimePayload);
      } catch {
        markUnavailable();
      }
    };
  }

  function scheduleReconnect(): void {
    if (reconnectTimer !== null) return;
    reconnectTimer = window.setTimeout(() => {
      reconnectTimer = null;
      connectRealtimeSocket();
    }, 3000);
  }

  function disconnectRealtimeSocket(): void {
    if (reconnectTimer !== null) {
      window.clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
    if (socket) {
      const sock = socket;
      socket = null;
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

  /** 由 App 注入 token 提供者，避免 store 间循环依赖。 */
  function bindTokenProvider(provider: () => string | null): void {
    getToken = provider;
  }

  return {
    live,
    runtimeFallback,
    runtime,
    latestSample,
    recentSamples,
    alarms,
    recommendation,
    primaryDevice,
    liveStatus,
    liveLastUpdated,
    liveLastRefreshAt,
    realtimeConnected,
    refreshIntervalMs,
    nextRefreshDelayMs,
    refreshLive,
    applyLive,
    connectRealtimeSocket,
    disconnectRealtimeSocket,
    bindTokenProvider
  };
});
