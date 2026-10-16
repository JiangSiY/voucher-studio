// @vitest-environment jsdom
/**
 * 任务中心浮层的渲染行为（契约测试只读源码，这里真的挂载看一眼）。
 * 覆盖三条用户能感知的行为：展开/最小化、确定态与不确定态进度条、完成后的去处按钮。
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { mount } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import TaskCenter from "../TaskCenter.vue";
import { useTaskStore } from "../../stores/tasks";

beforeEach(() => {
  setActivePinia(createPinia());
});

describe("TaskCenter 组件", () => {
  it("没有任务时完全不渲染（不留空壳占位）", () => {
    const w = mount(TaskCenter);
    expect(w.find(".task-center").exists()).toBe(false);
  });

  it("开任务后显示标题、汇总与确定态进度", async () => {
    const store = useTaskStore();
    const id = store.start("import", "导入 3 张图片", 3);
    store.progress(id, { done: 2, detail: "正在处理第 3 张" });

    const w = mount(TaskCenter);
    expect(w.text()).toContain("导入 3 张图片");
    expect(w.text()).toContain("1 个任务进行中");
    expect(w.text()).toContain("2 / 3");
    expect(w.text()).toContain("正在处理第 3 张");

    const bar = w.find('[role="progressbar"]');
    expect(bar.attributes("aria-valuenow")).toBe("67");
    expect(w.find(".tc-fill").attributes("style")).toContain("width: 67%");
  });

  it("总量未知时走不确定态（不假装 0%）", () => {
    useTaskStore().start("ocr", "批量识别待识别附件");
    const w = mount(TaskCenter);
    expect(w.find(".tc-bar").classes()).toContain("is-indeterminate");
    expect(w.find('[role="progressbar"]').attributes("aria-valuenow")).toBeUndefined();
  });

  it("完成后给去处按钮，点了执行回调并移除这一条", async () => {
    const store = useTaskStore();
    const onClick = vi.fn();
    const id = store.start("export", "生成 PDF（3 张凭证）");
    store.finish(id, { detail: "B-2026-001", action: { label: "打开文件", onClick } });

    const w = mount(TaskCenter);
    const action = w.findAll("button").find((b) => b.text() === "打开文件")!;
    expect(action).toBeTruthy();
    await action.trigger("click");
    expect(onClick).toHaveBeenCalledOnce();
    expect(w.find(".task-center").exists()).toBe(false);
  });

  it("最小化只收起列表，头部仍在（否则再也展不开）", async () => {
    useTaskStore().start("backup", "关闭前自动备份");
    // 必须 attachTo：jsdom 里脱离文档的元素算不出 display（isVisible 会恒为 true）
    const w = mount(TaskCenter, { attachTo: document.body });
    expect(w.find("#task-center-list").isVisible()).toBe(true);

    await w.find(".tc-header").trigger("click");
    expect(w.find("#task-center-list").isVisible()).toBe(false);
    expect(w.find(".tc-header").exists()).toBe(true);
    expect(w.find(".tc-header").attributes("aria-expanded")).toBe("false");
    w.unmount();
  });

  it("失败条目显示错误原文，不再显示进度", () => {
    const store = useTaskStore();
    const id = store.start("export", "生成 PDF");
    store.fail(id, "磁盘空间不足");
    const w = mount(TaskCenter);
    expect(w.text()).toContain("磁盘空间不足");
    expect(w.find('[role="progressbar"]').exists()).toBe(false);
  });
});
