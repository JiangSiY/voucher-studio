<script setup lang="ts">
/**
 * 打印中心（对应 UI 方案页面二 + 页面三）：
 * 左侧选凭证 + 打印选项，右侧实时预览，生成 PDF（Rust 后端 printpdf，A4、中文、省墨）。
 * 一图一凭证的默认导出形态：「按天分组」——每天一份凭证日报（汇总表 + 图片附录）。
 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { safeListen } from "../composables/safeListen";
import { convertFileSrc } from "@tauri-apps/api/core";
import { pickCandidate } from "../composables/useAttachmentFallback";
import { api, type ExportBatch, type Voucher, type VoucherStatus, type VoucherThumb } from "../api/client";
import EmptyState from "../components/EmptyState.vue";
import FluentIcon from "../components/FluentIcon.vue";
import VoucherCard from "../components/VoucherCard.vue";
import { STATUS_META, fmtAmount } from "../composables/format";
import { useConfigStore } from "../stores/config";
import { toast, toastError } from "../stores/toast";

const configStore = useConfigStore();

const vouchers = ref<Voucher[]>([]);
const selected = ref<Set<number>>(new Set());
const thumbs = ref<Record<number, VoucherThumb>>({});

const title = ref("凭证整理单");
const cardStyle = ref<"standard" | "clean" | "ledger">("standard");
const columns = ref<1 | 2 | 3>(3);
const includeImages = ref(true);
const groupByDay = ref(true);
/** v0.10：导出格式（PDF 保留原有按天/合并两形态；Excel/CSV 走表格式导出） */
const exportFormat = ref<"pdf" | "xlsx" | "csv">("pdf");

// ---------- 筛选器（按日期范围 / 状态 / 标签） ----------
const filterDateFrom = ref("");
const filterDateTo = ref("");
const filterStatus = ref<"all" | VoucherStatus>("all");
const filterTag = ref("");

/** 筛选后的可选凭证 */
/** 渲染上限（2026-09-23 审计 B16）：打印中心原先把**全部**筛选结果渲染成 <tr>，
 *  上万条时 WKWebView 会卡死数秒。只限制预览渲染行数，导出仍走 filteredVouchers 全量。 */
const RENDER_CAP = 200;
const visibleRows = computed(() => filteredVouchers.value.slice(0, RENDER_CAP));
const truncated = computed(() => Math.max(0, filteredVouchers.value.length - RENDER_CAP));

const filteredVouchers = computed(() => {
  return vouchers.value.filter((v) => {
    if (filterDateFrom.value && v.voucherDate < filterDateFrom.value) return false;
    if (filterDateTo.value && v.voucherDate > filterDateTo.value) return false;
    if (filterStatus.value !== "all" && v.status !== filterStatus.value) return false;
    if (filterTag.value.trim()) {
      const tag = filterTag.value.trim().toLowerCase();
      if (!v.tags.some((t) => t.toLowerCase().includes(tag))) return false;
    }
    return true;
  });
});

/** 全选当前筛选结果；未筛选时全选全部 */
function selectAll() {
  const list = filteredVouchers.value;
  selected.value = new Set(list.map((v) => v.id));
}

/** 取消全选 */
function selectNone() {
  selected.value = new Set();
}

/** 反选（仅在筛选结果内反选，不在筛选外的保持不变） */
function invertSelection() {
  const list = filteredVouchers.value;
  for (const v of list) {
    if (selected.value.has(v.id)) selected.value.delete(v.id);
    else selected.value.add(v.id);
  }
}

/** 清空筛选 */
function clearFilters() {
  filterDateFrom.value = "";
  filterDateTo.value = "";
  filterStatus.value = "all";
  filterTag.value = "";
}

const busy = ref(false);
const lastBatches = ref<ExportBatch[]>([]);

/** 生成按钮文案（格式 × 按天/合并） */
const generateLabel = computed(() => {
  if (busy.value) return "生成中…";
  if (exportFormat.value === "xlsx") return "导出 Excel";
  if (exportFormat.value === "csv") return "导出 CSV";
  return groupByDay.value ? "生成凭证日报" : "生成 PDF";
});

/** 只刷新可选凭证列表（不动用户已调整的打印选项） */
async function loadVouchers() {
  try {
    // 分页拉全：只取首页会漏掉第 100 张以后的凭证，导致无法勾选打印
    const all: Voucher[] = [];
    const allThumbs: Record<number, VoucherThumb> = {};
    let page = 1;
    for (;;) {
      const result = await api.listVouchers({ page, pageSize: 200 });
      all.push(...result.items);
      Object.assign(allThumbs, result.thumbs ?? {});
      if (all.length >= result.total || result.items.length === 0) break;
      page++;
    }
    vouchers.value = all;
    thumbs.value = allThumbs;
  } catch (e) {
    toastError(e);
  }
}

// ---------- D-9：图片附录预览（与 PDF 附录取图同规则） ----------

/** 选中凭证的附录缩略图行（key = voucher.id，与 PDF 附录顺序一致：预览列表已按日期序） */
const previewThumbRows = computed(() =>
  previewVouchers.value
    .map((v) => ({ voucher: v, thumb: thumbs.value[v.id] }))
    .filter((r): r is { voucher: Voucher; thumb: VoucherThumb } => r.thumb != null),
);

/**
 * D-9：缩略图 URL——与 Rust 端 export::pdf_image_abs 同规则：
 * 扫描件用增强版（5 步矫正后更清晰可印），截图/未识别用原图（截图增强版仅 OCR 用）。
 * 预览即所见即所得，用户在预览里看到的图就是 PDF 附录里印出来的图。
 */
/** 选择列表的缩略图（比 PDF 附录简单：直接取首附件，缺失走占位） */
function rowThumbUrl(v: Voucher): string | null {
  const t = thumbs.value[v.id];
  if (!t) return null;
  const abs = pickCandidate(t, 0);
  return abs ? convertFileSrc(abs) : null;
}

function appendixThumbUrl(t: VoucherThumb): string {
  // 地址用后端拼好的绝对路径（不要在前端拼分隔符）
  const abs =
    t.enhancedSource === "scan" && t.enhancedAbs ? t.enhancedAbs : t.fileAbs;
  return abs ? convertFileSrc(abs) : "";
}

async function load() {
  await loadVouchers();

  // 打印选项默认值来自设置（config.print），而非硬编码
  const p = configStore.config?.print;
  if (p) {
    title.value = p.title || "凭证整理单";
    if (p.cardStyle === "clean" || p.cardStyle === "ledger") {
      cardStyle.value = p.cardStyle;
    } else {
      cardStyle.value = "standard";
    }
    columns.value = (p.columns === 1 || p.columns === 2 ? p.columns : 3) as 1 | 2 | 3;
    includeImages.value = p.includeImages;
    groupByDay.value = p.groupByDay !== false;
  }
}

function toggle(id: number) {
  if (selected.value.has(id)) selected.value.delete(id);
  else selected.value.add(id);
}

const previewVouchers = computed(() => vouchers.value.filter((v) => selected.value.has(v.id)));

const cardsPerPage = computed(() => columns.value * 7);

/** 预览按天分组：Map 按插入序（日期升序，列表已按日期排序） */
const previewByDay = computed(() => {
  const map = new Map<string, Voucher[]>();
  for (const v of previewVouchers.value) {
    const list = map.get(v.voucherDate);
    if (list) list.push(v);
    else map.set(v.voucherDate, [v]);
  }
  return map;
});

async function generate() {
  if (!selected.value.size) {
    toast.warning(exportFormat.value === "pdf" ? "请先勾选要打印的凭证" : "请先勾选要导出的凭证");
    return;
  }
  busy.value = true;
  try {
    // v0.10：表格式导出（Excel / CSV）——列：凭证号/日期/摘要/对方/金额/状态/标签/备注 + 合计行
    if (exportFormat.value !== "pdf") {
      const batch = await api.exportTable([...selected.value], exportFormat.value);
      lastBatches.value = [batch];
      toast.success(
        `已导出 ${exportFormat.value === "xlsx" ? "Excel" : "CSV"}：${batch.batchNo}（${batch.voucherCount} 张凭证）`,
        {
          label: "打开文件",
          onClick: () => void api.openPath(batch.pdfPath),
        },
      );
      return;
    }
    // v0.8：dailyTemplate 走全局打印设置（SettingsPage 可切 archive/compact）
    const options = {
      title: title.value,
      cardStyle: cardStyle.value,
      columns: columns.value,
      includeImages: includeImages.value,
      dailyTemplate: configStore.config?.print.dailyTemplate ?? "compact",
    };
    if (groupByDay.value) {
      const batches = await api.exportPdfDaily([...selected.value], options);
      lastBatches.value = batches;
      toast.success(
        batches.length === 1
          ? `凭证日报已生成：${batches[0].batchNo}（${batches[0].voucherCount} 张凭证）`
          : `已按 ${batches.length} 天生成 ${batches.length} 份凭证日报（合计 ${batches.reduce((n, b) => n + b.voucherCount, 0)} 张凭证）`,
      );
    } else {
      const batch = await api.exportPdf([...selected.value], options);
      lastBatches.value = [batch];
      toast.success(`PDF 已生成：${batch.batchNo}（${batch.voucherCount} 张凭证）`);
    }
  } catch (e) {
    toastError(e);
  } finally {
    busy.value = false;
  }
}

async function openLastPdf() {
  try {
    await Promise.all(lastBatches.value.map((b) => api.openPath(b.pdfPath)));
  } catch (e) {
    toastError(e);
  }
}

// 停留本页期间拖放入库的新凭证，需出现在左侧可选列表（不重置打印选项）
let unlistenImportDone: UnlistenFn | null = null;

onMounted(async () => {
  load();
  unlistenImportDone = await safeListen("import:done", () => loadVouchers());
});

onUnmounted(() => {
  unlistenImportDone?.();
});
</script>

<template>
  <div class="print-page">
    <!-- 顶部操作条（v0.18）：一眼看到"选了几张 / 导出什么格式 / 去哪导出" -->
    <div class="action-bar">
      <div class="segmented" role="tablist">
        <button class="seg-item" role="tab" :aria-selected="exportFormat === 'pdf'"
          :class="{ active: exportFormat === 'pdf' }" @click="exportFormat = 'pdf'">PDF（打印归档）</button>
        <button class="seg-item" role="tab" :aria-selected="exportFormat === 'xlsx'"
          :class="{ active: exportFormat === 'xlsx' }" @click="exportFormat = 'xlsx'">Excel</button>
        <button class="seg-item" role="tab" :aria-selected="exportFormat === 'csv'"
          :class="{ active: exportFormat === 'csv' }" @click="exportFormat = 'csv'">CSV</button>
      </div>
      <span class="scope">已选 {{ selected.size }} 张 · 筛选出 {{ filteredVouchers.length }} 张 · 共 {{ vouchers.length }} 张</span>
      <span class="flex-1"></span>
      <button class="btn" :disabled="!selected.size" @click="selectAll">全选筛选结果</button>
      <button class="btn" :disabled="!selected.size" @click="selectNone">清空</button>
      <button class="btn btn-primary" :disabled="!selected.size || busy" @click="generate()">
        <FluentIcon v-if="!busy" name="print" :size="15" /> {{ generateLabel }}
      </button>
    </div>

  <div class="print-split">
    <!-- 左：选择 + 选项 -->
    <div>
      <div class="fluent-panel">
        <div class="fluent-panel-header">
          <h3>选择凭证</h3>
        </div>
        <!-- 筛选器 -->
        <div class="fluent-panel-body" style="padding: 8px 12px; border-bottom: 1px solid var(--stroke)">
          <div style="display: flex; flex-wrap: wrap; gap: 8px; align-items: center">
            <input type="date" v-model="filterDateFrom" class="input" style="width: 130px" title="起始日期" />
            <span style="color: var(--text-3); font-size: 12px">至</span>
            <input type="date" v-model="filterDateTo" class="input" style="width: 130px" title="结束日期" />
            <select v-model="filterStatus" class="input" style="width: 100px">
              <option value="all">全部状态</option>
              <option value="draft">草稿</option>
              <option value="reviewed">已复核</option>
              <option value="exported">已导出</option>
              <option value="archived">已归档</option>
            </select>
            <input v-model="filterTag" class="input" style="flex: 1; min-width: 120px" placeholder="标签过滤（模糊匹配）" />
            <button class="btn btn-subtle" @click="clearFilters">清除筛选</button>
          </div>
          <div style="display: flex; gap: 4px; margin-top: 8px">
            <!-- 全选/清空已在顶部操作条；这里只留"反选"这个局部操作 -->
            <button class="btn" @click="invertSelection">反选</button>
          </div>
        </div>
        <div class="fluent-panel-body print-selected-list" style="padding: 0">
          <!-- v0.18：与资料库一致的**双行列表**（表格 5 列在放大字号下太挤；这里只需要"挑哪几张"） -->
          <ul class="pick-rows" v-if="filteredVouchers.length">
            <li
              v-for="v in visibleRows"
              :key="v.id"
              class="pick-row"
              :class="{ on: selected.has(v.id) }"
              @click="toggle(v.id)"
            >
              <input type="checkbox" class="cb" :checked="selected.has(v.id)" @click.stop="toggle(v.id)" />
              <div class="pick-thumb">
                <img v-if="rowThumbUrl(v)" :src="rowThumbUrl(v)!" :alt="v.voucherNo" loading="lazy" />
                <FluentIcon v-else name="image" :size="16" />
              </div>
              <div class="pick-main">
                <div class="line1">
                  <span class="desc">{{ v.description || "（无摘要）" }}</span>
                  <span class="flex-1"></span>
                  <span class="amount">{{ fmtAmount(v.amountCents) }}</span>
                </div>
                <div class="line2">
                  <span class="mono">{{ v.voucherNo }}</span>
                  <span class="dot">·</span>
                  <span>{{ v.voucherDate }}</span>
                  <span class="flex-1"></span>
                  <span class="tag" :class="STATUS_META[v.status].tag">{{ STATUS_META[v.status].label }}</span>
                </div>
              </div>
            </li>
            <li v-if="truncated > 0" class="pick-more">
              仅预览前 {{ RENDER_CAP }} 条（另有 {{ truncated }} 条未显示，导出不受影响）
            </li>
          </ul>
          <EmptyState v-else icon="folder" :title="vouchers.length === 0 ? '还没有凭证' : '筛选后无结果'" :desc="vouchers.length === 0 ? '先到资料库导入或新建' : '试试调整筛选条件'" />
        </div>
      </div>

      <div class="fluent-panel">
        <div class="fluent-panel-header"><h3>{{ exportFormat === "pdf" ? "打印选项" : "导出选项" }}</h3></div>
        <div class="fluent-panel-body">
          <!-- v0.18：导出格式已提到顶部操作条（唯一入口），此处不再重复 -->
          <div class="desc" v-if="exportFormat !== 'pdf'">
            表格式导出（凭证号 / 日期 / 摘要 / 对方 / 金额 / 状态 / 标签 / 备注 + 合计行），适合对账与导入记账软件
          </div>
          <template v-if="exportFormat === 'pdf'">
          <div style="display: flex; align-items: center; justify-content: space-between; padding: 8px 0 12px">
            <div>
              <div class="field-strong">按天分组导出</div>
              <div class="muted-small">每个日期一份「凭证日报」PDF（汇总表 + 图片附录），便于按日归档</div>
            </div>
            <label class="toggle">
              <input type="checkbox" v-model="groupByDay" />
              <span class="slider"></span>
            </label>
          </div>
          <div class="field">
            <label>{{ groupByDay ? "日报标题" : "整理单标题" }}</label>
            <input v-model="title" class="input" />
          </div>
          <!-- 卡片风格/列数仅用于合并单份；日报模板固定为「当日汇总表 + 图片页」 -->
          <template v-if="!groupByDay">
            <div class="field">
              <label>卡片风格</label>
              <div class="filter-chips">
                <button class="fchip" :class="{ active: cardStyle === 'standard' }" @click="cardStyle = 'standard'">标准式</button>
                <button class="fchip" :class="{ active: cardStyle === 'clean' }" @click="cardStyle = 'clean'">简洁式</button>
                <button class="fchip" :class="{ active: cardStyle === 'ledger' }" @click="cardStyle = 'ledger'">账簿式</button>
              </div>
              <div class="desc">以凭证编号为核心、文字为主、省墨设计，无二维码</div>
            </div>
            <div class="field">
              <label>每行列数</label>
              <div class="filter-chips">
                <button class="fchip" :class="{ active: columns === 3 }" @click="columns = 3">3 列 · 21 张/页</button>
                <button class="fchip" :class="{ active: columns === 2 }" @click="columns = 2">2 列 · 14 张/页</button>
                <button class="fchip" :class="{ active: columns === 1 }" @click="columns = 1">1 列 · 7 张/页</button>
              </div>
            </div>
          </template>
          <div style="display: flex; align-items: center; justify-content: space-between; padding: 8px 0">
            <div>
              <div style="font-size: 13px; font-weight: 600">附带附件图片页</div>
              <div class="muted-small">{{ groupByDay ? "在日报汇总表后追加当日图片附录（每页 2×2）" : "在凭证卡片后追加图片附录（每页 2×2）" }}</div>
            </div>
            <label class="toggle">
              <input type="checkbox" v-model="includeImages" />
              <span class="slider"></span>
            </label>
          </div>
          </template>
          <!-- v0.18：导出主操作已提到顶部操作条（唯一入口），此处不再重复 -->
          <button v-if="lastBatches.length" class="btn btn-subtle" style="width: 100%; justify-content: center; margin-top: 8px"
            @click="openLastPdf">
            打开上一次生成的文件（{{ lastBatches.length === 1 ? lastBatches[0].batchNo : `${lastBatches.length} 份日报` }}）
          </button>
        </div>
      </div>
    </div>

    <!-- 右：实时预览（PDF：卡片/日报预览；Excel/CSV：导出表格预览） -->
    <div>
      <!-- v0.10 表格式导出预览：与导出文件同列序，只展示前 12 行 -->
      <div v-if="exportFormat !== 'pdf'" class="print-area">
        <div style="text-align: center; font-weight: 700; font-size: 14px; margin-bottom: 8px">
          {{ exportFormat === "xlsx" ? "Excel" : "CSV" }} 导出预览（{{ previewVouchers.length }} 张）
        </div>
        <div class="dtable-wrap" v-if="previewVouchers.length">
          <table class="dtable">
            <thead>
              <tr>
                <th>凭证号</th><th>日期</th><th>摘要</th><th>对方账户</th>
                <th style="text-align: right">金额（元）</th><th>状态</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="v in previewVouchers.slice(0, 12)" :key="v.id">
                <td><span class="mono">{{ v.voucherNo }}</span></td>
                <td>{{ v.voucherDate }}</td>
                <td style="max-width: 140px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap">{{ v.description || "—" }}</td>
                <td style="max-width: 100px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap">{{ v.counterparty || "—" }}</td>
                <td style="text-align: right" class="mono">{{ v.amountCents != null ? (v.amountCents / 100).toFixed(2) : "" }}</td>
                <td>{{ v.status === 'draft' ? '草稿' : v.status === 'reviewed' ? '已复核' : v.status === 'exported' ? '已导出' : '已归档' }}</td>
              </tr>
            </tbody>
          </table>
        </div>
        <div v-else class="muted-center">
          勾选左侧凭证后预览导出内容
        </div>
        <div v-if="previewVouchers.length > 12" class="muted-center small">
          仅预览前 12 行（共 {{ previewVouchers.length }} 行 + 合计行），文件中完整输出
        </div>
        <div style="font-size: 11px; color: var(--text-4); text-align: center; margin-top: 8px">
          文件另含 标签 / 备注 两列与合计行；金额为数值单元格（Excel 可直接求和）
        </div>
      </div>

      <div v-else class="print-area">
        <div style="text-align: center; font-weight: 700; font-size: 14px; margin-bottom: 8px">{{ title }}</div>

        <!-- 按天分组预览：每天一个小节（日期 + 当日卡片） -->
        <template v-if="groupByDay && previewVouchers.length">
          <div v-for="[date, dayVouchers] in previewByDay" :key="date" style="margin-bottom: 12px">
            <div
              style="display: flex; align-items: baseline; gap: 8px; border-bottom: 1px solid #d8d8d8; padding-bottom: 4px; margin-bottom: 8px">
              <span style="font-weight: 700; font-size: 13px">{{ date }}</span>
              <span class="muted-small">{{ dayVouchers.length }} 张凭证 · 日报含汇总表与合计行</span>
            </div>
            <div class="print-grid cols-2">
              <VoucherCard v-for="v in dayVouchers.slice(0, 4)" :key="v.id" :voucher="v" :style="'clean'" :title="title" />
            </div>
            <div v-if="dayVouchers.length > 4" class="muted-center small">
              当日其余 {{ dayVouchers.length - 4 }} 张在 PDF 汇总表中完整输出
            </div>
          </div>
        </template>

        <!-- 合并单份预览：卡片网格 -->
        <div v-else-if="previewVouchers.length" class="print-grid" :class="`cols-${columns}`">
          <VoucherCard v-for="v in previewVouchers.slice(0, cardsPerPage)" :key="v.id" :voucher="v"
            :style="cardStyle" :title="title" />
        </div>

        <div v-else class="preview-empty">
          <FluentIcon name="print" :size="16" />
          <span>勾选左侧凭证后，这里按实际排版显示预览（高度随内容自适应）</span>
        </div>
        <div v-if="!groupByDay && previewVouchers.length > cardsPerPage" class="muted-center small">
          仅预览第 1 页（共 {{ Math.ceil(previewVouchers.length / cardsPerPage) }} 页），PDF 中完整输出
        </div>

        <!-- D-9：图片附录预览（与 PDF 附录同取图规则：扫描件用增强版，截图用原图） -->
        <template v-if="includeImages && previewThumbRows.length">
          <div class="appendix-divider"></div>
          <div class="appendix-title">图片附录预览（PDF 每页 2×2）</div>
          <div class="appendix-grid">
            <figure v-for="r in previewThumbRows.slice(0, 8)" :key="r.voucher.id" class="appendix-cell">
              <img :src="appendixThumbUrl(r.thumb)" :alt="r.voucher.voucherNo" loading="lazy" />
              <figcaption>
                <span class="mono">{{ r.voucher.voucherNo }}</span>
                <span v-if="r.thumb.enhancedSource === 'scan' && r.thumb.enhancedPath" class="appendix-badge"
                  title="PDF 附录将印 5 步矫正后的增强版"><FluentIcon name="sparkle" :size="9" /> 增强版</span>
              </figcaption>
            </figure>
          </div>
          <div v-if="previewThumbRows.length > 8" class="muted-center small">
            其余 {{ previewThumbRows.length - 8 }} 张图片在 PDF 附录中完整输出
          </div>
        </template>
      </div>
      <div class="muted-center small">预览仅供参考，实际打印以 PDF 输出为准</div>
    </div>
  </div>
  </div>
</template>

<style scoped>
/* v0.18：本页此前**没有 style 块**（样式散在 base.css 与内联 style 里），
   新增的双行选择列表因此毫无样式、渲染成一堆块级元素——补上这一节。 */
.print-page { display: flex; flex-direction: column; gap: var(--s-3); }

.action-bar { display: flex; align-items: center; gap: var(--s-2); flex-wrap: wrap; }
.action-bar .scope { font: var(--type-caption); color: var(--label-tertiary); }
.flex-1 { flex: 1; }

.pick-rows { list-style: none; margin: 0; padding: var(--s-2); display: flex; flex-direction: column; gap: 2px; }
.pick-row {
  display: flex;
  align-items: center;
  gap: var(--s-2);
  padding: 8px var(--s-2);
  border: 1px solid transparent;
  border-radius: var(--radius-control);
  cursor: default;
}
.pick-row:hover { background: var(--bg-selection); }
.pick-row.on { background: var(--bg-selection); border-color: var(--accent); }
.pick-thumb {
  width: 36px;
  height: 46px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  border: 1px solid var(--separator);
  border-radius: var(--radius-small);
  background: var(--surface-2);
  color: var(--label-quaternary);
  overflow: hidden;
}
.pick-thumb img { width: 100%; height: 100%; object-fit: cover; }
.pick-main { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.line1 { display: flex; align-items: baseline; gap: var(--s-2); min-width: 0; }
.desc { font: var(--type-body); color: var(--label-primary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.amount { font: var(--type-body); font-weight: 600; color: var(--label-primary); font-variant-numeric: tabular-nums; }
.line2 { display: flex; align-items: center; gap: var(--s-2); font: var(--type-caption); color: var(--label-tertiary); min-width: 0; }
.dot { color: var(--label-quaternary); }
.pick-more { padding: var(--s-2); text-align: center; font: var(--type-caption); color: var(--label-tertiary); }

.field-strong { font: var(--type-body); font-weight: 600; color: var(--label-primary); }

/* 预览面板：由内容决定高度（自适应）；预览区给"纸面"观感但不撑高容器 */
.preview-panel { align-self: start; }
.preview-panel .fluent-panel-body {
  display: block;            /* 不用 flex:1 —— 否则空态会被拉满整列高度 */
  padding: var(--s-3);
}
.preview-empty {
  display: flex;
  align-items: center;
  gap: var(--s-2);
  padding: var(--s-3);
  border: 1px dashed var(--separator-strong);
  border-radius: var(--radius-control);
  color: var(--label-tertiary);
  font: var(--type-caption);
}
.muted-small { font: var(--type-caption); color: var(--label-tertiary); }

/* 原先写死 #8e8e8e 的说明文字 → 语义令牌（深浅两色自动适配） */
.muted-center { text-align: center; color: var(--label-tertiary); padding: var(--s-8) 0; font: var(--type-body); }
.muted-center.small { padding: 4px 0 0; font: var(--type-caption); }
</style>
