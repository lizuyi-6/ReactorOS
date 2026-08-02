<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { ElMessage } from "element-plus";
import { useAuthStore } from "../stores/auth";
import { useLiveStore } from "../stores/live";
import { useLanguage } from "../i18n";
import { errorMessage } from "../api/errors";

const auth = useAuthStore();
const live = useLiveStore();
const route = useRoute();
const router = useRouter();
const { tr } = useLanguage();

const username = ref("");
const password = ref("");
const submitting = ref(false);

async function submit(): Promise<void> {
  if (!username.value.trim() || !password.value) {
    ElMessage.warning(tr("请输入用户名和密码", "Enter username and password"));
    return;
  }
  submitting.value = true;
  try {
    await auth.login(username.value, password.value);
    live.connectRealtimeSocket();
    ElMessage.success(tr("登录成功", "Signed in"));
    const redirect = typeof route.query.redirect === "string" ? route.query.redirect : "/monitor";
    await router.push(redirect);
  } catch (error) {
    ElMessage.error(errorMessage(error, tr("登录失败", "Sign in failed")));
  } finally {
    submitting.value = false;
  }
}

onMounted(() => {
  if (auth.isAuthenticated) {
    void router.replace("/monitor");
  }
});
</script>

<template>
  <div class="login-page">
    <div class="login-card">
      <div class="login-brand">
        <span class="login-mark">R</span>
        <div>
          <h1>ReactorOS</h1>
          <p class="muted">{{ tr("星宿智能反应釜 · 边缘上位机", "Xingshu Reactor · Edge HMI") }}</p>
        </div>
      </div>

      <el-alert
        v-if="auth.sessionExpired"
        type="warning"
        :title="tr('会话已过期，请重新登录', 'Session expired. Please sign in again.')"
        :closable="false"
        show-icon
        class="login-alert"
      />

      <form class="login-form" @submit.prevent="submit">
        <label class="login-field">
          <span>{{ tr("用户名", "Username") }}</span>
          <el-input v-model="username" size="large" autocomplete="username" :placeholder="tr('operator / engineer / admin', 'operator / engineer / admin')" />
        </label>
        <label class="login-field">
          <span>{{ tr("密码", "Password") }}</span>
          <el-input
            v-model="password"
            size="large"
            type="password"
            show-password
            autocomplete="current-password"
            :placeholder="tr('请输入密码', 'Enter password')"
            @keyup.enter="submit"
          />
        </label>
        <el-button type="primary" size="large" native-type="submit" :loading="submitting" class="login-submit">
          {{ tr("登录", "Sign in") }}
        </el-button>
      </form>

      <p class="login-hint muted">
        {{ tr("会话有效期 12 小时。权限由后端按角色控制（operator / engineer / admin）。", "Sessions last 12 hours. Permissions are enforced by the backend per role (operator / engineer / admin).") }}
      </p>
    </div>
  </div>
</template>

<style scoped>
.login-page {
  min-height: 100vh;
  display: grid;
  place-items: center;
  padding: var(--space-4);
  background:
    radial-gradient(1200px 600px at 20% -10%, rgba(76, 174, 157, 0.08), transparent 60%),
    var(--bg-app);
}

.login-card {
  width: 100%;
  max-width: 400px;
  background: var(--bg-surface);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-lg);
  padding: var(--space-8);
  box-shadow: var(--shadow-md);
}

.login-brand {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  margin-bottom: var(--space-6);
}

.login-mark {
  display: grid;
  place-items: center;
  width: 44px;
  height: 44px;
  border-radius: var(--radius-md);
  background: var(--color-brand-subtle);
  color: var(--color-brand-strong);
  font-weight: 700;
  font-size: var(--text-lg);
  font-family: var(--font-data);
}

.login-brand h1 {
  font-size: var(--text-xl);
}

.login-brand p {
  font-size: var(--text-sm);
  margin-top: 2px;
}

.login-alert {
  margin-bottom: var(--space-4);
}

.login-form {
  display: flex;
  flex-direction: column;
  gap: var(--space-4);
}

.login-field {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}

.login-field span {
  font-size: var(--text-sm);
  color: var(--text-secondary);
}

.login-submit {
  width: 100%;
  margin-top: var(--space-2);
}

.login-hint {
  margin-top: var(--space-5);
  font-size: var(--text-xs);
  line-height: 1.6;
}
</style>
