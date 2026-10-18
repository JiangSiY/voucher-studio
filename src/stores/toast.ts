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

/** 同屏最多显示的条目数：超出丢最旧的，避免一屏堆满把界面糊住 */
export const MAX_TOAST_ITEMS = 4;

export const useToastStore = defineStore("toast", () => {
  const items = ref<ToastItem[]>([]);

  function push(kind: ToastKind, text: string, duration = 3500, action?: ToastAction) {
    // 连续重复提示合并：同类型同文案只留一条，并把消失时间往后推。
    // 否则「重试三次保存」会叠三层一模一样的错误——用户分不清到底有几件事出错，也点不准关闭按钮。
    const last = items.value[items.value.length - 1];
    if (last && last.kind === kind && last.text === text) {
      if (action) last.action = action; // 新回调覆盖旧的（动作可能已指向新的批次）
      if (duration > 0) setTimeout(() => dismiss(last.id), duration);
      return last.id;
    }
    const id = nextId++;
    items.value.push({ id, kind, text, action });
    if (items.value.length > MAX_TOAST_ITEMS) items.value = items.value.slice(-MAX_TOAST_ITEMS);
    if (duration > 0) {
      // 定时器无需清理：这是「到点自动消失」的语义本身（不绑定组件生命周期；
      // 提前 dismiss 时定时器再触发也只是把已删的 id 再过滤一次，幂等无副作用）
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
const durationFor = (base: number, action?: ToastAction) => (action ? Math.max(base, 8000) : base);

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
