// @vitest-environment node
/**
 * **下钻参数契约**（审计 S3-② 守卫，2026-09-27 因真实缺陷补上）。
 *
 * 缺陷回放：洞察页的月度透视点一行会跳到资料库并带 `counterparty=` / `tags=`，
 * 但资料库的 `readQuery` 没读这两个键 → 跳过去**不过滤**，用户看到的是全量列表。
 * 这类"图表/卡片点了没反应"的问题靠人工测很难覆盖全（每个下钻源 × 每个参数），
 * 故把契约固化：**所有页面 push 到 `/vouchers` 的 query 键，资料库必须都能读**。
 *
 * 做法（静态读源码）：
 * 1) 扫描 `src/**` 里 `path: "/vouchers"` 的 `query: { ... }` 字面量，抽出键名；
 * 2) 抽出资料库 `readQuery()` 里读取的键（`q.<key>` 形式）；
 * 3) 断言前者 ⊆ 后者——多出来的键会被打印出来，直接指向漏读的那一个。
 */
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

function repoFile(rel: string): string {
  return readFileSync(new URL(`../../${rel}`, import.meta.url), "utf8");
}

/** 所有页面里"跳到 /vouchers 并带 query"的键集合 */
function pushedKeys(): Map<string, Set<string>> {
  const root = fileURLToPath(new URL("../../src", import.meta.url));
  const files = readdirSync(root, { recursive: true })
    .map((f) => String(f))
    .filter((f) => f.endsWith(".vue") && !f.includes("__tests__"));
  const out = new Map<string, Set<string>>();
  const anchor = /path:\s*"\/vouchers",\s*query:\s*\{/g;
  for (const rel of files) {
    const src = repoFile(`src/${rel}`);
    const keys = new Set<string>();
    for (const m of src.matchAll(anchor)) {
      // 从 `query: {` 起做**配对花括号**扫描（非贪婪会在嵌套对象处提前截断，
      // 那样 `...({ counterpartyEmpty: "1" })` 之后的键就看不见了——上一版正是这样漏掉 tags/counterparty）
      const open = m.index! + m[0].length - 1;
      let depth = 0;
      let end = open;
      for (let i = open; i < src.length; i++) {
        if (src[i] === "{") depth++;
        else if (src[i] === "}") {
          depth--;
          if (depth === 0) {
            end = i;
            break;
          }
        }
      }
      const block = src.slice(open + 1, end);
      // 去掉注释行，避免把注释里的词当成键
      const code = block
        .split("\n")
        .filter((l) => !l.trim().startsWith("//"))
        .join("\n");
      for (const k of code.matchAll(/(?:^|[\s,{])([a-zA-Z][a-zA-Z0-9_]*)\s*:/g)) keys.add(k[1]);
    }
    if (keys.size) out.set(rel, keys);
  }
  return out;
}

/** 资料库 readQuery 里实际读取的键 */
function readKeys(): Set<string> {
  const src = repoFile("src/pages/LibraryPage.vue");
  const body = src.slice(src.indexOf("function readQuery()"), src.indexOf("const activeFilters"));
  const keys = new Set<string>();
  for (const m of body.matchAll(/\bq\.([a-zA-Z][a-zA-Z0-9_]*)/g)) keys.add(m[1]);
  return keys;
}

describe("下钻参数契约（/vouchers）", () => {
  it("任何页面跳到 /vouchers 时带的 query 键，资料库都必须读取（否则点了不过滤）", () => {
    const pushed = pushedKeys();
    const read = readKeys();
    expect(pushed.size, "未找到任何下钻源，检查解析规则").toBeGreaterThan(0);

    const missing: string[] = [];
    for (const [file, keys] of pushed) {
      for (const k of keys) {
        if (!read.has(k)) missing.push(`${file} 推了 ${k}，但资料库 readQuery 没读`);
      }
    }
    expect(missing, `下钻参数未被读取：\n${missing.join("\n")}`).toEqual([]);
  });

  it("资料库对 query 变化也重新读取（同一路由内再次下钻必须生效）", () => {
    const src = repoFile("src/pages/LibraryPage.vue");
    // 除 onMounted 首读外，还要 watch route.query
    expect(src).toMatch(/watch\(\s*\(\)\s*=>\s*route\.query/);
  });

  it("已知的下钻源至少覆盖：洞察透视（对方/标签/未填）与收件箱（金额未填/待识别）", () => {
    const pushed = pushedKeys();
    const all = new Set<string>();
    for (const keys of pushed.values()) keys.forEach((k) => all.add(k));
    for (const k of ["counterparty", "tags", "counterpartyEmpty", "amountEmpty", "ocrStatus"]) {
      expect([...all], `缺少下钻源覆盖：${k}`).toContain(k);
    }
  });
});
