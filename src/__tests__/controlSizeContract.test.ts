// @vitest-environment node
/**
 * 控件尺寸一致性守护（用户 2026-09-27 截图报"控件大小不一致"）。
 *
 * 根因：同一行里的控件各自算高度——`.input` 靠 padding 撑、`.btn` 靠 min-height、
 * 原生下拉与日期框又有 WebKit 自带的高度与内边距，于是搜索框、下拉、日期、按钮高矮不一。
 * 修法是**高度只由 `--control-h-*` 决定**（`box-sizing: border-box` + 固定 height）。
 *
 * 本测试把这条约定固化成断言：任何人把某个控件改回 padding/min-height 撑高，
 * 或往模板里塞内联 height/padding 覆盖，都会红。
 */
import { describe, expect, it } from "vitest";
import { repoFile, srcFilesRelative, styleBlocks } from "./helpers/designGuard";

const css = repoFile("src/styles/base.css");

/** 取某条规则块（从选择器到配对的 `}`，只支持无嵌套的单层块） */
function rule(selector: string): string {
  const idx = css.indexOf(`\n${selector} {`);
  expect(idx, `base.css 缺少规则：${selector}`).toBeGreaterThan(-1);
  const end = css.indexOf("}", idx);
  return css.slice(idx, end);
}

describe("控件高度一致性", () => {
  it("输入框 / 按钮用同一高度令牌；下拉与日期框继承它、不得另写高度", () => {
    for (const sel of [".input", ".btn"]) {
      const r = rule(sel);
      expect(r, `${sel} 应用 height: var(--control-h-md)`).toContain("height: var(--control-h-md)");
      expect(r, `${sel} 应 box-sizing: border-box`).toContain("box-sizing: border-box");
    }
    // select.input / input[type=date].input 是 `.input` 的增强规则：不得出现别的 height
    for (const sel of ["select.input", 'input[type="date"].input']) {
      const r = rule(sel);
      const heights = [...r.matchAll(/(?:^|[;\s])height:\s*([^;]+);/g)].map((m) => m[1].trim());
      for (const h of heights) {
        expect(h, `${sel} 的高度若有覆盖，必须仍走 --control-h-md`).toBe("var(--control-h-md)");
      }
    }
  });

  it("输入框与按钮不再用垂直 padding/min-height 撑高（那是「各算一套」的根源）", () => {
    expect(rule(".input")).not.toMatch(/padding:\s*\d+px\s+\d+px/);
    expect(rule(".btn")).not.toMatch(/min-height/);
    expect(rule(".btn")).not.toMatch(/padding:\s*\d+px\s+\d+px/);
  });

  it("搜索框容器走控件高度令牌，且只有 base.css 一份实现", () => {
    const r = rule(".search");
    expect(r).toContain("height: var(--control-h-md)");
    expect(r).toContain("box-sizing: border-box");
    // 页面不得再自带一套 .search / .search-input 样式（那正是"两套高度算法"的来源）
    const files = srcFilesRelative([".vue"]);
    const reimpl = files.filter((rel) => styleBlocks(repoFile(`src/${rel}`)).trim() !== "" && /(^|\n)\s*\.search(-input)?\s*\{/.test(styleBlocks(repoFile(`src/${rel}`))));
    expect(reimpl, `这些页面重复实现了搜索框样式：${reimpl.join(", ")}`).toEqual([]);
  });

  it("控件元素（button/input/select 或 .btn/.input）不得内联覆盖 height / 垂直 padding", () => {
    // 只查**控件本身**：容器（div/section）上的 padding 是布局，不属于本约定
    const files = srcFilesRelative([".vue"]);
    const offenders: string[] = [];
    const tags = [
      /<(?:button|input|select)\b[^>]*?style="([^"]*)"/g,
      /<[a-z-]+\b[^>]*?class="[^"]*\b(?:btn|input)\b[^"]*"[^>]*?style="([^"]*)"/g,
    ];
    for (const rel of files) {
      const src = repoFile(`src/${rel}`);
      for (const re of tags) {
        for (const m of src.matchAll(re)) {
          const s = m[1];
          const literalHeight = [...s.matchAll(/(?:^|;)\s*height:\s*([^;]+)/g)].some(
            (m) => !m[1].includes("var(--control-h-"),
          );
          const literalPad = /(^|;)\s*padding:\s*[^;]*\d/.test(s);
          if (literalHeight || literalPad) offenders.push(`${rel}: ${s}`);
        }
      }
    }
    expect(offenders, `控件尺寸应走令牌，不要内联覆盖：\n${offenders.join("\n")}`).toEqual([]);
  });
});
