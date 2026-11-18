<script setup lang="ts">
/**
 * 弹层基座（v0.27 工作台重构）：所有辅助功能共用一套焦点/关闭/滚动语义。
 *
 * 约定：
 * - `open` 由 URL query 驱动（`useModalRoute`），本组件不做路由；
 * - Esc / 遮罩点击 / 右上角关闭钮都会 emit close；
 * - 打开时把焦点移入卡片并做 Tab 循环，关闭后归还触发元素；
 * - 只锁 `documentElement` 的滚动，不动页面布局。
 */
import { nextTick, onBeforeUnmount, ref, watch } from "vue";
import FluentIcon from "../FluentIcon.vue";

const props = withDefaults(
  defineProps<{
    open: boolean;
    title: string;
    wide?: boolean;
    narrow?: boolean;
  }>(),
  { wide: false, narrow: false },
);

const emit = defineEmits<{ close: [] }>();

const card = ref<HTMLElement | null>(null);
let lastFocused: HTMLElement | null = null;

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

function onKeydown(e: KeyboardEvent) {
  if (!props.open) return;
  if (e.key === "Escape") {
    e.preventDefault();
    emit("close");
    return;
  }
  if (e.key !== "Tab" || !card.value) return;
  const nodes = [...card.value.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
    (el) => el.offsetParent !== null,
  );
  if (!nodes.length) return;
  const first = nodes[0];
  const last = nodes[nodes.length - 1];
  const active = document.activeElement as HTMLElement | null;
  if (!e.shiftKey && active === last) {
    e.preventDefault();
    first.focus();
  } else if (e.shiftKey && (active === first || !card.value.contains(active))) {
    e.preventDefault();
    last.focus();
  }
}

watch(
  () => props.open,
  async (open) => {
    if (typeof document === "undefined") return;
    if (open) {
      lastFocused = document.activeElement as HTMLElement | null;
      document.addEventListener("keydown", onKeydown);
      document.documentElement.style.overflow = "hidden";
      await nextTick();
      card.value?.focus();
    } else {
      document.removeEventListener("keydown", onKeydown);
      document.documentElement.style.overflow = "";
      lastFocused?.focus?.();
      lastFocused = null;
    }
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  if (typeof document === "undefined") return;
  document.removeEventListener("keydown", onKeydown);
  document.documentElement.style.overflow = "";
});

function onMaskDown(e: MouseEvent) {
  if (e.target === e.currentTarget) emit("close");
}
</script>

<template>
  <Teleport to="body">
    <div v-if="open" class="modal-mask" @mousedown="onMaskDown">
      <div
        ref="card"
        class="modal-card"
        :class="{ 'modal-card--wide': wide, 'modal-card--narrow': narrow }"
        role="dialog"
        aria-modal="true"
        aria-labelledby="modal-title"
        tabindex="-1"
      >
        <header class="modal-head">
          <h2 id="modal-title" class="modal-title">{{ title }}</h2>
          <button class="modal-close" type="button" aria-label="关闭" @click="emit('close')">
            <FluentIcon name="close" :size="16" />
          </button>
        </header>
        <div class="modal-body">
          <slot />
        </div>
        <footer v-if="$slots.foot" class="modal-foot">
          <slot name="foot" />
        </footer>
      </div>
    </div>
  </Teleport>
</template>
