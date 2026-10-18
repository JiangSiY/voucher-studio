import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useTaskStore } from "../tasks";

beforeEach(() => {
  setActivePinia(createPinia());
});

describe("任务中心 store", () => {
  it("开任务：进列表、置为运行中、并自动展开浮层", () => {
    const s = useTaskStore();
    s.toggleCollapsed(); // 用户先最小化了
    expect(s.collapsed).toBe(true);

    const id = s.start("import", "导入 12 张图片", 12);

    expect(s.items).toHaveLength(1);
    expect(s.items[0]).toMatchObject({
      id,
      kind: "import",
      title: "导入 12 张图片",
      total: 12,
      state: "running",
    });
    // 有新活儿就展开：最小化状态下悄悄开跑等于没有任务中心
    expect(s.collapsed).toBe(false);
    expect(s.runningCount).toBe(1);
    expect(s.summary).toBe("1 个任务进行中");
  });

  it("进度只对运行中的任务生效（结束后的迟到事件不写回）", () => {
    const s = useTaskStore();
    const id = s.start("ocr", "批量识别", -1);

    s.progress(id, { done: 3, total: 10, detail: "第 3 张" });
    expect(s.items[0]).toMatchObject({ done: 3, total: 10, detail: "第 3 张" });

    s.finish(id, { detail: "已识别 10 张" });
    s.progress(id, { done: 99 });
    expect(s.items[0].done).toBe(3);
    expect(s.items[0]).toMatchObject({ state: "done", detail: "已识别 10 张" });
  });

  it("失败保留错误原文，完成后可带去处动作", () => {
    const s = useTaskStore();
    const onClick = vi.fn();
    const bad = s.start("export", "生成 PDF");
    s.fail(bad, "磁盘空间不足");
    expect(s.items[0]).toMatchObject({ state: "failed", error: "磁盘空间不足" });

    const good = s.start("export", "生成 PDF");
    s.finish(good, { detail: "B-2026-001", action: { label: "打开文件", onClick } });
    expect(s.items[1].state).toBe("done");
    s.items[1].action?.onClick();
    expect(onClick).toHaveBeenCalledOnce();

    expect(s.summary).toBe("有任务失败"); // 全跑完后胶囊优先报失败
  });

  it("已完成最多留 3 条，超出丢最旧的；运行中的永不丢", () => {
    const s = useTaskStore();
    const running = s.start("import", "还在导入");
    for (let i = 1; i <= 5; i++) {
      const id = s.start("export", `导出 ${i}`);
      s.finish(id, { detail: `批次 ${i}` });
    }

    expect(s.items.filter((t) => t.state === "running")).toHaveLength(1);
    expect(s.items.filter((t) => t.state !== "running")).toHaveLength(3);
    expect(s.items[0].id).toBe(running);
    // 留下的是最近三条
    expect(s.finished.map((t) => t.detail)).toEqual(["批次 3", "批次 4", "批次 5"]);
  });

  it("清除已完成与逐条关闭", () => {
    const s = useTaskStore();
    const a = s.start("import", "A");
    const b = s.start("import", "B");
    s.finish(a, {});
    s.finish(b, {});

    s.clearFinished();
    expect(s.items).toHaveLength(0);

    const c = s.start("import", "C");
    s.dismiss(c);
    expect(s.items).toHaveLength(0);
    expect(s.summary).toBe("任务已完成");
  });

  it("空列表时胶囊文案是兜底的「任务已完成」（浮层本身不渲染）", () => {
    const s = useTaskStore();
    expect(s.items).toHaveLength(0);
    expect(s.summary).toBe("任务已完成");
  });
});
