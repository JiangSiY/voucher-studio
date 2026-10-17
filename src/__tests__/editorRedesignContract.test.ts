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
      // 行内控件同排：RelatedVouchers 用两行 flex（`.rel-pick-row` 放三件、说明单独一行），
      // VoucherGroups 用两列网格；两者都必须是"同排同高"，且说明/备注不得再撑宽 auto 列
      expect(rule.includes("display: grid") || rule.includes("display: flex")).toBe(true);
      if (f === RELATED) expect(rule).toContain("flex-direction: column");
      else expect(rule).toContain("grid-template-columns");
      // **同一档高度**——`.btn-sm` 是 26px、`.input` 是 32px，同排会"大小不一致"（用户 2026-10-16 截图）
      expect(src).toContain("--control-h-md");
      expect(src).not.toMatch(/class="btn btn-sm"[^>]*>(搜索|建立并加入|加入)</);
      // 标题行右侧只有一个主按钮（＋ 建立关联 / ＋ 新建组），头部按钮可继续用小号
      expect(src).toMatch(/fluent-panel-header[\s\S]{0,400}?btn btn-sm/);
    }
  });

  it("四个面板并排（相关凭证/事项组/识别结果/修改历史）+ 字段面板标题 + 空翻张条不渲染", () => {
    const editor = read(EDITOR);
    // ① 四个面板同一网格：够宽（容器 ≥560px）2×2，否则单列
    expect(editor).toContain('class="editor-panels"');
    expect(editor).toContain('<slot name="relations" />');
    expect(editor).toContain('<slot name="groups" />');
    expect(editor).toMatch(
      /@container \(min-width: 560px\)\s*\{[^}]*\.editor-panels\s*\{[^}]*grid-template-columns:\s*repeat\(2/,
    );
    // ② 字段面板有标题，且与「凭证图片」用同一套 fluent-panel 头
    expect(editor).toMatch(/<div class="fluent-panel-header">\s*<h3>凭证字段<\/h3>/);
    // ③ 没有翻张信息时不渲染翻张条（去掉展开/收起按钮左侧的留空）
    expect(read(LIBRARY)).toContain('<div v-if="navNeighbor" class="ins-nav"');
  });

  it("凭证预览按竖向比例且不裁图（评测集 90% 是竖图）", () => {
    const css = read(BASE_CSS);
    // 竖向预览框：aspect-ratio ≈ 1:2.2（评测集中位高宽比 2.21）
    const frame = css.slice(css.indexOf(".att-card .att-img {"));
    expect(frame.slice(0, 400)).toContain("aspect-ratio: 10 / 22");
    // 不裁中文凭内容：contain 而不是 cover
    const img = css.slice(css.indexOf(".att-card img {"));
    expect(img.slice(0, 200)).toContain("object-fit: contain");
    expect(img.slice(0, 200)).not.toContain("object-fit: cover");
    // 对比模式（两图叠放）仍是固定高
    expect(css).toMatch(/\.att-card \.att-img\.is-compare\s*\{[^}]*height:\s*320px/);
  });

  it("常规档右栏够宽时也能并排（容器查询 380px；老 WebKit 自动退回堆叠）", () => {
    const editor = read(EDITOR);
    expect(editor).toMatch(/\.editor\s*\{[^}]*container-type:\s*inline-size/s);
    expect(editor).toMatch(
      /@container \(min-width: 380px\)\s*\{[^}]*\.editor-main\s*\{[^}]*grid-template-columns:\s*minmax\(120px[\s\S]{0,80}?minmax\(220px/,
    );
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

  it("「标签」筛选统一为「收支方向」多选下拉（列表 + 打印中心），不再有自由文本标签输入", () => {
    const lib = read("src/pages/LibraryPage.vue");
    // 多选下拉（2026-10-17 用户要求「下拉菜单做成可多选」）
    expect(lib).toContain('<MultiSelect v-model="directions" label="收支方向"');
    expect(lib).toContain(":exclusive=\"['none']\"");
    expect(lib).toContain('value: "收入"');
    expect(lib).toContain('value: "支出"');
    expect(lib).toContain('value: "none", label: "未标"');
    // 映射到既有 tags（方向本身）与新 directionNone（未标）
    expect(lib).toContain('tags: directions.value.filter((v) => v !== "none")');
    expect(lib).toContain('directionNone: directions.value.includes("none")');
    // 旧文案不得回归
    expect(lib).not.toContain("标签（逗号分隔）");
    expect(lib).not.toContain("tagsText");
    // 打印中心仍为下拉
    const print = read("src/pages/PrintCenterPage.vue");
    expect(print).toContain('title="按收支方向筛选"');
    expect(print).not.toContain("filterTag");
  });

  it("用户可见的「标签」措辞已统一为「收支方向」（洞察透视 / 排行卡 / 帮助页 / 导出说明）", () => {
    expect(read("src/pages/InsightsPage.vue")).toContain(">按收支方向</button>");
    expect(read("src/components/dashboard/TopListsPanel.vue")).toContain("收支方向（按金额");
    expect(read("src/pages/HelpPage.vue")).toContain("补全摘要、金额、收支方向");
    // 后端导出表头（xlsx/csv 第 7 列）
    const table = readFileSync(resolve(process.cwd(), "crates/voucher-core/src/services/table.rs"), "utf8");
    expect(table).toContain('"收支方向"');
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

  it("表单高频前置：摘要 → 日期 → 金额 → 状态 → 对方 → 收支方向 → 备注；只读编号行已移出表单", () => {
    const editor = read(EDITOR);
    const idx = (f: string) => editor.indexOf(`data-field="${f}"`);
    const order = ["description", "date", "amount", "status", "counterparty", "tags", "note"];
    for (const f of order) expect(idx(f), `缺少字段 ${f}`).toBeGreaterThan(-1);
    for (let i = 1; i < order.length; i += 1) {
      expect(idx(order[i - 1]), `${order[i - 1]} 应在 ${order[i]} 之前`).toBeLessThan(idx(order[i]));
    }
    // 编号不再占表单一行（右栏头部已显示）
    expect(editor).not.toContain('data-field="no"');
    // 备注自适应 + 操作条吸底
    expect(editor).toContain("field-sizing: content");
    expect(editor).toMatch(/\.editor-fields > \.ed-ops\s*\{[^}]*position:\s*sticky/);
  });

  it("「标签」改为收支方向分段按钮，且**不再**与对方账户并排（每字段独占一行）", () => {
    const editor = read(EDITOR);
    const idx = (f: string) => editor.indexOf(`data-field="${f}"`);
    expect(idx("counterparty")).toBeGreaterThan(-1);
    expect(idx("tags")).toBeGreaterThan(idx("counterparty"));
    const tagsBlock = editor.slice(idx("tags"), idx("tags") + 1300);
    expect(tagsBlock).toContain('role="radiogroup"');
    expect(tagsBlock).toContain("data-direction");
    expect(tagsBlock).not.toContain("<input");
    expect(editor).toContain('{ value: "", label: "未标" }');
    expect(editor).toContain('{ value: "收入", label: "收入" }');
    expect(editor).toContain('{ value: "支出", label: "支出" }');
    expect(editor).toContain("otherTags");
    expect(editor).toContain("其他标签");
    expect(editor).toContain("function setDirection");
    expect(editor).toContain("DIRECTION_TAGS");
    // 方案 A：并排已取消
    expect(editor).not.toContain("counterparty-direction");
    expect(editor).not.toContain("ed-pair");
  });

  it("字段标签在**输入框上方**：所有输入框同起点同宽，不再有左侧标签列", () => {
    const editor = read(EDITOR);
    // 单列堆叠 + 每个字段「标签行在上、控件行在下」
    expect(editor).toMatch(/\.editor-form\s*\{[^}]*grid-template-columns:\s*minmax\(0,\s*1fr\)/);
    expect(editor).toMatch(/\.ed-field\s*\{[^}]*display:\s*grid[^}]*gap:\s*4px/);
    expect(editor).toMatch(/\.ed-field-label\s*\{[^}]*font:\s*var\(--type-caption\)/);
    // 旧的一列标签 / 成对行实现全部退休
    expect(editor).not.toContain("max-content");
    expect(editor).not.toContain("ed-row");
    expect(editor).not.toContain("ed-pair");
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

  it("图片与字段左右并排只在展宽档（方案 A）；展宽档右栏下限抬到 720px", () => {
    const editor = read(EDITOR);
    // 两列外壳：基础单列（常规档上下堆叠）
    const mainStart = editor.indexOf(".editor-main {");
    const main = editor.slice(mainStart, mainStart + 400);
    expect(main).toContain("grid-template-columns: minmax(0, 1fr)");
    // 展宽档才两列（图片 | 字段），每列有下限
    expect(editor).toMatch(
      /\.editor:not\(\.editor-dense\) \.editor-main\s*\{[^}]*grid-template-columns:\s*minmax\(240px[\s\S]{0,80}?minmax\(280px/,
    );
    // 右栏下限 560 → 720px（否则两列都会挤）
    const lib = read(LIBRARY);
    expect(lib).toContain("minmax(720px, 1.6fr)");
    expect(lib).not.toContain("minmax(560px, 1.6fr)");
  });
});
