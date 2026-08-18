<script setup lang="ts">
// 通用面板卡片：双语标题 + 右侧操作区插槽。
// 用法：<PanelCard en="Live Trend" zh="实时趋势">...</PanelCard>
withDefaults(
  defineProps<{
    en: string;
    zh?: string;
    icon?: string;
    flush?: boolean;
    scrollable?: boolean;
  }>(),
  { zh: "", icon: "", flush: false, scrollable: false }
);
</script>

<template>
  <section class="panel">
    <header class="panel-header">
      <h3 class="panel-title">
        {{ en }}<span v-if="zh" class="zh">{{ zh }}</span>
      </h3>
      <slot name="actions" />
    </header>
    <div class="panel-body" :class="{ flush, scrollable }">
      <slot />
    </div>
  </section>
</template>
