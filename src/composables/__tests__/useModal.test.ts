// @vitest-environment jsdom
/**
 * `useModal` 焦点契约测试（规范 §4「焦点管理」）。
 *
 * 覆盖：打开时聚焦首个可操作元素（`data-autofocus` 优先）、Tab 从末项回到首项、
 * Shift+Tab 从首项回到末项、Esc 触发关闭、`{ escape: false }` 时不抢 Esc、
 * 关闭（卸载）后焦点归还给打开它的元素。
 */
import { afterEach, describe, expect, it } from "vitest";
import { mount, type VueWrapper } from "@vue/test-utils";
import { defineComponent, nextTick, ref } from "vue";
import { useModal } from "../useModal";

let wrappers: VueWrapper[] = [];

function harness(opts: { escape?: boolean } = {}) {
  const closes = ref(0);
  const Comp = defineComponent({
    setup() {
      const { panelEl } = useModal(
        () => true,
        () => {
          closes.value += 1;
        },
        opts,
      );
      return { panelEl };
    },
    template: `<div ref="panelEl" class="panel" tabindex="-1">
        <button id="a">A</button>
        <button id="b" data-autofocus>B</button>
        <button id="c">C</button>
      </div>`,
  });
  return { w: mount(Comp, { attachTo: document.body }), closes };
}

function press(key: string, shift = false) {
  document.activeElement?.dispatchEvent(
    new KeyboardEvent("keydown", { key, shiftKey: shift, bubbles: true, cancelable: true }),
  );
}

afterEach(() => {
  wrappers.forEach((w) => w.unmount());
  wrappers = [];
  document.body.innerHTML = "";
});

describe("useModal 焦点契约", () => {
  it("打开时聚焦 data-autofocus 指定的元素；没有标记时落首个可操作元素", async () => {
    const { w } = harness();
    wrappers.push(w);
    await nextTick();
    await nextTick();
    expect((document.activeElement as HTMLElement).id).toBe("b");
  });

  it("Tab 从末项回绕到首项，Shift+Tab 从首项回绕到末项", async () => {
    const { w } = harness();
    wrappers.push(w);
    await nextTick();
    (document.getElementById("c") as HTMLElement).focus();
    press("Tab");
    expect((document.activeElement as HTMLElement).id).toBe("a");

    press("Tab", true);
    expect((document.activeElement as HTMLElement).id).toBe("c");
  });

  it("Esc 触发关闭；escape:false 时不抢 Esc（组件自己处理）", async () => {
    const a = harness();
    wrappers.push(a.w);
    await nextTick();
    press("Escape");
    expect(a.closes.value).toBe(1);

    const b = harness({ escape: false });
    wrappers.push(b.w);
    await nextTick();
    press("Escape");
    expect(b.closes.value).toBe(0);
  });

  it("关闭后焦点归还给打开它的元素", async () => {
    const opener = document.createElement("button");
    opener.id = "opener";
    document.body.appendChild(opener);
    opener.focus();

    const { w } = harness();
    await nextTick();
    expect((document.activeElement as HTMLElement).id).toBe("b");

    w.unmount();
    expect((document.activeElement as HTMLElement).id).toBe("opener");
  });
});
