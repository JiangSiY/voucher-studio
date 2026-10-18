// @vitest-environment node
/**
 * 术语与页面命名守卫（用户 2026-10-16：「做术语统一」）。
 *
 * **一个东西只有一个名字**——侧栏条目标签、工具栏标题（`router` 的 `meta.title`）、
 * 正文与帮助页文案必须用同一套词：
 *
 * | 页面 | 唯一名称 | 已废弃的旧称 |
 * |---|---|---|
 * | `/inbox` | 待处理 | 收件箱（界面上） |
 * | `/vouchers` | 全部凭证 | 资料库、凭证列表 |
 * | `/print` | 打印与导出 | 打印中心 |
 *
 * 旧称仍在**代码注释**里作为架构名/历史说明出现（`inbox` store、`InboxPage.vue` 的
 * 文件名与注释等），所以断言前先剥掉注释再找——注释里写历史是允许的，用户看得见的地方不行。
 */
import { readdirSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { repoFile } from "./helpers/designGuard";

/** 剥掉 HTML / 块 / 行注释（行注释要求前面是行首或空白，避免误伤 `https://` 这类内容） */
function stripComments(text: string): string {
  return text
    .replace(/<!--[\s\S]*?-->/g, "")
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/(^|\s)\/\/[^\n]*/g, "$1");
}

/** 收集 src 下所有面向用户的源码（排除测试：测试里可以引用旧名做断言） */
function userFacingFiles(dir = "src"): string[] {
  const out: string[] = [];
  for (const e of readdirSync(new URL(`../../${dir}`, import.meta.url), { withFileTypes: true })) {
    const rel = `${dir}/${e.name}`;
    if (e.isDirectory()) out.push(...userFacingFiles(rel));
    // 测试文件排除在外：它们需要引用旧名来做「旧称已清」这类断言
    else if (/\.(ts|vue)$/.test(e.name) && !/\.test\./.test(e.name)) out.push(rel);
  }
  return out;
}

const LAYOUT = "src/layouts/WorkbenchLayout.vue";
const ROUTER = "src/router/index.ts";

/** 侧栏条目：`{ to: "/x", ... label: "名字" }`（同一行内） */
function navLabels(): Map<string, string> {
  const src = repoFile(LAYOUT);
  const map = new Map<string, string>();
  for (const m of src.matchAll(/\{\s*to:\s*"(\/[^"]*)",[\s\S]*?label:\s*"([^"]+)"/g)) {
    map.set(m[1], m[2]);
  }
  return map;
}

/** 路由标题：`{ path: "x", meta: { title: "名字" } }`——Prettier 后两者常不在同一行，按 path 切段解析 */
function routeTitles(): Map<string, string> {
  const src = repoFile(ROUTER);
  const map = new Map<string, string>();
  for (const chunk of src.split(/path:\s*"/).slice(1)) {
    const path = chunk.slice(0, chunk.indexOf('"'));
    const title = chunk.match(/title:\s*"([^"]+)"/);
    if (title) map.set(path.startsWith("/") ? path : `/${path}`, title[1]);
  }
  return map;
}

describe("术语统一", () => {
  it("用户可见文案里不得再出现旧称（资料库 / 凭证列表 / 打印中心 / 收件箱）", () => {
    const legacy = ["资料库", "凭证列表", "打印中心", "收件箱"];
    const offenders: string[] = [];
    for (const rel of userFacingFiles()) {
      const text = stripComments(repoFile(rel));
      for (const term of legacy) {
        for (const [i, line] of text.split("\n").entries()) {
          if (line.includes(term))
            offenders.push(`${rel}:${i + 1} 出现旧称「${term}」：${line.trim().slice(0, 80)}`);
        }
      }
    }
    expect(offenders, `这些地方仍在用旧称：\n${offenders.join("\n")}`).toEqual([]);
  });

  it("工具栏标题与侧栏条目同名（同一页面两个叫法最伤记忆）", () => {
    const nav = navLabels();
    const titles = routeTitles();
    expect(nav.size).toBe(9);
    const mismatched: string[] = [];
    for (const [path, label] of nav) {
      const title = titles.get(path);
      if (title !== label)
        mismatched.push(`${path}：侧栏「${label}」 vs 标题「${title ?? "(缺 meta.title)"}」`);
    }
    expect(mismatched, `侧栏与标题不一致：\n${mismatched.join("\n")}`).toEqual([]);
  });

  it("侧栏九项用的是定稿名称与顺序（顺序即分区，不能随手改名）", () => {
    expect([...navLabels().values()]).toEqual([
      "待处理",
      "全部凭证",
      "重复核查",
      "洞察",
      "打印与导出",
      "导出历史",
      "设置",
      "备份与恢复",
      "帮助",
    ]);
  });
});
