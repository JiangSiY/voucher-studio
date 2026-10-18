// @vitest-environment node
/**
 * 设计尺度契约（U3）：图标尺寸枚举 / 圆角只走令牌 / 间距白名单 / 控件高度只走令牌。
 *
 * 依据：`docs/voucher-studio-audit-2026-10-18.md`（2026-10-18 UI/UX 审美审计）
 * 与 `docs/UI-设计规范-macOS27.md` §1.2 / §1.5。
 *
 * 为什么要有这个契约：审计发现同一界面里图标用了 16 种尺寸、圆角有 2/3/5/8px 四套、
 * 间距出现 13/14/28px 等非节奏值——它们单看不刺眼，叠在一起就是"手工感"。
 * 统一到有限枚举后，新人照抄旁边代码也不会再引入新值。
 */
import { describe, expect, it } from "vitest";
import { repoFile, srcFilesRelative as srcSourceFiles, styleBlocks } from "./helpers/designGuard";

/** 图标尺寸枚举：12 行内小标 / 14 行内与按钮 / 16 工具条 / 18 侧栏 / 24 插图 / 42 空状态 */
const ICON_SIZES = new Set([12, 14, 16, 18, 24, 42]);

/** 间距白名单：4 的倍数为主，保留 macOS 常用的 2/6/10 与 1px 细缝 */
const SPACING_PX = new Set([0, 1, 2, 4, 6, 8, 10, 12, 16, 20, 24, 32, 40, 48, 64]);

describe("图标尺寸枚举", () => {
  it("所有 FluentIcon 的 :size 只能是 12/14/16/18/24/42", () => {
    const bad: string[] = [];
    for (const rel of srcSourceFiles([".vue"])) {
      const text = repoFile(`src/${rel}`);
      for (const m of text.matchAll(/:size="(\d+)"/g)) {
        const n = Number(m[1]);
        if (!ICON_SIZES.has(n)) bad.push(`${rel}: :size="${n}"`);
      }
    }
    expect(bad, `图标尺寸越界（只允许 ${[...ICON_SIZES].join("/")}）：\n${bad.join("\n")}`).toEqual(
      [],
    );
  });
});

describe("圆角只走令牌", () => {
  it("border-radius 只能是 var(--radius-*) 或胶囊/圆形（100px/999px/50%）", () => {
    const bad: string[] = [];
    const files = srcSourceFiles([".vue", ".css"]).filter((f) => f !== "styles/tokens.css");
    for (const rel of files) {
      const file = repoFile(`src/${rel}`);
      const css = rel.endsWith(".vue") ? styleBlocks(file) : file;
      for (const m of css.matchAll(/border-radius:\s*([^;]+);/g)) {
        const value = m[1].trim();
        const ok =
          value.includes("var(--radius-") ||
          value === "100px" ||
          value === "999px" ||
          value === "50%";
        if (!ok) bad.push(`${rel}: border-radius: ${value}`);
      }
    }
    expect(
      bad,
      `圆角字面量（请改用 --radius-small/control/window/panel）：\n${bad.join("\n")}`,
    ).toEqual([]);
  });
});

describe("间距白名单", () => {
  it("padding/margin/gap 的正数像素值必须在白名单内（负值是定位补偿，放行）", () => {
    const bad: string[] = [];
    const files = srcSourceFiles([".vue", ".css"]).filter((f) => f !== "styles/tokens.css");
    for (const rel of files) {
      const file = repoFile(`src/${rel}`);
      const css = rel.endsWith(".vue") ? styleBlocks(file) : file;
      for (const decl of css.matchAll(/(?:padding|margin|gap)(?:-[a-z]+)?:\s*([^;{}]+);/g)) {
        for (const m of decl[1].matchAll(/(?<![\w-])(-?)([0-9.]+)px/g)) {
          if (m[1] === "-") continue; // margin-left: -14px 之类是几何补偿，不参与节奏
          const n = Number(m[2]);
          if (!SPACING_PX.has(n)) bad.push(`${rel}: ${decl[0].trim()}`);
        }
      }
    }
    expect(bad, `间距越界（白名单外的新值请先落令牌）：\n${bad.join("\n")}`).toEqual([]);
  });
});

describe("控件高度只走令牌", () => {
  it("任何样式里都不得写 height: 26/32/36px（控件高度由 --control-h-* 决定）", () => {
    const bad: string[] = [];
    const files = srcSourceFiles([".vue", ".css"]).filter((f) => f !== "styles/tokens.css");
    for (const rel of files) {
      const file = repoFile(`src/${rel}`);
      const css = rel.endsWith(".vue") ? styleBlocks(file) : file;
      for (const m of css.matchAll(/height:\s*(26|32|36)px/g)) bad.push(`${rel}: ${m[0]}`);
    }
    expect(bad, `控件高度字面量（请用 --control-h-sm/md/lg）：\n${bad.join("\n")}`).toEqual([]);
  });

  it("分页按钮也走 --control-h-sm", () => {
    const css = repoFile("src/styles/base.css");
    const idx = css.indexOf("\n.pager button {");
    expect(idx, "base.css 缺少 .pager button").toBeGreaterThan(-1);
    const body = css.slice(idx, css.indexOf("}", idx));
    expect(body).toContain("height: var(--control-h-sm)");
  });
});
