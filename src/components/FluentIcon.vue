<script setup lang="ts">
/**
 * FluentIcon —— 手写 Fluent System Icons 风格 SVG 图标组件（不依赖第三方库）
 *
 * 规范：24×24 网格 / 1.5px 描边 / 圆角线帽线 join / currentColor 继承文字色，
 * 自动适配主题（侧栏激活态、hover、禁用态等均由 CSS 颜色继承完成）。
 * 用法：<FluentIcon name="search" :size="16" />
 */
import { computed } from "vue";

const props = withDefaults(defineProps<{ name: string; size?: number }>(), {
  size: 16,
});

/** 图标 path 字典（name → svg inner 标记；静态内置内容，不含用户输入） */
const ICONS: Record<string, string> = {
  // ---- 导航 ----
  dashboard:
    '<rect x="3.5" y="3.5" width="7" height="7" rx="1.5"/><rect x="13.5" y="3.5" width="7" height="7" rx="1.5"/><rect x="3.5" y="13.5" width="7" height="7" rx="1.5"/><rect x="13.5" y="13.5" width="7" height="7" rx="1.5"/>',
  search: '<circle cx="10.5" cy="10.5" r="6"/><path d="M15 15l5 5"/>',
  list: '<path d="M4.5 6.5h15M4.5 12h15M4.5 17.5h9"/>',
  print:
    '<path d="M7.25 8V5.5A1.5 1.5 0 0 1 8.75 4h6.5A1.5 1.5 0 0 1 16.75 5.5V8"/><path d="M7.25 16.5H5.5A1.5 1.5 0 0 1 4 15V10a1.5 1.5 0 0 1 1.5-1.5h13A1.5 1.5 0 0 1 20 10v5a1.5 1.5 0 0 1-1.5 1.5h-1.75"/><rect x="7.25" y="13.5" width="9.5" height="6.5" rx="1"/>',
  chart: '<path d="M4 20h16M7 20v-5.5M12 20V9M17 20v-8"/>',
  settings:
    '<circle cx="12" cy="12" r="8.25"/><circle cx="12" cy="12" r="3.25"/><path d="M12 1.75v2M12 20.25v2M1.75 12h2M20.25 12h2M17.83 6.17l1.42-1.42M6.17 17.83l-1.42 1.42M6.17 6.17L4.75 4.75M17.83 17.83l1.42 1.42"/>',
  save: '<path d="M5 4.5A1.5 1.5 0 0 0 3.5 6v12.5A1.5 1.5 0 0 0 5 20h14a1.5 1.5 0 0 0 1.5-1.5V7.75L16.75 4.5H5z"/><path d="M8.5 4.5v5h7v-5"/><rect x="8" y="13" width="8" height="7" rx="1"/>',
  help: '<circle cx="12" cy="12" r="8.5"/><path d="M9.6 9.6a2.5 2.5 0 1 1 3.55 2.27c-.68.33-1.15.83-1.15 1.6v.28"/><path d="M12 16.75v.01"/>',

  // ---- 空状态 / 文件 ----
  folder:
    '<path d="M3.5 6.5A1.5 1.5 0 0 1 5 5h4.25l2 2.5H19a1.5 1.5 0 0 1 1.5 1.5V17a1.5 1.5 0 0 1-1.5 1.5H5A1.5 1.5 0 0 1 3.5 17V6.5z"/>',
  image:
    '<rect x="3.5" y="4.5" width="17" height="15" rx="1.5"/><circle cx="9" cy="9.75" r="1.6"/><path d="M4.5 17.75l4.9-4.9a1 1 0 0 1 1.4 0l2.4 2.4 2.6-2.6a1 1 0 0 1 1.4 0l3.3 3.3"/>',
  file: '<path d="M13 3v4.5a1 1 0 0 0 1 1h4.5"/><path d="M6 2.5h7.2L19.5 8.8V20a1.5 1.5 0 0 1-1.5 1.5H6A1.5 1.5 0 0 1 4.5 20V4A1.5 1.5 0 0 1 6 2.5z"/>',

  // ---- 操作 ----
  upload:
    '<path d="M12 14.5V4M8 8l4-4 4 4"/><path d="M4.5 15.5V18A1.5 1.5 0 0 0 6 19.5h12a1.5 1.5 0 0 0 1.5-1.5v-2.5"/>',
  download:
    '<path d="M12 4v10.5M8 10.5l4 4 4-4"/><path d="M4.5 15.5V18A1.5 1.5 0 0 0 6 19.5h12a1.5 1.5 0 0 0 1.5-1.5v-2.5"/>',
  delete:
    '<path d="M4.5 6.5h15"/><path d="M9.5 6.5V5A1.5 1.5 0 0 1 11 3.5h2A1.5 1.5 0 0 1 14.5 5v1.5"/><path d="M6.5 6.5l.7 12.1A1.5 1.5 0 0 0 8.7 20h6.6a1.5 1.5 0 0 0 1.5-1.4l.7-12.1"/><path d="M10 10.5v5.5M14 10.5v5.5"/>',
  edit: '<path d="M4.5 19.5l.7-3.55L16.3 4.85a1.8 1.8 0 0 1 2.55 0l.3.3a1.8 1.8 0 0 1 0 2.55L8.05 18.8l-3.55.7z"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  minus: '<path d="M5 12h14"/>',
  check: '<path d="M5 12.5l4.5 4.5L19 7"/>',
  checkCircle: '<circle cx="12" cy="12" r="8.5"/><path d="M8.25 12.25l2.5 2.5 5-5.5"/>',
  close: '<path d="M5.5 5.5l13 13M18.5 5.5l-13 13"/>',
  undo: '<path d="M4.5 9.5h9.75a5.25 5.25 0 0 1 0 10.5H8.5"/><path d="M8.5 5.5l-4 4 4 4"/>',
  refresh: '<path d="M19.5 12a7.5 7.5 0 1 1-2.2-5.3"/><path d="M19.5 4v4.5H15"/>',
  rotate: '<path d="M4.5 12a7.5 7.5 0 1 0 2.2-5.3"/><path d="M4.5 4v4.5H9"/>',
  compare: '<path d="M8 8.5h12M17 5.5l3 3-3 3"/><path d="M16 15.5H4M7 12.5l-3 3 3 3"/>',
  flash: '<path d="M13.5 2.5L5.5 13.5h5.25L9.5 21.5l8-11h-5.25l1.25-8z"/>',
  sparkle:
    '<path d="M12 3.5c.6 3.2 2.8 5.4 6 6-3.2.6-5.4 2.8-6 6-.6-3.2-2.8-5.4-6-6 3.2-.6 5.4-2.8 6-6z"/><path d="M18.6 15.6c.3 1.6 1.4 2.7 3 3-1.6.3-2.7 1.4-3 3-.3-1.6-1.4-2.7-3-3 1.6-.3 2.7-1.4 3-3z"/>',
  flask:
    '<path d="M9.5 3.5h5M10.5 3.5V9l-4.86 8.6A2 2 0 0 0 7.4 20.5h9.2a2 2 0 0 0 1.76-2.9L13.5 9V3.5"/><path d="M7.9 14.5h8.2"/>',
  bulb: '<path d="M12 3.5a6 6 0 0 1 3.6 10.8c-.65.46-1.1 1.13-1.1 1.94v.76h-5v-.76c0-.81-.45-1.48-1.1-1.94A6 6 0 0 1 12 3.5z"/><path d="M10 20.25h4"/>',

  // ---- 云 / 备份 ----
  cloud:
    '<path d="M7.2 18.5a4.2 4.2 0 0 1-.6-8.36 6 6 0 0 1 11.68 1.47A3.83 3.83 0 0 1 17.6 18.5H7.2z"/>',
  cloudUp:
    '<path d="M7.2 18.5a4.2 4.2 0 0 1-.6-8.36 6 6 0 0 1 11.68 1.47A3.83 3.83 0 0 1 17.6 18.5H7.2z"/><path d="M12 16v-4.5M10 13.5l2-2 2 2"/>',

  // ---- 安全 ----
  key: '<circle cx="7.5" cy="12" r="3.75"/><path d="M11.25 12h9M17.5 12v3M20.25 12v2.25"/>',
  lock: '<rect x="5" y="10.5" width="14" height="10" rx="1.5"/><path d="M8 10.5V7.5a4 4 0 0 1 8 0v3"/>',

  // ---- 状态 ----
  warning:
    '<path d="M12 4l8.66 14.5a1 1 0 0 1-.87 1.5H4.21a1 1 0 0 1-.87-1.5L12 4z"/><path d="M12 10v4.5M12 17.5v.01"/>',
  info: '<circle cx="12" cy="12" r="8.5"/><path d="M12 11v5.25M12 7.75v.01"/>',
  inbox:
    '<path d="M4.5 12.75h4l1 2h5l1-2h4"/><path d="M6 4.5h12l2 8.25v4.75a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 4 17.5v-4.75L6 4.5z"/>',
  duplicate:
    '<rect x="8.5" y="8.5" width="11.5" height="11.5" rx="1.5"/><path d="M15.5 8.5V5.5A1.5 1.5 0 0 0 14 4H5.5A1.5 1.5 0 0 0 4 5.5V14a1.5 1.5 0 0 0 1.5 1.5h3"/>',

  // ---- 查看 ----
  zoomIn: '<circle cx="10.5" cy="10.5" r="6"/><path d="M15 15l5 5M10.5 8.25v4.5M8.25 10.5h4.5"/>',
  zoomOut: '<circle cx="10.5" cy="10.5" r="6"/><path d="M15 15l5 5M8.25 10.5h4.5"/>',
  chevronLeft: '<path d="M14.5 5.5L8 12l6.5 6.5"/>',
  chevronRight: '<path d="M9.5 5.5L16 12l-6.5 6.5"/>',
  chevronDown: '<path d="M5.5 9.5L12 16l6.5-6.5"/>',
  // 侧栏开关（2026-10-16）：面板形图标，一眼看出"这是侧边栏"。
  // 原用 chevronLeft/chevronRight 两态对调，真机走查被当成"返回"按钮 → 换成语义图标 + 选中态提示开合状态。
  sidebar:
    '<rect x="3.5" y="4.5" width="17" height="15" rx="1.5"/><path d="M9.5 4.5v15"/><path d="M6.25 8.25h.01M6.25 12.25h.01"/>',
  pin: '<path d="M12 4a5.5 5.5 0 0 1 5.5 5.5c0 4-5.5 10-5.5 10S6.5 13.5 6.5 9.5A5.5 5.5 0 0 1 12 4z"/><circle cx="12" cy="9.5" r="1.8"/>',
};

const body = computed(() => ICONS[props.name] ?? "");
</script>

<template>
  <svg
    class="fluent-icon"
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.5"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    focusable="false"
    v-html="body"
  />
</template>

<style>
.fluent-icon {
  display: inline-block;
  vertical-align: -0.15em;
  flex-shrink: 0;
}
</style>
