import { defineStore } from "pinia";
import { computed, ref, watch } from "vue";
import { api, type AppConfig, type AppInfo } from "../api/client";
import { applySidebarWidth, clampSidebarWidth } from "../composables/useSidebarResize";

export const useConfigStore = defineStore("config", () => {
  const config = ref<AppConfig | null>(null);
  const appInfo = ref<AppInfo | null>(null);
  const loaded = ref(false);

  const systemDark = ref(
    window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches,
  );
  if (window.matchMedia) {
    window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", (e) => {
      systemDark.value = e.matches;
    });
  }

  const effectiveTheme = computed<"light" | "dark">(() => {
    const t = config.value?.appearance.theme ?? "system";
    if (t === "system") return systemDark.value ? "dark" : "light";
    return t;
  });

  function applyTheme() {
    document.documentElement.dataset.theme = effectiveTheme.value;
  }
  watch(effectiveTheme, applyTheme, { immediate: true });

  /** 把已保存的侧边栏宽度写进 CSS 变量（唯一 JS→CSS 通道；配置未加载时不写，保持 CSS 默认值） */
  function applyLayout() {
    const w = config.value?.appearance.sidebarWidth;
    if (typeof w === "number") applySidebarWidth(w);
  }
  watch(() => config.value?.appearance.sidebarWidth, applyLayout, { immediate: true });

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

  return { config, appInfo, loaded, effectiveTheme, init, save, saveSidebarWidth };
});
