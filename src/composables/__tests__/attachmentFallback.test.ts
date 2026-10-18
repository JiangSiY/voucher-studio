// @vitest-environment node
/**
 * 附件图片候选路径与回退阶梯（纯逻辑）。
 *
 * 场景来源一：2026-09-17 缺陷——备份不含增强版（可重建缓存），恢复快照后
 * DB 的 `enhanced_path` 指向磁盘上不存在的文件，界面若直接照用就是一块灰白。
 * 后端已在下发时剥掉坏路径（`services::attachment_view`），这里的阶梯是第二道保险。
 *
 * 场景来源二（2026-09-23 缺陷，本文件据此改口径）：候选一律用**后端下发的绝对路径**
 * （`fileAbs` / `enhancedAbs`，Rust 侧按平台原生分隔符拼好）。前端历史上自行拼绝对路径
 * 并把 `/` 替换成分隔符，在 macOS 上拼出的是 `...VoucherStudio\attachments\...` 这种
 * 并不存在的路径，被 asset protocol 逐条拒绝 → 缩略图/预览全站空白。这里锁住
 * 「只消费绝对路径、原样透传不做任何替换、字段缺失按占位处理」，避免相对路径拼接回潮。
 */
import { describe, expect, it } from "vitest";
import { candidatePaths, nextFallbackStep, pickCandidate } from "../useAttachmentFallback";

const ABS = "/Users/x/Library/Application Support/VoucherStudio/attachments";

describe("candidatePaths（绝对路径口径）", () => {
  it("有增强版时：增强优先、原图兜底", () => {
    const a = {
      fileAbs: `${ABS}/2026/08/x.jpg`,
      enhancedAbs: `${ABS}/2026/08/x.enhanced.jpg`,
    };
    expect(candidatePaths(a)).toEqual([`${ABS}/2026/08/x.enhanced.jpg`, `${ABS}/2026/08/x.jpg`]);
  });

  it("绝对路径原样透传（不做任何分隔符替换，也不做归一化）", () => {
    const odd = "/Volumes/备份 盘/attachments/2026/08/x.jpg";
    expect(candidatePaths({ fileAbs: odd, enhancedAbs: null })).toEqual([odd]);
    // 含特殊字符/空格的路径同样逐字透传（后端给什么就用什么）
    expect(candidatePaths({ fileAbs: `${ABS}/2026/08/含 空格 & 符号.jpg` })).toEqual([
      `${ABS}/2026/08/含 空格 & 符号.jpg`,
    ]);
  });

  it("增强版缺失（null/undefined/空串）时只用原图", () => {
    expect(candidatePaths({ fileAbs: "a.jpg", enhancedAbs: null })).toEqual(["a.jpg"]);
    expect(candidatePaths({ fileAbs: "a.jpg" })).toEqual(["a.jpg"]);
    expect(candidatePaths({ fileAbs: "a.jpg", enhancedAbs: "   " })).toEqual(["a.jpg"]);
  });

  it("forceOriginal（裁剪基底 / A-B 对比底层）时忽略增强版", () => {
    const a = { fileAbs: "a.jpg", enhancedAbs: "a.enhanced.jpg" };
    expect(candidatePaths(a, true)).toEqual(["a.jpg"]);
  });

  it("增强与原图相同时不重复", () => {
    expect(candidatePaths({ fileAbs: "a.jpg", enhancedAbs: "a.jpg" })).toEqual(["a.jpg"]);
  });

  it("字段全缺 → 空数组（调用方显示占位，绝不自行拼路径）", () => {
    expect(candidatePaths({})).toEqual([]);
    expect(candidatePaths({ fileAbs: null, enhancedAbs: null })).toEqual([]);
  });
});

describe("pickCandidate / nextFallbackStep", () => {
  const a = { fileAbs: "a.jpg", enhancedAbs: "a.enhanced.jpg" };

  it("按步取候选，用尽返回 null（调用方显示占位）", () => {
    expect(pickCandidate(a, 0)).toBe("a.enhanced.jpg");
    expect(pickCandidate(a, 1)).toBe("a.jpg");
    expect(pickCandidate(a, 2)).toBeNull();
    expect(pickCandidate(a, 99)).toBeNull();
  });

  it("只有原图时第 0 步就是原图、第 1 步即用尽", () => {
    const b = { fileAbs: "a.jpg" };
    expect(pickCandidate(b, 0)).toBe("a.jpg");
    expect(pickCandidate(b, 1)).toBeNull();
  });

  it("失败步进不超过候选数（用尽后停在末值，避免越界）", () => {
    expect(nextFallbackStep(0, 2)).toBe(1);
    expect(nextFallbackStep(1, 2)).toBe(2);
    expect(nextFallbackStep(2, 2)).toBe(2);
  });
});
