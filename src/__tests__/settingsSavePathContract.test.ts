// @vitest-environment node
/**
 * 设置的两条「整包提交」路径都必须保住运行期外观项。
 *
 * `appearance.sidebarWidth`（侧栏拖拽手柄维护）与 `appearance.sidebarCollapsed`（侧栏底部开关维护）
 * 都不在设置页编辑，草稿里是**进页时的旧快照**。设置页保存与备份 Tab 的「测试连接」
 * 都会整包落盘草稿，任一路径漏覆盖就会出现
 * 「拖过侧栏 / 收起过侧栏 → 保存（或点一次测试连接）」把它回退（2026-10-16 界面细节审查）。
 * 覆盖逻辑收敛到 config store 的 `withLiveAppearance`，两条路径都必须走它。
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const store = readFileSync(new URL("../stores/config.ts", import.meta.url), "utf8");
const settings = readFileSync(new URL("../pages/SettingsPage.vue", import.meta.url), "utf8");
const backupTab = readFileSync(new URL("../components/settings/BackupTab.vue", import.meta.url), "utf8");

describe("侧栏外观项的整包提交覆盖", () => {
  it("config store 提供唯一覆盖入口 withLiveAppearance 并导出", () => {
    expect(store).toContain("function withLiveAppearance(cfg: AppConfig): AppConfig");
    expect(store).toContain("sidebarWidth: cur?.sidebarWidth ?? cfg.appearance.sidebarWidth");
    expect(store).toContain("sidebarCollapsed: cur?.sidebarCollapsed ?? cfg.appearance.sidebarCollapsed");
    expect(store).toMatch(/return \{[\s\S]*withLiveAppearance,/);
  });

  it("设置页保存走 helper（不再内联拼 payload）", () => {
    expect(settings).toContain("configStore.save(configStore.withLiveAppearance(draft.cfg))");
    expect(settings).not.toContain("const payload: AppConfig");
  });

  it("备份 Tab「测试连接」也走 helper，并在提示里讲清会一并保存", () => {
    expect(backupTab).toContain("configStore.save(configStore.withLiveAppearance(draftCfg.value))");
    expect(backupTab).not.toContain("configStore.save(draftCfg.value)");
    expect(backupTab).toContain("当前设置已保存");
  });
});
