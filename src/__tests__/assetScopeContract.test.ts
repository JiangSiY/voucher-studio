// @vitest-environment node
/**
 * assetProtocol scope ↔ 附件目录一致性守卫（**分平台**）。
 *
 * 背景一（2026-09-17）：scope 少了 `data` 一层 → Windows 上图片请求全被拒。
 * 背景二（2026-09-23，macOS 首次打包）：反过来了——`data` 层只在 Windows 存在
 * （`ProjectDirs::data_dir()` 只对 Windows 追加 `data`；macOS 是
 * `~/Library/Application Support/<app>`，没有 `data`），于是 macOS 上 scope 匹配不到任何
 * 附件 → 所有 `convertFileSrc` 请求被 asset protocol 拒绝 → 缩略图/预览**全站空白**。
 *
 * 结论：scope 必须**按平台写**——基础 `tauri.conf.json` 用 macOS/Linux 形态（无 `data`），
 * `tauri.windows.conf.json` 覆盖为含 `data` 的形态。
 * 注意 Tauri 的平台配置按 JSON Merge Patch 合并，**数组是整体替换**，
 * 所以 Windows 侧要把 scope 数组写全（而不是只写增量）。
 *
 * 本测试把「两份配置的 scope 各自等于其平台的数据根形态」以及
 * 「前端不得自行拼附件路径」固化成断言：改 `paths.rs` 的项目目录名、或改任一份 scope
 * 而不同步另一方、或有人把相对路径拼接写回前端时，这里会红。
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

function repoFile(rel: string): string {
  return readFileSync(new URL(`../../${rel}`, import.meta.url), "utf8");
}

const pathsSrc = repoFile("crates/voucher-core/src/utils/paths.rs");
const baseConf = repoFile("src-tauri/tauri.conf.json");
const winConf = repoFile("src-tauri/tauri.windows.conf.json");

/** 从 paths.rs 提取 ProjectDirs::from 的应用名（附件目录的根段） */
function projectDirName(): string {
  const m = pathsSrc.match(/ProjectDirs::from\(\s*"[^"]*"\s*,\s*"[^"]*"\s*,\s*"([^"]+)"\s*\)/);
  if (!m) throw new Error("paths.rs 里找不到 ProjectDirs::from(...)");
  return m[1];
}

/** 提取某份配置里的 assetProtocol.scope 数组（未覆盖 scope 时返回空数组） */
function assetScopes(src: string): string[] {
  const m = src.match(/"assetProtocol"[\s\S]*?"scope"\s*:\s*\[([^\]]*)\]/);
  if (!m) return [];
  return m[1]
    .split(",")
    .map((s) => s.trim().replace(/^"|"$/g, ""))
    .filter(Boolean);
}

const app = projectDirName();

describe("assetProtocol scope ↔ 附件目录一致性（分平台）", () => {
  it("基础配置 = macOS/Linux 形态：$DATA/<app>/attachments/**/*（无 data 层）", () => {
    const scopes = assetScopes(baseConf);
    expect(scopes).toContain(`$DATA/${app}/attachments/**/*`);
    expect(scopes).not.toContain(`$DATA/${app}/data/attachments/**/*`);
  });

  it("Windows 覆盖 = 含 data 的形态：$DATA/<app>/data/attachments/**/*", () => {
    const scopes = assetScopes(winConf);
    expect(scopes).toContain(`$DATA/${app}/data/attachments/**/*`);
    expect(scopes).not.toContain(`$DATA/${app}/attachments/**/*`);
  });

  it("两份 scope 都收窄到附件子树（不得回退为 ** 或整盘/家目录）", () => {
    for (const scopes of [assetScopes(baseConf), assetScopes(winConf)]) {
      expect(scopes.length).toBeGreaterThan(0);
      expect(scopes).not.toContain("**");
      expect(scopes.some((s) => s.includes("$HOME") || s.includes("$DATA/**"))).toBe(false);
      expect(scopes.every((s) => s.includes("/attachments/"))).toBe(true);
    }
  });

  it("前端一律消费后端下发的绝对路径（不得自行拼分隔符）", () => {
    const sources = [
      "src/components/voucher/AttachmentPanel.vue",
      "src/pages/VoucherListPage.vue",
      "src/pages/PrintCenterPage.vue",
      "src/composables/useAttachmentFallback.ts",
    ];
    for (const rel of sources) {
      const s = repoFile(rel);
      // 已废弃的「前端拼附件目录」写法不得回归
      expect(s, rel).not.toContain("attachmentsDir");
      // 历史上的 / → \ 替换（macOS 上拼出不存在路径的元凶）
      expect(s, rel).not.toMatch(/replace\(\/\\?\//);
      expect(s, rel).toContain("Abs");
    }
  });
});
