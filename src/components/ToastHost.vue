<script setup lang="ts">
import { useToastStore } from "../stores/toast";
import FluentIcon from "./FluentIcon.vue";

const store = useToastStore();

const icons: Record<string, string> = {
  info: "info",
  success: "checkCircle",
  warning: "warning",
  danger: "close",
};
</script>

<template>
  <div class="toast-host" aria-live="polite">
    <div v-for="t in store.items" :key="t.id" class="toast-item" :class="t.kind" role="status">
      <span class="t-icon"><FluentIcon :name="icons[t.kind]" :size="16" /></span>
      <span style="flex: 1">{{ t.text }}</span>
      <button class="btn-icon" style="width: 20px; height: 20px" aria-label="关闭"
        @click="store.dismiss(t.id)"><FluentIcon name="close" :size="11" /></button>
    </div>
  </div>
</template>
