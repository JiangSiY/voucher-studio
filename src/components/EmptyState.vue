<script setup lang="ts">
import FluentIcon from "./FluentIcon.vue";

defineProps<{ icon?: string; title: string; desc?: string }>();
</script>

<template>
  <div class="empty-state">
    <div class="es-icon"><FluentIcon :name="icon ?? 'folder'" :size="42" /></div>
    <div class="es-title">{{ title }}</div>
    <div v-if="desc" class="es-desc">{{ desc }}</div>
    <slot />
  </div>
</template>
