// @vitest-environment node
/**
 * 「上一张 / 下一张」前端实装的契约（2026-10-16 用户要求「上一张、下一张前端实装」）。
 *
 * 三处容易只做一半：
 *   1. 只有快捷键、没有可见入口（或反过来）——两者都要，且都走同一条动线；
 *   2. ⌥←/→ 抢掉输入框里的「按词移动光标」——必须排在输入框早退之后；
 *   3. 翻张上下文与列表筛选脱钩（切了筛选还按旧口径翻）——必须用列表自己的 `query()`，
 *      并在筛选变化时重拉位置。
 * 规范和帮助页也要同步（⌥←/→ 从「未实装」改为「已实装」）。
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const read = (rel: string) => readFileSync(new URL(`../../${rel}`, import.meta.url), "utf8");

const page = read("src/pages/LibraryPage.vue");
const composable = read("src/composables/neighborNav.ts");
const help = read("src/pages/HelpPage.vue");
const spec = read("docs/UI-设计规范-macOS27.md");

describe("上一张 / 下一张", () => {
  it("右栏头部有可见入口，禁用态跟着相邻张有无走", () => {
    expect(page).toMatch(/class="ins-nav" role="group" aria-label="上下文翻张"/);
    expect(page).toMatch(/class="btn-icon" :disabled="!navNeighbor\?\.prevId" title="上一张（⌥←）" aria-label="上一张"/);
    expect(page).toMatch(/class="btn-icon" :disabled="!navNeighbor\?\.nextId" title="下一张（⌥→）" aria-label="下一张"/);
    expect(page).toMatch(/\{\{ navPosition \}\}/);
    // 顺序与列表排序无关，必须在 title 里说清（否则按金额排序时会以为翻错了）
    expect(page).toMatch(/按日期顺序翻张（当前筛选共/);
    expect(page).toContain("与列表排序无关");
  });

  it("⌥←/→ 走同一条动线，且不抢输入框里的按键", () => {
    expect(page).toMatch(/e\.altKey && \(e\.key === "ArrowLeft" \|\| e\.key === "ArrowRight"\)/);
    expect(page).toMatch(/goNeighbor\(e\.key === "ArrowLeft" \? "prev" : "next"\)/);
    // 输入框早退必须在 ⌥ 分支之前：inputs 里 alt+方向键是按词移动光标
    const guard = page.indexOf('if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA") return;');
    const altBranch = page.indexOf('if (e.altKey && (e.key === "ArrowLeft" || e.key === "ArrowRight"))');
    expect(guard).toBeGreaterThan(-1);
    expect(altBranch).toBeGreaterThan(guard);
  });

  it("翻张上下文就是列表的筛选（同一份 query），并在筛选/换凭证时重拉", () => {
    expect(page).toMatch(/query: computed\(\(\) => query\(\) as VoucherQuery\)/);
    expect(page).toMatch(/watch\(\s*\(\) => shown\.value\?\.id \?\? null,/);
    expect(page.match(/void loadNeighbor\(\)/g) ?? []).toHaveLength(3); // 选中变化 + 两个筛选 watcher
    expect(composable).toContain("api.getVoucherNeighbor(id, opts.query.value)");
    // 在途响应保护（快速连按时旧响应不得写回）
    expect(composable).toMatch(/let seq = 0/);
    expect(composable).toMatch(/if \(my === seq\) neighbor\.value = n/);
  });

  it("「保存并下一张」优先走上下文翻张（能跨页），不再是「页尾就卡住」", () => {
    expect(page).toMatch(/if \(navNeighbor\.value\?\.nextId\) \{\s*await goNeighbor\("next"\);/);
  });

  it("走既有安全动线：页外凭证也要过脏数据确认", () => {
    expect(page).toMatch(/await showFocused\(id\)/);
  });

  it("帮助页与规范同步为「已实装」", () => {
    expect(help).toMatch(/\{ keys: "⌥← ⌥→", what: "上一张 \/ 下一张/);
    expect(spec).toMatch(/\| ⌥← \/ ⌥→ \| \*\*上一张 \/ 下一张（2026-10-16 实装）\*\*/);
    // ⌘⇧Enter 仍是未实装，不能顺手写成已实现
    expect(spec).toMatch(/\| ⌘⇧Enter \|[^|\n]*\*\*仍未实装\*\*/);
  });
});
