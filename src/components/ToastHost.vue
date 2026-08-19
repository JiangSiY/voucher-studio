<script setup lang="ts">
import { useToastStore } from "../stores/toast";

const store = useToastStore();

const icons: Record<string, string> = {
  info: "ℹ",
  success: "✓",
  warning: "⚠",
  danger: "✕",
};
</script>

<template>
  <div class="toast-host" aria-live="polite">
    <div v-for="t in store.items" :key="t.id" class="toast-item" :class="t.kind" role="status">
      <span class="t-icon">{{ icons[t.kind] }}</span>
      <span style="flex: 1">{{ t.text }}</span>
      <button class="btn-icon" style="width: 20px; height: 20px; font-size: 11px" aria-label="关闭"
        @click="store.dismiss(t.id)">✕</button>
    </div>
  </div>
</template>
