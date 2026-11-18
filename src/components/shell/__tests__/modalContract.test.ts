// @vitest-environment node
/** 弹层契约（v0.27 B4）：五个入口各一项，不重复页内动作。 */
import { describe, expect, it } from "vitest";
import { repoFile } from "../../../__tests__/helpers/designGuard";

const host = repoFile("src/components/shell/ModalHost.vue");

describe("弹层宿主", () => {
  it("五个 modal 各接一个组件", () => {
    expect(host).toContain("HelpDialog");
    expect(host).toContain("SettingsDialog");
    expect(host).toContain("BackupDialog");
    expect(host).toContain("ExportDialog");
    expect(host).toContain("DuplicatesDialog");
  });

  it("导出弹层含导出与历史两片", () => {
    const src = repoFile("src/components/dialogs/ExportDialog.vue");
    expect(src).toContain("PrintCenterPage");
    expect(src).toContain("ExportHistoryPage");
    expect(src).toContain("tab === 'history'");
  });

  it("设置/备份/重复核查复用现有页，避免双份实现", () => {
    expect(repoFile("src/components/dialogs/SettingsDialog.vue")).toContain("SettingsPage");
    expect(repoFile("src/components/dialogs/BackupDialog.vue")).toContain("RestorePage");
    expect(repoFile("src/components/dialogs/DuplicatesDialog.vue")).toContain(
      "DuplicateReviewPage",
    );
  });
});
