/**
 * v0.22 关联批契约（源码级断言）。
 *
 * 锁住三条纪律（方案 v2）：
 * 1. **关联 ≠ 抵消**：右栏关联区块不得做任何金额合并/合计展示；
 * 2. **组内合计是派生值**：必须显式标注「派生值」，防止被当成事实金额参与口径；
 * 3. 右栏两个区块挂在**同一个 `shown.id`** 上（深链聚焦/翻张时跟着变），
 *    且都走既有深链聚焦动线（`focusFromLink`），不自己造跳转。
 */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

const read = (p: string) => readFileSync(resolve(process.cwd(), p), "utf8");

const LIBRARY = "src/pages/LibraryPage.vue";
const RELATED = "src/components/voucher/RelatedVouchers.vue";
const GROUPS = "src/components/voucher/VoucherGroups.vue";
const CLIENT = "src/api/client.ts";

describe("v0.22 关联批契约", () => {
  it("右栏挂载「相关凭证」「所属事项组」两个区块，且都用 shown.id", () => {
    const src = read(LIBRARY);
    expect(src).toContain("<RelatedVouchers :voucher-id=\"shown.id\"");
    expect(src).toContain("<VoucherGroups :voucher-id=\"shown.id\"");
    // 两个区块都在右栏（.inspector）之内、ins-meta 之前
    const inspector = src.indexOf('class="inspector"');
    const related = src.indexOf("<RelatedVouchers");
    const meta = src.indexOf('class="ins-meta"');
    expect(inspector).toBeGreaterThan(-1);
    expect(related).toBeGreaterThan(inspector);
    expect(meta).toBeGreaterThan(related);
    // 跳转复用既有深链聚焦动线
    expect(src).toContain("focusFromLink");
    expect(src).toContain("await showFocused(id)");
  });

  it("关联区块只做语义关系：类型是中英一一对应的两种、且不做金额合计", () => {
    const src = read(RELATED);
    expect(src).toContain("explains: \"附件说明\"");
    expect(src).toContain("detail: \"明细\"");
    // 调用的三个关联命令
    expect(src).toContain("api.listVoucherLinks");
    expect(src).toContain("api.linkVouchers");
    expect(src).toContain("api.unlinkVouchers");
    // 关联 ≠ 抵消：**模板里**不得出现"合计"字样（金额只逐行展示对方凭证的金额；
    // 文件头注释里说明"不改合计口径"是允许的，所以只断言 <template> 段）
    const tpl = src.slice(src.indexOf("<template>"));
    expect(tpl).not.toContain("合计");
    // 展示对方凭证的编号/日期/金额（用了共享的金额格式化）
    expect(src).toContain("fmtAmount");
  });

  it("事项组的组内合计显式标注为派生值，且不写任何导出/统计字段", () => {
    const src = read(GROUPS);
    expect(src).toContain("组内合计");
    expect(src).toContain("派生值");
    expect(src).toContain("api.voucherGroupsOf");
    expect(src).toContain("api.listVoucherGroups");
    expect(src).toContain("api.createVoucherGroup");
    expect(src).toContain("api.addVouchersToGroup");
    expect(src).toContain("api.removeVoucherFromGroup");
  });

  it("前端 API 与命令名一一对应（10 个）", () => {
    const src = read(CLIENT);
    for (const [method, cmd] of [
      ["linkVouchers", "link_vouchers"],
      ["unlinkVouchers", "unlink_vouchers"],
      ["listVoucherLinks", "list_voucher_links"],
      ["listVoucherLinkEvents", "list_voucher_link_events"],
      ["createVoucherGroup", "create_voucher_group"],
      ["addVouchersToGroup", "add_vouchers_to_group"],
      ["removeVoucherFromGroup", "remove_voucher_from_group"],
      ["voucherGroupsOf", "voucher_groups_of"],
      ["listVoucherGroups", "list_voucher_groups"],
      ["deleteVoucherGroup", "delete_voucher_group"],
    ] as const) {
      expect(src, `${method} 应走 ${cmd}`).toContain(`${method}:`);
      expect(src, `缺少命令名 ${cmd}`).toContain(`invoke<`);
      expect(src).toContain(`"${cmd}"`);
    }
  });

  it("无图凭证（说明条目）有显式入口，且不走导入路径", () => {
    const form = read("src/pages/VoucherFormPage.vue");
    expect(form).toContain("新建说明条目（不选图）");
    expect(form).toContain("api.createMemoVoucher");
    expect(form).toContain("没有图片的凭证？");
    // 用创建后聚焦右栏的既有动线，不另造跳转
    expect(form).toContain("openInLibrary(v.id)");
    const client = read(CLIENT);
    expect(client).toContain('"create_memo_voucher"');
    // 「一图一凭证」的主线仍在：导入路径照旧（本入口不碰它）
    expect(form).toContain("api.pickAndImport(true)");
  });

  it("列表对无图凭证有缩略图占位（不会渲染成破图）", () => {
    const src = read(LIBRARY);
    expect(src).toContain("thumbUrl(v)");
    expect(src).toContain('name="image"');
  });

  it("打印中心带「相关凭证清单」开关且**默认开**（用户选择），并把它传给出参", () => {
    const page = read("src/pages/PrintCenterPage.vue");
    expect(page).toContain("const includeLinks = ref(true);");
    expect(page).toContain("includeLinks: includeLinks.value,");
    expect(page).toContain("附带相关凭证清单");
    const client = read(CLIENT);
    expect(client).toContain("includeLinks?: boolean;");
  });

  it("建立关联的「说明」必须带 rel-note 类（否则会挤掉关键词输入框——2026-10-17 线上缺陷回放）", () => {
    const src = read("src/components/voucher/RelatedVouchers.vue");
    // 说明输入框必须挂 rel-note，CSS 的 `.rel-note { grid-column: 1 / -1 }` 才生效
    expect(src).toMatch(/class="input rel-note"[\s\S]{0,120}aria-label="关系说明"/);
    expect(src).toMatch(/\.rel-note\s*\{[^}]*grid-column:\s*1 \/ -1/);
    // 关键词输入框必须存在且可搜索（用户「没有位置可以关联其他凭证」的直接病灶）
    expect(src).toContain('aria-label="搜索凭证"');
    expect(src).toContain("async function search()");
    expect(src).toContain("api.linkVouchers(");
  });

  it("列表提供「未被佐证」筛选：已并入「待补项」下拉（控件、URL 回显、清空三处都在）", () => {
    const src = read(LIBRARY);
    // 2026-10-17：三个复选框合并成「缺项」**多选**下拉，未被佐证是其中一个选项
    expect(src).toContain('{ value: "unlinked", label: "未被佐证" }');
    expect(src).toContain('q.unlinked === "1"');
    expect(src).toContain('unlinked: pendingFilters.value.includes("unlinked") ? true : null');
    expect(src).toContain("pendingFilters.value = [];");
  });
});
