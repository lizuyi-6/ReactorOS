<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useRoute } from "vue-router";
import { routes } from "./router";
import { hmiNavItems, useAppShellState } from "./app-shell";
import { HMI_REFRESH_INTERVAL_MS, usePlantStore } from "./stores/plant";

const store = usePlantStore();
const route = useRoute();
let refreshTimer: number | null = null;
let clockTimer: number | null = null;
const now = ref(new Date());

const navItems = routes.filter((item) => item.path !== "/" && item.meta);
const activePath = computed(() => route.path);
const hmiScreenPage = ref(0);
const hmiPageCounts: Record<string, number> = {
  "/control": 4,
  "/ai": 2,
  "/history": 3,
  "/audit": 2,
  "/modbus": 4,
  "/settings": 7
};
const {
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
  productionLineStatusType,
  productionLineText,
  safetyStatusType,
  safetySummaryText,
  scenarioStatusType,
  scenarioText,
  sessionRoleLabel
} = useAppShellState(store, activePath, now);
const hmiPageCount = computed(() => hmiPageCounts[activePath.value] ?? 1);
const hmiPageButtons = computed(() => Array.from({ length: hmiPageCount.value }, (_, index) => index));
const hmiContentClasses = computed(() => ({
  ...contentClasses.value,
  [`hmi-page-${hmiScreenPage.value}`]: hmiPageCount.value > 1
}));

function setHmiScreenPage(page: number): void {
  hmiScreenPage.value = Math.min(Math.max(page, 0), hmiPageCount.value - 1);
}

function routeText(item: (typeof navItems)[number], zhKey: "zh" | "subZh", enKey: "en" | "subEn"): string {
  const meta = item.meta as Record<string, unknown> | undefined;
  return store.tr(String(meta?.[zhKey] ?? meta?.label ?? item.path), String(meta?.[enKey] ?? meta?.label ?? item.path));
}

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
  }, HMI_REFRESH_INTERVAL_MS);
  clockTimer = window.setInterval(() => {
    now.value = new Date();
  }, 1000);
});

onBeforeUnmount(() => {
  store.disconnectRealtimeSocket();
  if (refreshTimer !== null) window.clearInterval(refreshTimer);
  if (clockTimer !== null) window.clearInterval(clockTimer);
});

watch(activePath, () => {
  hmiScreenPage.value = 0;
});

watch(hmiPageCount, (count) => {
  if (hmiScreenPage.value >= count) hmiScreenPage.value = 0;
});
</script>

<template>
  <el-container class="app-shell monitor-route">
    <el-header class="topbar" :class="{ 'has-hmi-pager': hmiPageCount > 1 }" height="48px">
      <div class="brand-line">
        <strong>ReactorOS</strong>
        <span class="top-divider"></span>
        <span class="batch-label">{{ batchLabel }}</span>
        <small>ReactorOS HMI</small>
      </div>

      <div class="status-cluster">
        <el-tag :type="alarmStatusType">{{ alarmSummaryText }}</el-tag>
        <el-tag :type="scenarioStatusType">{{ scenarioText }}</el-tag>
        <el-tag :type="productionLineStatusType">{{ productionLineText }}</el-tag>
        <el-tag :type="store.liveStatus === 'fresh' ? 'success' : 'danger'">{{ liveStatusText }}</el-tag>
        <el-tag :type="safetyStatusType">{{ safetySummaryText }}</el-tag>
        <el-tag :type="commandStatusType">{{ commandReceiptText }}</el-tag>
      </div>

      <div v-if="hmiPageCount > 1" class="hmi-screen-pager" aria-label="Fixed HMI screen pages">
        <button
          v-for="page in hmiPageButtons"
          :key="page"
          type="button"
          class="hmi-page-button"
          :class="{ active: hmiScreenPage === page }"
          @click="setHmiScreenPage(page)"
        >
          P{{ page + 1 }}
        </button>
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
      <RouterLink
        v-for="item in hmiNavItems"
        :key="item.path"
        :to="item.path"
        class="legacy-nav-item"
        :class="{ active: activePath === item.path }"
      >
        <span class="legacy-nav-icon">{{ item.icon }}</span>
        <span>{{ item.label }}</span>
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

    <el-main class="content" :class="hmiContentClasses">
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
