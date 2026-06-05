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
const lastUpdatedText = computed(() => store.lastUpdated ?? "--");

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
          <small>{{ store.tr("PRD Vue 技术栈", "PRD Vue Stack") }}</small>
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
          <span>{{ store.tr(String(item.meta?.zh ?? item.meta?.label), String(item.meta?.en ?? item.meta?.label)) }}</span>
          <small>{{ store.tr(String(item.meta?.en ?? item.meta?.label), String(item.meta?.zh ?? item.meta?.label)) }}</small>
        </RouterLink>
      </nav>

      <div class="auth-panel">
        <span class="panel-label">{{ store.tr("会话", "Session") }}</span>
        <strong>{{ store.user?.username ?? store.tr("未登录", "not signed in") }}</strong>
        <small>{{ store.tr(`${store.user?.permissions?.length ?? 0} 项权限`, `${store.user?.permissions?.length ?? 0} permissions`) }}</small>
        <div class="role-buttons">
          <el-button size="small" @click="login('operator')">{{ store.tr("操作员", "Operator") }}</el-button>
          <el-button size="small" @click="login('engineer')">{{ store.tr("工程师", "Engineer") }}</el-button>
          <el-button size="small" type="danger" @click="login('admin')">{{ store.tr("管理员", "Admin") }}</el-button>
        </div>
        <el-button v-if="store.isAuthenticated" size="small" plain @click="store.logout()">{{ store.tr("退出登录", "Sign out") }}</el-button>
      </div>
    </el-aside>

    <el-container>
      <el-header class="topbar" height="72px">
        <div>
          <strong>{{ store.tr("星宿智能反应釜上位机", "Xingshu Intelligent Reactor HMI") }}</strong>
          <small>{{ store.tr("Vue 3 / Element Plus / ECharts / Pinia 迁移分支", "Vue 3 / Element Plus / ECharts / Pinia migration branch") }}</small>
        </div>
        <div class="topbar-actions">
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
          <span class="muted">{{ store.tr("更新于", "Updated") }} {{ lastUpdatedText }}</span>
          <el-button :loading="store.loading" @click="store.refreshAll()">{{ store.tr("刷新", "Refresh") }}</el-button>
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
