<script setup lang="ts">
/**
 * 打印中心（对应 UI 方案页面二 + 页面三）：
 * 左侧选凭证 + 打印选项，右侧实时预览，生成 PDF（Rust 后端 printpdf，A4、中文、省墨）。
 * 一图一凭证的默认导出形态：「按天分组」——每天一份凭证日报（汇总表 + 图片附录）。
 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, type ExportBatch, type Voucher, type VoucherThumb } from "../api/client";
import EmptyState from "../components/EmptyState.vue";
import VoucherCard from "../components/VoucherCard.vue";
import { useConfigStore } from "../stores/config";
import { toast, toastError } from "../stores/toast";

const configStore = useConfigStore();

const vouchers = ref<Voucher[]>([]);
const selected = ref<Set<number>>(new Set());
/** D-9：各页拉到的凭证首附件缩略图（key = voucher.id），供图片附录预览 */
const thumbs = ref<Record<number, VoucherThumb>>({});
const attachmentsDir = computed(() =>
  configStore.appInfo ? `${configStore.appInfo.dataDir}\\attachments` : "",
);

const title = ref("凭证整理单");
const cardStyle = ref<"standard" | "clean" | "ledger">("standard");
const columns = ref<1 | 2 | 3>(3);
const includeImages = ref(true);
/** 按天分组导出（每天一份日报 PDF）；false = 全部合并为一份整理单 */
const groupByDay = ref(true);

const busy = ref(false);
const lastBatches = ref<ExportBatch[]>([]);

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
function appendixThumbUrl(t: VoucherThumb): string {
  const rel =
    t.enhancedSource === "scan" && t.enhancedPath ? t.enhancedPath : t.filePath;
  return convertFileSrc(`${attachmentsDir.value}\\${rel.replace(/\//g, "\\")}`);
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
    toast.warning("请先勾选要打印的凭证");
    return;
  }
  busy.value = true;
  try {
    const options = {
      title: title.value,
      cardStyle: cardStyle.value,
      columns: columns.value,
      includeImages: includeImages.value,
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

// 停留本页期间拖放/微信入库的新凭证，需出现在左侧可选列表（不重置打印选项）
let unlistenImportDone: UnlistenFn | null = null;
let unlistenWxImported: UnlistenFn | null = null;

onMounted(async () => {
  load();
  unlistenImportDone = await listen("import:done", () => loadVouchers());
  unlistenWxImported = await listen("wechat:imported", () => loadVouchers());
});

onUnmounted(() => {
  unlistenImportDone?.();
  unlistenWxImported?.();
});
</script>

<template>
  <div class="split" style="display: grid; grid-template-columns: 1fr 1fr; gap: 16px; align-items: start">
    <!-- 左：选择 + 选项 -->
    <div>
      <div class="fluent-panel">
        <div class="fluent-panel-header">
          <h3>选择凭证（已选 {{ selected.size }}）</h3>
        </div>
        <div class="fluent-panel-body" style="padding: 0; max-height: 340px; overflow-y: auto">
          <div class="dtable-wrap" v-if="vouchers.length">
            <table class="dtable">
              <thead>
                <tr>
                  <th style="width: 32px"></th>
                  <th>编号</th>
                  <th>日期</th>
                  <th>摘要</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="v in vouchers" :key="v.id">
                  <td><input type="checkbox" class="cb" :checked="selected.has(v.id)" @change="toggle(v.id)" /></td>
                  <td><span class="mono">{{ v.voucherNo }}</span></td>
                  <td>{{ v.voucherDate }}</td>
                  <td style="max-width: 160px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap">{{ v.description || "—" }}</td>
                </tr>
              </tbody>
            </table>
          </div>
          <EmptyState v-else icon="🗂" title="还没有凭证" desc="先到凭证列表页导入或新建" />
        </div>
      </div>

      <div class="fluent-panel">
        <div class="fluent-panel-header"><h3>打印选项</h3></div>
        <div class="fluent-panel-body">
          <div style="display: flex; align-items: center; justify-content: space-between; padding: 8px 0 12px">
            <div>
              <div style="font-size: 13px; font-weight: 600">按天分组导出</div>
              <div style="font-size: 12px; color: var(--text-3)">每个日期一份「凭证日报」PDF（汇总表 + 图片附录），便于按日归档</div>
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
              <div style="font-size: 12px; color: var(--text-3)">{{ groupByDay ? "在日报汇总表后追加当日图片附录（每页 2×2）" : "在凭证卡片后追加图片附录（每页 2×2）" }}</div>
            </div>
            <label class="toggle">
              <input type="checkbox" v-model="includeImages" />
              <span class="slider"></span>
            </label>
          </div>
          <button class="btn btn-primary" style="width: 100%; justify-content: center; padding: 8px" :disabled="busy"
            @click="generate">
            {{ busy ? "生成中…" : groupByDay ? "🖨 生成凭证日报" : "🖨 生成 PDF" }}
          </button>
          <button v-if="lastBatches.length" class="btn btn-subtle" style="width: 100%; justify-content: center; margin-top: 8px"
            @click="openLastPdf">
            打开上一次生成的 PDF（{{ lastBatches.length === 1 ? lastBatches[0].batchNo : `${lastBatches.length} 份日报` }}）
          </button>
        </div>
      </div>
    </div>

    <!-- 右：实时预览 -->
    <div>
      <div class="print-area">
        <div style="text-align: center; font-weight: 700; font-size: 14px; margin-bottom: 8px">{{ title }}</div>

        <!-- 按天分组预览：每天一个小节（日期 + 当日卡片） -->
        <template v-if="groupByDay && previewVouchers.length">
          <div v-for="[date, dayVouchers] in previewByDay" :key="date" style="margin-bottom: 12px">
            <div
              style="display: flex; align-items: baseline; gap: 8px; border-bottom: 1px solid #d8d8d8; padding-bottom: 4px; margin-bottom: 8px">
              <span style="font-weight: 700; font-size: 13px">{{ date }}</span>
              <span style="font-size: 11px; color: #8e8e8e">{{ dayVouchers.length }} 张凭证 · 日报含汇总表与合计行</span>
            </div>
            <div class="print-grid cols-2">
              <VoucherCard v-for="v in dayVouchers.slice(0, 4)" :key="v.id" :voucher="v" :style="'clean'" :title="title" />
            </div>
            <div v-if="dayVouchers.length > 4" style="text-align: center; color: #8e8e8e; font-size: 11px; margin-top: 4px">
              当日其余 {{ dayVouchers.length - 4 }} 张在 PDF 汇总表中完整输出
            </div>
          </div>
        </template>

        <!-- 合并单份预览：卡片网格 -->
        <div v-else-if="previewVouchers.length" class="print-grid" :class="`cols-${columns}`">
          <VoucherCard v-for="v in previewVouchers.slice(0, cardsPerPage)" :key="v.id" :voucher="v"
            :style="cardStyle" :title="title" />
        </div>

        <div v-else style="text-align: center; color: #8e8e8e; padding: 48px 0; font-size: 13px">
          勾选左侧凭证后在此预览打印效果
        </div>
        <div v-if="!groupByDay && previewVouchers.length > cardsPerPage" style="text-align: center; color: #8e8e8e; font-size: 11px; margin-top: 8px">
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
                  title="PDF 附录将印 5 步矫正后的增强版">✨ 增强版</span>
              </figcaption>
            </figure>
          </div>
          <div v-if="previewThumbRows.length > 8" style="text-align: center; color: #8e8e8e; font-size: 11px; margin-top: 4px">
            其余 {{ previewThumbRows.length - 8 }} 张图片在 PDF 附录中完整输出
          </div>
        </template>
      </div>
      <div style="font-size: 11px; color: var(--text-4); text-align: center; margin-top: 8px">
        预览仅供参考，实际打印以 PDF 输出为准
      </div>
    </div>
  </div>
</template>
