import { createRouter, createWebHashHistory } from "vue-router";

import AiView from "./views/AiView.vue";
import AuditView from "./views/AuditView.vue";
import ControlView from "./views/ControlView.vue";
import HistoryView from "./views/HistoryView.vue";
import LoginView from "./views/LoginView.vue";
import ModbusView from "./views/ModbusView.vue";
import MonitorView from "./views/MonitorView.vue";
import SettingsView from "./views/SettingsView.vue";

export const routes = [
  { path: "/", redirect: "/monitor" },
  {
    path: "/login",
    component: LoginView,
    meta: { public: true, zh: "登录", en: "Sign in" }
  },
  {
    path: "/monitor",
    component: MonitorView,
    meta: { zh: "实时监控", en: "Monitor", subZh: "传感器与趋势", subEn: "Sensors & trends", icon: "▦" }
  },
  {
    path: "/control",
    component: ControlView,
    meta: { zh: "参数配置", en: "Control", subZh: "工艺与安全", subEn: "Process & safety", icon: "≋" }
  },
  {
    path: "/ai",
    component: AiView,
    meta: { zh: "AI 决策", en: "AI Lab", subZh: "推荐与复核", subEn: "Advice & review", icon: "AI" }
  },
  {
    path: "/history",
    component: HistoryView,
    meta: { zh: "历史数据", en: "History", subZh: "批次与结果", subEn: "Batches & results", icon: "▤" }
  },
  {
    path: "/audit",
    component: AuditView,
    meta: { zh: "审计日志", en: "Audit", subZh: "哈希链", subEn: "Hash chain", icon: "↺", requiresAuth: true }
  },
  {
    path: "/modbus",
    component: ModbusView,
    meta: { zh: "Modbus 调试", en: "Modbus", subZh: "寄存器映射", subEn: "Register map", icon: "MB" }
  },
  {
    path: "/settings",
    component: SettingsView,
    meta: { zh: "系统配置", en: "Settings", subZh: "设备与集成", subEn: "Device & integrations", icon: "⚙" }
  }
];

export const router = createRouter({
  history: createWebHashHistory(),
  routes
});

// 路由守卫：仅对标记 requiresAuth 的页面强制登录；其余页面公开可读，
// 页面内写操作仍由后端 401/403 拦截（配合全局登出）。
router.beforeEach((to) => {
  if (to.meta?.public || !to.meta?.requiresAuth) return true;
  const hasToken = Boolean(localStorage.getItem("reactoros.vue.auth.token"));
  if (!hasToken) {
    return { path: "/login", query: { redirect: to.fullPath } };
  }
  return true;
});
