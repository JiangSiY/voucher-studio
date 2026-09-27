<script setup lang="ts">
/**
 * 全部凭证（资料库）—— v0.18 重构：**双行列表（邮件式）+ 右侧检视器**。
 *
 * 与旧「凭证列表」的区别（用户要求的是结构重做，不是换皮）：
 * - 行形态从"10 列表格"改为**两行信息块**：上行 = 缩略图 + 摘要 + 金额（眼睛先看这两样），
 *   下行 = 编号 · 日期 · 对方 · 状态。信息层次按"核对时真正要看的顺序"排，而不是按字段表排。
 * - **选中即在右侧看大图与字段**（检视器），不再"点进去、返回再重新筛选"；
 *   双击或回车才进全宽编辑页（专注录入/改字段时用它）。
 * - 列表与检视器共享同一份数据，选中行为与键盘上下键联动（↑↓ 移动选择）。
 *
 * 待补（下一步）：跨页选择集、按筛选全选、检视器内就地改常用字段（保存按钮）、
 * 列表虚拟滚动（量级上千时）。当前保持既有批量操作能力不变。
 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { safeListen } from "../composables/safeListen";
import { api, type Attachment, type Voucher, type VoucherDetail, type VoucherStatus } from "../api/client";
import ConfirmDialog from "../components/ConfirmDialog.vue";
import EmptyState from "../components/EmptyState.vue";
import FluentIcon from "../components/FluentIcon.vue";
import { STATUS_META, fmtAmount, fmtDate } from "../composables/format";
import { centsToYuanInput, parseTags, parseYuanToCents } from "../composables/money";
import { candidatePaths, nextFallbackStep, pickCandidate } from "../composables/useAttachmentFallback";
import { toast, toastError } from "../stores/toast";

const route = useRoute();
const router = useRouter();

// ---------- 筛选（沿用既有 query 参数通道：仪表盘/收件箱下钻都用它） ----------

const status = ref<VoucherStatus | "all">("all");
const keyword = ref("");
const dateFrom = ref("");
const dateTo = ref("");

const items = ref<Voucher[]>([]);
const total = ref(0);
const page = ref(1);
const pageSize = 50;
const loading = ref(false);

/** 高级筛选（对应"未填金额 / 待识别 / 从未导出"等待办的精确下钻） */
const amountEmpty = ref(false);
const ocrStatus = ref<string>("");
const exported = ref<string>("");
const sort = ref<string>("date_desc");

function query() {
  return {
    status: status.value === "all" ? null : status.value,
    keyword: keyword.value.trim() || null,
    dateFrom: dateFrom.value || null,
    dateTo: dateTo.value || null,
    amountEmpty: amountEmpty.value,
    ocrStatus: ocrStatus.value || null,
    exported: (exported.value || null) as "never" | "any" | null,
    sort: sort.value,
    page: page.value,
    pageSize,
  };
}

async function load() {
  loading.value = true;
  try {
    const res = await api.listVouchers(query());
    items.value = res.items;
    total.value = res.total;
    thumbs.value = res.thumbs;
    // 选中项若已不在本页（翻页/筛选变化）→ 落到第一行；否则保持选中并同步编辑缓冲
    const stillHere = items.value.find((v) => v.id === selectedId.value);
    if (!stillHere) {
      // 落到第一行：连详情一起加载（附件/快照要它，否则检视器只有字段没有图）
      const first = items.value[0];
      if (first) await select(first);
      else {
        selectedId.value = null;
        detail.value = null;
        resetEdit(null);
      }
    } else {
      resetEdit(stillHere);
    }
  } catch (e) {
    toastError(e);
  } finally {
    loading.value = false;
  }
}

/** 从 URL 读筛选（收件箱/趋势下钻过来时带回显） */
function readQuery() {
  const q = route.query;
  status.value = (q.status as VoucherStatus) ?? "all";
  keyword.value = (q.keyword as string) ?? "";
  dateFrom.value = (q.dateFrom as string) ?? "";
  dateTo.value = (q.dateTo as string) ?? "";
  amountEmpty.value = q.amountEmpty === "1" || q.amountEmpty === "true";
  ocrStatus.value = (q.ocrStatus as string) ?? "";
  exported.value = (q.exported as string) ?? "";
  sort.value = (q.sort as string) ?? "date_desc";
}

const activeFilters = computed(() => {
  const out: { label: string; clear: () => void }[] = [];
  if (status.value !== "all") out.push({ label: `状态：${STATUS_META[status.value].label}`, clear: () => (status.value = "all") });
  if (keyword.value) out.push({ label: `关键词：${keyword.value}`, clear: () => (keyword.value = "") });
  if (dateFrom.value || dateTo.value)
    out.push({ label: `日期：${dateFrom.value || "…"} ~ ${dateTo.value || "…"}`, clear: () => ((dateFrom.value = ""), (dateTo.value = "")) });
  if (amountEmpty.value) out.push({ label: "金额未填", clear: () => (amountEmpty.value = false) });
  if (ocrStatus.value)
    out.push({ label: `OCR：${ocrStatus.value === "pending" ? "待识别" : ocrStatus.value}`, clear: () => (ocrStatus.value = "") });
  if (exported.value)
    out.push({ label: exported.value === "never" ? "从未导出" : "已导出过", clear: () => (exported.value = "") });
  return out;
});

// ---------- 缩略图与检视器 ----------

const thumbs = ref<Record<number, { filePath: string; enhancedPath: string | null; enhancedSource: string; fileAbs?: string | null; enhancedAbs?: string | null }>>({});
const thumbStep = ref<Record<number, number>>({});

function thumbUrl(v: Voucher): string | null {
  const t = thumbs.value[v.id];
  if (!t) return null;
  const abs = pickCandidate(t, thumbStep.value[v.id] ?? 0);
  return abs ? convertFileSrc(abs) : null;
}
function onThumbError(v: Voucher) {
  const t = thumbs.value[v.id];
  if (!t) return;
  const cur = thumbStep.value[v.id] ?? 0;
  thumbStep.value = { ...thumbStep.value, [v.id]: nextFallbackStep(cur, candidatePaths(t).length) };
}

const selectedId = ref<number | null>(null);
const detail = ref<VoucherDetail | null>(null);
const detailBusy = ref(false);
const detailStep = ref(0);

// ---------- 就地编辑（检视器内改常用字段，不必跳编辑页） ----------

const edit = ref<{ amount: string; description: string; counterparty: string; tags: string; note: string } | null>(null);
const savingEdit = ref(false);

/** 选中项变化 → 重置编辑缓冲（避免把上一张的值改到这一张上） */
function resetEdit(v: Voucher | null) {
  edit.value = v
    ? {
        amount: centsToYuanInput(v.amountCents),
        description: v.description ?? "",
        counterparty: v.counterparty ?? "",
        tags: (v.tags ?? []).join("、"),
        note: v.note ?? "",
      }
    : null;
}

const dirty = computed(() => {
  const v = selected.value;
  const e = edit.value;
  if (!v || !e) return false;
  return (
    (parseYuanToCents(e.amount).ok ? (parseYuanToCents(e.amount) as { cents: number | null }).cents : v.amountCents) !== v.amountCents ||
    e.description.trim() !== (v.description ?? "") ||
    e.counterparty.trim() !== (v.counterparty ?? "") ||
    e.note.trim() !== (v.note ?? "") ||
    parseTags(e.tags).join("、") !== (v.tags ?? []).join("、")
  );
});

/** 保存就地编辑：只提交改动的字段；金额空串 → 显式清空（clearAmount） */
async function saveEdit() {
  const v = selected.value;
  const e = edit.value;
  if (!v || !e) return;
  const parsed = parseYuanToCents(e.amount);
  if (!parsed.ok) {
    toast.warning(parsed.reason);
    return;
  }
  savingEdit.value = true;
  try {
    const saved = await api.updateVoucher(v.id, {
      amountCents: parsed.cents,
      clearAmount: parsed.cents == null,
      description: e.description.trim(),
      counterparty: e.counterparty.trim(),
      tags: parseTags(e.tags),
      note: e.note.trim(),
    });
    // 就地更新列表行与检视器（不整页重载，保持滚动位置与选中项）
    const idx = items.value.findIndex((x) => x.id === saved.id);
    if (idx >= 0) items.value.splice(idx, 1, saved);
    if (detail.value) detail.value = { ...detail.value, voucher: saved };
    resetEdit(saved);
    toast.success("已保存修改");
  } catch (err) {
    toastError(err);
  } finally {
    savingEdit.value = false;
  }
}

const selected = computed(() => items.value.find((v) => v.id === selectedId.value) ?? null);

async function select(v: Voucher) {
  selectedId.value = v.id;
  detailStep.value = 0;
  resetEdit(v);
  detailBusy.value = true;
  try {
    detail.value = await api.getVoucher(v.id);
  } catch (e) {
    toastError(e);
    detail.value = null;
  } finally {
    detailBusy.value = false;
  }
}

/** 检视器里的大图（增强版 → 原图 → 占位） */
const inspectorImage = computed<string | null>(() => {
  const att: Attachment | undefined = detail.value?.attachments[0];
  if (!att) return null;
  const abs = pickCandidate(att, detailStep.value);
  return abs ? convertFileSrc(abs) : null;
});

function onInspectorError() {
  const att = detail.value?.attachments[0];
  if (!att) return;
  detailStep.value = nextFallbackStep(detailStep.value, candidatePaths(att).length);
}

function openEditor(id: number) {
  router.push(`/vouchers/${id}`);
}

/** ↑↓ 移动选择（macOS 列表的常规操作） */
function onKeydown(e: KeyboardEvent) {
  if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
  const tag = (e.target as HTMLElement)?.tagName;
  if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA") return;
  const idx = items.value.findIndex((v) => v.id === selectedId.value);
  if (idx < 0) return;
  const next = e.key === "ArrowDown" ? Math.min(idx + 1, items.value.length - 1) : Math.max(idx - 1, 0);
  if (next !== idx) {
    e.preventDefault();
    void select(items.value[next]);
  }
}

// ---------- 批量操作（保持既有能力） ----------

// 跨页选择集：**不随翻页/筛选清空**（旧实现每次 load 都 clear，导致"勾了 3 页只剩最后 1 页"），
// 只由用户显式清除、或批量操作后重置。
const selectedIds = ref<Set<number>>(new Set());
const confirmDelete = ref(false);
const selectingAll = ref(false);

/** 按**当前筛选**全选（走只读的 list_voucher_ids，不把 N 条对象拉回前端） */
async function selectAllMatching() {
  selectingAll.value = true;
  try {
    const ids = await api.listVoucherIds(query());
    selectedIds.value = new Set(ids);
    toast.info(`已选中当前筛选下的 ${ids.length} 张`);
  } catch (e) {
    toastError(e);
  } finally {
    selectingAll.value = false;
  }
}

function clearSelection() {
  selectedIds.value = new Set();
}
const batchStatus = ref<VoucherStatus>("reviewed");

/** 批量状态流转（单事务逐张快照，后端保证；迁自旧列表页） */
async function applyBatchStatus() {
  if (!selectedIds.value.size) return;
  try {
    const n = await api.batchUpdateStatus([...selectedIds.value], batchStatus.value);
    toast.success(`已把 ${n} 张改为「${STATUS_META[batchStatus.value].label}」`);
    clearSelection();
    await load();
  } catch (e) {
    toastError(e);
  }
}

function toggle(v: Voucher) {
  const s = new Set(selectedIds.value);
  if (s.has(v.id)) s.delete(v.id);
  else s.add(v.id);
  selectedIds.value = s;
}

async function removeSelected() {
  confirmDelete.value = false;
  try {
    const n = await api.deleteVouchers([...selectedIds.value]);
    toast.success(`已删除 ${n} 张`);
    clearSelection();
    detail.value = null;
    await load();
  } catch (e) {
    toastError(e);
  }
}

const totalPages = computed(() => Math.max(1, Math.ceil(total.value / pageSize)));
function gotoPage(p: number) {
  page.value = Math.min(Math.max(1, p), totalPages.value);
  void load();
}

let unlistenImport: UnlistenFn | null = null;

onMounted(async () => {
  readQuery();
  await load();
  unlistenImport = await safeListen("import:done", () => load());
  window.addEventListener("keydown", onKeydown);
});

onUnmounted(() => {
  unlistenImport?.();
  window.removeEventListener("keydown", onKeydown);
});

// 筛选变化即重查（保留在 URL 里，便于返回时回显）
watch([status, dateFrom, dateTo, amountEmpty, ocrStatus, exported, sort], () => {
  page.value = 1;
  void load();
});
watch(keyword, () => {
  page.value = 1;
  void load();
});
</script>

<template>
  <div class="library">
    <!-- 工具栏：筛选（保持 URL 参数通道）+ 批量动作 -->
    <div class="filter-bar">
      <div class="search">
        <FluentIcon name="search" :size="16" />
        <input v-model="keyword" class="search-input" placeholder="搜索编号 / 摘要 / 对方 / 备注" />
      </div>
      <select v-model="status" class="input" style="max-width: 160px">
        <option value="all">全部状态</option>
        <option v-for="(meta, key) in STATUS_META" :key="key" :value="key">{{ meta.label }}</option>
      </select>
      <input v-model="dateFrom" class="input" type="date" style="max-width: 160px" title="起始日期" />
      <input v-model="dateTo" class="input" type="date" style="max-width: 160px" title="结束日期" />
      <template v-for="f in activeFilters" :key="f.label">
        <button class="chip" :title="`清除筛选：${f.label}`" @click="f.clear()">
          {{ f.label }} <FluentIcon name="close" :size="12" />
        </button>
      </template>
      <select v-model="ocrStatus" class="input" style="max-width: 140px" title="按 OCR 状态筛选">
        <option value="">OCR 状态</option>
        <option value="pending">待识别</option>
        <option value="failed">识别失败</option>
        <option value="done">已识别</option>
      </select>
      <select v-model="exported" class="input" style="max-width: 140px" title="按导出覆盖度筛选">
        <option value="">导出覆盖</option>
        <option value="never">从未导出</option>
        <option value="any">导出过</option>
      </select>
      <label class="inline-check" title="只看金额未填的凭证">
        <input v-model="amountEmpty" type="checkbox" /> 金额未填
      </label>
      <select v-model="sort" class="input" style="max-width: 150px" title="排序">
        <option value="date_desc">日期（新→旧）</option>
        <option value="date_asc">日期（旧→新）</option>
        <option value="updated_desc">最近修改</option>
        <option value="amount_desc">金额（大→小）</option>
        <option value="amount_asc">金额（小→大）</option>
      </select>
      <span class="flex-1"></span>
      <span class="count">{{ total }} 张</span>
      <template v-if="selectedIds.size">
        <span class="sel-count">已选 {{ selectedIds.size }} 张</span>
        <select v-model="batchStatus" class="input" style="max-width: 150px" title="批量改状态">
          <option v-for="(meta, key) in STATUS_META" :key="key" :value="key">{{ meta.label }}</option>
        </select>
        <button class="btn" @click="applyBatchStatus()">应用</button>
        <button class="btn" @click="confirmDelete = true">
          <FluentIcon name="delete" :size="14" /> 删除
        </button>
        <button class="btn" title="清除选择" @click="clearSelection()">清除</button>
      </template>
      <button v-else class="btn" :disabled="selectingAll" title="按当前筛选选中全部（跨页生效）"
        @click="selectAllMatching()">
        {{ selectingAll ? "选择中…" : `按筛选全选 ${total} 张` }}
      </button>
      <button class="btn btn-primary" @click="router.push('/vouchers/new')">
        <FluentIcon name="plus" :size="14" /> 新建
      </button>
    </div>

    <!-- 主体：左列表（双行）+ 右检视器 -->
    <div class="split">
      <section class="list-pane">
        <div v-if="loading" class="hint">加载中…</div>
        <EmptyState v-else-if="!items.length" icon="folder" title="没有符合条件的凭证"
          desc="试试清除筛选，或把图片拖进窗口导入。" />

        <ul v-else class="rows">
          <li
            v-for="v in items"
            :key="v.id"
            class="row"
            :class="{ sel: v.id === selectedId }"
            @click="select(v)"
            @dblclick="openEditor(v.id)"
          >
            <input class="row-check" type="checkbox" :checked="selectedIds.has(v.id)" @click.stop="toggle(v)" />
            <div class="row-thumb">
              <img v-if="thumbUrl(v)" :src="thumbUrl(v)!" :alt="v.voucherNo" loading="lazy" @error="onThumbError(v)" />
              <FluentIcon v-else name="image" :size="18" />
            </div>
            <div class="row-main">
              <div class="line1">
                <span class="desc">{{ v.description || "（无摘要）" }}</span>
                <span class="flex-1"></span>
                <span class="amount">{{ fmtAmount(v.amountCents) }}</span>
              </div>
              <div class="line2">
                <span class="mono">{{ v.voucherNo }}</span>
                <span class="dot">·</span>
                <span>{{ v.voucherDate }}</span>
                <span class="dot">·</span>
                <span class="party">{{ v.counterparty || "（无对方）" }}</span>
                <span class="flex-1"></span>
                <span class="tag" :class="STATUS_META[v.status].tag">{{ STATUS_META[v.status].label }}</span>
              </div>
            </div>
          </li>
        </ul>

        <div v-if="totalPages > 1" class="pager">
          <button class="btn" :disabled="page <= 1" @click="gotoPage(page - 1)">上一页</button>
          <span class="muted">第 {{ page }} / {{ totalPages }} 页</span>
          <button class="btn" :disabled="page >= totalPages" @click="gotoPage(page + 1)">下一页</button>
        </div>
      </section>

      <!-- 检视器：选中即看，不跳页 -->
      <aside class="inspector" v-if="selected">
        <header class="ins-head">
          <span class="mono">{{ selected.voucherNo }}</span>
          <span class="flex-1"></span>
          <button class="btn" title="在编辑页打开（双击列表行同效）" @click="openEditor(selected.id)">
            <FluentIcon name="edit" :size="14" /> 编辑
          </button>
        </header>

        <div class="ins-image">
          <img v-if="inspectorImage" :src="inspectorImage" :alt="selected.voucherNo" @error="onInspectorError" />
          <div v-else class="ins-image-empty">
            <FluentIcon name="image" :size="22" />
            <span>{{ detailBusy ? "加载中…" : "无附件或图片不可显示" }}</span>
          </div>
        </div>

        <!-- 就地编辑：常用字段直接改，保存后列表行同步更新（不必进编辑页） -->
        <div v-if="edit" class="ins-edit">
          <label class="ins-row">
            <span>金额</span>
            <input v-model="edit.amount" class="input" placeholder="空 = 未填" />
          </label>
          <label class="ins-row">
            <span>摘要</span>
            <input v-model="edit.description" class="input" />
          </label>
          <label class="ins-row">
            <span>对方</span>
            <input v-model="edit.counterparty" class="input" />
          </label>
          <label class="ins-row">
            <span>标签</span>
            <input v-model="edit.tags" class="input" placeholder="用、或,分隔" />
          </label>
          <label class="ins-row">
            <span>备注</span>
            <input v-model="edit.note" class="input" />
          </label>
          <div class="ins-ops">
            <button class="btn btn-primary" :disabled="!dirty || savingEdit" @click="saveEdit()">
              {{ savingEdit ? "保存中…" : "保存" }}
            </button>
            <button class="btn" :disabled="!dirty || savingEdit" @click="resetEdit(selected)">还原</button>
            <span class="flex-1"></span>
            <span v-if="dirty" class="dirty-hint">有未保存的修改</span>
          </div>
        </div>

        <dl class="ins-meta">
          <div><dt>日期</dt><dd>{{ selected.voucherDate }}</dd></div>
          <div><dt>状态</dt><dd><span class="tag" :class="STATUS_META[selected.status].tag">{{ STATUS_META[selected.status].label }}</span></dd></div>
          <div><dt>最后修改</dt><dd>{{ fmtDate(selected.updatedAt) }}</dd></div>
        </dl>

        <p class="ins-hint">
          就地核对与修改：改完点「保存」即生效；改日期/编号/状态等其余字段请点右上「编辑」（或双击列表行）。
        </p>
      </aside>
    </div>

    <ConfirmDialog
      :visible="confirmDelete"
      title="删除选中的凭证？"
      :body="`将删除 ${selectedIds.size} 张凭证及其图片文件，此操作不可恢复。`"
      confirm-text="删除"
      @confirm="removeSelected"
      @cancel="confirmDelete = false"
    />
  </div>
</template>

<style scoped>
.library { display: flex; flex-direction: column; gap: var(--s-3); height: 100%; }

.filter-bar { display: flex; align-items: center; gap: var(--s-2); flex-wrap: wrap; }
.flex-1 { flex: 1; }
.count { font: var(--type-caption); color: var(--label-tertiary); }
.inline-check { display: inline-flex; align-items: center; gap: 4px; font: var(--type-caption); color: var(--label-secondary); }
.search {
  display: flex; align-items: center; gap: var(--s-2);
  padding: 0 var(--s-2);
  min-width: 260px;
  border: 1px solid var(--separator);
  border-radius: var(--radius-control);
  background: var(--bg-control);
  color: var(--label-tertiary);
}
.search-input { flex: 1; border: none; outline: none; background: transparent; color: var(--label-primary); font: var(--type-body); padding: 7px 0; }
.chip {
  display: inline-flex; align-items: center; gap: 4px;
  height: var(--control-h-sm);
  padding: 0 var(--s-2);
  border: 1px solid var(--separator);
  border-radius: 999px;
  background: var(--bg-selection);
  color: var(--accent);
  font: var(--type-caption);
  cursor: default;
}

.split { display: grid; grid-template-columns: minmax(0, 1.6fr) minmax(320px, 1fr); gap: var(--s-3); align-items: start; }
.list-pane { display: flex; flex-direction: column; gap: var(--s-2); min-width: 0; }
.hint { color: var(--label-tertiary); font: var(--type-body); padding: var(--s-4); }

.rows { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
.row {
  display: flex; align-items: center; gap: var(--s-2);
  padding: var(--s-2) var(--s-3);
  border: 1px solid transparent;
  border-radius: var(--radius-panel);
  cursor: default;
}
.row:hover { background: var(--bg-selection); }
.row.sel { background: var(--bg-selection); border-color: var(--accent); }
.row-check { margin: 0; }
.row-thumb {
  width: 44px; height: 56px; flex-shrink: 0;
  display: flex; align-items: center; justify-content: center;
  border: 1px solid var(--separator);
  border-radius: var(--radius-small);
  background: var(--surface-2);
  color: var(--label-quaternary);
  overflow: hidden;
}
.row-thumb img { width: 100%; height: 100%; object-fit: cover; }
.row-main { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.line1 { display: flex; align-items: baseline; gap: var(--s-2); min-width: 0; }
.desc { font: var(--type-body); color: var(--label-primary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.amount { font: var(--type-body); font-weight: 600; color: var(--label-primary); font-variant-numeric: tabular-nums; }
.line2 { display: flex; align-items: center; gap: var(--s-2); font: var(--type-caption); color: var(--label-tertiary); min-width: 0; }
.line2 .party { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.dot { color: var(--label-quaternary); }
.mono { font-family: var(--font-mono); }

.pager { display: flex; align-items: center; gap: var(--s-3); justify-content: center; padding: var(--s-2); }
.muted { color: var(--label-tertiary); font: var(--type-caption); }

.inspector {
  display: flex; flex-direction: column; gap: var(--s-3);
  padding: var(--s-3);
  border: 1px solid var(--separator);
  border-radius: var(--radius-panel);
  background: var(--bg-content);
  position: sticky; top: 0;
}
.ins-head { display: flex; align-items: center; gap: var(--s-2); }
.ins-image {
  height: 260px; display: flex; align-items: center; justify-content: center;
  border: 1px solid var(--separator); border-radius: var(--radius-control);
  background: var(--surface-2); overflow: hidden;
}
.ins-image img { max-width: 100%; max-height: 100%; object-fit: contain; }
.ins-image-empty { display: flex; flex-direction: column; align-items: center; gap: 6px; color: var(--label-quaternary); font: var(--type-caption); }
.ins-edit { display: flex; flex-direction: column; gap: var(--s-2); }
.ins-row { display: grid; grid-template-columns: 56px 1fr; gap: var(--s-2); align-items: center; }
.ins-row > span { color: var(--label-tertiary); font: var(--type-caption); }
.ins-row .input { padding: 5px 8px; }
.ins-ops { display: flex; align-items: center; gap: var(--s-2); }
.dirty-hint { color: var(--warning); font: var(--type-caption); }
.ins-meta { display: flex; flex-direction: column; gap: 6px; margin: 0; }
.ins-meta > div { display: grid; grid-template-columns: 56px 1fr; gap: var(--s-2); align-items: baseline; }
.ins-meta dt { color: var(--label-tertiary); font: var(--type-caption); }
.ins-meta dd { margin: 0; font: var(--type-body); color: var(--label-primary); overflow-wrap: anywhere; }
.sel-count { font: var(--type-caption); color: var(--accent); white-space: nowrap; }
.ins-hint { margin: 0; font: var(--type-caption); color: var(--label-tertiary); }

@media (max-width: 1100px) {
  .split { grid-template-columns: minmax(0, 1fr); }
  .inspector { position: static; }
}
</style>
