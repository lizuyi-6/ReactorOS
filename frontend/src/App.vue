<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted } from "vue";
import { useRoute } from "vue-router";
import { routes } from "./router";
import { usePlantStore } from "./stores/plant";

const store = usePlantStore();
const route = useRoute();
let refreshTimer: number | null = null;

const navItems = routes.filter((item) => item.path !== "/" && item.meta);
const healthStatus = computed(() => String(store.health?.status ?? store.health?.service ?? "unknown"));
const activePath = computed(() => route.path);

async function login(role: string): Promise<void> {
  try {
    await store.login(role);
  } catch (error) {
    store.error = error instanceof Error ? error.message : String(error);
  }
}

onMounted(() => {
  void store.refreshAll();
  refreshTimer = window.setInterval(() => {
    void store.refreshAll();
  }, 5000);
});

onBeforeUnmount(() => {
  if (refreshTimer !== null) window.clearInterval(refreshTimer);
});
</script>

<template>
  <el-container class="app-shell">
    <el-aside class="sidebar" width="268px">
      <div class="brand">
        <span class="brand-mark">XS</span>
        <div>
          <strong>ReactorOS HMI</strong>
          <small>PRD Vue Stack</small>
        </div>
      </div>

      <nav class="nav-list">
        <RouterLink
          v-for="item in navItems"
          :key="item.path"
          :to="item.path"
          class="nav-link"
          :class="{ active: activePath === item.path }"
        >
          <span>{{ item.meta?.zh }}</span>
          <small>{{ item.meta?.label }}</small>
        </RouterLink>
      </nav>

      <div class="auth-panel">
        <span class="panel-label">Session</span>
        <strong>{{ store.user?.username ?? "not signed in" }}</strong>
        <small>{{ store.user?.permissions?.length ?? 0 }} permissions</small>
        <div class="role-buttons">
          <el-button size="small" @click="login('operator')">Operator</el-button>
          <el-button size="small" @click="login('engineer')">Engineer</el-button>
          <el-button size="small" type="danger" @click="login('admin')">Admin</el-button>
        </div>
        <el-button v-if="store.isAuthenticated" size="small" plain @click="store.logout()">Sign out</el-button>
      </div>
    </el-aside>

    <el-container>
      <el-header class="topbar" height="72px">
        <div>
          <strong>星宿智能反应釜上位机</strong>
          <small>Vue 3 / Element Plus / ECharts / Pinia migration branch</small>
        </div>
        <div class="topbar-actions">
          <el-tag :type="healthStatus === 'healthy' || healthStatus === 'ok' ? 'success' : 'warning'">
            {{ healthStatus }}
          </el-tag>
          <span class="muted">Updated {{ store.lastUpdated ?? "--" }}</span>
          <el-button :loading="store.loading" @click="store.refreshAll()">Refresh</el-button>
        </div>
      </el-header>

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
  </el-container>
</template>
