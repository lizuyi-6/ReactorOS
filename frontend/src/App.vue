<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { ElMessageBox } from "element-plus";
import { routes } from "./router";
import { useAuthStore } from "./stores/auth";
import { useLiveStore } from "./stores/live";
import { useLanguage } from "./i18n";
import { boolText } from "./utils/format";
import { DEVICE_ID } from "./api";
import AppIcon from "./components/AppIcon.vue";

const auth = useAuthStore();
const live = useLiveStore();
const route = useRoute();
const router = useRouter();
const { language, setLanguage, tr } = useLanguage();

const now = ref(new Date());
let refreshTimer: number | null = null;
let refreshScheduleToken = 0;
let schedulerStopped = false;
let clockTimer: number | null = null;

const navItems = computed(() => routes.filter((item) => item.path !== "/" && item.path !== "/login" && item.meta));
const activePath = computed(() => route.path);
const isLoginPage = computed(() => activePath.value === "/login");

const clockTime = computed(() =>
  now.value.toLocaleTimeString(language.value === "zh" ? "zh-CN" : "en-US", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false
  })
);
const clockDate = computed(() => {
  const locale = language.value === "zh" ? "zh-CN" : "en-US";
  const date = now.value.toLocaleDateString(locale, { year: "numeric", month: "2-digit", day: "2-digit" });
  const weekday = now.value.toLocaleDateString(locale, { weekday: "short" });
  return date + " " + weekday;
});

const runtime = computed(() => live.runtime);
const activeBatchId = computed(() => {
  const id = runtime.value?.active_batch_id;
  return id === null || id === undefined ? null : Number(id);
});
const activeBatchName = computed(() => {
  const batches = live.live?.recent_batches;
  const hit = Array.isArray(batches) ? batches.find((b) => b.id === activeBatchId.value) : null;
  return hit?.name ?? (activeBatchId.value !== null ? "#" + activeBatchId.value : "—");
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
const alarmTotal = computed(() => alarmCounts.value.high + alarmCounts.value.warning + alarmCounts.value.info);
const alarmTone = computed<"ok" | "warn" | "bad">(() => {
  if (alarmCounts.value.high > 0) return "bad";
  if (alarmCounts.value.warning > 0) return "warn";
  return "ok";
});

const safetyState = computed<{ tone: "ok" | "warn" | "bad"; label: string }>(() => {
  const rt = runtime.value;
  if (boolText(rt?.control_loop_terminated)) return { tone: "bad", label: tr("环终止", "LOOP STOP") };
  if (boolText(rt?.emergency_stop)) return { tone: "bad", label: tr("急停", "E-STOP") };
  if (rt?.last_sensor_error) return { tone: "bad", label: tr("传感器故障", "SENSOR") };
  if (rt?.last_control_error) return { tone: "warn", label: tr("控制故障", "CTRL FLT") };
  if (boolText(rt?.manual_lock)) return { tone: "warn", label: tr("人工锁定", "M-LOCK") };
  return { tone: "ok", label: "OK" };
});

const operatorName = computed(() => auth.user?.username ?? tr("未登录", "Guest"));
// V29 修复：角色徽章绑定真实登录角色（此前硬编码 Operator/操作员）
const roleLabel = computed(() => {
  const r = auth.role;
  if (r === "admin") return { zh: "管理员", en: "Admin" };
  if (r === "engineer") return { zh: "工程师", en: "Engineer" };
  if (r === "operator") return { zh: "操作员", en: "Operator" };
  return { zh: "访客", en: "Guest" };
});

async function handleLogout(): Promise<void> {
  try {
    await ElMessageBox.confirm(
      tr("确认退出登录？退出后需重新登录才能操作。", "Sign out? You will need to sign in again to operate."),
      tr("退出登录", "Sign Out"),
      {
        confirmButtonText: tr("退出", "Sign Out"),
        cancelButtonText: tr("取消", "Cancel"),
        type: "warning"
      }
    );
  } catch {
    return;
  }
  auth.logout();
  live.disconnectRealtimeSocket();
  await router.push("/login");
}

function canRefreshLive(): boolean {
  return auth.isAuthenticated && !isLoginPage.value && !document.hidden;
}

function stopRefreshTimer(): void {
  schedulerStopped = true;
  refreshScheduleToken += 1;
  if (refreshTimer !== null) {
    window.clearTimeout(refreshTimer);
    refreshTimer = null;
  }
}

function scheduleLiveRefresh(): void {
  stopRefreshTimer();
  if (!canRefreshLive()) return;
  schedulerStopped = false;
  const token = refreshScheduleToken;
  refreshTimer = window.setTimeout(() => {
    refreshTimer = null;
    if (token !== refreshScheduleToken || !canRefreshLive()) return;
    void refreshAndReschedule();
  }, live.nextRefreshDelayMs);
}

async function refreshAndReschedule(force = false): Promise<void> {
  if (!canRefreshLive()) {
    stopRefreshTimer();
    return;
  }
  schedulerStopped = false;
  if (force || live.nextRefreshDelayMs <= 0) await live.refreshLive();
  if (!schedulerStopped && canRefreshLive()) scheduleLiveRefresh();
}

function handleVisibilityChange(): void {
  if (document.hidden) stopRefreshTimer();
  else void refreshAndReschedule();
}

async function boot(): Promise<void> {
  live.bindTokenProvider(() => auth.token);
  await auth.restoreSession();
  if (!auth.isAuthenticated) return;
  // 启动时先完整读取 /api/live，再建立实时连接；连接态后续只做 60s 校准。
  await refreshAndReschedule(true);
  live.connectRealtimeSocket();
}

watch(
  () => [auth.isAuthenticated, isLoginPage.value, live.realtimeConnected] as const,
  ([authenticated, loginPage], previous) => {
    if (!authenticated || loginPage) {
      stopRefreshTimer();
      return;
    }
    const becameEligible = !previous || !previous[0] || previous[1];
    const connectionChanged = Boolean(previous && previous[2] !== live.realtimeConnected);
    if (becameEligible || connectionChanged) void refreshAndReschedule();
    else scheduleLiveRefresh();
  }
);

onMounted(() => {
  document.addEventListener("visibilitychange", handleVisibilityChange);
  void boot();
  clockTimer = window.setInterval(() => {
    now.value = new Date();
  }, 1000);
});

onBeforeUnmount(() => {
  document.removeEventListener("visibilitychange", handleVisibilityChange);
  live.disconnectRealtimeSocket();
  stopRefreshTimer();
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
          <svg viewBox="0 0 32 32" width="26" height="26" fill="none">
            <path d="M16 2l12 7v14l-12 7-12-7V9z" stroke="#57b4ff" stroke-width="2" />
            <path d="M16 9l6 3.5v7L16 23l-6-3.5v-7z" fill="rgba(47,155,255,0.35)" stroke="#2f9bff" stroke-width="1.5" />
          </svg>
        </div>
        <div class="brand-info">
          <h1 class="brand-title">ReactorOS</h1>
          <span class="brand-subtitle">Smart Reactor Control</span>
        </div>
      </div>

      <div class="topbar-cards">
        <div class="tb-card" :class="live.liveStatus === 'fresh' ? 'ok' : 'bad'">
          <span class="tb-icon"><AppIcon name="live" :size="17" /></span>
          <span class="tb-text">
            <span class="tb-label">Live Data</span>
            <span class="tb-sub">{{ tr("实时数据", "Real-time") }}</span>
          </span>
          <span v-if="live.liveStatus !== 'fresh'" class="tb-badge bad">!</span>
        </div>

        <div class="tb-card" :class="alarmTone">
          <span class="tb-icon"><AppIcon name="alarm" :size="17" /></span>
          <span class="tb-text">
            <span class="tb-label">Alarms</span>
            <span class="tb-sub">{{ tr("报警", "Alarms") }}</span>
          </span>
          <span v-if="alarmTotal > 0" class="tb-badge" :class="alarmTone">{{ alarmTotal }}</span>
        </div>

        <div class="tb-card" :class="safetyState.tone">
          <span class="tb-icon"><AppIcon name="shield" :size="17" /></span>
          <span class="tb-text">
            <span class="tb-label">Safety Interlock</span>
            <span class="tb-sub">{{ tr("安全联锁", "Interlock") }}</span>
          </span>
          <span class="tb-state" :class="safetyState.tone">{{ safetyState.label }}</span>
        </div>

        <div class="tb-card neutral">
          <span class="tb-text">
            <span class="tb-label">Batch ID</span>
            <span class="tb-sub">{{ tr("批次号", "Batch") }}</span>
          </span>
          <span class="tb-value mono">{{ activeBatchName }}</span>
        </div>

        <div class="tb-card neutral clickable" @click="auth.isAuthenticated ? handleLogout() : router.push('/login')">
          <span class="tb-text">
            <span class="tb-label">{{ roleLabel.en }}</span>
            <span class="tb-sub zh">{{ roleLabel.zh }}</span>
          </span>
          <span class="tb-value">{{ operatorName }}</span>
        </div>
      </div>

      <div class="topbar-right">
        <button class="lang-toggle" :title="tr('切换语言', 'Switch language')" @click="setLanguage(language === 'zh' ? 'en' : 'zh')">
          {{ language === "zh" ? "EN" : "中" }}
        </button>
        <div class="clock-box">
          <AppIcon name="clock" :size="22" />
          <div class="clock-text">
            <span class="clock-time mono">{{ clockTime }}</span>
            <span class="clock-date">{{ clockDate }}</span>
          </div>
        </div>
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
          <span class="nav-icon"><AppIcon :name="String(item.meta?.icon ?? 'monitor')" :size="20" /></span>
          <span class="nav-content">
            <span class="nav-title">{{ item.meta?.en }}</span>
            <span class="nav-subtitle zh">{{ item.meta?.zh }}</span>
          </span>
        </RouterLink>
      </nav>

      <div class="sidebar-footer">
        <div class="edge-info">
          <div class="edge-row">
            <span class="edge-label">Edge Node</span>
            <span class="edge-value mono">RX-EDGE-01</span>
          </div>
          <div class="edge-row">
            <span class="edge-label">Version</span>
            <span class="edge-value mono">v2.4.1</span>
          </div>
        </div>
        <div class="health-row" :class="live.liveStatus === 'fresh' ? 'ok' : 'bad'">
          <span class="status-dot" :class="live.liveStatus === 'fresh' ? 'ok' : 'bad'"></span>
          <span>{{ live.liveStatus === "fresh" ? "System Healthy" : tr("数据中断", "Data Lost") }}</span>
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
.brand {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 210px;
  flex: none;
}
.brand-logo {
  width: 38px;
  height: 38px;
  display: flex;
  align-items: center;
  justify-content: center;
  filter: drop-shadow(0 0 10px rgba(47, 155, 255, 0.35));
}
.brand-title {
  margin: 0;
  font-size: 19px;
  font-weight: 700;
  letter-spacing: 0.3px;
  color: var(--text-primary);
  line-height: 1.1;
}
.brand-subtitle {
  font-size: 11px;
  color: var(--text-tertiary);
  letter-spacing: 0.4px;
}

.topbar-cards {
  display: flex;
  align-items: center;
  gap: 10px;
  flex: 1;
  justify-content: center;
  min-width: 0;
  overflow: hidden;
}
.tb-card {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 6px 14px;
  border-radius: var(--radius-md);
  background: var(--bg-panel);
  border: 1px solid var(--border-glass);
  min-width: 0;
  white-space: nowrap;
}
.tb-card.clickable { cursor: pointer; }
.tb-card.clickable:hover { border-color: var(--border-strong); }
.tb-icon {
  width: 30px;
  height: 30px;
  border-radius: 7px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--accent-dim);
  color: var(--accent-strong);
  flex: none;
}
.tb-card.ok .tb-icon { background: rgba(47, 212, 123, 0.12); color: var(--ind-green); }
.tb-card.warn .tb-icon { background: rgba(245, 166, 35, 0.12); color: var(--ind-amber); }
.tb-card.bad .tb-icon { background: rgba(255, 82, 82, 0.12); color: var(--ind-red); }
.tb-text { display: flex; flex-direction: column; line-height: 1.25; }
.tb-label { font-size: 12px; font-weight: 600; color: var(--text-secondary); }
.tb-sub { font-size: 11px; color: var(--text-tertiary); }
.tb-badge {
  min-width: 20px;
  height: 20px;
  border-radius: 10px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 11px;
  font-weight: 700;
  padding: 0 5px;
  background: var(--ind-red);
  color: #fff;
}
.tb-badge.ok { background: var(--ind-green); color: #06130c; }
.tb-badge.warn { background: var(--ind-amber); color: #1a1206; }
.tb-state { font-size: 12px; font-weight: 700; }
.tb-state.ok { color: var(--ind-green); }
.tb-state.warn { color: var(--ind-amber); }
.tb-state.bad { color: var(--ind-red); }
.tb-value { font-size: 13px; font-weight: 600; color: var(--text-primary); }

.topbar-right {
  display: flex;
  align-items: center;
  gap: 14px;
  flex: none;
}
.lang-toggle {
  width: 30px;
  height: 30px;
  border-radius: 7px;
  border: 1px solid var(--border-glass);
  background: var(--bg-inset);
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 700;
  cursor: pointer;
}
.lang-toggle:hover { border-color: var(--accent); color: var(--accent); }
.clock-box {
  display: flex;
  align-items: center;
  gap: 10px;
  color: var(--text-secondary);
}
.clock-text { display: flex; flex-direction: column; line-height: 1.2; }
.clock-time { font-size: 18px; font-weight: 700; color: var(--text-primary); letter-spacing: 1px; }
.clock-date { font-size: 11px; color: var(--text-tertiary); }

.side-nav {
  display: flex;
  flex-direction: column;
  gap: 4px;
  flex: 1;
  min-height: 0;
  overflow-y: auto;
}
.nav-item {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 11px 12px;
  border-radius: var(--radius-md);
  color: var(--text-secondary);
  text-decoration: none;
  border: 1px solid transparent;
  transition: background 0.15s, color 0.15s;
}
.nav-item:hover { background: var(--bg-hover); color: var(--text-primary); }
.nav-item.active {
  background: linear-gradient(90deg, rgba(47, 155, 255, 0.18), rgba(47, 155, 255, 0.05));
  border-color: rgba(47, 155, 255, 0.35);
  color: var(--accent-strong);
}
.nav-icon { flex: none; display: flex; opacity: 0.9; }
.nav-content { display: flex; flex-direction: column; line-height: 1.3; min-width: 0; }
.nav-title { font-size: 14px; font-weight: 600; }
.nav-subtitle { font-size: 11px; color: var(--text-tertiary); }
.nav-item.active .nav-subtitle { color: rgba(87, 180, 255, 0.75); }

.sidebar-footer {
  flex: none;
  padding-top: 12px;
  border-top: 1px solid var(--border-glass);
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.edge-info { display: flex; flex-direction: column; gap: 5px; padding: 0 4px; }
.edge-row { display: flex; flex-direction: column; gap: 1px; }
.edge-label { font-size: 11px; color: var(--text-tertiary); }
.edge-value { font-size: 13px; font-weight: 600; color: var(--accent-strong); }
.health-row {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  font-weight: 600;
  padding: 8px 10px;
  border-radius: var(--radius-md);
  background: var(--bg-inset);
  border: 1px solid var(--border-glass);
}
.health-row.ok { color: var(--ind-green); }
.health-row.bad { color: var(--ind-red); }

@media (max-width: 1400px) {
  .tb-card { padding: 5px 10px; gap: 7px; }
  .tb-icon { width: 26px; height: 26px; }
}
@media (max-width: 1100px) {
  .brand-info { display: none; }
  .brand { min-width: auto; }
  .nav-content { display: none; }
  .nav-item { justify-content: center; padding: 13px; }
  .edge-row .edge-label { display: none; }
  /* V32：窄图标栏隐藏节点 ID/版本值（mono 长串溢出 68px 栏） */
  .edge-row .edge-value { display: none; }
  .health-row span:last-child { display: none; }
  .health-row { justify-content: center; }
}
@media (max-width: 900px) {
  .topbar-cards .tb-card:nth-child(n + 4) { display: none; }
  /* V32：药丸不再互相叠印——允许横向滚动而不是硬挤 */
  .topbar-cards { overflow-x: auto; justify-content: flex-start; scrollbar-width: none; }
  .tb-card { flex: none; padding: 4px 9px; }
  .tb-text .tb-sub { display: none; }
  .clock-box { flex: none; }
}
</style>