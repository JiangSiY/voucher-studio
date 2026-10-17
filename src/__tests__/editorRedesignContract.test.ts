/**
 * 右栏编辑器重设计契约（v0.22 B 批，用户 2026-10-16 反馈）。
 *
 * 锁四件事（都是用户点名的）：
 * 1. **相关凭证 / 所属事项组在识别结果之前**（此前排在识别结果与修改历史之后，"太靠下"）；
 * 2. **操作按钮排在同一行**（`.rel-pick` / `.vg-row` 是 flex 行；附件操作行不再 wrap，未展宽时收成纯图标）；
 * 3. **识别结果 / 修改历史默认折叠**（都经 `CollapsiblePanel`，且不传 `default-open`），折叠时给徽标；
 * 4. **未保存状态提到右栏头部**（`.ins-head` 里的小圆点由编辑器 `dirty` 事件驱动）。
 */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

const read = (p: string) => readFileSync(resolve(process.cwd(), p), "utf8");

const EDITOR = "src/components/voucher/VoucherEditor.vue";
const LIBRARY = "src/pages/LibraryPage.vue";
const OCR = "src/components/OcrResultCard.vue";
const SNAP = "src/components/voucher/SnapshotPanel.vue";
const RELATED = "src/components/voucher/RelatedVouchers.vue";
const GROUPS = "src/components/voucher/VoucherGroups.vue";
const BASE_CSS = "src/styles/base.css";

describe("右栏编辑器重设计契约（B 批）", () => {
  it("关系与组插在「核心字段之后、识别结果之前」", () => {
    const editor = read(EDITOR);
    const slot = editor.indexOf('<slot name="relations" />');
    const form = editor.indexOf('class="editor-form"');
    const ocr = editor.indexOf("<OcrResultCard");
    expect(form).toBeGreaterThan(-1);
    expect(slot).toBeGreaterThan(form);
    expect(ocr).toBeGreaterThan(slot);
    // 宿主通过 #relations 插槽挂载两个区块（不再直接排在 ins-meta 之前）
    const lib = read(LIBRARY);
    expect(lib).toContain("<template #relations>");
    expect(lib).toContain('<RelatedVouchers :voucher-id="shown.id"');
    expect(lib).toContain('<VoucherGroups :voucher-id="shown.id"');
  });

  it("识别结果与修改历史改成可折叠，默认折叠，且折叠时有徽标", () => {
    const ocr = read(OCR);
    expect(ocr).toContain("CollapsiblePanel");
    expect(ocr).toContain('title="识别结果"');
    expect(ocr).toContain(":badge=");
    expect(ocr).not.toContain("default-open");
    const snap = read(SNAP);
    expect(snap).toContain("CollapsiblePanel");
    expect(snap).toContain("修改历史");
    expect(snap).toContain(":badge=");
    expect(snap).not.toContain("default-open");
    // 面板本身默认收起（defaultOpen 默认 false）
    const panel = read("src/components/CollapsiblePanel.vue");
    expect(panel).toContain("defaultOpen: false");
    expect(panel).toContain(":aria-expanded=");
    expect(panel).toContain("v-show=");
  });

  it("两个区块的行内控件同排同高（输入与按钮同一档高度，不再错位）", () => {
    for (const [f, rowClass] of [
      [RELATED, ".rel-pick"],
      [GROUPS, ".vg-row"],
    ] as const) {
      const src = read(f);
      expect(src, `${f} 缺少 scoped 样式`).toContain("<style scoped>");
      const rule = src.slice(src.indexOf(`${rowClass} {`), src.indexOf(`${rowClass} {`) + 400);
      // 行内是显式网格：输入与按钮同排
      expect(rule).toContain("display: grid");
      expect(rule).toContain("grid-template-columns");
      // **同一档高度**——`.btn-sm` 是 26px、`.input` 是 32px，同排会"大小不一致"（用户 2026-10-16 截图）
      expect(rule).toContain("--control-h-md");
      expect(src).not.toMatch(/class="btn btn-sm"[^>]*>(搜索|建立并加入|加入)</);
      // 标题行右侧只有一个主按钮（＋ 建立关联 / ＋ 新建组），头部按钮可继续用小号
      expect(src).toMatch(/fluent-panel-header[\s\S]{0,400}?btn btn-sm/);
    }
  });

  it("附件操作按钮同行，未展宽时收成纯图标", () => {
    const css = read(BASE_CSS);
    expect(css).toMatch(/\.att-ops\s*\{[^}]*flex-wrap:\s*nowrap/);
    expect(css).toMatch(/\.att-ops\.isDense\s+\.ops-label\s*\{\s*display:\s*none/);
    const panel = read("src/components/voucher/AttachmentPanel.vue");
    expect(panel).toContain(":class=\"{ isDense: dense }\"");
    expect(read(EDITOR)).toContain(':dense="dense"');
  });

  it("排版一致性：与输入框同排的按钮一律用 md（`.btn`），不用小号", () => {
    // 用户 2026-10-16 反馈「按钮和输入框大小不一致」的同类问题一次性排查：
    // 搜索页的「清除筛选」与整排 `.input` 同排，此前是 `.btn-sm`（26px vs 32px）
    const search = read("src/pages/SearchPage.vue");
    expect(search).toContain('class="btn" @click="clearFilters">清除筛选');
    expect(search).not.toContain('class="btn btn-sm" @click="clearFilters"');
  });

  it("未保存状态显示在右栏头部（由编辑器 dirty 事件驱动）", () => {
    const editor = read(EDITOR);
    expect(editor).toContain("dirty: [v: boolean]");
    expect(editor).toContain('emit("dirty"');
    const lib = read(LIBRARY);
    expect(lib).toContain('@dirty="editorUnsaved = $event"');
    expect(lib).toContain('v-if="editorUnsaved"');
    expect(lib).toContain("● 未保存");
  });

  it("表单高频前置：摘要 → 日期/金额（同排） → 状态；只读编号行已移出表单", () => {
    const editor = read(EDITOR);
    const idx = (f: string) => editor.indexOf(`data-field="${f}"`);
    expect(idx("description")).toBeGreaterThan(-1);
    expect(idx("description")).toBeLessThan(idx("date-amount"));
    // 日期与金额在**同一行**里（子字段仍保留各自的 data-field 钩子，自动化选择器不用改道）
    expect(idx("date")).toBeGreaterThan(idx("date-amount"));
    expect(idx("amount")).toBeGreaterThan(idx("date"));
    expect(idx("date-amount")).toBeLessThan(idx("status"));
    // 编号不再占表单一行（右栏头部已显示）
    expect(editor).not.toContain('data-field="no"');
    // 备注自适应 + 操作条吸底
    expect(editor).toContain("field-sizing: content");
    expect(editor).toMatch(/\.editor > \.ed-ops\s*\{[^}]*position:\s*sticky/);
  });

  it("状态改成分段按钮（radiogroup、四个状态、支持 ←/→），不再用下拉", () => {
    const editor = read(EDITOR);
    expect(editor).toContain('role="radiogroup"');
    expect(editor).toContain('role="radio"');
    expect(editor).toContain(":aria-checked=");
    for (const [value, label] of [
      ["draft", "草稿"],
      ["reviewed", "已复核"],
      ["exported", "已导出"],
      ["archived", "已归档"],
    ] as const) {
      expect(editor).toContain(`value: "${value}", label: "${label}"`);
    }
    expect(editor).toContain("onStatusKeydown");
    expect(editor).toContain("ArrowRight");
    // 状态行里不得再有 select
    const statusRow = editor.slice(editor.indexOf('data-field="status"'), editor.indexOf('data-field="status"') + 700);
    expect(statusRow).not.toContain("<select");
  });

  it("日期/金额按控件利用率并排：仅展宽档两列，常规档堆叠", () => {
    const editor = read(EDITOR);
    // 基础规则 = 单列（堆叠）
    const base = editor.slice(editor.indexOf(".ed-pair-fields {"));
    expect(base.slice(0, 160)).toContain("grid-template-columns: minmax(0, 1fr)");
    // 展宽档（非 dense）才两列
    expect(editor).toMatch(
      /\.editor:not\(\.editor-dense\) \.ed-pair-fields\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\)\s+minmax\(0,\s*1fr\)/,
    );
  });
});
