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

  it("连续重复提示合并为一条，并把消失时间往后推", () => {
    const store = useToastStore();
    toast.danger("保存失败：数据库被锁");
    toast.danger("保存失败：数据库被锁");
    toast.danger("保存失败：数据库被锁");
    expect(store.items).toHaveLength(1);

    // 文案不同则不合并（用户要知道是两件事）
    toast.danger("另一次失败");
    expect(store.items).toHaveLength(2);
  });

  it("合并时用新回调覆盖旧动作（动作可能已指向新批次）", () => {
    const store = useToastStore();
    const first = vi.fn();
    const second = vi.fn();
    toast.success("导出完成", { label: "打开", onClick: first });
    toast.success("导出完成", { label: "打开", onClick: second });
    expect(store.items).toHaveLength(1);
    store.items[0].action?.onClick();
    expect(second).toHaveBeenCalledOnce();
    expect(first).not.toHaveBeenCalled();
  });

  it("同屏最多 4 条，超出丢最旧的", () => {
    const store = useToastStore();
    for (let i = 1; i <= 6; i++) store.push("info", `第 ${i} 条`, 0);
    expect(store.items).toHaveLength(4);
    expect(store.items.map((t) => t.text)).toEqual(["第 3 条", "第 4 条", "第 5 条", "第 6 条"]);
  });
});
