// @vitest-environment node
/**
 * CSS 令牌契约守卫：**被引用的自定义属性必须有定义**（或有 fallback）。
 *
 * 背景（2026-09-23 审计 A6）：命令面板、裁剪框、联想下拉等组件引用了
 * `--bg-1` / `--bg-2` / `--bg-3` / `--surface-1` / `--text-1` / `--border` 这批早期命名，
 * 而 `tokens.css` 里从未定义过它们——有 fallback 的地方恒取浅色（深色模式下命令面板
 * 成了 #fff 白底 + 继承来的白字 = 完全不可读），无 fallback 的（如 `--border`）
 * 退化成 `currentColor`。修复方式是在 tokens.css 末尾加**单向别名**（新命名仍是唯一真源）。
 *
 * 本测试是**回归锁**：把「谁被引用、谁有定义」固化成断言，防止再出现未定义令牌。
 *
 * ⚠️ **已知盲区**：它对 `var(--bg-1, #fff)` 这类「有 fallback 但 fallback 与主题无关」
 * 的写法不敏感（那正是 A6 的原始形态）。要覆盖这类，需要"fallback 不得是字面量颜色"级别的
 * 约定，会误伤大量合理用法，故本测试只守"完全没有定义"这一档。
 */
import { readFileSync, readdirSync } from "node:fs";
import { describe, expect, it } from "vitest";

function repoFile(rel: string): string {
  return readFileSync(new URL(`../../${rel}`, import.meta.url), "utf8");
}

/** 运行时由 JS 写入、不来自 tokens.css 的自定义属性（见 useSidebarResize.ts 的 setProperty） */
const RUNTIME_VARS = ["--sidebar-w"];

/** tokens.css 里定义的全部令牌（行首 `--name:`，不匹配注释里的提及） */
function definedTokens(): Set<string> {
  const css = repoFile("src/styles/tokens.css");
  const out = new Set<string>();
  for (const line of css.split("\n")) {
    const m = line.match(/^\s*(--[A-Za-z0-9-]+)\s*:/);
    if (m) out.add(m[1]);
  }
  return out;
}

/** `src/**` 里被引用的令牌：name → 是否存在**无 fallback** 的用法 */
function usedTokens(): Map<string, string[]> {
  const root = new URL("../../src", import.meta.url);
  const files = readdirSync(root, { recursive: true })
    .map((f) => String(f))
    .filter((f) => /\.(vue|css|ts)$/.test(f) && !f.includes("__tests__"));
  const out = new Map<string, string[]>();
  for (const rel of files) {
    const text = repoFile(`src/${rel}`);
    // var(--x) 或 var(--x, fallback)
    for (const m of text.matchAll(/var\(\s*(--[A-Za-z0-9-]+)\s*([,)])/g)) {
      const [, name, next] = m;
      const hasFallback = next === ",";
      if (hasFallback) continue; // 有 fallback 的引用不在本测试的判据内（见文件头"已知盲区"）
      const list = out.get(name) ?? [];
      list.push(`src/${rel}`);
      out.set(name, list);
    }
  }
  return out;
}

describe("CSS 令牌契约（引用必须有定义）", () => {
  const defined = definedTokens();
  const used = usedTokens();

  it("tokens.css 定义了基础令牌（防止误读空文件导致测试恒绿）", () => {
    expect(defined.size).toBeGreaterThan(40);
    for (const must of ["--surface", "--text", "--stroke", "--accent", "--sidebar-w"]) {
      expect(defined.has(must), `tokens.css 应定义 ${must}`).toBe(true);
    }
  });

  it("无 fallback 的引用都有定义（或在 RUNTIME_VARS 白名单里）", () => {
    const missing: string[] = [];
    for (const [name, files] of used) {
      if (defined.has(name) || RUNTIME_VARS.includes(name)) continue;
      missing.push(`${name}（${[...new Set(files)].slice(0, 3).join(", ")}）`);
    }
    expect(missing, `以下令牌被引用但从未定义：\n${missing.join("\n")}`).toEqual([]);
  });

  it("别名链的目标也有定义（`--x: var(--y)` 的 --y 不能悬空）", () => {
    const css = repoFile("src/styles/tokens.css");
    const dangling: string[] = [];
    for (const line of css.split("\n")) {
      const m = line.match(/^\s*(--[A-Za-z0-9-]+)\s*:\s*var\(\s*(--[A-Za-z0-9-]+)/);
      if (!m) continue;
      const [, owner, target] = m;
      if (!defined.has(target) && !RUNTIME_VARS.includes(target)) {
        dangling.push(`${owner} → ${target}`);
      }
    }
    expect(dangling, `别名目标未定义：\n${dangling.join("\n")}`).toEqual([]);
  });
});
