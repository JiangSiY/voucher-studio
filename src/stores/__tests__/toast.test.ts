import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { toast, useToastStore } from "../toast";

beforeEach(() => {
  setActivePinia(createPinia());
});

describe("toast store", () => {
  it("push/dismiss 生命周期", () => {
    const store = useToastStore();
    const id = store.push("info", "hello", 0);
    expect(store.items).toHaveLength(1);
    expect(store.items[0]).toMatchObject({ kind: "info", text: "hello" });
    store.dismiss(id);
    expect(store.items).toHaveLength(0);
  });

  it("带 action 的 toast 持有回调与文案", () => {
    const onClick = vi.fn();
    toast.success("导出完成", { label: "打开", onClick });
    const store = useToastStore();
    expect(store.items[0].action?.label).toBe("打开");
    store.items[0].action?.onClick();
    expect(onClick).toHaveBeenCalledOnce();
  });

  it("带 action 的 toast 自动关闭时间延长（给用户点按钮的时间）", () => {
    vi.useFakeTimers();
    try {
      const store = useToastStore();
      // 无 action：success 默认 3.5s 关闭
      toast.success("无动作");
      vi.advanceTimersByTime(3600);
      expect(store.items).toHaveLength(0);

      // 有 action：至少 8s（durationFor 下限）
      toast.success("有动作", { label: "打开", onClick: () => {} });
      vi.advanceTimersByTime(3600);
      expect(store.items).toHaveLength(1);
      vi.advanceTimersByTime(4600); // 累计 8.2s
      expect(store.items).toHaveLength(0);
    } finally {
      vi.useRealTimers();
    }
  });
});
