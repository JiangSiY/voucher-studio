// @vitest-environment node
/**
 * 死代码守卫（用户 2026-10-16：「清除死代码」）。
 *
 * 依据：`src/components/StatCard.vue` 全仓零引用（`stat-grid` / `.stat*` 一整套样式跟着它一起闲置）、
 * `src/composables/dirtyGuard.ts` 只有自己的测试在引用——两处都是"写了但没接线"，谁也不会发现。
 * 这里把「组件 / composable / store 必须真的被别处引用」固化成门禁：新写一个文件忘了接线，
 * 测试会直接点名，而不是等到某次清理时才被翻出来。
 *
 * 判据：其他源文件里出现该文件的**文件名**（`import X from "../components/Foo.vue"`）
 * 或**文件名主干**（`import { useFoo } from "../composables/foo"`）即算被引用。
 * 页面（`src/pages`）不在此守卫范围：它们由 `registrationContract.test.ts` 按 router/侧栏/命令面板三处登记来管。
 */
import { readdirSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const ROOTS = ["src/components", "src/composables", "src/stores"];

function read(rel: string): string {
  return readFileSync(new URL(`../../${rel}`, import.meta.url), "utf8");
}

function walk(dir: string): string[] {
  const out: string[] = [];
  for (const e of readdirSync(new URL(`../../${dir}`, import.meta.url), { withFileTypes: true })) {
    const rel = `${dir}/${e.name}`;
    if (e.isDirectory()) {
      if (e.name !== "__tests__") out.push(...walk(rel));
    } else if (/\.(ts|vue)$/.test(e.name)) out.push(rel);
  }
  return out;
}

describe("死代码", () => {
  it("components / composables / stores 里的每个文件都被别处引用（写了就要接线）", () => {
    // 引用方必须是**全部**源码（`src/layouts` 这类目录也常是组件/组合式函数的唯一调用点；
    // 少了它们会误报 SidebarResizeHandle.vue 这种在布局里引用的文件）
    const all = walk("src");
    const sources = new Map(all.map((rel) => [rel, read(rel)]));
    const orphans: string[] = [];
    for (const rel of sources.keys()) {
      if (!ROOTS.some((r) => rel.startsWith(`${r}/`))) continue;
      const base = rel.split("/").pop()!;
      const stem = base.replace(/\.(ts|vue)$/, "");
      const referenced = [...sources.entries()].some(([other, otherText]) => {
        if (other === rel) return false;
        if (otherText.includes(base)) return true;
        return new RegExp(`\\b${stem}\\b`).test(otherText);
      });
      if (!referenced) orphans.push(rel);
    }
    expect(orphans, `这些文件没有任何引用（死代码，删掉或接线）：\n${orphans.join("\n")}`).toEqual([]);
  });
});
