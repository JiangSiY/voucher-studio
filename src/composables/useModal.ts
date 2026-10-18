/**
 * 浮层焦点契约（`docs/UI-设计规范-macOS27.md` §4「焦点管理」）。
 *
 * 规范要求三条，2026-10-16 审查发现对话框一条都没做（`ConfirmDialog.vue` 打开后焦点仍留在
 * 背后的「删除」按钮上、Tab 会跑出浮层、Esc 关不掉；`CropDialog.vue` 同样）：
 *   1. 打开时聚焦浮层内首个可操作元素（可用 `data-autofocus` 指定）；
 *   2. `Tab` / `Shift+Tab` 循环留在浮层内；
 *   3. `Esc` 关闭，关闭后**把焦点归还给打开它的元素**。
 *
 * 用法（对话框组件）：
 * ```ts
 * const { panelEl } = useModal(() => props.visible, () => emit("cancel"));
 * ```
 * 面板根节点挂 `ref="panelEl"`；需要自定义首焦点时给某个按钮加 `data-autofocus`。
 * 组件自己已经处理 Esc 时传 `{ escape: false }`，避免两次关闭。
 */
import { nextTick, onBeforeUnmount, ref, watch } from "vue";

const FOCUSABLE = 'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])';

function focusablesIn(root: HTMLElement | null): HTMLElement[] {
  if (!root) return [];
  // 不按几何可见性过滤：jsdom 无排版，getClientRects()/offsetParent 恒为空，
  // 按它们过滤会在测试里把整个浮层判成"没有可聚焦元素"。只排除显式禁用/隐藏的。
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
    (el) =>
      !el.hasAttribute("disabled") &&
      el.getAttribute("aria-hidden") !== "true" &&
      !el.closest("[hidden]"),
  );
}

export function useModal(
  visible: () => boolean,
  onClose: () => void,
  opts: { escape?: boolean } = {},
) {
  const panelEl = ref<HTMLElement | null>(null);
  const escapeEnabled = opts.escape !== false;
  let lastFocused: HTMLElement | null = null;
  let listening = false;

  async function focusInto() {
    lastFocused = (document.activeElement as HTMLElement | null) ?? null;
    await nextTick();
    const list = focusablesIn(panelEl.value);
    const preferred = panelEl.value?.querySelector<HTMLElement>("[data-autofocus]");
    // 默认落在**首个**可操作元素：危险确认框的取消按钮排在前面 → 默认焦点是安全的那个
    (preferred ?? list[0] ?? panelEl.value)?.focus();
  }

  function restoreFocus() {
    const el = lastFocused;
    lastFocused = null;
    // 元素可能已随视图销毁（如删除后行消失），focus() 前确认还挂在文档上
    if (el && document.contains(el)) el.focus();
  }

  function onKeydown(e: KeyboardEvent) {
    if (escapeEnabled && e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onClose();
      return;
    }
    if (e.key !== "Tab") return;
    const list = focusablesIn(panelEl.value);
    if (!list.length) return;
    const first = list[0];
    const last = list[list.length - 1];
    const active = document.activeElement as HTMLElement | null;
    const inside = !!active && !!panelEl.value?.contains(active);
    if (e.shiftKey && (!inside || active === first)) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && (!inside || active === last)) {
      e.preventDefault();
      first.focus();
    }
  }

  function start() {
    if (listening) return;
    // 捕获阶段：面板内部若有组件自行 stopPropagation，Tab/Esc 依然有效
    document.addEventListener("keydown", onKeydown, true);
    listening = true;
    void focusInto();
  }

  function stop() {
    if (!listening) return;
    document.removeEventListener("keydown", onKeydown, true);
    listening = false;
    restoreFocus();
  }

  watch(
    visible,
    (v) => {
      if (v) start();
      else stop();
    },
    { immediate: true },
  );

  onBeforeUnmount(stop);

  return { panelEl };
}

/** 供测试使用：不依赖组件生命周期的最小可测表面 */
export const __modalInternals = { focusablesIn };
