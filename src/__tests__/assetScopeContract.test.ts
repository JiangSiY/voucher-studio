// @vitest-environment node
/**
 * assetProtocol scope ↔ 附件目录一致性守卫（**单平台：macOS**）。
 *
 * 背景（2026-09-17 → 2026-09-23）：scope 曾缺失/多出 `data` 一层——
 * `ProjectDirs::data_dir()` 只对 Windows 追加 `data`，macOS 是
 * `~/Library/Application Support/<app>`（无 `data`）。2026-09-23 首次 macOS
 * 打包时 scope 仍是 Windows 形态，于是 macOS 上匹配不到任何附件 →
 * 所有 `convertFileSrc` 请求被 asset protocol 拒绝 → 缩略图/预览**全站空白**。
 *
 * 当时靠两份平台配置（基础 + `tauri.windows.conf.json`）解决。**自 v0.17.0 起
 * 项目只支持 macOS**，Windows 专属配置与资源已移除，因此 scope 只保留一种形态：
 * `$DATA/<app>/attachments/` 之下的全部层级（macOS/Linux 形态，Linux 用于 CI 的快门禁 job）。
 *
 * 本测试把「scope 等于 macOS 数据根形态」「前端不得自行拼附件路径」「Windows
 * 平台配置/脚本不得回归」固化成断言。
 */
import { existsSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { repoFile } from "./helpers/designGuard";


const pathsSrc = repoFile("crates/voucher-core/src/utils/paths.rs");
const baseConf = repoFile("src-tauri/tauri.conf.json");

/** 从 paths.rs 提取 ProjectDirs::from 的应用名（附件目录的根段） */
function projectDirName(): string {
  const m = pathsSrc.match(/ProjectDirs::from\(\s*"[^"]*"\s*,\s*"[^"]*"\s*,\s*"([^"]+)"\s*\)/);
  if (!m) throw new Error("paths.rs 里找不到 ProjectDirs::from(...)");
  return m[1];
}

/** 提取配置里的 assetProtocol.scope 数组（未配置 scope 时返回空数组） */
function assetScopes(src: string): string[] {
  const m = src.match(/"assetProtocol"[\s\S]*?"scope"\s*:\s*\[([^\]]*)\]/);
  if (!m) return [];
  return m[1]
    .split(",")
    .map((s) => s.trim().replace(/^"|"$/g, ""))
    .filter(Boolean);
}

const app = projectDirName();

describe("assetProtocol scope ↔ 附件目录一致性（单平台）", () => {
  it("scope = macOS 形态：$DATA/<app>/attachments/**/*（无 data 层）", () => {
    const scopes = assetScopes(baseConf);
    expect(scopes).toContain(`$DATA/${app}/attachments/**/*`);
    expect(scopes).not.toContain(`$DATA/${app}/data/attachments/**/*`);
  });

  it("scope 收窄到附件子树（不得回退为 ** 或整盘/家目录）", () => {
    const scopes = assetScopes(baseConf);
    expect(scopes.length).toBeGreaterThan(0);
    expect(scopes).not.toContain("**");
    expect(scopes.some((s) => s.includes("$HOME") || s.includes("$DATA/**"))).toBe(false);
    expect(scopes.every((s) => s.includes("/attachments/"))).toBe(true);
  });

  it("前端一律消费后端下发的绝对路径（不得自行拼分隔符）", () => {
    const sources = [
      "src/components/voucher/AttachmentPanel.vue",
      // v0.18：旧 10 列凭证列表已由「资料库」（双行列表 + 检视器）取代，二者都必须只消费绝对路径
      "src/pages/LibraryPage.vue",
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

  it("Windows 平台配置与脚本不得回归（v0.17.0 起只支持 macOS）", () => {
    const repoRoot = fileURLToPath(new URL("../../", import.meta.url));
    // 平台覆盖配置：Tauri 不会自动发现，历史上靠它做 Windows 形态，现已删除
    expect(existsSync(`${repoRoot}src-tauri/tauri.windows.conf.json`)).toBe(false);
    expect(existsSync(`${repoRoot}src-tauri/tauri.macos.conf.json`)).toBe(false);
    // Windows 开发环境脚本（被 AGENTS.md 引用过，删了必须同步改文档）
    expect(existsSync(`${repoRoot}activate-dev.ps1`)).toBe(false);
    // 常见落点不得再出现 .ps1（不做全仓递归——那会白扫 node_modules/target）
    const dirs = [
      repoRoot,
      `${repoRoot}spikes`,
      `${repoRoot}spikes/ocr-poc`,
      `${repoRoot}spikes/print-poc`,
      `${repoRoot}src-tauri`,
    ];
    const ps1 = dirs
      .filter((d) => existsSync(d))
      .flatMap((d) =>
        readdirSync(d)
          .map((f) => String(f))
          .filter((f) => f.endsWith(".ps1"))
          .map((f) => `${d}${f}`),
      );
    expect(ps1, `不应存在 Windows 脚本：${ps1.join(", ")}`).toEqual([]);
  });
});
