import { acceptHMRUpdate, defineStore } from "pinia";
import { computed, ref, watch } from "vue";
import { api, type AppConfig, type AppInfo } from "../api/client";

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
  }

  return {
    config,
    appInfo,
    loaded,
    effectiveTheme,
    init,
    save,
  };
});

/**
 * store 源码改动后热更新实例本身。
 *
 * 不加这句时，dev 下改了 store（新增/改名方法/字段）只有定义被替换、
 * **实例仍是旧的那个**——调用方拿到 undefined 才炸（2026-10-16 用户实报
 * `configStore.withLiveAppearance is not a function`）。合并逻辑已移到
 * `composables/liveAppearance.ts` 的纯函数，这里再补上 HMR 接收，双保险。
 */
if (import.meta.hot) import.meta.hot.accept(acceptHMRUpdate(useConfigStore, import.meta.hot));
