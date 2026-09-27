// @vitest-environment node
/**
 * 前端资源清理守卫（审计 A-10，2026-09-27 批次 5）。
 *
 * 背景：组件里 `setTimeout` 不接句柄、`addEventListener` 不配对，是"改动窗口/切页后偶发"
 * 这类问题的常见成因——回调仍持有已卸载组件的 ref、或同一个监听被叠加多次。此前
 * `DescriptionInput.vue`（blur 延迟收起）与 `EnhanceTab.vue`（5 秒撤销态）都没有句柄，
 * `stores/config.ts` 的 matchMedia 监听因用匿名函数**结构上无法移除**。
 *
 * 判据（可豁免，但必须写明理由，见文件末的标记约定）：
 *   1. `src/**` 非测试文件出现 `setTimeout(` ⇒ 同文件必须有 `clearTimeout(`；
 *   2. `src/**` 非测试文件出现 `addEventListener(` ⇒ 同文件必须有 `removeEventListener(`。
 *
 * 豁免不是"关掉检查"，而是在代码里留下 `// 定时器无需清理：<理由>` 这类可被搜到的说明——
 * 与 tokenContract 的 RUNTIME_VARS 白名单同一个思路：例外必须显式。
 */
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const root = fileURLToPath(new URL("../../src", import.meta.url));

/** 递归列出 src 下的源码文件（排除 __tests__ 与 node_modules） */
function sourceFiles(exts: string[]): string[] {
  const out: string[] = [];
  const walk = (dir: string) => {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
      const p = join(dir, e.name);
      if (e.isDirectory()) {
        if (e.name === "__tests__" || e.name === "node_modules") continue;
        walk(p);
      } else if (exts.some((x) => e.name.endsWith(x))) {
        out.push(p);
      }
    }
  };
  walk(root);
  return out;
}

const read = (p: string) => readFileSync(p, "utf8");
const rel = (p: string) => p.slice(p.indexOf("/src/") + 1);

/** 豁免标记要求带理由：`定时器无需清理：…` / `监听无需移除：…`（理由至少 4 字） */
const TIMER_EXEMPT = /定时器无需清理：\S{4,}/;
const LISTENER_EXEMPT = /监听无需移除：\S{4,}/;

describe("前端资源清理：定时器与监听必须可回收（或显式豁免）", () => {
  it("setTimeout 必须有对应的 clearTimeout", () => {
    const offenders = sourceFiles([".ts", ".vue"])
      .filter((p) => read(p).includes("setTimeout("))
      .filter((p) => {
        const src = read(p);
        return !src.includes("clearTimeout(") && !TIMER_EXEMPT.test(src);
      })
      .map(rel);
    expect(
      offenders,
      `这些组件用了 setTimeout 但没有 clearTimeout（卸载/切页后回调仍会执行；` +
        `确实不需要清理的请写注释「定时器无需清理：<理由>」）：\n${offenders.join("\n")}`,
    ).toEqual([]);
  });

  it("addEventListener 必须配对 removeEventListener", () => {
    const offenders = sourceFiles([".ts", ".vue"])
      .filter((p) => read(p).includes("addEventListener("))
      .filter((p) => {
        const src = read(p);
        return !src.includes("removeEventListener(") && !LISTENER_EXEMPT.test(src);
      })
      .map(rel);
    expect(
      offenders,
      `这些文件加了监听但没有移除（组件重挂载或 HMR 时会叠加；` +
        `确实需要全程存活的请写注释「监听无需移除：<理由>」）：\n${offenders.join("\n")}`,
    ).toEqual([]);
  });

  it("豁免标记必须带理由（不能只写标记名）", () => {
    const bare = sourceFiles([".ts", ".vue"])
      .filter((p) => {
        const src = read(p);
        const hasMarker = src.includes("定时器无需清理") || src.includes("监听无需移除");
        if (!hasMarker) return false;
        // 允许"标记 + 冒号 + 理由"的写法；只有光秃秃的标记名（后面没理由）才算违规
        const okTimer = !src.includes("定时器无需清理") || TIMER_EXEMPT.test(src);
        const okListener = !src.includes("监听无需移除") || LISTENER_EXEMPT.test(src);
        return !(okTimer && okListener);
      })
      .map(rel);
    expect(bare, `豁免标记必须写明理由：\n${bare.join("\n")}`).toEqual([]);
  });
});
