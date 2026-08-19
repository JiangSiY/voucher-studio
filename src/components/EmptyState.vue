<script setup lang="ts">
defineProps<{ icon?: string; title: string; desc?: string }>();
</script>

<template>
  <div class="empty-state">
    <div class="es-icon">{{ icon ?? "🗂" }}</div>
    <div class="es-title">{{ title }}</div>
    <div v-if="desc" class="es-desc">{{ desc }}</div>
    <slot />
  </div>
</template>
