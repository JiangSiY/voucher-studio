import { defineStore } from "pinia";
import { computed, ref, watch } from "vue";
import { api, type AppConfig, type AppInfo } from "../api/client";

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

  return { config, appInfo, loaded, effectiveTheme, init, save };
});
