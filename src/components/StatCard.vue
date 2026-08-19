<script setup lang="ts">
defineProps<{ label: string; value: number | string; delta?: string }>();
</script>

<template>
  <div class="stat">
    <div class="label">{{ label }}</div>
    <div class="value">{{ value }}</div>
    <div v-if="delta" class="delta">{{ delta }}</div>
  </div>
</template>
