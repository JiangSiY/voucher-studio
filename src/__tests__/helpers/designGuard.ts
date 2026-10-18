/**
 * 契约测试共用的「读源码」工具（2026-10-18，契约去重）。
 *
 * 背景：`tokenContract` / `designScalesContract` / `visualConsistencyContract` /
 * `controlSizeContract` / `deadCodeContract` 各自复制了一遍 `repoFile` / 扫目录 / 取 `<style>` 块，
 * 一份写法漂移就要改五处。统一收在这里；新增契约只 import，不再复制。
 *
 * 约定：**只读**——这些工具给「读源码文本做断言」的契约用例使用，不参与运行时代码。
 * 扫描一律排除 `__tests__`（测试自己的字面量不该被守卫判断据）。
 */
import { readdirSync, readFileSync } from "node:fs";

/** 读仓库内相对路径的文件（`rel` 相对仓库根） */
export function repoFile(rel: string): string {
  return readFileSync(new URL(`../../../${rel}`, import.meta.url), "utf8");
}

/**
 * 递归列出目录下的文件（相对路径，按字典序），跳过 `__tests__` 与不匹配的扩展名。
 * @param dir 仓库内相对目录，如 `"src"`
 * @param exts 允许的扩展名，如 `[".vue", ".css"]`
 */
export function walkFiles(dir: string, exts: string[]): string[] {
  const out: string[] = [];
  const walk = (rel: string) => {
    for (const e of readdirSync(new URL(`../../../${rel}`, import.meta.url), { withFileTypes: true })) {
      const child = `${rel}/${e.name}`;
      if (e.isDirectory()) {
        if (e.name !== "__tests__") walk(child);
      } else if (exts.some((x) => e.name.endsWith(x))) {
        out.push(child);
      }
    }
  };
  walk(dir);
  return out.sort();
}

/** `src` 下全部 `.vue` 文件（相对仓库根，含 `src/` 前缀），排除测试目录 */
export function srcVueFiles(): string[] {
  return walkFiles("src", [".vue"]);
}

/** `src` 下全部源码文件（相对仓库根），排除测试目录；`exts` 形如 `[".vue", ".css", ".ts"]` */
export function srcSourceFiles(exts: string[]): string[] {
  return walkFiles("src", exts);
}

/** 取出文本里所有 `<style ...>…</style>` 块并拼接（scoped / 普通都算）；无则空串 */
export function styleBlocks(text: string): string {
  return [...text.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map((m) => m[1]).join("\n");
}

/** 取 `src/<rel>` 的样式文本：`.vue` 取 `<style>` 块，`.css` 整篇即样式 */
export function srcStyleText(rel: string): string {
  const text = repoFile(`src/${rel}`);
  return rel.endsWith(".css") ? text : styleBlocks(text);
}

/**
 * 取 CSS 里某条规则块（从选择器行到配对 `}`，只支持无嵌套的单层块）。
 * 找不到时返回空串——调用方自己断言（通常 `expect(rule).not.toBe("")`）。
 */
export function cssRule(css: string, selector: string): string {
  const idx = css.indexOf(`\n${selector} {`);
  if (idx < 0) return "";
  const end = css.indexOf("}", idx);
  return end < 0 ? "" : css.slice(idx, end);
}

/** `src` 下源码文件（相对 `src/` 的路径，**无** `src/` 前缀），排除测试目录；便于 `repoFile(`src/${rel}`)` 的旧写法迁移 */
export function srcFilesRelative(exts: string[]): string[] {
  return walkFiles("src", exts).map((f) => f.slice(4));
}
