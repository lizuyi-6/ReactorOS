// 告警翻译：仅对固定枚举值（type/level）做 key 翻译；message/suggestion 透传后端原文。
// （旧前端对整句英文做正则翻译，后端改词即静默失效——此处刻意只翻枚举 key。）

import type { Alarm } from "../api/types";

type Tr = (zh: string, en: string) => string;

const ALARM_TYPE_LABELS: Record<string, { zh: string; en: string }> = {
  emergency_stop: { zh: "急停", en: "Emergency stop" },
  communication_error: { zh: "通信错误", en: "Communication error" },
  sensor_error: { zh: "传感器错误", en: "Sensor error" },
  temperature_limit: { zh: "温度越限", en: "Temperature limit" },
  pressure_limit: { zh: "压力越限", en: "Pressure limit" },
  stirrer_limit: { zh: "搅拌越限", en: "Stirrer limit" },
  shake_speed_limit: { zh: "摇摆速度越限", en: "Shake speed limit" },
  tilt_angle_limit: { zh: "倾角越限", en: "Tilt angle limit" },
  flow_rate_limit: { zh: "流量越限", en: "Flow rate limit" },
  product_concentration_limit: { zh: "产物浓度越限", en: "Product concentration limit" },
  ph_limit: { zh: "pH 越限", en: "pH limit" },
  unfinished_batch_recovery: { zh: "批次恢复", en: "Batch recovery" }
};

const ALARM_LEVEL_LABELS: Record<string, { zh: string; en: string }> = {
  high: { zh: "高", en: "High" },
  medium: { zh: "中", en: "Medium" },
  warning: { zh: "预警", en: "Warning" },
  low: { zh: "低", en: "Low" }
};

export function alarmLevel(alarm: Alarm): string {
  return String(alarm.level ?? alarm.severity ?? "info").toLowerCase();
}

export function alarmTypeLabel(alarm: Alarm, tr: Tr): string {
  const type = String(alarm.type ?? alarm.code ?? "");
  const label = ALARM_TYPE_LABELS[type];
  return label ? tr(label.zh, label.en) : type || "--";
}

export function alarmLevelLabel(alarm: Alarm, tr: Tr): string {
  const level = alarmLevel(alarm);
  const label = ALARM_LEVEL_LABELS[level];
  return label ? tr(label.zh, label.en) : level;
}

export function alarmTone(alarm: Alarm): "danger" | "warning" | "info" {
  const level = alarmLevel(alarm);
  if (level === "high" || level === "critical" || level === "error") return "danger";
  if (level === "medium" || level === "warning" || level === "warn") return "warning";
  return "info";
}

export function alarmMessage(alarm: Alarm): string {
  return String(alarm.message ?? "--");
}

export function alarmSuggestion(alarm: Alarm): string {
  return String(alarm.suggestion ?? "");
}
