// @vitest-environment node
/**
 * 打印中心的两处结构守卫（静态读源码，不挂载）。
 *
 * 背景（2026-09-27 用户截图报"选择列表错行"）：本页**此前没有 `<style>` 块**，
 * 我把新样式"追加到 `</style>` 之前"时静默失败——类名写进了模板却没有任何样式，
 * 于是双行列表退化成"块级元素堆叠"，看起来就是复选框和两行文字乱序。
 *
 * 教训：**引入新类名必须同时确认样式落地**（这次是"目标文件根本没有 style 块"）。
 * 本测试把两件事固化成断言：
 * 1) 本页存在 `<style scoped>` 且覆盖选择列表的关键类；
 * 2) 双行列表的行内顺序是「摘要 + 金额」在上、「编号 · 日期 · 状态」在下
 *    （与资料库同一套语言；顺序写反会让用户先看到编号，摘要被压到第二行）。
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const src = readFileSync(new URL("../PrintCenterPage.vue", import.meta.url), "utf8");
const styleBlocks = [...src.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map((m) => m[1]);
const css = styleBlocks.join("\n");

describe("打印中心：选择列表", () => {
  it("存在 style 块，且覆盖选择列表与操作条的关键类（防止再次「改了类名却没样式」）", () => {
    expect(styleBlocks.length).toBeGreaterThan(0);
    for (const cls of [
      ".pick-row",
      ".pick-main",
      ".line1",
      ".line2",
      ".action-bar",
      ".muted-center",
    ]) {
      expect(css, `样式缺少 ${cls}`).toContain(cls);
    }
    // 行是 flex 布局（缺了它就会退化成块级堆叠——本次缺陷的直接原因）
    expect(css).toMatch(/\.pick-row\s*\{[^}]*display:\s*flex/);
  });

  it("行内顺序：摘要 + 金额在上，编号 · 日期 · 状态在下", () => {
    const row = src.slice(src.indexOf('class="pick-row"'), src.indexOf('class="pick-more"'));
    const iLine1 = row.indexOf('class="line1"');
    const iLine2 = row.indexOf('class="line2"');
    expect(iLine1).toBeGreaterThan(-1);
    expect(iLine2).toBeGreaterThan(-1);
    expect(iLine1).toBeLessThan(iLine2);
    // line1 放摘要与金额；line2 放编号、日期与状态标签
    const line1 = row.slice(iLine1, iLine2);
    const line2 = row.slice(iLine2);
    expect(line1).toContain("desc");
    expect(line1).toContain("amount");
    expect(line2).toContain("voucherNo");
    expect(line2).toContain("voucherDate");
    expect(line2).toContain("STATUS_META");
  });

  it("预览面板高度跟随内容（不设 min-height / flex:1 拉满整列）", () => {
    const styleIdx = src.indexOf("<style");
    const css = src.slice(styleIdx);
    // 预览面板的 body 必须是普通块（内容多高就多高）
    expect(css).toMatch(/\.preview-panel \.fluent-panel-body \{[^}]*display:\s*block/);
    // 空态是紧凑提示，不是大块占位
    expect(src).toContain("preview-empty");
    expect(css).toMatch(/\.preview-empty \{[^}]*padding:\s*var\(--s-3\)/);
    // 预览面板不得被拉伸：右栏容器不应给 min-height
    expect(css).not.toMatch(/\.preview-panel[^}]*min-height/);
  });

  it("导出格式只有一个入口（顶部操作条），右栏不再重复一份", () => {
    const occurrences = [...src.matchAll(/exportFormat\s*=\s*'(pdf|xlsx|csv)'/g)].length;
    expect(occurrences, "导出格式的分段控件只应出现在顶部操作条").toBeLessThanOrEqual(3);
    expect(src).not.toContain("<label>导出格式</label>");
  });

  it("「全选筛选结果」在未勾选时也可用（禁用条件只看有无可选项 / 是否忙）", () => {
    // 背景：此前禁用条件是 !selected.size，于是"一张都没勾"时最常用的入口不可用，
    // 必须先手动勾一张（资料库页早已是"未选→显示全选"，两页口径曾相反）。
    expect(src).toMatch(
      /<button\s+class="btn"\s+:disabled="!filteredVouchers\.length \|\| busy"\s+@click="selectAll"\s*>\s*全选筛选结果\s*<\/button>/,
    );
    expect(src).not.toMatch(/:disabled="!selected\.size" @click="selectAll"/);
  });
});
