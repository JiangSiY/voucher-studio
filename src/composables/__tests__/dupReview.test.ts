import { describe, expect, it } from "vitest";
import type { DupDecision, PairEvidence, Voucher } from "../../api/client";
import {
  EVIDENCE_META,
  compareRows,
  decisionSummary,
  decisionTag,
  diffCount,
  evidenceMeta,
  evidenceReasons,
  groupHeadline,
  pairKey,
} from "../dupReview";

function v(over: Partial<Voucher> = {}): Voucher {
  return {
    id: 1,
    voucherNo: "记-2026-08-001",
    voucherDate: "2026-08-05",
    description: "租金",
    counterparty: "甲物业",
    amountCents: 123_456,
    status: "draft",
    tags: [],
    note: "",
    createdAt: "2026-08-05T10:00:00",
    updatedAt: "2026-08-05T10:00:00",
    ...over,
  } as Voucher;
}

function ev(over: Partial<PairEvidence> = {}): PairEvidence {
  return { sameImage: false, sameCounterparty: false, sameDescription: false, level: "low", ...over };
}

describe("evidenceReasons / evidenceMeta", () => {
  it("同图是最高证据（单独出现也说明白）", () => {
    expect(evidenceReasons(ev({ sameImage: true, level: "high" }))).toEqual(["图片内容完全相同"]);
  });

  it("同对方 + 同摘要并列列出", () => {
    expect(evidenceReasons(ev({ sameCounterparty: true, sameDescription: true, level: "high" }))).toEqual([
      "对方账户相同",
      "摘要相同",
    ]);
  });

  it("仅判定口径命中时明确说明「仅同日期同金额」，不留空白", () => {
    expect(evidenceReasons(ev())).toEqual(["仅同日期同金额"]);
  });

  it("三档都有展示口径，未知档位兜底为低（后端将来加档不出现空白标签）", () => {
    for (const lv of ["high", "medium", "low"] as const) {
      expect(EVIDENCE_META[lv].label).toBeTruthy();
      expect(EVIDENCE_META[lv].tag).toBeTruthy();
    }
    expect(evidenceMeta("unknown-level").label).toBe("低");
  });
});

describe("compareRows", () => {
  it("日期与金额必然同值（判定口径本身），并如实标为「相同」", () => {
    const rows = compareRows(v(), v({ id: 2, voucherNo: "记-2026-08-002" }));
    const byKey = Object.fromEntries(rows.map((r) => [r.key, r]));
    expect(byKey.voucherDate.same).toBe(true);
    expect(byKey.amountCents.same).toBe(true);
    // 编号不同 → 差异行（提示要用它区分两张）
    expect(byKey.voucherNo.same).toBe(false);
  });

  it("字段顺序固定且齐全（页面直接照序渲染）", () => {
    expect(compareRows(v(), v()).map((r) => r.key)).toEqual([
      "voucherNo",
      "voucherDate",
      "amountCents",
      "description",
      "counterparty",
      "status",
      "tags",
      "note",
    ]);
  });

  it("空值统一显示为 —（避免空单元格看不出有没有内容）", () => {
    const rows = compareRows(
      v({ description: "  ", counterparty: "", tags: [] }),
      v({ id: 2, description: "", counterparty: "", tags: [] }),
    );
    const byKey = Object.fromEntries(rows.map((r) => [r.key, r]));
    expect(byKey.description.left).toBe("—");
    expect(byKey.counterparty.left).toBe("—");
    expect(byKey.tags.left).toBe("—");
    // 两侧都是空 → 同值（不是"不同"）
    expect(byKey.counterparty.same).toBe(true);
    expect(byKey.description.same).toBe(true);
  });

  it("金额按分转元格式化为同一文本（123456 分两侧同为 1,234.56）", () => {
    const rows = compareRows(v({ amountCents: 123_456 }), v({ amountCents: 123_456 }));
    const amount = rows.find((r) => r.key === "amountCents")!;
    expect(amount.left).toBe("1,234.56");
    expect(amount.same).toBe(true);
  });

  it("diffCount 只数不同项", () => {
    const rows = compareRows(
      v(),
      v({ id: 2, voucherNo: "记-2026-08-002", description: "水电", counterparty: "乙" }),
    );
    expect(diffCount(rows)).toBe(3); // 编号 + 摘要 + 对方
  });
});

describe("decisionSummary / decisionTag", () => {
  const base: DupDecision = {
    id: 1,
    aId: 1,
    bId: 2,
    decision: "duplicate_removed",
    keepId: 1,
    removedId: 2,
    note: "",
    createdAt: "2026-08-05T10:00:00",
  };

  it("删除类记录说清谁留谁删（留痕里带保留侧编号）", () => {
    const d: DupDecision = {
      ...base,
      removedPayload: { voucher: v({ voucherNo: "记-2026-08-002" }), keptVoucherNo: "记-2026-08-001" },
    };
    expect(decisionSummary(d)).toBe("保留 记-2026-08-001 · 删除 记-2026-08-002");
  });

  it("留痕缺保留侧编号时回退成 #id，不显示空白", () => {
    const d: DupDecision = { ...base, removedPayload: { voucher: v({ voucherNo: "记-2026-08-002" }) } };
    expect(decisionSummary(d)).toBe("保留 #1 · 删除 记-2026-08-002");
  });

  it("「不是重复」明确标注未改动凭证（用户最关心有没有被删）", () => {
    const d: DupDecision = { ...base, decision: "not_duplicate", keepId: null, removedId: null };
    expect(decisionSummary(d)).toBe("不是重复（未改动凭证）");
    expect(decisionTag(d).label).toBe("不是重复");
  });

  it("两类结论的标签配色不同（删除是危险色）", () => {
    expect(decisionTag({ ...base }).tag).toBe("tag-danger");
    expect(decisionTag({ ...base, decision: "not_duplicate" }).tag).not.toBe("tag-danger");
  });
});

describe("groupHeadline / pairKey", () => {
  it("组头一句说清「哪天 · 多少钱 · 几张」", () => {
    expect(groupHeadline({ voucherDate: "2026-08-05", amountCents: 123_456, vouchers: [v(), v()] })).toBe(
      "2026-08-05 · ¥1,234.56 × 2 张",
    );
  });

  it("pairKey 与后端 normalize_pair 同口径（小的在前，正反向同键）", () => {
    expect(pairKey(9, 3)).toBe("3-9");
    expect(pairKey(3, 9)).toBe("3-9");
  });
});
