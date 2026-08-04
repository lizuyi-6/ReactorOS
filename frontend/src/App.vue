<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { routes } from "./router";
import { useAuthStore } from "./stores/auth";
import { useLiveStore, HMI_REFRESH_INTERVAL_MS } from "./stores/live";
import { useLanguage } from "./i18n";
import { boolText } from "./utils/format";
import { DEVICE_ID } from "./api";

const auth = useAuthStore();
const live = useLiveStore();
const route = useRoute();
const router = useRouter();
const { language, setLanguage, tr } = useLanguage();

const now = ref(new Date());
let refreshTimer: number | null = null;
let clockTimer: number | null = null;

const navItems = computed(() => routes.filter((item) => item.path !== "/" && item.path !== "/login" && item.meta));
const activePath = computed(() => route.path);
const isLoginPage = computed(() => activePath.value === "/login");

const clockText = computed(() =>
  now.value.toLocaleTimeString(language.value === "zh" ? "zh-CN" : "en-US", { hour: "2-digit", minute: "2-digit", second: "2-digit" })
);

const runtime = computed(() => live.runtime);
const activeBatchId = computed(() => {
  const id = runtime.value?.active_batch_id;
  return id === null || id === undefined ? null : Number(id);
});

const alarmCounts = computed(() => {
  const counts = { high: 0, warning: 0, info: 0 };
  for (const alarm of live.alarms) {
    const level = String(alarm.level ?? alarm.severity ?? "info").toLowerCase();
    if (["critical", "fatal", "high", "danger", "error"].includes(level)) counts.high += 1;
    else if (["warning", "warn", "medium"].includes(level)) counts.warning += 1;
    else counts.info += 1;
  }
  return counts;
});

const alarmTone = computed<"ok" | "warn" | "bad">(() => {
  if (alarmCounts.value.high > 0) return "bad";
  if (alarmCounts.value.warning > 0) return "warn";
  return "ok";
});

const safetyState = computed<{ tone: "ok" | "warn" | "bad"; label: string }>(() => {
  const rt = runtime.value;
  if (boolText(rt?.control_loop_terminated)) return { tone: "bad", label: tr("控制环终止", "LOOP STOP") };
  if (boolText(rt?.emergency_stop)) return { tone: "bad", label: tr("急停中", "E-STOP") };
  if (rt?.last_sensor_error) return { tone: "bad", label: tr("传感器故障", "SENSOR") };
  if (rt?.last_control_error) return { tone: "warn", label: tr("控制故障", "CTRL FLT") };
  if (boolText(rt?.manual_lock)) return { tone: "warn", label: tr("人工锁定", "M-LOCK") };
  return { tone: "ok", label: tr("联锁正常", "OK") };
});

const roleLabel = computed(() => {
  const role = auth.user?.role;
  if (!role) return tr("未登录", "Guest");
  const labels: Record<string, { zh: string; en: string }> = {
    operator: { zh: "操作员", en: "Operator" },
    engineer: { zh: "工程师", en: "Engineer" },
    admin: { zh: "管理员", en: "Admin" }
  };
  const label = labels[role];
  return label ? tr(label.zh, label.en) : role;
});

async function handleLogout(): Promise<void> {
  auth.logout();
  live.disconnectRealtimeSocket();
  await router.push("/login");
}

async function boot(): Promise<void> {
  live.bindTokenProvider(() => auth.token);
  await auth.restoreSession();
  await live.refreshLive();
  if (auth.isAuthenticated) live.connectRealtimeSocket();
}

onMounted(() => {
  void boot();
  refreshTimer = window.setInterval(() => {
    void live.refreshLive();
  }, HMI_REFRESH_INTERVAL_MS);
  clockTimer = window.setInterval(() => {
    now.value = new Date();
  }, 1000);
});

onBeforeUnmount(() => {
  live.disconnectRealtimeSocket();
  if (refreshTimer !== null) window.clearInterval(refreshTimer);
  if (clockTimer !== null) window.clearInterval(clockTimer);
});
</script>

<template>
  <RouterView v-if="isLoginPage" />

  <div v-else class="app-shell">
    <!-- 顶部状态栏 -->
    <header class="app-header">
      <div class="brand">
        <div class="brand-logo">
          <span class="logo-icon">R</span>
        </div>
        <div class="brand-info">
          <h1 class="brand-title">ReactorOS</h1>
          <span class="brand-subtitle">{{ tr("星宿智能反应釜控制系统", "Xingshu Smart Reactor Control") }}</span>
        </div>
      </div>

      <div class="topbar-center">
        <div class="status-pill" :class="live.liveStatus === 'fresh' ? 'ok' : 'bad'">
          <span class="status-light" :class="live.liveStatus === 'fresh' ? 'ok' : 'error'"></span>
          <span class="status-text">{{ live.liveStatus === "fresh" ? tr("实时数据", "Live data") : tr("数据中断", "Data interrupted") }}</span>
        </div>
        <div class="status-pill" :class="alarmTone">
          <span class="status-label">{{ tr("报警", "Alarms") }}</span>
          <span class="status-value">{{ alarmCounts.high }}/{{ alarmCounts.warning }}/{{ alarmCounts.info }}</span>
        </div>
        <div class="status-pill" :class="safetyState.tone">
          <span class="status-label">{{ tr("联锁", "Safety") }}</span>
          <span class="status-value">{{ safetyState.label }}</span>
        </div>
        <div v-if="activeBatchId !== null" class="status-pill ok">
          <span class="status-label">{{ tr("批次", "Batch") }}</span>
          <span class="status-value mono">#{{ activeBatchId }}</span>
        </div>
      </div>

      <div class="topbar-right">
        <div class="clock mono">{{ clockText }}</div>
        <el-segmented
          :model-value="language"
          size="small"
          :options="[
            { label: '中', value: 'zh' },
            { label: 'EN', value: 'en' }
          ]"
          @update:model-value="(value) => setLanguage(value as 'zh' | 'en')"
        />
        <div class="user-profile" v-if="auth.isAuthenticated">
          <div class="user-info">
            <span class="user-name">{{ auth.user?.username }}</span>
            <span class="user-role">{{ roleLabel }}</span>
          </div>
          <div class="user-avatar">
            {{ auth.user?.username?.charAt(0).toUpperCase() }}
          </div>
          <el-button size="small" class="logout-btn" @click="handleLogout">{{ tr("退出", "Logout") }}</el-button>
        </div>
        <el-button v-else size="small" type="primary" class="login-btn" @click="router.push('/login')">
          {{ tr("登录系统", "Sign in") }}
        </el-button>
      </div>
    </header>

    <!-- 侧边导航 -->
    <aside class="app-sidebar">
      <nav class="side-nav">
        <RouterLink
          v-for="item in navItems"
          :key="item.path"
          :to="item.path"
          class="nav-item"
          :class="{ active: activePath === item.path }"
        >
          <div class="nav-icon">{{ item.meta?.icon }}</div>
          <div class="nav-content">
            <span class="nav-title">{{ tr(String(item.meta?.zh ?? item.path), String(item.meta?.en ?? item.path)) }}</span>
            <span class="nav-subtitle">{{ tr(String(item.meta?.subZh ?? ""), String(item.meta?.subEn ?? "")) }}</span>
          </div>
          <div class="nav-indicator"></div>
        </RouterLink>
      </nav>
      <div class="sidebar-footer">
        <div class="device-tag">
          <span class="device-label">{{ tr("当前设备", "Device") }}</span>
          <span class="device-id mono">{{ DEVICE_ID }}</span>
        </div>
      </div>
    </aside>

    <!-- 主内容区 -->
    <main class="app-main">
      <RouterView />
    </main>
  </div>
</template>

<style scoped>
/* 顶部状态栏 */
.brand {
  display: flex;
  align-items: center;
  gap: 16px;
  min-width: 280px;
}

.brand-logo {
  width: 40px;
  height: 40px;
  background: linear-gradient(135deg, var(--ind-blue), #1e40af);
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 0 20px rgba(41, 121, 255, 0.3);
}

.logo-icon {
  color: white;
  font-weight: bold;
  font-size: 20px;
  font-family: var(--font-data);
}

.brand-info {
  display: flex;
  flex-direction: column;
}

.brand-title {
  margin: 0;
  font-size: 20px;
  font-weight: 700;
  letter-spacing: 0.5px;
  background: linear-gradient(90deg, #fff, #94a3b8);
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
}

.brand-subtitle {
  font-size: 12px;
  color: var(--text-tertiary);
  letter-spacing: 1px;
}

.topbar-center {
  display: flex;
  align-items: center;
  gap: 12px;
  flex: 1;
  justify-content: center;
}

.status-pill {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 16px;
  border-radius: 20px;
  background: rgba(255,255,255,0.03);
  border: 1px solid var(--border-glass);
  font-size: 13px;
  font-weight: 600;
}

.status-pill.ok { border-color: rgba(0, 200, 83, 0.3); color: var(--ind-green); }
.status-pill.warn { border-color: rgba(255, 171, 0, 0.3); color: var(--ind-amber); }
.status-pill.bad { border-color: rgba(255, 61, 0, 0.3); color: var(--ind-red); }

.status-label { color: var(--text-tertiary); font-weight: 400; }
.status-value { font-family: var(--font-data); }

.topbar-right {
  display: flex;
  align-items: center;
  gap: 20px;
}

.clock {
  font-size: 18px;
  font-weight: 700;
  color: var(--text-secondary);
  letter-spacing: 1px;
}

.user-profile {
  display: flex;
  align-items: center;
  gap: 12px;
  padding-left: 20px;
  border-left: 1px solid var(--border-glass);
}

.user-info {
  display: flex;
  flex-direction: column;
  align-items: flex-end;
}

.user-name { font-size: 14px; font-weight: 600; }
.user-role { font-size: 12px; color: var(--text-tertiary); }

.user-avatar {
  width: 36px;
  height: 36px;
  background: var(--bg-inset);
  border: 1px solid var(--border-glass);
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-weight: bold;
  color: var(--ind-blue);
}

.logout-btn {
  background: transparent;
  border: 1px solid var(--border-glass);
  color: var(--text-secondary);
}
.logout-btn:hover {
  border-color: var(--ind-red);
  color: var(--ind-red);
}

/* 侧边导航 */
.side-nav {
  display: flex;
  flex-direction: column;
  gap: 8px;
  flex: 1;
}

.nav-item {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 14px 16px;
  border-radius: var(--radius-md);
  color: var(--text-secondary);
  text-decoration: none;
  position: relative;
  transition: all 0.2s;
  border: 1px solid transparent;
}

.nav-item:hover {
  background: rgba(255,255,255,0.03);
  color: var(--text-primary);
}

.nav-item.active {
  background: linear-gradient(90deg, rgba(41, 121, 255, 0.1), transparent);
  border-color: rgba(41, 121, 255, 0.2);
  color: var(--ind-blue);
}

.nav-icon {
  font-size: 20px;
  width: 24px;
  text-align: center;
  opacity: 0.8;
}

.nav-content {
  display: flex;
  flex-direction: column;
  flex: 1;
}

.nav-title {
  font-size: 15px;
  font-weight: 600;
  letter-spacing: 0.5px;
}

.nav-subtitle {
  font-size: 12px;
  color: var(--text-tertiary);
  margin-top: 2px;
}

.nav-indicator {
  width: 4px;
  height: 24px;
  border-radius: 2px;
  background: transparent;
  transition: all 0.2s;
}

.nav-item.active .nav-indicator {
  background: var(--ind-blue);
  box-shadow: 0 0 10px var(--ind-blue);
}

.sidebar-footer {
  margin-top: auto;
  padding-top: 20px;
  border-top: 1px solid var(--border-glass);
}

.device-tag {
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 12px;
  background: var(--bg-inset);
  border-radius: var(--radius-md);
  border: 1px solid var(--border-glass);
}

.device-label { font-size: 11px; color: var(--text-tertiary); text-transform: uppercase; letter-spacing: 1px; }
.device-id { font-size: 14px; font-weight: 700; color: var(--ind-blue); }

/* 响应式 */
@media (max-width: 1200px) {
  .brand { min-width: auto; }
  .brand-info { display: none; }
  .nav-content { display: none; }
  .nav-item { justify-content: center; padding: 16px; }
  .device-label { display: none; }
  .device-tag { padding: 8px; text-align: center; }
}
</style>
