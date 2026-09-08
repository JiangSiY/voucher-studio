<script setup lang="ts">
import { useToastStore, type ToastItem } from "../stores/toast";
import FluentIcon from "./FluentIcon.vue";

const store = useToastStore();

const icons: Record<string, string> = {
  info: "info",
  success: "checkCircle",
  warning: "warning",
  danger: "close",
};

/** 执行动作并关闭（动作型 toast 用户已明确响应） */
function runAction(t: ToastItem) {
  t.action?.onClick();
  store.dismiss(t.id);
}
</script>

<template>
  <div class="toast-host" aria-live="polite">
    <div v-for="t in store.items" :key="t.id" class="toast-item" :class="t.kind" role="status">
      <span class="t-icon"><FluentIcon :name="icons[t.kind]" :size="16" /></span>
      <span style="flex: 1">{{ t.text }}</span>
      <button v-if="t.action" class="btn btn-sm toast-action" @click="runAction(t)">
        {{ t.action.label }}
      </button>
      <button class="btn-icon" style="width: 20px; height: 20px" aria-label="关闭"
        @click="store.dismiss(t.id)"><FluentIcon name="close" :size="11" /></button>
    </div>
  </div>
</template>
