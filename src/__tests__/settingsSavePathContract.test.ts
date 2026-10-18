// @vitest-environment node
/**
 * 设置的两条「整包提交」路径都必须保住运行期外观项。
 *
 * `appearance.sidebarWidth`（侧栏拖拽手柄维护）与 `appearance.sidebarCollapsed`（侧栏底部开关维护）
 * 都不在设置页编辑，草稿里是**进页时的旧快照**。设置页保存与备份 Tab 的「测试连接」
 * 都会整包落盘草稿，任一路径漏覆盖就会出现
 * 「拖过侧栏 / 收起过侧栏 → 保存（或点一次测试连接）」把它回退（2026-10-16 界面细节审查）。
 *
 * 覆盖逻辑是 `src/composables/liveAppearance.ts` 的**纯函数**，两条路径都必须走它。
 * 为什么不是 store 方法：pinia 实例按 id 缓存，dev 下改 store 只换定义不换实例，
 * 页面会拿到没有新方法的旧实例（用户实报 `configStore.withLiveAppearance is not a function`）。
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const read = (rel: string) => readFileSync(new URL(rel, import.meta.url), "utf8");
const helper = read("../composables/liveAppearance.ts");
const store = read("../stores/config.ts");
const settings = read("../pages/SettingsPage.vue");
const backupTab = read("../components/settings/BackupTab.vue");

describe("侧栏外观项的整包提交覆盖", () => {
  it("覆盖逻辑住在纯函数模块里（不挂在 store 实例上）", () => {
    expect(helper).toContain("export function withLiveAppearance(");
    expect(helper).toContain('live: AppConfig["appearance"] | null | undefined');
    expect(helper).toContain("sidebarWidth: live?.sidebarWidth ?? cfg.appearance.sidebarWidth");
    expect(helper).toContain(
      "sidebarCollapsed: live?.sidebarCollapsed ?? cfg.appearance.sidebarCollapsed",
    );
    // store 里既不该再有这个方法，也不该把它挂上返回对象（否则又会被实例缓存坑一次）
    expect(store).not.toContain("function withLiveAppearance");
    expect(store).not.toMatch(/return \{[\s\S]*withLiveAppearance,/);
  });

  it("config store 接收 HMR（改 store 源码时实例跟着换）", () => {
    expect(store).toContain('import { acceptHMRUpdate, defineStore } from "pinia"');
    expect(store).toContain("acceptHMRUpdate(useConfigStore, import.meta.hot)");
  });

  it("设置页保存走 helper（不再内联拼 payload）", () => {
    expect(settings).toContain(
      "configStore.save(withLiveAppearance(draft.cfg, configStore.config?.appearance))",
    );
    expect(settings).not.toContain("const payload: AppConfig");
    expect(settings).toContain('from "../composables/liveAppearance"');
  });

  it("备份 Tab「测试连接」也走 helper，并在提示里讲清会一并保存", () => {
    expect(backupTab).toContain(
      "configStore.save(withLiveAppearance(draftCfg.value, configStore.config?.appearance))",
    );
    expect(backupTab).not.toContain("configStore.save(draftCfg.value)");
    expect(backupTab).toContain("当前设置已保存");
    expect(backupTab).toContain('from "../../composables/liveAppearance"');
  });
});
