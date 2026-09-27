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

function query() {
  return {
    status: status.value === "all" ? null : status.value,
    keyword: keyword.value.trim() || null,
    dateFrom: dateFrom.value || null,
    dateTo: dateTo.value || null,
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
    if (!items.value.some((v) => v.id === selectedId.value)) {
      selectedId.value = items.value[0]?.id ?? null;
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
}

const activeFilters = computed(() => {
  const out: { label: string; clear: () => void }[] = [];
  if (status.value !== "all") out.push({ label: `状态：${STATUS_META[status.value].label}`, clear: () => (status.value = "all") });
  if (keyword.value) out.push({ label: `关键词：${keyword.value}`, clear: () => (keyword.value = "") });
  if (dateFrom.value || dateTo.value)
    out.push({ label: `日期：${dateFrom.value || "…"} ~ ${dateTo.value || "…"}`, clear: () => ((dateFrom.value = ""), (dateTo.value = "")) });
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

const selected = computed(() => items.value.find((v) => v.id === selectedId.value) ?? null);

async function select(v: Voucher) {
  selectedId.value = v.id;
  detailStep.value = 0;
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

const selectedIds = ref<Set<number>>(new Set());
const confirmDelete = ref(false);
const batchStatus = ref<VoucherStatus>("reviewed");

/** 批量状态流转（单事务逐张快照，后端保证；迁自旧列表页） */
async function applyBatchStatus() {
  if (!selectedIds.value.size) return;
  try {
    const n = await api.batchUpdateStatus([...selectedIds.value], batchStatus.value);
    toast.success(`已把 ${n} 张改为「${STATUS_META[batchStatus.value].label}」`);
    selectedIds.value = new Set();
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
    selectedIds.value = new Set();
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
watch([status, dateFrom, dateTo], () => {
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
      <span class="flex-1"></span>
      <span class="count">{{ total }} 张</span>
      <template v-if="selectedIds.size">
        <select v-model="batchStatus" class="input" style="max-width: 150px" title="批量改状态">
          <option v-for="(meta, key) in STATUS_META" :key="key" :value="key">{{ meta.label }}</option>
        </select>
        <button class="btn" @click="applyBatchStatus()">应用</button>
        <button class="btn" @click="confirmDelete = true">
          <FluentIcon name="delete" :size="14" /> 删除 {{ selectedIds.size }}
        </button>
      </template>
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

        <dl class="ins-fields">
          <div><dt>日期</dt><dd>{{ selected.voucherDate }}</dd></div>
          <div><dt>金额</dt><dd class="amount">{{ fmtAmount(selected.amountCents) }}</dd></div>
          <div><dt>摘要</dt><dd>{{ selected.description || "—" }}</dd></div>
          <div><dt>对方账户</dt><dd>{{ selected.counterparty || "—" }}</dd></div>
          <div><dt>标签</dt><dd>{{ selected.tags?.length ? selected.tags.join("、") : "—" }}</dd></div>
          <div><dt>备注</dt><dd>{{ selected.note || "—" }}</dd></div>
          <div><dt>状态</dt><dd><span class="tag" :class="STATUS_META[selected.status].tag">{{ STATUS_META[selected.status].label }}</span></dd></div>
          <div><dt>最后修改</dt><dd>{{ fmtDate(selected.updatedAt) }}</dd></div>
        </dl>

        <p class="ins-hint">
          就地核对：看大图与字段是否一致；要改字段请点右上「编辑」（或双击列表行）。
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
.ins-fields { display: flex; flex-direction: column; gap: 6px; margin: 0; }
.ins-fields > div { display: grid; grid-template-columns: 84px 1fr; gap: var(--s-2); align-items: baseline; }
.ins-fields dt { color: var(--label-tertiary); font: var(--type-caption); }
.ins-fields dd { margin: 0; font: var(--type-body); color: var(--label-primary); overflow-wrap: anywhere; }
.ins-hint { margin: 0; font: var(--type-caption); color: var(--label-tertiary); }

@media (max-width: 1100px) {
  .split { grid-template-columns: minmax(0, 1fr); }
  .inspector { position: static; }
}
</style>
