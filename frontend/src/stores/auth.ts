// 认证 store：登录/登出/会话恢复/401 统一处理。

import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { authApi } from "../api";
import { setAuthToken, setUnauthorizedHandler } from "../api/http";
import type { AuthUser, Role } from "../api/types";

const TOKEN_KEY = "reactoros.vue.auth.token";
const USER_KEY = "reactoros.vue.auth.user";

function readStoredUser(): AuthUser | null {
  const raw = localStorage.getItem(USER_KEY);
  if (!raw) return null;
  try {
    return JSON.parse(raw) as AuthUser;
  } catch {
    return null;
  }
}

export const useAuthStore = defineStore("auth", () => {
  const token = ref<string | null>(localStorage.getItem(TOKEN_KEY));
  const user = ref<AuthUser | null>(readStoredUser());
  /** 会话因 401 失效（用于登录页提示"会话已过期"）。 */
  const sessionExpired = ref(false);

  const isAuthenticated = computed(() => Boolean(token.value && user.value));
  const role = computed<Role>(() => (user.value?.role as Role) ?? "guest");

  function applySession(nextToken: string, nextUser: AuthUser): void {
    token.value = nextToken;
    user.value = nextUser;
    localStorage.setItem(TOKEN_KEY, nextToken);
    localStorage.setItem(USER_KEY, JSON.stringify(nextUser));
    setAuthToken(nextToken);
    sessionExpired.value = false;
  }

  function clearSession(expired = false): void {
    token.value = null;
    user.value = null;
    localStorage.removeItem(TOKEN_KEY);
    localStorage.removeItem(USER_KEY);
    setAuthToken(null);
    sessionExpired.value = expired;
  }

  async function login(username: string, password: string): Promise<void> {
    const payload = await authApi.login(username.trim(), password);
    applySession(payload.token, payload.user);
  }

  function logout(): void {
    clearSession(false);
  }

  /** 启动时恢复会话：有本地 token 则用 /api/auth/me 验证，失败则清除。 */
  async function restoreSession(): Promise<void> {
    if (!token.value) return;
    setAuthToken(token.value);
    try {
      const me = await authApi.me();
      user.value = me;
      localStorage.setItem(USER_KEY, JSON.stringify(me));
    } catch {
      clearSession(true);
    }
  }

  // 注册全局 401 处理：任何请求返回 401 时自动登出。
  setUnauthorizedHandler(() => {
    if (token.value) clearSession(true);
  });

  function hasPermission(permission: string): boolean {
    return Boolean(user.value?.permissions?.includes(permission));
  }

  const isEngineerOrAdmin = computed(() => role.value === "engineer" || role.value === "admin");
  const isAdmin = computed(() => role.value === "admin");

  return {
    token,
    user,
    sessionExpired,
    isAuthenticated,
    role,
    isEngineerOrAdmin,
    isAdmin,
    hasPermission,
    login,
    logout,
    restoreSession,
    clearSession
  };
});
