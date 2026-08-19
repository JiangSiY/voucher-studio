import { defineStore } from "pinia";
import { ref } from "vue";

export type ToastKind = "info" | "success" | "warning" | "danger";

export interface ToastItem {
  id: number;
  kind: ToastKind;
  text: string;
}

let nextId = 1;

export const useToastStore = defineStore("toast", () => {
  const items = ref<ToastItem[]>([]);

  function push(kind: ToastKind, text: string, duration = 3500) {
    const id = nextId++;
    items.value.push({ id, kind, text });
    if (duration > 0) {
      setTimeout(() => dismiss(id), duration);
    }
    return id;
  }

  function dismiss(id: number) {
    items.value = items.value.filter((t) => t.id !== id);
  }

  return { items, push, dismiss };
});

/** 快捷方法 */
export const toast = {
  info: (t: string) => useToastStore().push("info", t),
  success: (t: string) => useToastStore().push("success", t),
  warning: (t: string) => useToastStore().push("warning", t, 6000),
  danger: (t: string) => useToastStore().push("danger", t, 8000),
};

/** 统一错误提示（后端返回的错误字符串 → toast） */
export function toastError(e: unknown) {
  const msg = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
  toast.danger(msg);
}
