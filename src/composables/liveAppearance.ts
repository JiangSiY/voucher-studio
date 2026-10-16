import type { AppConfig } from "../api/client";

/**
 * 把「不属于设置草稿」的运行期外观项用当前值覆盖回配置对象。
 *
 * `appearance.sidebarWidth`（拖拽手柄维护）与 `appearance.sidebarCollapsed`（侧栏底部开关维护）
 * 都**不在设置页里编辑**，草稿里装的是进页时的旧快照；而设置页与备份 Tab 都会整包提交草稿
 * （备份 Tab 的「测试连接」也会先落盘一次配置）。不覆盖就会出现
 * 「拖过侧栏 / 收起过侧栏 → 改任一设置（或点一次测试连接）→ 保存」把这两项一起回退。
 *
 * **为什么是纯函数而不是 store 方法**：pinia 的 store 实例按 id 缓存在 pinia 上，
 * dev 下改 store 源码（新增/改名方法）时 HMR 只换了 defineStore 的定义、**没有换实例**，
 * 页面会拿到「没有新方法的旧实例」——用户实测就报
 * `configStore.withLiveAppearance is not a function`（改主题时炸在 save 里）。
 * 纯函数 import 不受实例缓存影响，两个调用方共用同一入口，也就不必给每个 store 记得加
 * `acceptHMRUpdate` 才能改。
 */
export function withLiveAppearance(
  cfg: AppConfig,
  live: AppConfig["appearance"] | null | undefined,
): AppConfig {
  return {
    ...cfg,
    appearance: {
      ...cfg.appearance,
      sidebarWidth: live?.sidebarWidth ?? cfg.appearance.sidebarWidth,
      sidebarCollapsed: live?.sidebarCollapsed ?? cfg.appearance.sidebarCollapsed,
    },
  };
}
