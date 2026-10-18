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
    <!-- 危险提示必须是 assertive：polite 会等读屏把当前句子念完，
         而"删除失败/备份失败"这类信息晚几秒才知道就已误事。
         条目级 aria-live 覆盖容器的 polite（role="alert" 本身即断言式）。 -->
    <div
      v-for="t in store.items"
      :key="t.id"
      class="toast-item"
      :class="t.kind"
      :role="t.kind === 'danger' ? 'alert' : 'status'"
      :aria-live="t.kind === 'danger' ? 'assertive' : 'polite'"
    >
      <span class="t-icon"><FluentIcon :name="icons[t.kind]" :size="16" /></span>
      <span style="flex: 1">{{ t.text }}</span>
      <button v-if="t.action" class="btn btn-sm toast-action" @click="runAction(t)">
        {{ t.action.label }}
      </button>
      <button
        class="btn-icon"
        style="width: var(--control-h-xs); height: var(--control-h-xs)"
        aria-label="关闭"
        @click="store.dismiss(t.id)"
      >
        <FluentIcon name="close" :size="12" />
      </button>
    </div>
  </div>
</template>
