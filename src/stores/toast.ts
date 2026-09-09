import { defineStore } from "pinia";
import { ref } from "vue";
import { errMsg } from "../api/errors";

export type ToastKind = "info" | "success" | "warning" | "danger";

/** toast 内嵌动作按钮（如「打开 PDF」「开始核对」）：点击后执行回调并关闭 toast */
export interface ToastAction {
  label: string;
  onClick: () => void;
}

export interface ToastItem {
  id: number;
  kind: ToastKind;
  text: string;
  action?: ToastAction;
}

let nextId = 1;

export const useToastStore = defineStore("toast", () => {
  const items = ref<ToastItem[]>([]);

  function push(kind: ToastKind, text: string, duration = 3500, action?: ToastAction) {
    const id = nextId++;
    items.value.push({ id, kind, text, action });
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

/** 带动作的 toast 至少停留 8s（给用户点按钮的时间） */
const durationFor = (base: number, action?: ToastAction) =>
  action ? Math.max(base, 8000) : base;

/** 快捷方法 */
export const toast = {
  info: (t: string, action?: ToastAction) =>
    useToastStore().push("info", t, durationFor(3500, action), action),
  success: (t: string, action?: ToastAction) =>
    useToastStore().push("success", t, durationFor(3500, action), action),
  warning: (t: string, action?: ToastAction) =>
    useToastStore().push("warning", t, durationFor(6000, action), action),
  danger: (t: string, action?: ToastAction) =>
    useToastStore().push("danger", t, durationFor(8000, action), action),
};

/** 统一错误提示：{code,message} 对象取 message、字符串原样、Error 取 message */
export function toastError(e: unknown) {
  toast.danger(errMsg(e));
}
