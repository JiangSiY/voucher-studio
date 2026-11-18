import { computed } from "vue";
import { useRoute, useRouter, type LocationQuery } from "vue-router";

export type ModalName = "settings" | "backup" | "export" | "duplicates" | "help";

/**
 * 弹层状态即 URL（v0.27）：
 * `?modal=settings|backup|export|duplicates|help`，附带 `pair`/`tab` 等参数。
 * 这样刷新、后退、从旧路由（/settings 等）重定向过来都能直接落到同一个弹层。
 */
export function useModalRoute() {
  const route = useRoute();
  const router = useRouter();

  const modal = computed<ModalName | "">(() => {
    const v = route.query.modal;
    return typeof v === "string" &&
      ["settings", "backup", "export", "duplicates", "help"].includes(v)
      ? (v as ModalName)
      : "";
  });

  function open(name: ModalName, extra: LocationQuery = {}) {
    void router.replace({ query: { ...route.query, ...extra, modal: name } });
  }

  function close() {
    const next: LocationQuery = { ...route.query };
    delete next.modal;
    delete next.pair;
    delete next.tab;
    void router.replace({ query: next });
  }

  return { modal, open, close };
}
