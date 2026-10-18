// @vitest-environment node
/**
 * CSS 令牌契约守卫：**被引用的自定义属性必须有定义**（或有 fallback），且**旧命名不得回归**。
 *
 * 历史（2026-09-23 审计 A6）：命令面板、裁剪框、联想下拉等组件引用了
 * `--bg-1` / `--bg-2` / `--bg-3` / `--surface-1` / `--text-1` / `--border` 这批早期命名，
 * 而 `tokens.css` 里从未定义过它们——有 fallback 的地方恒取浅色（深色模式下命令面板
 * 成了 #fff 白底 + 继承来的白字 = 完全不可读），无 fallback 的（如 `--border`）退化成
 * `currentColor`。当时先加**单向别名**兜住；2026-09-27（批次 7）把 24 处引用按别名目标
 * 等价替换为语义令牌并**删除了别名块**，同时在此加**禁名断言**：旧名再出现即变红。
 *
 * ⚠️ **已知盲区**：本测试对 `var(--x, #fff)` 这类"有 fallback 但 fallback 与主题无关"的写法
 * 不敏感（那正是 A6 的原始形态）；现在的防线是禁名断言 + 语义令牌本身不带字面量 fallback。
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

/**
 * 已收编的旧命名：2026-09-23 的兼容别名已随引用迁移一并删除（批次 7）。
 * 这些名字**不得再出现在 src 里**（新增引用会让深色主题与主题切换重新出现 A6 类问题）。
 */
const RETIRED_TOKENS = [
  "--bg-1", "--bg-2", "--bg-3", "--surface-1", "--text-1", "--border",
];

/** 全量 var() 引用（含带 fallback 的写法——禁名判据不能漏掉它们） */
function allVarReferences(): string[] {
  const root = new URL("../../src", import.meta.url);
  const files = readdirSync(root, { recursive: true })
    .map((f) => String(f))
    .filter((f) => /\.(vue|css|ts)$/.test(f) && !f.includes("__tests__"));
  const out: string[] = [];
  for (const rel of files) {
    for (const m of repoFile(`src/${rel}`).matchAll(/var\(\s*(--[A-Za-z0-9-]+)/g)) {
      out.push(`${m[1]} @ src/${rel}`);
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

  it("已收编的旧命名不得再被引用（禁名断言）", () => {
    const offenders = allVarReferences().filter((r) =>
      RETIRED_TOKENS.some((name) => r.startsWith(`${name} `)),
    );
    expect(
      offenders,
      `以下位置引用了已收编的旧令牌（请改用语义令牌：--surface / --surface-3 / --text / --stroke）：\n` +
        offenders.join("\n"),
    ).toEqual([]);
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

/**
 * 排版与视觉语言契约（2026-10-18 U1/U2 收编）：
 * - 字号只允许走 `--fs-*`，不得再出现 `font-size: Npx` 字面量（历史上 159 处字面量与令牌双轨，
 *   导致「界面字号」档位只改了约 1/3 界面——见 docs/voucher-studio-audit-2026-10-18.md §1.1）；
 * - 颜色只允许语义令牌，不得再出现 hex 字面量（hex 不被深色主题/提高对比度接管）；
 * - 已下线的旧视觉族不得回归（v0.22 卡片版式与旧列表工具类，见审计 §2.2）。
 */
describe("排版与视觉语言契约（U1/U2 收编）", () => {
  const styleFiles = readdirSync(new URL("../../src", import.meta.url), { recursive: true })
    .map((f) => String(f))
    .filter((f) => /\.(vue|css)$/.test(f) && !f.includes("__tests__") && !f.endsWith("styles/tokens.css"));
  const texts = new Map(styleFiles.map((rel) => [rel, repoFile(`src/${rel}`)] as const));

  it("样式文件里不得再出现 font-size 字面量（必须走 --fs-*）", () => {
    const offenders: string[] = [];
    for (const [rel, text] of texts) {
      text.split("\n").forEach((line, i) => {
        if (/font-size:\s*[0-9.]+px/.test(line)) offenders.push(`src/${rel}:${i + 1} ${line.trim()}`);
      });
    }
    expect(
      offenders,
      `以下位置仍写死字号（改用 --fs-title / --fs-body / --fs-caption）：\n${offenders.join("\n")}`,
    ).toEqual([]);
  });

  it("样式文件里不得再出现 hex 颜色字面量（必须走语义令牌）", () => {
    const offenders: string[] = [];
    for (const [rel, text] of texts) {
      text.split("\n").forEach((line, i) => {
        const trimmed = line.trim();
        if (trimmed.startsWith("/*") || trimmed.startsWith("*") || trimmed.startsWith("//")) return;
        if (/#[0-9a-fA-F]{3,8}\b/.test(trimmed)) offenders.push(`src/${rel}:${i + 1} ${trimmed}`);
      });
    }
    expect(
      offenders,
      `以下位置仍写死颜色（改用 --accent / --label-* / --separator / --bg-* / --surface*）：\n${offenders.join("\n")}`,
    ).toEqual([]);
  });

  it("已下线的旧视觉族不得回归（v0.22 卡片版式 / 旧列表工具类）", () => {
    const css = repoFile("src/styles/base.css");
    for (const dead of ["vcard", "fchip", "filter-chips", "print-grid", "list-thumb", "cell-ellipsis", "card-body", "card-header"]) {
      expect(css.includes(dead), `base.css 不应再定义 ${dead}（旧视觉族，见审计 §2.2）`).toBe(false);
    }
    expect(css).not.toContain("#6366f1");
    expect(css).not.toContain("#8b5cf6");
    expect(css).not.toContain("#0078d4");
  });

  it("字号令牌与 --type-* 同源（都以 --font-scale 为基数）", () => {
    const css = repoFile("src/styles/tokens.css");
    expect(css).toMatch(/--fs-title:\s*calc\(17px \* var\(--font-scale\)\)/);
    expect(css).toMatch(/--fs-body:\s*calc\(15px \* var\(--font-scale\)\)/);
    expect(css).toMatch(/--fs-caption:\s*calc\(13px \* var\(--font-scale\)\)/);
    expect(css).toMatch(/--fs-display:\s*calc\(20px \* var\(--font-scale\)\)/);
  });
});
