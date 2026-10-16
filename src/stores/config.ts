import { defineStore } from "pinia";
import { computed, ref, watch } from "vue";
import { api, type AppConfig, type AppInfo } from "../api/client";
import { applySidebarWidth, clampSidebarWidth } from "../composables/useSidebarResize";
import { applySidebarCollapsed } from "../composables/sidebarCollapse";

export const useConfigStore = defineStore("config", () => {
  const config = ref<AppConfig | null>(null);
  const appInfo = ref<AppInfo | null>(null);
  const loaded = ref(false);

  const darkQuery = window.matchMedia ? window.matchMedia("(prefers-color-scheme: dark)") : null;
  const systemDark = ref(darkQuery?.matches ?? false);
  /**
   * 监听系统深浅色。此前是匿名函数 + 不保存 `MediaQueryList`，**结构上无法移除**
   * （审计 A-10）：store 是单例，正式运行时确实全程存活，但 **dev HMR 每次热更都会
   * 重新执行 setup** —— 旧闭包会继续写入上一轮 store 的 ref，切主题时表现漂移。
   * 现在保存句柄，并在 HMR 释放时移除。
   */
  const onSystemThemeChange = (e: MediaQueryListEvent) => {
    systemDark.value = e.matches;
  };
  darkQuery?.addEventListener("change", onSystemThemeChange);
  const disposeSystemThemeWatch = () => {
    darkQuery?.removeEventListener("change", onSystemThemeChange);
  };
  if (import.meta.hot) import.meta.hot.dispose(disposeSystemThemeWatch);

  const effectiveTheme = computed<"light" | "dark">(() => {
    const t = config.value?.appearance.theme ?? "system";
    if (t === "system") return systemDark.value ? "dark" : "light";
    return t;
  });

  function applyTheme() {
    document.documentElement.dataset.theme = effectiveTheme.value;
  }
  watch(effectiveTheme, applyTheme, { immediate: true });

  /** 把界面字号倍数写进 CSS 变量（未加载时不写，保持 CSS 里的默认值 1） */
  function applyFontScale() {
    const s = config.value?.appearance.uiScale;
    if (typeof s === "number" && Number.isFinite(s) && s > 0) {
      document.documentElement.style.setProperty("--font-scale", String(s));
    }
  }
  watch(() => config.value?.appearance.uiScale, applyFontScale, { immediate: true });

  /** 把已保存的侧边栏宽度写进 CSS 变量（唯一 JS→CSS 通道；配置未加载时不写，保持 CSS 默认值） */
  function applyLayout() {
    const w = config.value?.appearance.sidebarWidth;
    if (typeof w === "number") applySidebarWidth(w);
  }
  watch(() => config.value?.appearance.sidebarWidth, applyLayout, { immediate: true });

  /**
   * 侧栏收起状态（用户 2026-10-16 要求：可收起，收起后只显示图标）。
   *
   * 持久化真源是 `appearance.sidebarCollapsed`（Rust `AppearanceConfig`）；这里另存一份本地
   * ref，是因为**浏览器预览模式下 `config` 为 null**（App.vue 跳过后端初始化），
   * 用户点开关也要立刻见效。落盘值变化时由下面的 watch 同步回来；
   * 切换一律走 `toggleSidebarCollapsed`（唯一写入点）。
   */
  const sidebarCollapsed = ref(false);
  watch(
    () => config.value?.appearance.sidebarCollapsed,
    (v) => {
      if (typeof v === "boolean") sidebarCollapsed.value = v;
    },
    { immediate: true },
  );
  // 应用方式与 `--sidebar-w` 同源：写根属性，由 CSS 决定 56px 图标栏观感
  watch(sidebarCollapsed, applySidebarCollapsed, { immediate: true });

  async function init() {
    try {
      [config.value, appInfo.value] = await Promise.all([api.getConfig(), api.getAppInfo()]);
    } finally {
      loaded.value = true;
    }
  }

  async function save(next: AppConfig) {
    config.value = await api.saveConfig(next);
    applyTheme();
    applyLayout();
  }

  /**
   * 侧边栏拖拽结束后的持久化入口（唯一写入点）。
   * 浏览器预览模式下 `config` 为 null（App.vue 跳过后端初始化）→ 只改界面不落盘。
   */
  async function saveSidebarWidth(width: number) {
    const w = clampSidebarWidth(width);
    applySidebarWidth(w);
    const cur = config.value;
    if (!cur || cur.appearance.sidebarWidth === w) return;
    await save({ ...cur, appearance: { ...cur.appearance, sidebarWidth: w } });
  }

  /**
   * 侧栏收起开关（工具栏按钮的唯一写入点；展开/收起两态都要落盘）。
   *
   * 先改本地状态再落盘：预览模式（`config` 为 null）下只改界面不落盘，
   * 与 `saveSidebarWidth` 同口径；落盘失败时界面已切换，由调用方 toast 提示。
   */
  async function toggleSidebarCollapsed() {
    const next = !sidebarCollapsed.value;
    sidebarCollapsed.value = next;
    const cur = config.value;
    if (!cur || cur.appearance.sidebarCollapsed === next) return;
    await save({ ...cur, appearance: { ...cur.appearance, sidebarCollapsed: next } });
  }

  return {
    config,
    appInfo,
    loaded,
    effectiveTheme,
    sidebarCollapsed,
    init,
    save,
    saveSidebarWidth,
    toggleSidebarCollapsed,
  };
});
