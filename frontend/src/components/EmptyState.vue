<script setup lang="ts">
defineProps<{
  icon?: string;
  title: string;
  description?: string;
}>();
</script>

<template>
  <div class="empty-state">
    <span class="empty-icon">{{ icon ?? "◌" }}</span>
    <strong>{{ title }}</strong>
    <p v-if="description" class="muted">{{ description }}</p>
    <slot />
  </div>
</template>
