// @vitest-environment node
/**
 * 术语守卫（v0.27）：一个东西只有一个名字。
 *
 * 四个页签：工作台 / 数据 / 对账 / 会计投影；辅助功能叫「弹层」，不再叫页面。
 * 旧称（资料库 / 凭证列表 / 打印中心 / 收件箱 / 侧栏）不得出现在新壳层与四个页面里。
 * 旧页面文件在 B4 删除，本守卫只扫活跃文件，避免被待删文件拖红。
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const read = (rel: string) => readFileSync(new URL(`../../${rel}`, import.meta.url), "utf8");

function stripComments(text: string): string {
  return text
    .replace(/<!--[\s\S]*?-->/g, "")
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/(^|\s)\/\/[^\n]*/g, "$1");
}

const ACTIVE = [
  "src/layouts/AppShell.vue",
  "src/components/shell/TopBar.vue",
  "src/components/shell/ModalHost.vue",
  "src/components/dialogs/HelpDialog.vue",
  "src/components/ui/Dialog.vue",
  "src/pages/WorkbenchPage.vue",
  "src/pages/DataPage.vue",
  "src/pages/ReconcilePage.vue",
  "src/pages/AccountingPage.vue",
  "src/pages/LoginPage.vue",
  "src/components/CommandPalette.vue",
];

const router = read("src/router/index.ts");

describe("术语统一", () => {
  it("活跃界面不得再出现旧称（资料库 / 凭证列表 / 打印中心 / 收件箱 / 侧栏）", () => {
    const legacy = ["资料库", "凭证列表", "打印中心", "收件箱", "侧边栏", "侧栏"];
    const offenders: string[] = [];
    for (const rel of ACTIVE) {
      const text = stripComments(read(rel));
      for (const term of legacy) {
        for (const [i, line] of text.split("\n").entries()) {
          if (line.includes(term)) {
            offenders.push(`${rel}:${i + 1} 出现旧称「${term}」：${line.trim().slice(0, 80)}`);
          }
        }
      }
    }
    expect(offenders, `这些地方仍在用旧称：\n${offenders.join("\n")}`).toEqual([]);
  });

  it("四个页签与路由标题同名", () => {
    const topBar = read("src/components/shell/TopBar.vue");
    const tabs = [...topBar.matchAll(/to:\s*"(\/[^"]*)",\s*label:\s*"([^"]+)"/g)].map((m) => [
      m[1],
      m[2],
    ]);
    expect(tabs).toEqual([
      ["/workbench", "工作台"],
      ["/data", "数据"],
      ["/reconcile", "对账"],
      ["/accounting", "会计投影"],
    ]);
    for (const [path, label] of tabs) {
      const chunk = router.split(`path: "${path.slice(1)}"`)[1] ?? "";
      const title = chunk.match(/title:\s*"([^"]+)"/);
      expect(title?.[1], `${path} 的 meta.title 应为「${label}」`).toBe(label);
    }
  });
});
