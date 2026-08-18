<script setup lang="ts">
// 轻量内联 SVG 图标库（stroke 风格，24 视窗）。
// 用法：<AppIcon name="monitor" :size="18" />
const props = withDefaults(defineProps<{ name: string; size?: number }>(), { size: 18 });

const paths: Record<string, string> = {
  // 监控：四宫格
  monitor:
    '<rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/>',
  // 控制：滑杆
  control:
    '<line x1="4" y1="7" x2="20" y2="7"/><circle cx="9" cy="7" r="2.2" fill="currentColor" stroke="none"/><line x1="4" y1="12" x2="20" y2="12"/><circle cx="15" cy="12" r="2.2" fill="currentColor" stroke="none"/><line x1="4" y1="17" x2="20" y2="17"/><circle cx="7" cy="17" r="2.2" fill="currentColor" stroke="none"/>',
  // AI 决策：芯片
  ai: '<rect x="6" y="6" width="12" height="12" rx="2"/><rect x="10" y="10" width="4" height="4" rx="1"/><path d="M9 2v2M15 2v2M9 20v2M15 20v2M2 9h2M2 15h2M20 9h2M20 15h2"/>',
  // 历史：时钟文档
  history: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3.5 2"/>',
  // 审计：盾牌勾
  audit: '<path d="M12 3l7 3v5c0 4.5-3 8.5-7 10-4-1.5-7-5.5-7-10V6z"/><path d="M9 12l2.2 2.2L15.5 9.5"/>',
  // Modbus：链路
  modbus:
    '<rect x="3" y="8" width="6" height="8" rx="1.5"/><rect x="15" y="8" width="6" height="8" rx="1.5"/><path d="M9 12h6M12 9v6"/>',
  // 设置：齿轮
  settings:
    '<circle cx="12" cy="12" r="3.2"/><path d="M19.4 15a1.7 1.7 0 0 0 .34 1.87l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.7 1.7 0 0 0-1.87-.34 1.7 1.7 0 0 0-1 1.55V21a2 2 0 1 1-4 0v-.09a1.7 1.7 0 0 0-1-1.55 1.7 1.7 0 0 0-1.87.34l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.7 1.7 0 0 0 .34-1.87 1.7 1.7 0 0 0-1.55-1H3a2 2 0 1 1 0-4h.09a1.7 1.7 0 0 0 1.55-1 1.7 1.7 0 0 0-.34-1.87l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.7 1.7 0 0 0 1.87.34h.01a1.7 1.7 0 0 0 1-1.55V3a2 2 0 1 1 4 0v.09a1.7 1.7 0 0 0 1 1.55h.01a1.7 1.7 0 0 0 1.87-.34l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.7 1.7 0 0 0-.34 1.87v.01a1.7 1.7 0 0 0 1.55 1H21a2 2 0 1 1 0 4h-.09a1.7 1.7 0 0 0-1.55 1z"/>',
  // 其他常用
  live: '<path d="M2 12h4l3-8 4 16 3-8h6"/>',
  alarm: '<path d="M12 3l10 17H2z"/><path d="M12 10v4M12 17.5v.01"/>',
  shield: '<path d="M12 3l7 3v5c0 4.5-3 8.5-7 10-4-1.5-7-5.5-7-10V6z"/><path d="M9 12l2.2 2.2L15.5 9.5"/>',
  batch: '<path d="M6 3h12M8 3v5l-4 9a3 3 0 0 0 2.7 4h10.6A3 3 0 0 0 20 17l-4-9V3"/><path d="M8.5 14h7"/>',
  operator: '<circle cx="12" cy="8" r="4"/><path d="M4 21c0-4 3.6-6.5 8-6.5s8 2.5 8 6.5"/>',
  clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3.5 2"/>',
  export: '<path d="M12 3v12M7 10l5 5 5-5"/><path d="M4 19h16"/>',
  report: '<path d="M7 3h8l4 4v14H7z"/><path d="M15 3v4h4M10 12h6M10 16h6"/>',
  search: '<circle cx="11" cy="11" r="7"/><path d="M21 21l-4.3-4.3"/>',
  reset: '<path d="M3 12a9 9 0 1 0 2.6-6.3M3 4v5h5"/>',
  play: '<path d="M7 4l13 8-13 8z"/>',
  pause: '<rect x="6" y="4" width="4" height="16" rx="1"/><rect x="14" y="4" width="4" height="16" rx="1"/>',
  stop: '<rect x="5" y="5" width="14" height="14" rx="2"/>',
  check: '<path d="M4 12.5l5 5L20 6.5"/>',
  flask: '<path d="M9 3h6M10 3v6l-5 9a2.5 2.5 0 0 0 2.2 3.6h9.6A2.5 2.5 0 0 0 19 18l-5-9V3"/><path d="M7.5 15h9"/>',
  valve: '<path d="M4 8v8l6-4zM20 8v8l-6-4zM10 12h4M12 12V6M9 6h6"/>',
  heater: '<path d="M6 20V10a6 6 0 0 1 12 0v10"/><path d="M6 16h12M9 6.5v3M15 6.5v3"/>',
  motor: '<circle cx="12" cy="12" r="8"/><path d="M12 4v3M12 17v3M4 12h3M17 12h3M6.3 6.3l2.2 2.2M15.5 15.5l2.2 2.2M17.7 6.3l-2.2 2.2M8.5 15.5l-2.2 2.2"/>',
  gauge: '<path d="M4 14a8 8 0 1 1 16 0"/><path d="M12 14l4-5"/><path d="M4 18h16"/>'
};
</script>

<template>
  <svg
    :width="props.size"
    :height="props.size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.7"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    v-html="paths[props.name] ?? paths.monitor"
  ></svg>
</template>
