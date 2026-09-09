// @vitest-environment jsdom
/**
 * ConfirmDialog 组件测试（v0.14-D 组件测试补至 3 场景，体检 #16）。
 *
 * 覆盖：visible 门控（隐藏时不渲染弹层）、标题/正文/按钮文案与 aria-label、
 * 确认/取消回调 emit、遮罩 self-click 关闭与弹层内部点击不误关。
 * 组件使用 <Teleport to="body">，测试挂载到 document.body 后经 DOM 查询断言。
 */
import { describe, expect, it, afterEach } from "vitest";
import { mount, type VueWrapper } from "@vue/test-utils";
import { nextTick } from "vue";
import ConfirmDialog from "../ConfirmDialog.vue";

afterEach(() => {
  document.body.innerHTML = "";
});

async function openDialog(
  overrides: Partial<{
    visible: boolean;
    title: string;
    body: string;
    confirmText: string;
    danger: boolean;
  }> = {},
): Promise<VueWrapper> {
  const w = mount(ConfirmDialog, {
    props: {
      visible: true,
      title: "确认操作",
      body: "将删除 3 张凭证",
      confirmText: "确定",
      danger: true,
      ...overrides,
    },
    attachTo: document.body,
  });
  await nextTick();
  return w;
}

const bySel = (sel: string): HTMLElement | null => document.querySelector(sel);
const dialogButtons = () =>
  Array.from(document.querySelectorAll<HTMLButtonElement>(".dialog-ops button"));

describe("ConfirmDialog", () => {
  it("visible=false 时不渲染弹层", async () => {
    const w = mount(ConfirmDialog, {
      props: { visible: false, body: "x" },
      attachTo: document.body,
    });
    await nextTick();
    expect(bySel(".dialog-mask")).toBeNull();
    w.unmount();
  });

  it("渲染标题/正文/默认按钮与 aria-label", async () => {
    await openDialog({ title: "删除确认", body: "将删除 3 张凭证" });
    const dialog = bySel(".dialog");
    expect(bySel(".dialog-mask")).not.toBeNull();
    expect(dialog?.getAttribute("role")).toBe("alertdialog");
    expect(dialog?.getAttribute("aria-label")).toBe("删除确认");
    expect(bySel(".dialog h3")?.textContent).toBe("删除确认");
    expect(bySel(".dialog-body")?.textContent).toContain("将删除 3 张凭证");
    const buttons = dialogButtons();
    expect(buttons).toHaveLength(2);
    expect(buttons[0]?.textContent?.trim()).toBe("取消");
    expect(buttons[1]?.textContent?.trim()).toBe("确定");
  });

  it("danger=true 时确认按钮带危险样式；danger=false 走主按钮", async () => {
    await openDialog();
    expect(bySel(".btn-danger")).not.toBeNull();
    expect(bySel(".btn-primary")).toBeNull();

    document.body.innerHTML = "";
    await openDialog({ danger: false, confirmText: "保存" });
    expect(bySel(".btn-primary")).not.toBeNull();
    expect(bySel(".btn-danger")).toBeNull();
    expect(dialogButtons()[1]?.textContent?.trim()).toBe("保存");
  });

  it("点击取消仅 emit cancel", async () => {
    const w = await openDialog();
    const cancel = dialogButtons().find((b) => b.textContent?.trim() === "取消");
    cancel?.click();
    await nextTick();
    expect(w.emitted("cancel")).toHaveLength(1);
    expect(w.emitted("confirm")).toBeUndefined();
    w.unmount();
  });

  it("点击确定仅 emit confirm", async () => {
    const w = await openDialog();
    dialogButtons()[1]?.click();
    await nextTick();
    expect(w.emitted("confirm")).toHaveLength(1);
    expect(w.emitted("cancel")).toBeUndefined();
    w.unmount();
  });

  it("点击遮罩自身（@click.self）emit cancel", async () => {
    const w = await openDialog();
    bySel(".dialog-mask")?.click();
    await nextTick();
    expect(w.emitted("cancel")).toHaveLength(1);
    w.unmount();
  });

  it("点击弹层内部不触发 cancel（.self 守卫）", async () => {
    const w = await openDialog();
    bySel(".dialog")?.click();
    await nextTick();
    expect(w.emitted("cancel")).toBeUndefined();
    w.unmount();
  });
});
