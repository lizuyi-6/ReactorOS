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
  {
    path: "/monitor",
    component: MonitorView,
    meta: { label: "Realtime Monitor", zh: "实时监控", en: "Realtime Monitor", subZh: "传感器与曲线", subEn: "Sensors and trends" }
  },
  {
    path: "/control",
    component: ControlView,
    meta: { label: "Process Control", zh: "参数配置", en: "Process Control", subZh: "安全限幅", subEn: "Safety limits" }
  },
  {
    path: "/ai",
    component: AiView,
    meta: { label: "AI Decision", zh: "AI 决策", en: "AI Decision", subZh: "推荐与复核", subEn: "Advice and review" }
  },
  {
    path: "/history",
    component: HistoryView,
    meta: { label: "History Data", zh: "历史数据", en: "History Data", subZh: "批次与结果", subEn: "Batches and results" }
  },
  {
    path: "/audit",
    component: AuditView,
    meta: { label: "Audit Log", zh: "审计日志", en: "Audit Log", subZh: "哈希链", subEn: "Hash chain" }
  },
  {
    path: "/modbus",
    component: ModbusView,
    meta: { label: "Modbus Debug", zh: "Modbus 调试", en: "Modbus Debug", subZh: "寄存器映射", subEn: "Register map" }
  },
  {
    path: "/settings",
    component: SettingsView,
    meta: { label: "System Settings", zh: "系统配置", en: "System Settings", subZh: "设备与安全", subEn: "Device and security" }
  }
];

export const router = createRouter({
  history: createWebHashHistory(),
  routes
});
