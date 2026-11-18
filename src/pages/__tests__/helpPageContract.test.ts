// @vitest-environment node
/**
 * 帮助弹层底线（v0.27）：只列已实装快捷键；页签名与四页签一致；不再提侧栏。
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { flat } from "../../__tests__/helpers/designGuard";

const dialog = readFileSync(
  new URL("../../components/dialogs/HelpDialog.vue", import.meta.url),
  "utf8",
);
const topBar = readFileSync(new URL("../../components/shell/TopBar.vue", import.meta.url), "utf8");

const shortcutList = dialog.slice(
  dialog.indexOf("const shortcuts"),
  dialog.indexOf("];", dialog.indexOf("const shortcuts")),
);

describe("帮助弹层：快捷键只列已实装", () => {
  it("列出实际存在的快捷键", () => {
    for (const keys of ["⌘K", "⌘S", "⌘Enter", "⌘⇧E", "Esc"]) {
      expect(shortcutList).toContain(keys);
    }
  });

  it("⌥←/→ 与 ⌘⇧Enter 已实装故必须列出", () => {
    expect(shortcutList).toContain("⌥← ⌥→");
    expect(shortcutList).toContain("⌘⇧Enter");
  });
});

describe("帮助弹层：与四页签一致", () => {
  it("四个页签都能在帮助里找到说明", () => {
    for (const name of ["工作台", "数据", "对账", "会计投影"]) {
      expect(dialog).toContain(name);
      expect(topBar).toContain(name);
    }
    expect(dialog).not.toContain("凭证列表");
    expect(dialog).not.toContain("打印中心");
  });

  it("平台说明为 macOS，保留搜索上限，且不再提侧栏", () => {
    expect(dialog).not.toContain("Windows 凭据管理器");
    expect(flat(dialog)).toContain("macOS 钥匙串");
    expect(flat(dialog)).toContain("100 条");
    expect(dialog).not.toContain("收起侧边栏");
  });
});
