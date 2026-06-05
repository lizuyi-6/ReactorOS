import { createRouter, createWebHashHistory } from "vue-router";

import AiView from "./views/AiView.vue";
import AuditView from "./views/AuditView.vue";
import ControlView from "./views/ControlView.vue";
import HistoryView from "./views/HistoryView.vue";
import ModbusView from "./views/ModbusView.vue";
import MonitorView from "./views/MonitorView.vue";
import SettingsView from "./views/SettingsView.vue";

export const routes = [
  { path: "/", redirect: "/monitor" },
  { path: "/monitor", component: MonitorView, meta: { label: "Realtime Monitor", zh: "实时监控", en: "Realtime Monitor" } },
  { path: "/control", component: ControlView, meta: { label: "Process Control", zh: "参数配置", en: "Process Control" } },
  { path: "/ai", component: AiView, meta: { label: "AI Decision", zh: "AI 决策", en: "AI Decision" } },
  { path: "/history", component: HistoryView, meta: { label: "History Data", zh: "历史数据", en: "History Data" } },
  { path: "/audit", component: AuditView, meta: { label: "Audit Log", zh: "审计日志", en: "Audit Log" } },
  { path: "/modbus", component: ModbusView, meta: { label: "Modbus Debug", zh: "Modbus 调试", en: "Modbus Debug" } },
  { path: "/settings", component: SettingsView, meta: { label: "System Settings", zh: "系统配置", en: "System Settings" } }
];

export const router = createRouter({
  history: createWebHashHistory(),
  routes
});
