// @vitest-environment jsdom
/**
 * VoucherNavBar 组件测试（组件测试基建首批样例，报告第三批「前端组件测试基建」）。
 *
 * 覆盖：相邻张按钮的禁用态（prevId/nextId 为空时置灰）、点击 emit('prev'/'next')、
 * 位置标签与筛选 keyword tooltip 渲染。
 */
import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import VoucherNavBar from "../VoucherNavBar.vue";

function mountNav(overrides: Partial<{
  prevId: number | null;
  nextId: number | null;
  positionLabel: string;
  ctxKeyword: string;
}> = {}) {
  return mount(VoucherNavBar, {
    props: {
      prevId: null,
      nextId: null,
      positionLabel: "3 / 10",
      ctxKeyword: "",
      ...overrides,
    },
  });
}

describe("VoucherNavBar", () => {
  it("prev/next 均无相邻时两个按钮都禁用", () => {
    const w = mountNav();
    const buttons = w.findAll("button");
    expect(buttons).toHaveLength(2);
    expect(buttons[0].attributes("disabled")).toBeDefined();
    expect(buttons[1].attributes("disabled")).toBeDefined();
  });

  it("存在前后相邻时按钮启用并显示位置标签", () => {
    const w = mountNav({ prevId: 4, nextId: 6, positionLabel: "5 / 20" });
    const buttons = w.findAll("button");
    expect(buttons[0].attributes("disabled")).toBeUndefined();
    expect(buttons[1].attributes("disabled")).toBeUndefined();
    expect(w.find(".vnav-pos").text()).toBe("5 / 20");
  });

  it("prevId 为空时上一张禁用、下一张可用（部分相邻）", () => {
    const w = mountNav({ prevId: null, nextId: 7 });
    const buttons = w.findAll("button");
    expect(buttons[0].attributes("disabled")).toBeDefined();
    expect(buttons[1].attributes("disabled")).toBeUndefined();
  });

  it("点击上一张 / 下一张分别 emit prev / next", async () => {
    const w = mountNav({ prevId: 4, nextId: 6 });
    const buttons = w.findAll("button");
    await buttons[0].trigger("click");
    expect(w.emitted("prev")).toHaveLength(1);
    expect(w.emitted("next")).toBeUndefined();

    await buttons[1].trigger("click");
    expect(w.emitted("next")).toHaveLength(1);
    expect(w.emitted("prev")).toHaveLength(1);
  });

  it("禁用按钮点击不触发事件", async () => {
    const w = mountNav();
    const buttons = w.findAll("button");
    await buttons[0].trigger("click");
    await buttons[1].trigger("click");
    expect(w.emitted("prev")).toBeUndefined();
    expect(w.emitted("next")).toBeUndefined();
  });

  it("ctxKeyword 为空时 tooltip 显示兜底文案，非空时显示筛选来源", () => {
    const w = mountNav({ positionLabel: "2 / 5" });
    const pos = w.find(".vnav-pos");
    expect(pos.attributes("title")).toBe("当前列表筛选");

    const w2 = mountNav({ positionLabel: "2 / 5", ctxKeyword: "差旅费" });
    expect(w2.find(".vnav-pos").attributes("title")).toBe("筛选：差旅费");
  });
});
