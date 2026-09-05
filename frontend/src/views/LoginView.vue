<script setup lang="ts">
// 登录页：全屏深海军蓝品牌布局（左品牌区 + 右登录面板）。
// 登录逻辑保持原骨架：auth.login 成功 → connectRealtimeSocket → redirect ?? /monitor。

import { computed, nextTick, onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { useAuthStore } from "../stores/auth";
import { useLiveStore } from "../stores/live";
import { useLanguage } from "../i18n";
import { authApi } from "../api";
import { errorMessage } from "../api/errors";
import type { PermissionRolesResponse } from "../api/types";
import AppIcon from "../components/AppIcon.vue";

const auth = useAuthStore();
const live = useLiveStore();
const route = useRoute();
const router = useRouter();
const { language, toggleLanguage, tr } = useLanguage();

const username = ref("");
const password = ref("");
const submitting = ref(false);
const loginError = ref("");
const passwordInputRef = ref<{ focus: () => void } | null>(null);

// 默认账户提示（后端 /api/permissions/roles，无需登录即可读）
const roleInfo = ref<PermissionRolesResponse | null>(null);

const defaultUsers = computed(() => {
  const list = roleInfo.value?.default_users;
  if (!Array.isArray(list)) return [];
  // 后端返回 { username, role } 对象（兼容纯字符串形式）
  return list
    .map((u) => (typeof u === "string" ? u : (u as { username?: unknown })?.username))
    .filter((name): name is string => typeof name === "string" && name.length > 0);
});

const ttlText = computed(() => {
  const v = roleInfo.value?.session_ttl_hours;
  return typeof v === "number" && Number.isFinite(v) ? v + " h" : "--";
});

async function loadRoles(): Promise<void> {
  try {
    roleInfo.value = await authApi.roles();
  } catch {
    roleInfo.value = null; // 拉取失败时显示 "--"，不阻断登录
  }
}

function fillUser(name: string): void {
  username.value = name;
  // 切换账户时清空密码框，避免残留上一个账户的旧密码串号
  password.value = "";
  void nextTick(() => passwordInputRef.value?.focus());
}

async function submit(): Promise<void> {
  if (!username.value.trim() || !password.value) {
    loginError.value = tr("请输入用户名和密码", "Enter username and password");
    return;
  }
  submitting.value = true;
  loginError.value = "";
  try {
    await auth.login(username.value, password.value);
    live.connectRealtimeSocket();
    ElMessage.success(tr("登录成功", "Signed in"));
    const redirect = typeof route.query.redirect === "string" ? route.query.redirect : "/monitor";
    await router.push(redirect);
  } catch (error) {
    // V20 修复：内联错误条是唯一失败反馈（不再叠加顶部 toast 重复提示）
    loginError.value = errorMessage(error, tr("登录失败", "Sign in failed"));
  } finally {
    submitting.value = false;
  }
}

onMounted(() => {
  void loadRoles();
  if (auth.isAuthenticated) {
    void router.replace("/monitor");
  }
});
</script>

<template>
  <div class="login-page">
    <!-- 背景：径向渐变 + 网格线 + 光晕（纯 CSS 装饰层） -->
    <div class="bg-decor" aria-hidden="true">
      <div class="bg-grid"></div>
      <div class="bg-glow g1"></div>
      <div class="bg-glow g2"></div>
      <div class="bg-glow g3"></div>
      <span class="bg-mark m1">+</span>
      <span class="bg-mark m2">+</span>
      <span class="bg-mark m3">+</span>
    </div>

    <!-- 右上角语言切换 -->
    <button
      class="lang-toggle"
      :title="tr('切换语言', 'Switch language')"
      type="button"
      @click="toggleLanguage()"
    >
      {{ language === "zh" ? "EN" : "中" }}
    </button>

    <!-- 主体：左品牌 + 右登录卡片 -->
    <div class="login-stage">
      <!-- 品牌区 -->
      <section class="brand-pane">
        <div class="hex-wrap">
          <span class="hex-ring"></span>
          <svg viewBox="0 0 32 32" width="92" height="92" fill="none" class="hex-logo">
            <path d="M16 2l12 7v14l-12 7-12-7V9z" stroke="#57b4ff" stroke-width="1.6" />
            <path
              d="M16 9l6 3.5v7L16 23l-6-3.5v-7z"
              fill="rgba(47,155,255,0.35)"
              stroke="#2f9bff"
              stroke-width="1.2"
            />
            <circle cx="16" cy="16" r="1.6" fill="#38c8f2" />
          </svg>
        </div>

        <h1 class="brand-name">ReactorOS</h1>
        <p class="brand-en">Smart Reactor Control</p>
        <p class="brand-zh">{{ tr("星宿智能反应釜上位机", "Xingshu Smart Reactor HMI") }}</p>

        <p class="brand-tagline">
          {{ tr(
            "可审计 · 可解释 · 可离线运行的反应釜边缘控制层",
            "Auditable · explainable · offline-first edge control for reactors"
          ) }}
        </p>

        <ul class="brand-chips">
          <li><AppIcon name="gauge" :size="13" /><span>Edge Control {{ tr("边缘控制", "Edge") }}</span></li>
          <li><AppIcon name="shield" :size="13" /><span>Safety Interlock {{ tr("安全联锁", "Interlock") }}</span></li>
          <li><AppIcon name="audit" :size="13" /><span>Batch Audit {{ tr("批次审计", "Audit") }}</span></li>
        </ul>
      </section>

      <!-- 登录卡片列 -->
      <section class="login-col">
        <div class="login-card">
          <header class="card-head">
            <h2 class="card-title">Sign In <span class="zh">{{ tr("用户登录", "Sign In") }}</span></h2>
            <p class="card-sub">
              {{ tr("认证后进入运行监控工作台", "Authenticate to open the operations workspace") }}
            </p>
          </header>

          <el-alert
            v-if="auth.sessionExpired"
            type="warning"
            :title="tr('会话已过期，请重新登录', 'Session expired. Please sign in again.')"
            :closable="false"
            show-icon
            class="card-alert"
          />

          <div v-if="loginError" class="form-error">
            <AppIcon name="alarm" :size="14" />
            <span>{{ loginError }}</span>
          </div>

          <form class="login-form" @submit.prevent="submit">
            <label class="field">
              <span class="field-label">
                <span class="en">Username</span>
                <span class="zh">{{ tr("用户名", "Username") }}</span>
              </span>
              <el-input
                v-model="username"
                size="large"
                autocomplete="username"
                :placeholder="tr('输入用户名', 'Enter username')"
              >
                <template #prefix>
                  <span class="input-ic"><AppIcon name="operator" :size="15" /></span>
                </template>
              </el-input>
            </label>

            <label class="field">
              <span class="field-label">
                <span class="en">Password</span>
                <span class="zh">{{ tr("密码", "Password") }}</span>
              </span>
              <el-input
                ref="passwordInputRef"
                v-model="password"
                size="large"
                type="password"
                show-password
                autocomplete="current-password"
                :placeholder="tr('请输入密码', 'Enter password')"
                @keyup.enter="submit"
              >
                <template #prefix>
                  <span class="input-ic">
                    <svg
                      width="15"
                      height="15"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="1.7"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                    >
                      <rect x="5" y="11" width="14" height="9" rx="2" />
                      <path d="M8 11V7a4 4 0 0 1 8 0v4" />
                      <path d="M12 15v2" />
                    </svg>
                  </span>
                </template>
              </el-input>
            </label>

            <el-button
              type="primary"
              size="large"
              native-type="submit"
              :loading="submitting"
              class="login-submit"
            >
              {{ tr("登录", "Sign In") }}
            </el-button>
          </form>
        </div>

        <!-- 默认账户提示（后端真实数据；不可用时显示 "--"） -->
        <div class="roles-hint">
          <div class="hint-head">
            <span class="hint-en">Default Accounts</span>
            <span class="hint-zh">{{ tr("默认账户", "Default Accounts") }}</span>
            <span class="hint-ttl">
              {{ tr("会话有效期", "Session TTL") }} <span class="mono">{{ ttlText }}</span>
            </span>
          </div>
          <div class="hint-body">
            <template v-if="defaultUsers.length">
              <button
                v-for="user in defaultUsers"
                :key="user"
                type="button"
                class="user-chip mono"
                :title="tr('点击填入用户名', 'Click to fill username')"
                @click="fillUser(user)"
              >
                {{ user }}
              </button>
            </template>
            <span v-else class="mono hint-empty">--</span>
          </div>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
/* ---------- 页面骨架：满屏、禁止滚动 ---------- */
.login-page {
  position: relative;
  height: 100vh;
  height: 100dvh;
  overflow: hidden;
  display: grid;
  place-items: center;
  padding: var(--spacing);
  background:
    radial-gradient(1100px 620px at 16% -10%, rgba(47, 155, 255, 0.13), transparent 55%),
    radial-gradient(900px 560px at 90% 110%, rgba(56, 200, 242, 0.09), transparent 60%),
    radial-gradient(720px 520px at 50% 55%, rgba(176, 104, 240, 0.05), transparent 65%),
    var(--bg-app);
}

/* ---------- 背景装饰层 ---------- */
.bg-decor {
  position: absolute;
  inset: 0;
  pointer-events: none;
  overflow: hidden;
}

.bg-grid {
  position: absolute;
  inset: 0;
  background-image:
    linear-gradient(rgba(74, 127, 184, 0.055) 1px, transparent 1px),
    linear-gradient(90deg, rgba(74, 127, 184, 0.055) 1px, transparent 1px);
  background-size: 44px 44px;
  -webkit-mask-image: radial-gradient(ellipse 80% 70% at 50% 45%, #000 25%, transparent 78%);
  mask-image: radial-gradient(ellipse 80% 70% at 50% 45%, #000 25%, transparent 78%);
}

.bg-glow {
  position: absolute;
  border-radius: 50%;
  filter: blur(90px);
}
.bg-glow.g1 {
  width: 460px;
  height: 460px;
  left: -120px;
  top: -140px;
  background: rgba(47, 155, 255, 0.16);
  animation: breathe 9s ease-in-out infinite;
}
.bg-glow.g2 {
  width: 380px;
  height: 380px;
  right: -100px;
  bottom: -130px;
  background: rgba(56, 200, 242, 0.12);
  animation: breathe 11s ease-in-out infinite reverse;
}
.bg-glow.g3 {
  width: 300px;
  height: 300px;
  left: 42%;
  bottom: -160px;
  background: rgba(176, 104, 240, 0.08);
}

.bg-mark {
  position: absolute;
  font-family: var(--font-data);
  font-size: var(--fs-sm);
  color: rgba(74, 127, 184, 0.35);
}
.bg-mark.m1 { left: 18%; top: 24%; }
.bg-mark.m2 { right: 14%; top: 18%; }
.bg-mark.m3 { left: 30%; bottom: 16%; }

@keyframes breathe {
  0%, 100% { opacity: 0.75; transform: scale(1); }
  50% { opacity: 1; transform: scale(1.08); }
}

/* ---------- 语言切换 ---------- */
.lang-toggle {
  position: absolute;
  top: calc(var(--spacing) + 2px);
  right: calc(var(--spacing) + 2px);
  z-index: 3;
  width: 34px;
  height: 34px;
  border-radius: var(--radius-md);
  border: 1px solid var(--border-glass);
  background: var(--bg-panel);
  color: var(--text-secondary);
  font-size: var(--fs-sm);
  font-weight: 700;
  cursor: pointer;
  transition: border-color 0.15s, color 0.15s;
}
.lang-toggle:hover {
  border-color: var(--accent);
  color: var(--accent-strong);
}

/* ---------- 主体布局 ---------- */
.login-stage {
  position: relative;
  z-index: 2;
  display: grid;
  grid-template-columns: auto auto;
  align-items: center;
  gap: clamp(40px, 6vw, 96px);
  max-width: 1060px;
  width: 100%;
  justify-content: center;
  animation: rise 0.5s ease-out both;
}

@keyframes rise {
  from { opacity: 0; transform: translateY(14px); }
  to { opacity: 1; transform: translateY(0); }
}

/* ---------- 品牌区 ---------- */
.brand-pane {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  min-width: 0;
}

.hex-wrap {
  position: relative;
  width: 116px;
  height: 116px;
  display: grid;
  place-items: center;
  margin-bottom: calc(var(--spacing) + 6px);
}
.hex-logo {
  filter: drop-shadow(0 0 18px rgba(47, 155, 255, 0.45));
  animation: pulse-hex 5s ease-in-out infinite;
}
.hex-ring {
  position: absolute;
  inset: 0;
  border-radius: 50%;
  border: 1px dashed rgba(87, 180, 255, 0.28);
  animation: spin 26s linear infinite;
}
.hex-ring::after {
  content: "";
  position: absolute;
  top: -3px;
  left: 50%;
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--accent-cyan);
  box-shadow: 0 0 8px var(--accent-cyan);
}

@keyframes spin { to { transform: rotate(360deg); } }
@keyframes pulse-hex {
  0%, 100% { filter: drop-shadow(0 0 14px rgba(47, 155, 255, 0.35)); }
  50% { filter: drop-shadow(0 0 24px rgba(47, 155, 255, 0.6)); }
}

.brand-name {
  margin: 0;
  font-size: 44px;
  line-height: 1.05;
  font-weight: 700;
  letter-spacing: 0.5px;
  color: var(--text-primary);
}
.brand-en {
  margin: 6px 0 0;
  font-size: var(--fs-lg);
  font-weight: 600;
  letter-spacing: 2.5px;
  text-transform: uppercase;
  color: var(--accent-strong);
}
.brand-zh {
  margin: 4px 0 0;
  font-size: var(--fs-md);
  color: var(--text-secondary);
  letter-spacing: 1px;
}
.brand-tagline {
  margin: calc(var(--spacing) + 2px) 0 0;
  max-width: 380px;
  font-size: var(--fs-base);
  line-height: 1.7;
  color: var(--text-tertiary);
}

.brand-chips {
  list-style: none;
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin: calc(var(--spacing) + 4px) 0 0;
  padding: 0;
}
.brand-chips li {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 11px;
  border-radius: 999px;
  border: 1px solid var(--border-glass);
  background: var(--bg-panel);
  color: var(--text-secondary);
  font-size: var(--fs-xs);
  letter-spacing: 0.3px;
}
.brand-chips li :deep(svg) {
  color: var(--accent);
  flex: none;
}

/* ---------- 登录卡片 ---------- */
.login-col {
  display: flex;
  flex-direction: column;
  gap: var(--spacing);
  width: min(400px, 100%);
  min-width: 0;
}

.login-card {
  background: var(--bg-panel);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-panel);
  padding: calc(var(--spacing) + 8px);
  backdrop-filter: blur(8px);
}

.card-head {
  margin-bottom: calc(var(--spacing) + 2px);
}
.card-title {
  margin: 0;
  font-size: var(--fs-xl);
  font-weight: 700;
  color: var(--text-primary);
  letter-spacing: 0.4px;
  display: flex;
  align-items: baseline;
  gap: 8px;
}
.card-title .zh {
  font-size: var(--fs-base);
  font-weight: 500;
  color: var(--text-tertiary);
}
.card-sub {
  margin: 5px 0 0;
  font-size: var(--fs-sm);
  color: var(--text-tertiary);
}

.card-alert {
  margin-bottom: var(--spacing);
}

.form-error {
  display: flex;
  align-items: center;
  gap: 7px;
  margin-bottom: var(--spacing);
  padding: 8px 11px;
  border-radius: var(--radius-md);
  border: 1px solid rgba(255, 82, 82, 0.35);
  background: rgba(255, 82, 82, 0.09);
  color: var(--ind-red);
  font-size: var(--fs-sm);
  line-height: 1.4;
}
.form-error :deep(svg) { flex: none; }

.login-form {
  display: flex;
  flex-direction: column;
  gap: calc(var(--spacing) - 2px);
}

.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.field-label {
  display: flex;
  align-items: baseline;
  gap: 7px;
  font-size: var(--fs-sm);
}
.field-label .en {
  color: var(--text-secondary);
  font-weight: 600;
  letter-spacing: 0.4px;
}
.field-label .zh {
  color: var(--text-tertiary);
  font-size: var(--fs-xs);
}

.input-ic {
  display: inline-flex;
  align-items: center;
  color: var(--text-tertiary);
}

.login-submit {
  width: 100%;
  margin-top: 4px;
  font-weight: 700;
  letter-spacing: 1px;
  background: linear-gradient(135deg, var(--accent), var(--accent-cyan));
  border: none;
  box-shadow: 0 4px 16px rgba(47, 155, 255, 0.32);
}
.login-submit:hover,
.login-submit:focus {
  background: linear-gradient(135deg, var(--accent-strong), var(--accent-cyan));
  box-shadow: 0 6px 20px rgba(47, 155, 255, 0.45);
}

/* ---------- 默认账户提示 ---------- */
.roles-hint {
  background: var(--bg-inset);
  border: 1px solid var(--border-glass);
  border-radius: var(--radius-md);
  padding: 10px 14px;
}
.hint-head {
  display: flex;
  align-items: baseline;
  gap: 8px;
  flex-wrap: wrap;
  margin-bottom: 8px;
}
.hint-en {
  font-size: var(--fs-sm);
  font-weight: 600;
  color: var(--text-secondary);
  letter-spacing: 0.4px;
}
.hint-zh {
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
}
.hint-ttl {
  margin-left: auto;
  font-size: var(--fs-xs);
  color: var(--text-tertiary);
}
.hint-ttl .mono {
  color: var(--accent-strong);
  font-weight: 600;
}
.hint-body {
  display: flex;
  flex-wrap: wrap;
  gap: 7px;
  align-items: center;
}
.user-chip {
  padding: 4px 12px;
  border-radius: 999px;
  border: 1px solid var(--border-glass);
  background: var(--bg-panel-raised);
  color: var(--accent-strong);
  font-size: var(--fs-sm);
  cursor: pointer;
  transition: border-color 0.15s, background 0.15s;
}
.user-chip:hover {
  border-color: var(--accent);
  background: var(--bg-active);
}
.hint-empty {
  color: var(--text-tertiary);
  font-size: var(--fs-base);
}

/* ---------- 响应式：窄屏收敛为单卡片 ---------- */
@media (max-width: 960px) {
  .login-stage {
    grid-template-columns: 1fr;
    gap: var(--spacing);
    justify-content: stretch;
    align-content: center;
    overflow: hidden;
  }
  .brand-pane {
    display: none;
  }
  .login-col {
    width: min(400px, 100%);
    margin: 0 auto;
  }
}
</style>
