// 轻量双语（中/英）支持。沿用旧前端的 tr(zh, en) 模式，集中管理语言偏好。

import { computed, ref } from "vue";

export type UiLanguage = "zh" | "en";

const LANGUAGE_KEY = "reactoros.vue.language";

const language = ref<UiLanguage>(localStorage.getItem(LANGUAGE_KEY) === "en" ? "en" : "zh");

export function tr(zh: string, en: string): string {
  return language.value === "zh" ? zh : en;
}

export function useLanguage() {
  function setLanguage(next: UiLanguage): void {
    language.value = next;
    localStorage.setItem(LANGUAGE_KEY, next);
  }

  function toggleLanguage(): void {
    setLanguage(language.value === "zh" ? "en" : "zh");
  }

  return {
    language,
    isChinese: computed(() => language.value === "zh"),
    setLanguage,
    toggleLanguage,
    tr
  };
}
