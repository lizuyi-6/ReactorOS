import { computed, type Ref } from "vue";

import type { ApiRecord } from "./stores/plant";
import { arrayAt, textAt } from "./views/view-utils";

export interface AppShellStoreLike {
  tr(zh: string, en: string): string;
  health: ApiRecord | null;
  live: ApiRecord | null;
  config: ApiRecord | null;
  runtimeFallback: ApiRecord | null;
  liveStatus: string;
  liveLastUpdated: string | null;
  deviceStatus: ApiRecord | null;
  user: { username: string; role: string } | null;
  lastUpdated: string | null;
}

export const hmiNavItems = [
  { path: "/monitor", icon: "▦", label: "Monitor" },
  { path: "/history", icon: "▤", label: "Batches" },
  { path: "/control", icon: "≋", label: "Control" },
  { path: "/ai", icon: "AI", label: "AI Lab" },
  { path: "/audit", icon: "↺", label: "History" },
  { path: "/modbus", icon: "MB", label: "Modbus" },
  { path: "/settings", icon: "⚙", label: "Settings" }
] as const;

export function useAppShellState(store: AppShellStoreLike, activePath: Readonly<Ref<string>>, now: Readonly<Ref<Date>>) {
  const healthStatus = computed(() => String(store.health?.status ?? store.health?.service ?? "unknown"));
  const lastUpdatedText = computed(() => store.lastUpdated ?? "--");
  const runtime = computed(() => {
    const value = store.live?.runtime;
    return value && typeof value === "object" ? (value as ApiRecord) : store.runtimeFallback;
  });
  const batchLabel = computed(() => {
    const id = runtime.value?.active_batch_id;
    return id === null || id === undefined || id === "" ? "Batch --" : `Batch #${id}`;
  });
  const clockText = computed(() =>
    now.value.toLocaleTimeString("en-GB", {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit"
    })
  );
  const liveStatusText = computed(() =>
    store.liveStatus === "fresh"
      ? store.tr(`现场 ${store.liveLastUpdated ?? "--"}`, `Live ${store.liveLastUpdated ?? "--"}`)
      : store.tr("现场不可用", "Live unavailable")
  );
  const liveAlarms = computed(() => arrayAt<ApiRecord>(store.live, "alarms"));
  const fieldScenario = computed(() => {
    const fromLive = store.live?.field_scenario;
    if (fromLive && typeof fromLive === "object") return fromLive as ApiRecord;
    const fromConfig = store.config?.field_scenario;
    return fromConfig && typeof fromConfig === "object" ? (fromConfig as ApiRecord) : null;
  });
  const scenarioLabel = computed(() => {
    const kind = textAt(fieldScenario.value, "kind", "");
    const translations: Record<string, { zh: string; en: string }> = {
      lab_research: { zh: "实验室", en: "Lab" },
      pilot_scale: { zh: "中试", en: "Pilot" },
      legacy_retrofit: { zh: "改造线", en: "Retrofit" },
      offline_demo: { zh: "离线演示", en: "Demo" },
      petrochemical: { zh: "石油化", en: "Petrochem" }
    };
    const label = translations[kind];
    return label ? store.tr(label.zh, label.en) : textAt(fieldScenario.value, "label", "Scenario --");
  });
  const scenarioText = computed(() => `SCN ${scenarioLabel.value}`);
  const scenarioStatusType = computed(() => {
    if (textAt(fieldScenario.value, "petrochemical_handling_required", "false") === "true") return "warning";
    const kind = textAt(fieldScenario.value, "kind", "");
    if (kind === "offline_demo") return "info";
    return "success";
  });
  const deviceRows = computed(() => arrayAt<ApiRecord>(store.deviceStatus, "devices"));
  const alarmCounts = computed(() => {
    const counts = { high: 0, warning: 0, info: 0 };
    for (const alarm of liveAlarms.value) {
      const level = textAt(alarm, "level", textAt(alarm, "severity", "info")).toLowerCase();
      if (["critical", "fatal", "high", "danger", "error"].includes(level)) counts.high += 1;
      else if (["warning", "warn", "medium"].includes(level)) counts.warning += 1;
      else counts.info += 1;
    }
    return counts;
  });
  const alarmStatusType = computed(() => {
    if (alarmCounts.value.high > 0) return "danger";
    if (alarmCounts.value.warning > 0) return "warning";
    return "success";
  });
  const alarmSummaryText = computed(() =>
    store.tr(
      `报警 H${alarmCounts.value.high} W${alarmCounts.value.warning} I${alarmCounts.value.info}`,
      `ALM H${alarmCounts.value.high} W${alarmCounts.value.warning} I${alarmCounts.value.info}`
    )
  );
  const emergencyStopActive = computed(() => textAt(runtime.value, "emergency_stop", "false") === "true");
  const manualLockActive = computed(() => textAt(runtime.value, "manual_lock", "false") === "true");
  const controlLoopTerminated = computed(() => textAt(runtime.value, "control_loop_terminated", "false") === "true");
  const sensorFaultText = computed(() => textAt(runtime.value, "last_sensor_error", ""));
  const controlFaultText = computed(() => textAt(runtime.value, "last_control_error", ""));
  const safetyStatusType = computed(() => {
    if (controlLoopTerminated.value || emergencyStopActive.value || sensorFaultText.value) return "danger";
    if (manualLockActive.value || controlFaultText.value) return "warning";
    return "success";
  });
  const safetySummaryText = computed(() => {
    if (controlLoopTerminated.value) return store.tr("控制环终止", "CTRL LOOP STOP");
    if (emergencyStopActive.value) return "E-STOP ACTIVE";
    if (sensorFaultText.value) return store.tr("传感器故障", "SENSOR FAULT");
    if (controlFaultText.value) return store.tr("控制故障", "CONTROL FAULT");
    if (manualLockActive.value) return store.tr("人工锁定", "MANUAL LOCK");
    return store.tr("联锁正常", "INTERLOCK OK");
  });
  const commandReceipt = computed(() =>
    deviceRows.value.find((device) => textAt(device, "last_command_request_id", "") || textAt(device, "last_command_error", ""))
  );
  const commandStatusType = computed(() => {
    if (!commandReceipt.value) return "info";
    if (textAt(commandReceipt.value, "last_command_error", "")) return "danger";
    if (textAt(commandReceipt.value, "last_command_ok", "") === "true") return "success";
    if (textAt(commandReceipt.value, "last_command_ok", "") === "false") return "danger";
    return "warning";
  });
  const commandReceiptText = computed(() => {
    const device = commandReceipt.value;
    if (!device) return "CMD --";
    const id = textAt(device, "last_command_request_id", "--");
    if (textAt(device, "last_command_error", "")) return `CMD FAIL ${id}`;
    const ok = textAt(device, "last_command_ok", "");
    if (ok === "true") return `CMD OK ${id}`;
    if (ok === "false") return `CMD FAIL ${id}`;
    return `CMD PEND ${id}`;
  });
  const sessionRoleLabel = computed(() => {
    const role = store.user?.role;
    if (!role) return store.tr("未登录", "not signed in");
    const labels: Record<string, { zh: string; en: string }> = {
      operator: { zh: "操作员", en: "Operator" },
      engineer: { zh: "工程师", en: "Engineer" },
      admin: { zh: "管理员", en: "Administrator" }
    };
    const label = labels[role] ?? { zh: role, en: role };
    return store.tr(label.zh, label.en);
  });
  const contentClasses = computed(() => ({
    "monitor-screen": activePath.value === "/monitor",
    "hmi-fixed": activePath.value !== "/monitor",
    [`route-${activePath.value.replace(/^\//, "") || "monitor"}`]: true
  }));

  return {
    alarmStatusType,
    alarmSummaryText,
    batchLabel,
    clockText,
    commandReceiptText,
    commandStatusType,
    contentClasses,
    healthStatus,
    lastUpdatedText,
    liveStatusText,
    safetyStatusType,
    safetySummaryText,
    scenarioStatusType,
    scenarioText,
    sessionRoleLabel
  };
}
