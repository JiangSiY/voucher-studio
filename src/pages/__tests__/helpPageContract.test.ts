// @vitest-environment node
/**
 * 帮助页的两条底线（2026-10-16 界面细节审查）：
 *
 * 1. **只列已实装的快捷键**。规范 §2.3 里的 ⌘⇧Enter（复核通过并下一张）仍是规划项、
 *    代码里没有实现（还会被 ⌘Enter 分支吞掉）——帮助页写了按下去没反应，比不写更伤；
 *    ⌥←/→（上一张/下一张）2026-10-16 已实装，故反过来断言「必须有」。
 * 2. **入口名与侧栏一致**（旧称「凭证列表」「打印中心」已随 v0.16–v0.20 改名），
 *    且平台说明不再出现 Windows 残留（自 v0.17.0 起仅 macOS）。
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const page = readFileSync(new URL("../HelpPage.vue", import.meta.url), "utf8");
const layout = readFileSync(new URL("../../layouts/WorkbenchLayout.vue", import.meta.url), "utf8");

/** 只看 `shortcuts` 数组本体：脚本里的说明性注释允许提到未实装项 */
const shortcutList = page.slice(
  page.indexOf("const shortcuts"),
  page.indexOf("];", page.indexOf("const shortcuts")),
);

describe("帮助页：快捷键只列已实装", () => {
  it("列出实际存在的快捷键", () => {
    for (const keys of ["⌘K", "⌘S", "⌘Enter", "⌘⇧E", "Esc"]) {
      expect(shortcutList).toContain(keys);
    }
  });

  it("⌥←/→ 已实装故必须列出（2026-10-16 实装）", () => {
    expect(shortcutList).toContain("⌥← ⌥→");
  });

  it("仍不列未实装项（⌘⇧Enter 复核通过并下一张）", () => {
    expect(shortcutList).not.toContain("⌘⇧Enter");
  });
});

describe("帮助页：入口名与侧栏一致", () => {
  it("用当前入口名，不用旧称", () => {
    for (const name of ["全部凭证", "打印与导出", "备份与恢复", "重复核查", "导出历史"]) {
      expect(page).toContain(name);
    }
    expect(page).not.toContain("凭证列表");
    expect(page).not.toContain("打印中心");
  });

  it("入口清单与侧栏文案同源（每个 label 都能在帮助页找到）", () => {
    const labels = [...layout.matchAll(/label:\s*"([^"]+)"/g)].map((m) => m[1]);
    expect(labels.length).toBeGreaterThan(0);
    for (const label of labels) {
      expect(page, `帮助页缺少入口「${label}」的说明`).toContain(label);
    }
  });

  it("平台说明为 macOS（无 Windows 残留），并记录侧栏收起与搜索上限", () => {
    expect(page).not.toContain("Windows 凭据管理器");
    expect(page).toContain("macOS 钥匙串");
    expect(page).toContain("收起侧边栏");
    expect(page).toContain("100 条");
  });
});
