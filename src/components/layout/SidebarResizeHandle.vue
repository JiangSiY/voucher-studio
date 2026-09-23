<script setup lang="ts">
/**
 * 侧边栏宽度拖拽手柄（ARIA window splitter 模式）。
 *
 * 定位：`.app-shell` 的绝对定位子元素——**不能放进 `.sidebar`**：侧栏自身
 * `overflow-y: auto`，手柄会随内容滚走。水平位置由 CSS 跟随 `--sidebar-w`
 * （含 ≤900 档的 `min(--sidebar-w, 35vw)` 钳制、≤760 档隐藏），因此这里不做 JS 定位。
 *
 * 交互：拖拽（pointer 事件 + rAF 节流 + 仅像素变化才写 CSS 变量）、双击复位、←/→ 步进。
 * 持久化：拖拽过程只改 CSS 变量，pointerup 时经 `configStore.saveSidebarWidth` 落盘一次
 * （浏览器预览模式下 config 为 null，store 内部只改界面不落盘）。
 */
import { onBeforeUnmount, ref } from "vue";
import { useConfigStore } from "../../stores/config";
import { toastError } from "../../stores/toast";
import {
  applySidebarWidth,
  clampSidebarWidth,
  steppedSidebarWidth,
  widthFromPointer,
} from "../../composables/useSidebarResize";
import {
  SIDEBAR_WIDTH_DEFAULT,
  SIDEBAR_WIDTH_MAX,
  SIDEBAR_WIDTH_MIN,
} from "../../config-defaults";

const store = useConfigStore();

/** 落盘宽度：失败必须提示，不能静默吞掉（2026-09-23 审计 B15）。 */
function persistWidth(w: number) {
  void store.saveSidebarWidth(w).catch((e) => toastError(e));
}

const dragging = ref(false);

/** 当前生效宽度：优先读**渲染后**的侧栏宽度（含 CSS 档位钳制），读不到再退回配置值 */
function effectiveWidth(): number {
  const rendered = document.querySelector(".sidebar")?.getBoundingClientRect().width;
  if (typeof rendered === "number" && rendered > 0) return rendered;
  return store.config?.appearance.sidebarWidth ?? SIDEBAR_WIDTH_DEFAULT;
}

function ariaNow(): number {
  return Math.round(effectiveWidth());
}

let startX = 0;
let startWidth = 0;
let pendingWidth = 0;
let rafId = 0;
let lastApplied = -1;

/** 写入 CSS 变量；像素值未变则跳过（拖拽高频路径） */
function apply(width: number) {
  const next = clampSidebarWidth(width);
  if (next === lastApplied) return;
  lastApplied = next;
  applySidebarWidth(next);
}

function onPointerDown(e: PointerEvent) {
  // macOS 的 Ctrl+左键是次要点击（等价右键），不得当作拖拽（2026-09-23 审计 B17）
  if (e.button !== 0 || e.ctrlKey) return;
  dragging.value = true;
  startX = e.clientX;
  startWidth = effectiveWidth();
  pendingWidth = startWidth;
  // jsdom 等环境无指针捕获，特性检测后再用（拖动本身不依赖捕获）
  (e.currentTarget as HTMLElement).setPointerCapture?.(e.pointerId);
  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", onPointerUp, { once: true });
  e.preventDefault();
}

function onPointerMove(e: PointerEvent) {
  pendingWidth = widthFromPointer(startWidth, e.clientX - startX);
  if (rafId) return; // 一帧最多写一次，避免全文档样式重算拖垮低配机
  rafId = requestAnimationFrame(() => {
    rafId = 0;
    apply(pendingWidth);
  });
}

function onPointerUp() {
  window.removeEventListener("pointermove", onPointerMove);
  if (rafId) {
    cancelAnimationFrame(rafId);
    rafId = 0;
  }
  dragging.value = false;
  const final = clampSidebarWidth(pendingWidth);
  apply(final);
  persistWidth(final); // 落盘一次
}

function resetToDefault() {
  applySidebarWidth(SIDEBAR_WIDTH_DEFAULT);
  persistWidth(SIDEBAR_WIDTH_DEFAULT);
}

function onKeydown(e: KeyboardEvent) {
  if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
  e.preventDefault();
  const next = steppedSidebarWidth(effectiveWidth(), e.key === "ArrowRight" ? "right" : "left");
  applySidebarWidth(next);
  persistWidth(next);
}

onBeforeUnmount(() => {
  window.removeEventListener("pointermove", onPointerMove);
  if (rafId) cancelAnimationFrame(rafId);
});
</script>

<template>
  <div
    class="sidebar-resize-handle"
    :class="{ 'is-dragging': dragging }"
    role="separator"
    aria-orientation="vertical"
    aria-label="调整侧边栏宽度"
    :aria-valuenow="ariaNow()"
    :aria-valuemin="SIDEBAR_WIDTH_MIN"
    :aria-valuemax="SIDEBAR_WIDTH_MAX"
    tabindex="0"
    title="拖动调整侧边栏宽度（双击复位，← → 微调）"
    @pointerdown="onPointerDown"
    @dblclick="resetToDefault"
    @keydown="onKeydown"
  ></div>
</template>
