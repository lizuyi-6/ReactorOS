import { createRouter, createWebHashHistory } from "vue-router";

import AiView from "./views/AiView.vue";
import AuditView from "./views/AuditView.vue";
import ControlView from "./views/ControlView.vue";
import HistoryView from "./views/HistoryView.vue";
import LoginView from "./views/LoginView.vue";
import ModbusView from "./views/ModbusView.vue";
import MonitorView from "./views/MonitorView.vue";
import SettingsView from "./views/SettingsView.vue";

// meta: en/zh 为侧栏双语标签；icon 对应 components/AppIcon.vue 的 name。
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
    meta: { zh: "监控", en: "Monitor", titleZh: "反应釜总览", titleEn: "Reactor Overview", icon: "monitor" }
  },
  {
    path: "/control",
    component: ControlView,
    meta: { zh: "控制", en: "Control", titleZh: "工艺控制中心", titleEn: "Process Control", icon: "control" }
  },
  {
    path: "/ai",
    component: AiView,
    meta: { zh: "智能决策", en: "AI Decision", titleZh: "AI 决策中心", titleEn: "AI Decision Center", icon: "ai" }
  },
  {
    path: "/history",
    component: HistoryView,
    meta: { zh: "历史数据", en: "History", titleZh: "历史数据与批次记录", titleEn: "History & Batch Records", icon: "history" }
  },
  {
    path: "/audit",
    component: AuditView,
    meta: { zh: "审计追踪", en: "Audit", titleZh: "审计追踪", titleEn: "Audit Trail", icon: "audit", requiresAuth: true }
  },
  {
    path: "/modbus",
    component: ModbusView,
    meta: { zh: "设备通信", en: "Modbus", titleZh: "设备通信调试", titleEn: "Modbus Debug", icon: "modbus" }
  },
  {
    path: "/settings",
    component: SettingsView,
    meta: { zh: "系统设置", en: "Settings", titleZh: "系统设置", titleEn: "System Settings", icon: "settings" }
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
