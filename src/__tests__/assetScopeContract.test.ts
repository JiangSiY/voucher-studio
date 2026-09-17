// @vitest-environment node
/**
 * assetProtocol scope 与真实附件目录的一致性守卫。
 *
 * 背景（2026-09-17 发现）：scope 曾为 `$DATA/VoucherStudio/attachments/` 下的深层通配（少了 `data` 一层）。
 * 真实附件目录由 `paths.rs` 的 `ProjectDirs::from("", "", "VoucherStudio").data_dir()` 决定
 * （Windows = `%APPDATA%\VoucherStudio\data`），而 `$DATA` 在 Windows 解析为 `%APPDATA%`，
 * 于是该 scope 匹配不到任何附件 → 所有 `convertFileSrc` 请求被 asset protocol 拒绝，
 * 调试日志里逐条出现 `asset protocol not configured to allow the path: ...\data\attachments\...`（缩略图/图片空白）。
 * 该回归由 9/9「收窄 assetProtocol」引入，静态检查看不到（scope 值本身"看起来"是对的）。
 *
 * 本测试把「scope 末段 == 代码里的目录链」固化成断言：改动 `paths.rs` 的项目目录名或 `tauri.conf.json`
 * 的 scope 任一侧而不同步时，这里会红。
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

function repoFile(rel: string): string {
  return readFileSync(new URL(`../../${rel}`, import.meta.url), "utf8");
}

const pathsSrc = repoFile("crates/voucher-core/src/utils/paths.rs");
const confSrc = repoFile("src-tauri/tauri.conf.json");

/** 从 paths.rs 提取 ProjectDirs::from 的应用名（附件目录的根段） */
function projectDirName(): string {
  const m = pathsSrc.match(/ProjectDirs::from\(\s*"[^"]*"\s*,\s*"[^"]*"\s*,\s*"([^"]+)"\s*\)/);
  if (!m) throw new Error("paths.rs 里找不到 ProjectDirs::from(...)");
  return m[1];
}

/** 从 tauri.conf.json 提取 assetProtocol.scope 数组 */
function assetScopes(): string[] {
  const m = confSrc.match(/"assetProtocol"[\s\S]*?"scope"\s*:\s*\[([^\]]*)\]/);
  if (!m) throw new Error("tauri.conf.json 里找不到 assetProtocol.scope");
  return m[1]
    .split(",")
    .map((s) => s.trim().replace(/^"|"$/g, ""))
    .filter(Boolean);
}

describe("assetProtocol scope ↔ 附件目录一致性", () => {
  it("scope 指向 $DATA/<项目名>/data/attachments/**/*（少一层 data 会导致全部图片请求被拒）", () => {
    const app = projectDirName();
    const expected = `$DATA/${app}/data/attachments/**/*`;
    expect(assetScopes()).toContain(expected);
  });

  it("scope 已收窄到附件子树（不得回退为 ** 或整盘/家目录）", () => {
    const scopes = assetScopes();
    expect(scopes).not.toContain("**");
    expect(scopes.some((s) => s.includes("$HOME") || s.includes("$DATA/**"))).toBe(false);
    expect(scopes.every((s) => s.includes("/attachments/"))).toBe(true);
  });
});
