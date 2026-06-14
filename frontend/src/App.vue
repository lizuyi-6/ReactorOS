<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useRoute } from "vue-router";
import { routes } from "./router";
import { usePlantStore } from "./stores/plant";

const store = usePlantStore();
const route = useRoute();
let refreshTimer: number | null = null;
let clockTimer: number | null = null;
const now = ref(new Date());

const navItems = routes.filter((item) => item.path !== "/" && item.meta);
const healthStatus = computed(() => String(store.health?.status ?? store.health?.service ?? "unknown"));
const activePath = computed(() => route.path);
const lastUpdatedText = computed(() => store.lastUpdated ?? "--");
const runtime = computed(() => {
  const value = store.live?.runtime;
  return value && typeof value === "object" ? (value as Record<string, unknown>) : store.runtimeFallback;
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

function routeText(item: (typeof navItems)[number], zhKey: "zh" | "subZh", enKey: "en" | "subEn"): string {
  const meta = item.meta as Record<string, unknown> | undefined;
  return store.tr(String(meta?.[zhKey] ?? meta?.label ?? item.path), String(meta?.[enKey] ?? meta?.label ?? item.path));
}

// Display the logged-in user's ROLE rather than a raw permission-count: the
// backend login response does not always populate a permissions array, so
// "0 项权限" was shown to a logged-in engineer — misleading. The role label is
// the meaningful, always-available signal of what the session can do.
const roleLabels: Record<string, { zh: string; en: string }> = {
  operator: { zh: "操作员", en: "Operator" },
  engineer: { zh: "工程师", en: "Engineer" },
  admin: { zh: "管理员", en: "Administrator" }
};
const sessionRoleLabel = computed(() => {
  const role = store.user?.role;
  if (!role) return store.tr("未登录", "not signed in");
  const label = roleLabels[role] ?? { zh: role, en: role };
  return store.tr(label.zh, label.en);
});

async function login(role: string): Promise<void> {
  try {
    await store.login(role);
  } catch (error) {
    store.error = error instanceof Error ? error.message : String(error);
  }
}

onMounted(() => {
  void store.refreshAll();
  // Realtime: prefer the backend WebSocket push (~1 Hz). The 5 s interval below
  // remains as a transport-agnostic fallback (covers protected/public refresh
  // and keeps the UI alive if WS is blocked by a proxy).
  store.connectRealtimeSocket("reactor_001");
  refreshTimer = window.setInterval(() => {
    void store.refreshAll();
  }, 5000);
  clockTimer = window.setInterval(() => {
    now.value = new Date();
  }, 1000);
});

onBeforeUnmount(() => {
  store.disconnectRealtimeSocket();
  if (refreshTimer !== null) window.clearInterval(refreshTimer);
  if (clockTimer !== null) window.clearInterval(clockTimer);
});
</script>

<template>
  <el-container class="app-shell" :class="{ 'monitor-route': activePath === '/monitor' }">
    <el-header class="topbar" height="48px">
      <div class="brand-line">
        <strong>ReactorOS</strong>
        <span class="top-divider"></span>
        <span class="batch-label">{{ batchLabel }}</span>
        <small>ReactorOS HMI</small>
      </div>

      <div class="status-cluster">
        <el-tag type="warning">{{ store.tr("系统待机", "System standby") }}</el-tag>
        <el-tag :type="store.liveStatus === 'fresh' ? 'success' : 'danger'">{{ liveStatusText }}</el-tag>
        <el-tag type="primary">AI {{ store.tr("引擎就绪", "engine ready") }}</el-tag>
      </div>

      <div class="topbar-clock">
        <span class="runtime-clock">{{ clockText }}</span>
        <span class="muted">{{ lastUpdatedText }}</span>
      </div>

      <button class="legacy-estop" type="button" @click="store.triggerEmergencyStop()">
        E-STOP
      </button>
    </el-header>

    <aside class="legacy-sidebar" aria-label="Legacy HMI navigation">
      <RouterLink to="/monitor" class="legacy-nav-item active">
        <span class="legacy-nav-icon">▦</span>
        <span>Monitor</span>
      </RouterLink>
      <RouterLink to="/history" class="legacy-nav-item">
        <span class="legacy-nav-icon">▤</span>
        <span>Batches</span>
      </RouterLink>
      <RouterLink to="/control" class="legacy-nav-item">
        <span class="legacy-nav-icon">≋</span>
        <span>Control</span>
      </RouterLink>
      <RouterLink to="/ai" class="legacy-nav-item">
        <span class="legacy-nav-icon">AI</span>
        <span>AI Lab</span>
      </RouterLink>
      <RouterLink to="/audit" class="legacy-nav-item">
        <span class="legacy-nav-icon">↺</span>
        <span>History</span>
      </RouterLink>
      <RouterLink to="/settings" class="legacy-nav-item">
        <span class="legacy-nav-icon">⚠</span>
        <span>Alarms</span>
      </RouterLink>
      <RouterLink to="/settings" class="legacy-nav-item">
        <span class="legacy-nav-icon">⚙</span>
        <span>Settings</span>
      </RouterLink>
    </aside>

    <div class="utility-strip">
      <nav class="nav-list">
        <RouterLink
          v-for="item in navItems"
          :key="item.path"
          :to="item.path"
          class="nav-link"
          :class="{ active: activePath === item.path }"
        >
          <span>{{ routeText(item, "zh", "en") }}</span>
          <small>{{ routeText(item, "subZh", "subEn") }}</small>
        </RouterLink>
      </nav>

      <div class="auth-panel">
        <span class="session-name">{{ store.user?.username ?? store.tr("未登录", "not signed in") }}</span>
        <span class="muted">{{ sessionRoleLabel }}</span>
        <div class="role-buttons">
          <el-button size="small" @click="login('operator')">{{ store.tr("操作员", "Operator") }}</el-button>
          <el-button size="small" @click="login('engineer')">{{ store.tr("工程师", "Engineer") }}</el-button>
          <el-button size="small" type="danger" @click="login('admin')">{{ store.tr("管理员", "Admin") }}</el-button>
        </div>
        <el-segmented
          :model-value="store.language"
          size="small"
          :options="[
            { label: '中文', value: 'zh' },
            { label: 'EN', value: 'en' }
          ]"
          @update:model-value="(value) => store.setLanguage(value as 'zh' | 'en')"
        />
        <el-tag :type="healthStatus === 'healthy' || healthStatus === 'ok' ? 'success' : 'warning'">
          {{ healthStatus }}
        </el-tag>
        <el-tag :type="store.realtimeConnected ? 'success' : 'info'" size="small">
          WS {{ store.realtimeConnected ? store.tr("实时", "live") : store.tr("轮询", "poll") }}
        </el-tag>
        <el-button size="small" :loading="store.loading" @click="store.refreshAll()">{{ store.tr("刷新", "Refresh") }}</el-button>
        <el-button v-if="store.isAuthenticated" size="small" plain @click="store.logout()">{{ store.tr("退出", "Sign out") }}</el-button>
      </div>
    </div>

    <el-main class="content">
      <el-alert
        v-if="store.error"
        class="error-alert"
        type="warning"
        :title="store.error"
        show-icon
        :closable="false"
      />
      <RouterView />
    </el-main>
  </el-container>
</template>
