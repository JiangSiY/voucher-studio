/**
 * 响应式断点（2026-11-18 UI 重设计 P0）。
 *
 * 三档：
 * - `compact`：< 768px，手机。侧栏隐藏，改底部 5 项 tab bar；编辑器整页化。
 * - `medium`：768–1099px，平板/窄窗。保留侧栏（收窄成图标条），列表 + 编辑器仍左右分栏。
 * - `wide`：≥ 1100px，桌面。完整侧栏 + 双栏。
 *
 * 为什么单独抽出来：`base.css` 里原本有 1260/1100/1024/900/760 五个断点，散在各页；
 * 组件要读同一套档位（不只是 CSS 媒体查询）时，需要一个唯一真源。
 * 非浏览器环境（vitest 的 node 环境、预渲染）没有 `matchMedia`，直接按 `wide` 处理 ——
 * 单测默认看到桌面 DOM，不会因为「手机壳没渲染」而误红。
 */
import { computed, onScopeDispose, readonly, ref } from "vue";

export type Breakpoint = "compact" | "medium" | "wide";

/** 与 base.css 的媒体查询保持一致（改这里必须同步样式表） */
export const COMPACT_MAX = 767;
export const MEDIUM_MAX = 1099;

const COMPACT_QUERY = `(max-width: ${COMPACT_MAX}px)`;
const MEDIUM_QUERY = `(max-width: ${MEDIUM_MAX}px)`;

function hasMedia(): boolean {
  return typeof window !== "undefined" && typeof window.matchMedia === "function";
}

function detect(): Breakpoint {
  if (!hasMedia()) return "wide";
  if (window.matchMedia(COMPACT_QUERY).matches) return "compact";
  if (window.matchMedia(MEDIUM_QUERY).matches) return "medium";
  return "wide";
}

export function useBreakpoint() {
  const breakpoint = ref<Breakpoint>(detect());

  let compactQuery: MediaQueryList | null = null;
  let mediumQuery: MediaQueryList | null = null;

  const onChange = () => {
    breakpoint.value = detect();
  };

  if (hasMedia()) {
    compactQuery = window.matchMedia(COMPACT_QUERY);
    mediumQuery = window.matchMedia(MEDIUM_QUERY);
    // 同时监听两个查询：窗口从 1440 拖到 800、再拖到 390，两个断点各自触发
    compactQuery.addEventListener("change", onChange);
    mediumQuery.addEventListener("change", onChange);
  }

  onScopeDispose(() => {
    compactQuery?.removeEventListener("change", onChange);
    mediumQuery?.removeEventListener("change", onChange);
  });

  return {
    breakpoint: readonly(breakpoint),
    isCompact: computed(() => breakpoint.value === "compact"),
    isMedium: computed(() => breakpoint.value === "medium"),
    isWide: computed(() => breakpoint.value === "wide"),
  };
}
