// @vitest-environment node
/**
 * 凭证列表表格的结构守卫（静态读源码，不挂载、不依赖后端）。
 *
 * 能证什么：模板结构正确——**表头 th 数量等于数据行 td 数量**（2026-09-18 修过一次
 * 「表头 9 列 / 数据 10 列」导致的金额之后整体错位一列），以及窄窗隐藏列与固定列
 * 所需的类名存在。
 * 不能证什么：媒体查询的实际隐藏/固定行为——jsdom 不计算 CSS，那一层由浏览器截图验证。
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const src = readFileSync(new URL("../VoucherListPage.vue", import.meta.url), "utf8");

function count(text: string, re: RegExp): number {
  return (text.match(re) ?? []).length;
}

const thead = src.slice(src.indexOf("<thead>"), src.indexOf("</thead>"));
const tbody = src.slice(src.indexOf("<tbody>"), src.indexOf("</tbody>"));
const firstRow = tbody.slice(0, tbody.indexOf("</tr>"));

describe("VoucherListPage 表格结构", () => {
  it("表头列数与数据行列数一致（防止再次错位）", () => {
    const thCount = count(thead, /<th[\s>]/g);
    const tdCount = count(firstRow, /<td[\s>]/g);
    expect(thCount).toBeGreaterThan(0);
    expect(tdCount).toBe(thCount);
  });

  it("存在窄窗隐藏列（col-tag / col-thumb）与固定列（col-check / col-ops）所需的类", () => {
    for (const cls of ["col-check", "col-thumb", "col-tag", "col-ops"]) {
      expect(thead).toContain(`class="${cls}"`);
      expect(firstRow).toContain(`class="${cls}"`);
    }
  });

  it("摘要/对方列用可压缩的类而不是内联 max-width（内联会压过媒体查询）", () => {
    expect(firstRow).toContain("cell-ellipsis cell-desc");
    expect(firstRow).toContain("cell-ellipsis cell-party");
    expect(firstRow).not.toContain("style=\"max-width: 220px");
    expect(firstRow).not.toContain("style=\"max-width: 140px");
  });
});
