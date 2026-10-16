// @vitest-environment node
/**
 * 浮层焦点契约与提示通道的静态守卫（`docs/UI-设计规范-macOS27.md` §4）。
 *
 * 背景（2026-10-16 审查）：三个浮层（确认框 / 裁剪框 / 图片查看器）此前都没有
 * `aria-modal`、没有焦点契约，确认框连 Esc 都关不掉；命令面板则把同一个 keydown
 * 处理器同时挂在 window 和输入框上（↓ 一次跳 2 项、Enter 跑两遍）。
 * 这类"接线"问题组件测试容易漏，故用源码级断言钉住。
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

function src(rel: string): string {
  return readFileSync(new URL(`../../${rel}`, import.meta.url), "utf8");
}

const confirm = src("src/components/ConfirmDialog.vue");
const crop = src("src/components/CropDialog.vue");
const viewer = src("src/components/ImageViewerDialog.vue");
const palette = src("src/components/CommandPalette.vue");

describe("浮层焦点契约", () => {
  it("三个浮层都接了 useModal 并声明 aria-modal", () => {
    for (const [name, s] of [
      ["ConfirmDialog", confirm],
      ["CropDialog", crop],
      ["ImageViewerDialog", viewer],
    ] as const) {
      expect(s, `${name} 应使用 useModal`).toContain('from "../composables/useModal"');
      expect(s, `${name} 应有 aria-modal`).toContain('aria-modal="true"');
      expect(s, `${name} 面板应可编程聚焦`).toContain('tabindex="-1"');
    }
  });

  it("图片查看器自带 Esc，故以 escape:false 接入，避免两次关闭", () => {
    expect(viewer).toContain("escape: false");
  });

  it("命令面板的按键只注册一次（window 与输入框不得各挂一份）", () => {
    expect(palette).toContain("window.addEventListener(\"keydown\", onKeydown)");
    expect(palette).not.toContain('@keydown="onKeydown"');
  });
});
