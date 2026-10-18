// @vitest-environment node
/**
 * 工具链契约（挂账 #6，2026-10-18 引入 Prettier）：
 * 排版只有 Prettier 一个执行者，且它**必须在门禁里**——否则「有人手调格式」和
 * 「Prettier 版本升级后忘记重排」都会静默发生，直到下一个人跑 format 时炸出大片无关 diff。
 */
import { describe, expect, it } from "vitest";
import { repoFile } from "./helpers/designGuard";

const pkg = JSON.parse(repoFile("package.json")) as {
  scripts: Record<string, string>;
  devDependencies: Record<string, string>;
};

describe("Prettier 接入", () => {
  it("配置文件存在，且关键选项与全量格式化时一致", () => {
    const cfg = JSON.parse(repoFile(".prettierrc.json")) as Record<string, unknown>;
    expect(cfg.printWidth).toBe(100);
    expect(cfg.singleQuote).toBe(false);
    expect(cfg.semi).toBe(true);
    expect(cfg.endOfLine).toBe("lf");
  });

  it("忽略清单覆盖「不该被格式化的东西」（文档 / 锁文件 / 构建产物）", () => {
    const ignore = repoFile(".prettierignore");
    for (const pattern of [
      "node_modules/",
      "dist/",
      "target/",
      "package-lock.json",
      "*.md",
      "docs/",
    ]) {
      expect(ignore, `忽略清单缺少 ${pattern}`).toContain(pattern);
    }
  });

  it("npm 脚本提供 format 与 format:check，且 prettier 是 devDependency", () => {
    expect(pkg.scripts["format"]).toContain("prettier");
    expect(pkg.scripts["format:check"]).toContain("prettier --check");
    expect(pkg.devDependencies["prettier"]).toMatch(/^\^?3\./);
    expect(pkg.devDependencies["eslint-config-prettier"]).toBeTruthy();
  });

  it("ESLint 接 eslint-config-prettier，让两个工具不争同一件事", () => {
    expect(repoFile("eslint.config.js")).toContain("eslint-config-prettier/flat");
  });

  it("CI 与 audit.sh 都把 format:check 当门禁跑（单一真源不漂移）", () => {
    const ci = repoFile(".github/workflows/ci.yml");
    expect(ci.match(/npm run format:check/g)?.length ?? 0).toBeGreaterThanOrEqual(2);
    const audit = repoFile("scripts/audit.sh");
    expect(audit).toContain("npm run format:check");
    expect(audit).toContain("前端五门");
  });
});
