<script setup lang="ts">
/** 凭证列表（核心页）：筛选/搜索/分页/批量操作/导入/行内操作 */
import { computed, nextTick, onMounted, onUnmounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useConfigStore } from "../stores/config";
import { api, type Voucher, type VoucherStatus, type VoucherThumb, type VoucherListResult, type BatchEditPatch } from "../api/client";
import ConfirmDialog from "../components/ConfirmDialog.vue";
import EmptyState from "../components/EmptyState.vue";
import FluentIcon from "../components/FluentIcon.vue";
import { STATUS_META, fmtAmount } from "../composables/format";
import { toast, toastError } from "../stores/toast";

const router = useRouter();
const route = useRoute();
const configStore = useConfigStore();

type Filter = "all" | VoucherStatus;
const filters: { key: Filter; label: string }[] = [
  { key: "all", label: "全部" },
  { key: "draft", label: "草稿" },
  { key: "reviewed", label: "已复核" },
  { key: "exported", label: "已导出" },
  { key: "archived", label: "已归档" },
];

const items = ref<Voucher[]>([]);
// D-1：每张凭证的缩略图（enhanced 优先）。key = voucher.id
const thumbs = ref<Record<number, VoucherThumb>>({});
const total = ref(0);
const page = ref(1);
const pageSize = 20;
const filter = ref<Filter>("all");
const keyword = ref("");
// v0.9 动线：日期范围筛选（仪表盘月度下钻的落点；与打印中心筛选口径对齐）
const dateFrom = ref("");
const dateTo = ref("");
const selected = ref<Set<number>>(new Set());
const loading = ref(false);

// D-3 翻页预取缓存：key = `${filter}|${keyword}|${pageSize}`，value = page → VoucherListResult
// hover › 200ms 时预取下一页；点击 › 时优先消费缓存，避免翻页闪烁
type ListKey = string;
const prefetched = ref<Map<ListKey, Map<number, VoucherListResult>>>(new Map());
let nextPagePrefetchTimer: ReturnType<typeof setTimeout> | null = null;
function listKey(): ListKey {
  return `${filter.value}|${keyword.value}|${dateFrom.value}|${dateTo.value}|${pageSize}`;
}
function getPrefetched(p: number): VoucherListResult | null {
  return prefetched.value.get(listKey())?.get(p) ?? null;
}
function putPrefetched(p: number, data: VoucherListResult) {
  const k = listKey();
  let m = prefetched.value.get(k);
  if (!m) {
    m = new Map();
    prefetched.value.set(k, m);
  }
  m.set(p, data);
  // 限制每 key 最多 2 页缓存（防内存膨胀）
  if (m.size > 2) {
    const oldest = m.keys().next().value;
    if (oldest !== undefined) m.delete(oldest);
  }
}
function clearPrefetched() {
  if (nextPagePrefetchTimer) {
    clearTimeout(nextPagePrefetchTimer);
    nextPagePrefetchTimer = null;
  }
  prefetched.value.clear();
}

const attachmentsDir = computed(() =>
  configStore.appInfo ? `${configStore.appInfo.dataDir}\\attachments` : "",
);

/** D-1：列表缩略图 URL，enhanced 优先；convertFileSrc 把本地路径转 webview asset 协议 */
function thumbUrl(v: Voucher): string | null {
  const t = thumbs.value[v.id];
  if (!t) return null;
  const rel = t.enhancedPath ?? t.filePath;
  return convertFileSrc(`${attachmentsDir.value}\\${rel.replace(/\//g, "\\")}`);
}

// 删除确认
const confirmVisible = ref(false);
const confirmText = ref("");
let pendingDelete: number[] = [];

/** 列表查询条件（含日期范围；load 与预取共用同一口径） */
function listQuery(p: number) {
  return {
    page: p,
    pageSize,
    status: filter.value === "all" ? null : filter.value,
    keyword: keyword.value || null,
    dateFrom: dateFrom.value || null,
    dateTo: dateTo.value || null,
  };
}

async function load() {
  // D-3：先查预取缓存（用户可能刚点过 › 但 UI 还没消费；或 hover 预取完就点了 ›）
  const cached = getPrefetched(page.value);
  if (cached) {
    items.value = cached.items;
    total.value = cached.total;
    thumbs.value = cached.thumbs ?? {};
    selected.value.clear();
    // 已用过的缓存不再保留，释放内存
    const m = prefetched.value.get(listKey());
    m?.delete(page.value);
    return;
  }
  loading.value = true;
  try {
    const result = await api.listVouchers(listQuery(page.value));
    // 越界回弹：删除末页数据后页码可能超出总页数，回到最后一页重取
    const pages = Math.max(1, Math.ceil(result.total / pageSize));
    if (page.value > pages) {
      page.value = pages;
      const retry = await api.listVouchers(listQuery(page.value));
      items.value = retry.items;
      total.value = retry.total;
      thumbs.value = retry.thumbs ?? {};
    } else {
      items.value = result.items;
      total.value = result.total;
      thumbs.value = result.thumbs ?? {};
    }
    selected.value.clear();
  } catch (e) {
    toastError(e);
  } finally {
    loading.value = false;
    activeIndex.value = null; // 数据变了，高亮重置
  }
}

/** D-3：hover › 200ms 后静默预取下一页；不抢当前 load，不闪烁 */
function prefetchNextPage() {
  if (page.value >= totalPages()) return; // 末页不预取
  if (getPrefetched(page.value + 1)) return; // 已预取过
  if (nextPagePrefetchTimer) clearTimeout(nextPagePrefetchTimer);
  nextPagePrefetchTimer = setTimeout(async () => {
    nextPagePrefetchTimer = null;
    try {
      const data = await api.listVouchers(listQuery(page.value + 1));
      putPrefetched(page.value + 1, data);
    } catch {
      // 静默失败，不打扰用户
    }
  }, 200);
}
function cancelPrefetchNextPage() {
  if (nextPagePrefetchTimer) {
    clearTimeout(nextPagePrefetchTimer);
    nextPagePrefetchTimer = null;
  }
}

function switchFilter(f: Filter) {
  filter.value = f;
  page.value = 1;
  clearPrefetched();
  load();
}

function search() {
  page.value = 1;
  clearPrefetched();
  load();
}

const totalPages = () => Math.max(1, Math.ceil(total.value / pageSize));

/** 编辑页上下文导航携带的筛选（行点击 / 编辑按钮共用；编辑页返回时还原视图） */
function editQuery(): Record<string, string> {
  const q: Record<string, string> = {};
  if (filter.value !== "all") q.status = filter.value;
  if (keyword.value.trim()) q.keyword = keyword.value.trim();
  if (dateFrom.value) q.dateFrom = dateFrom.value;
  if (dateTo.value) q.dateTo = dateTo.value;
  return q;
}

function openEdit(id: number) {
  router.push({ path: `/vouchers/${id}`, query: editQuery() });
}

// ---------- v0.9 键盘导航：↑↓ 移动高亮行、Enter 编辑、Space 勾选（桌面惯例） ----------

const activeIndex = ref<number | null>(null);

function onKeydown(e: KeyboardEvent) {
  // 输入控件/按钮聚焦时不拦截（保持原生行为：打字、Enter 触发按钮）
  const t = e.target as HTMLElement | null;
  if (t && (t.tagName === "INPUT" || t.tagName === "SELECT" || t.tagName === "TEXTAREA" || t.tagName === "BUTTON")) return;
  if (e.ctrlKey || e.altKey || e.metaKey) return;
  if (!items.value.length) return;
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    activeIndex.value = e.key === "ArrowDown"
      ? Math.min(items.value.length - 1, (activeIndex.value ?? -1) + 1)
      : Math.max(0, (activeIndex.value ?? 0) - 1);
    nextTick(() => {
      document
        .querySelector<HTMLTableRowElement>(`tr[data-row-index="${activeIndex.value}"]`)
        ?.scrollIntoView({ block: "nearest" });
    });
  } else if (e.key === "Enter" && activeIndex.value != null) {
    e.preventDefault();
    openEdit(items.value[activeIndex.value].id);
  } else if (e.key === " " && activeIndex.value != null) {
    e.preventDefault(); // 阻止空格滚动页面
    toggle(items.value[activeIndex.value].id);
  }
}

onMounted(() => window.addEventListener("keydown", onKeydown));
onUnmounted(() => window.removeEventListener("keydown", onKeydown));

function toggle(id: number) {
  if (selected.value.has(id)) selected.value.delete(id);
  else selected.value.add(id);
}

function toggleAll() {
  if (selected.value.size === items.value.length) selected.value.clear();
  else items.value.forEach((v) => selected.value.add(v.id));
}

const importing = ref(false);

async function importPicked() {
  if (importing.value) return;
  importing.value = true;
  try {
    // Rust 侧对话框 + 导入一步完成（一图一凭证，每张图片各建一张凭证）
    const summary = await api.pickAndImport(false);
    if (!summary) return; // 用户取消
    const parts: string[] = [];
    if (summary.vouchers.length) parts.push(`已导入 ${summary.vouchers.length} 张图片，各建一张凭证`);
    if (summary.skippedDuplicates.length) parts.push(`跳过重复 ${summary.skippedDuplicates.length} 张`);
    if (summary.failed.length) parts.push(`失败 ${summary.failed.length} 张`);
    if (parts.length) {
      // 动线：导入完成即引导核对（跳到第一张新凭证）
      toast.success(parts.join(" · "), {
        label: "开始核对",
        onClick: () => {
          if (summary.vouchers.length) router.push(`/vouchers/${summary.vouchers[0].voucherId}`);
        },
      });
    }
    load();
  } catch (e) {
    toastError(e);
  } finally {
    importing.value = false;
  }
}

async function exportOne(v: Voucher) {
  try {
    const batch = await api.exportPdf([v.id]);
    // 动线：导出即得——toast 内直接打开，不必绕道导出历史页
    toast.success(`PDF 已生成：${batch.batchNo}`, {
      label: "打开",
      onClick: () => void api.openPath(batch.pdfPath),
    });
  } catch (e) {
    toastError(e);
  }
}

async function exportSelected() {
  const ids = [...selected.value];
  if (!ids.length) return;
  try {
    const batch = await api.exportPdf(ids);
    toast.success(`已导出 ${batch.voucherCount} 张凭证 → ${batch.batchNo}`, {
      label: "打开",
      onClick: () => void api.openPath(batch.pdfPath),
    });
    load();
  } catch (e) {
    toastError(e);
  }
}

// ---------- v0.7 B-1：批量状态流转（单事务逐张快照，后端保证原子性） ----------

const batchStatus = ref<VoucherStatus>("reviewed");
const batchStatusBusy = ref(false);

async function applyBatchStatus() {
  const ids = [...selected.value];
  if (!ids.length || batchStatusBusy.value) return;
  batchStatusBusy.value = true;
  try {
    const n = await api.batchUpdateStatus(ids, batchStatus.value);
    if (n < ids.length) {
      toast.warning(`已变更 ${n} 张（${ids.length - n} 张已是目标状态或不存在）`);
    } else {
      toast.success(`已将 ${n} 张凭证标记为「${STATUS_META[batchStatus.value].label}」`);
    }
    clearPrefetched();
    load();
  } catch (e) {
    toastError(e);
  } finally {
    batchStatusBusy.value = false;
  }
}

// ---------- 功能深化 2：批量编辑（日期/备注/标签，单事务逐张快照） ----------

const batchEditVisible = ref(false);
const batchEditBusy = ref(false);
const editDate = ref("");
const editNote = ref("");
const editTags = ref("");

/** 打开面板时回显当前选中集的公共值：全选中唯一值才预填（混合值留空，避免误导） */
function openBatchEdit() {
  const sel = items.value.filter((v) => selected.value.has(v.id));
  const dates = new Set(sel.map((v) => v.voucherDate));
  if (dates.size === 1) editDate.value = sel[0].voucherDate;
  else editDate.value = "";
  const notes = new Set(sel.map((v) => v.note));
  editNote.value = notes.size === 1 ? sel[0].note : "";
  const tagSets = new Set(sel.map((v) => v.tags.join(",")));
  editTags.value = tagSets.size === 1 ? sel[0].tags.join(", ") : "";
  batchEditVisible.value = true;
}

async function applyBatchEdit() {
  const ids = [...selected.value];
  if (!ids.length || batchEditBusy.value) return;
  const patch: BatchEditPatch = {};
  if (editDate.value.trim()) patch.voucherDate = editDate.value.trim();
  if (editNote.value.trim()) patch.note = editNote.value.trim();
  const tags = editTags.value
    .split(/[,，、]/)
    .map((t) => t.trim())
    .filter(Boolean);
  if (tags.length) patch.tags = tags;
  if (!Object.keys(patch).length) {
    toast.warning("至少填写一项要修改的内容");
    return;
  }
  batchEditBusy.value = true;
  try {
    const n = await api.batchEdit(ids, patch);
    if (n < ids.length) {
      toast.warning(`已修改 ${n} 张（${ids.length - n} 张已是目标值或不存在）`);
    } else {
      toast.success(`已修改 ${n} 张凭证`);
    }
    batchEditVisible.value = false;
    clearPrefetched();
    load();
  } catch (e) {
    toastError(e);
  } finally {
    batchEditBusy.value = false;
  }
}

function askDelete(ids: number[]) {
  pendingDelete = ids;
  const suffix = "删除后凭证、附件图片及历史快照将被永久移除，不可恢复。";
  confirmText.value =
    ids.length === 1
      ? `确定删除这 1 张凭证吗？${suffix}`
      : `确定删除选中的 ${ids.length} 张凭证吗？${suffix}`;
  confirmVisible.value = true;
}

async function doDelete() {
  confirmVisible.value = false;
  try {
    const n = await api.deleteVouchers(pendingDelete);
    if (n < pendingDelete.length) {
      toast.warning(`已删除 ${n} 张（${pendingDelete.length - n} 张不存在或已删除）`);
    } else {
      toast.success(`已删除 ${n} 张凭证`);
    }
    // D-3：删后数据变化（总数 / 页数）→ 清预取缓存
    clearPrefetched();
    load();
  } catch (e) {
    toastError(e);
  }
}

// 拖放导入由 Rust 侧处理（App.vue 全局反馈），此处监听入库事件刷新列表
let unlistenImportDone: UnlistenFn | null = null;

onMounted(async () => {
  // v0.7 R-2：从仪表盘状态卡片跳转进来时按 status 预筛选（非法值忽略）
  // v0.9：编辑页返回/仪表盘下钻也带 keyword/dateFrom/dateTo，完整还原来时视图
  const q = route.query;
  if (typeof q.status === "string" && filters.some((f) => f.key === q.status)) {
    filter.value = q.status as Filter;
  }
  if (typeof q.keyword === "string" && q.keyword.trim()) {
    keyword.value = q.keyword.trim();
  }
  if (typeof q.dateFrom === "string" && q.dateFrom) {
    dateFrom.value = q.dateFrom;
  }
  if (typeof q.dateTo === "string" && q.dateTo) {
    dateTo.value = q.dateTo;
  }
  load();
  unlistenImportDone = await listen("import:done", () => {
    // D-3：导入后总数变化 → 清预取缓存
    clearPrefetched();
    load();
  });
});

onUnmounted(() => {
  unlistenImportDone?.();
  cancelPrefetchNextPage();
});
</script>

<template>
  <div class="fluent-panel">
    <div class="fluent-panel-header">
      <h3>凭证</h3>
      <div style="display: flex; gap: 8px">
        <button class="btn btn-sm" :disabled="importing" @click="importPicked">
          <FluentIcon name="image" :size="14" /> {{ importing ? "导入中…" : "导入图片" }}
        </button>
        <button class="btn btn-sm btn-primary" @click="router.push('/vouchers/new')">＋ 新建凭证</button>
      </div>
    </div>

    <div style="display: flex; gap: 12px; padding: 12px; border-bottom: 1px solid var(--stroke); flex-wrap: wrap; align-items: center">
      <div class="search">
        <input v-model="keyword" placeholder="搜索编号 / 摘要 / 备注…（全文搜索请用搜索页）" @keyup.enter="search" />
      </div>
      <div class="filter-chips">
        <button v-for="f in filters" :key="f.key" class="fchip" :class="{ active: filter === f.key }"
          @click="switchFilter(f.key)">
          {{ f.label }}
        </button>
      </div>
      <!-- v0.9：日期范围筛选（仪表盘月度下钻落点；变更即查询） -->
      <div style="display: flex; align-items: center; gap: 6px; margin-left: auto">
        <input v-model="dateFrom" type="date" class="input" style="width: 130px" title="起始日期"
          @change="search" />
        <span style="color: var(--text-3); font-size: 12px">至</span>
        <input v-model="dateTo" type="date" class="input" style="width: 130px" title="结束日期"
          @change="search" />
      </div>
    </div>

    <div style="padding: 0 16px" v-if="selected.size > 0">
      <div style="display: flex; align-items: center; justify-content: space-between; padding: 8px 0; background: var(--accent-soft); margin: 8px 0; border-radius: 4px; padding: 8px 12px">
        <span style="font-size: 13px; color: var(--accent); font-weight: 600">已选择 {{ selected.size }} 项</span>
        <div style="display: flex; gap: 8px; align-items: center">
          <!-- v0.7 B-1：批量状态流转 -->
          <select v-model="batchStatus" class="input batch-status-select"
            title="选择目标状态后应用到全部选中凭证">
            <option value="draft">草稿</option>
            <option value="reviewed">已复核</option>
            <option value="exported">已导出</option>
            <option value="archived">已归档</option>
          </select>
          <button class="btn btn-sm" :disabled="batchStatusBusy" @click="applyBatchStatus">
            {{ batchStatusBusy ? "变更中…" : "批量改状态" }}
          </button>
          <button class="btn btn-sm" @click="openBatchEdit"><FluentIcon name="edit" :size="14" />批量编辑</button>
          <button class="btn btn-sm" @click="exportSelected"><FluentIcon name="upload" :size="14" />批量导出 PDF</button>
          <button class="btn btn-sm btn-danger" @click="askDelete([...selected])"><FluentIcon name="delete" :size="14" />删除选中</button>
        </div>
      </div>
      <!-- 功能深化 2：批量编辑面板（日期/备注/标签；改日期会按新日期重排档号） -->
      <div v-if="batchEditVisible" class="batch-edit-panel">
        <div class="batch-edit-row">
          <label>日期<input v-model="editDate" type="date" class="input" title="留空 = 不修改；变更后档号按新日期重排" /></label>
          <label>备注<input v-model="editNote" type="text" class="input" placeholder="留空 = 不修改" /></label>
          <label>标签<input v-model="editTags" type="text" class="input" placeholder="逗号分隔，如：差旅, 报销（整体替换）" /></label>
        </div>
        <div class="batch-edit-row" style="justify-content: flex-end">
          <span class="batch-edit-hint">留空的项不修改 · 已选中 {{ selected.size }} 张</span>
          <button class="btn btn-sm" @click="batchEditVisible = false">取消</button>
          <button class="btn btn-sm btn-primary" :disabled="batchEditBusy" @click="applyBatchEdit">
            {{ batchEditBusy ? "应用中…" : "应用到选中" }}
          </button>
        </div>
      </div>
    </div>

    <div class="fluent-panel-body" style="padding: 0">
      <div class="dtable-wrap">
        <table class="dtable" v-if="items.length">
          <thead>
            <tr>
              <th style="width: 32px"><input type="checkbox" class="cb" :checked="selected.size === items.length && items.length > 0" @change="toggleAll" /></th>
              <!-- D-1：缩略图列（增强版优先显示） -->
              <th style="width: 64px">图</th>
              <th>凭证编号</th>
              <th>日期</th>
              <th>摘要</th>
              <th style="text-align: right">金额</th>
              <th>标签</th>
              <th>状态</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            <!-- 场景11：行点击进编辑页；复选框/操作按钮 stop 冒泡不触发跳转 -->
            <tr v-for="(v, i) in items" :key="v.id" :data-row-index="i"
              :class="{ 'row-active': activeIndex === i }" style="cursor: pointer" @click="openEdit(v.id)">
              <td @click.stop><input type="checkbox" class="cb" :checked="selected.has(v.id)" @change="toggle(v.id)" /></td>
              <td>
                <div v-if="thumbUrl(v)" class="list-thumb" :title="thumbs[v.id]?.enhancedPath ? '增强版缩略图（点击行进入编辑页查看大图）' : '原图缩略图'">
                  <img :src="thumbUrl(v)!" :alt="v.voucherNo" loading="lazy" />
                  <span v-if="thumbs[v.id]?.enhancedSource === 'scan'" class="list-thumb-source" title="扫描件">扫</span>
                  <span v-else-if="thumbs[v.id]?.enhancedSource === 'screenshot'" class="list-thumb-source muted" title="截图">截</span>
                </div>
                <div v-else class="list-thumb list-thumb-empty" title="无附件"><FluentIcon name="image" :size="18" /></div>
              </td>
              <td><span class="mono">{{ v.voucherNo }}</span></td>
              <td>{{ v.voucherDate }}</td>
              <td style="max-width: 220px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap" :title="v.description">{{ v.description || "—" }}</td>
              <td style="max-width: 140px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap" :title="v.counterparty">{{ v.counterparty || "—" }}</td>
              <td class="num">{{ fmtAmount(v.amountCents) }}</td>
              <td>
                <span v-for="t in v.tags.slice(0, 2)" :key="t" class="tag tag-muted" style="margin-right: 4px">{{ t }}</span>
                <span v-if="v.tags.length > 2" style="font-size: 11px; color: var(--text-4)">+{{ v.tags.length - 2 }}</span>
              </td>
              <td><span class="tag" :class="STATUS_META[v.status].tag">{{ STATUS_META[v.status].label }}</span></td>
              <td @click.stop>
                <div class="action-row">
                  <button class="icon-btn" title="编辑" @click="openEdit(v.id)"><FluentIcon name="edit" :size="14" /></button>
                  <button class="icon-btn" title="导出 PDF" @click="exportOne(v)"><FluentIcon name="print" :size="14" /></button>
                  <button class="icon-btn danger" title="删除" @click="askDelete([v.id])"><FluentIcon name="delete" :size="14" /></button>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
        <EmptyState v-else-if="!loading" icon="folder" title="没有符合条件的凭证"
          desc="拖入图片快速建卡，或调整筛选条件">
          <button class="btn btn-primary" @click="router.push('/vouchers/new')">＋ 新建凭证</button>
        </EmptyState>
      </div>

      <div class="tbl-foot" style="padding: 12px 16px">
        <div class="info">
          共 {{ total }} 条记录
          <span style="color: var(--text-4); margin-left: 8px">↑↓ 选择 · Enter 编辑 · Space 勾选</span>
        </div>
        <div class="pager">
          <button :disabled="page <= 1" @mouseenter="cancelPrefetchNextPage" @click="page--; load()"><FluentIcon name="chevronLeft" :size="13" /></button>
          <button class="active">{{ page }}</button>
          <button :disabled="page >= totalPages()" @mouseenter="prefetchNextPage" @mouseleave="cancelPrefetchNextPage" @click="page++; load()"><FluentIcon name="chevronRight" :size="13" /></button>
        </div>
      </div>
    </div>
  </div>

  <ConfirmDialog :visible="confirmVisible" title="删除凭证" :body="confirmText" confirm-text="删除"
    @confirm="doDelete" @cancel="confirmVisible = false" />
</template>

<style scoped>
/* v0.9 键盘导航：高亮行 */
tr.row-active td {
  background: var(--accent-soft);
}

/* v0.7 B-1：批量状态选择器（工具栏内紧凑尺寸） */
.batch-status-select {
  width: auto;
  padding: 4px 8px;
  font-size: 13px;
}

/* 功能深化 2：批量编辑面板 */
.batch-edit-panel {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px 12px;
  background: var(--bg-2, rgba(0, 0, 0, 0.03));
  border: 1px solid var(--stroke);
  border-radius: 4px;
}
.batch-edit-row {
  display: flex;
  gap: 12px;
  align-items: center;
  flex-wrap: wrap;
}
.batch-edit-row label {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  color: var(--text-2);
}
.batch-edit-row .input {
  width: 200px;
  padding: 4px 8px;
  font-size: 13px;
}
.batch-edit-hint {
  font-size: 12px;
  color: var(--text-3);
  margin-right: auto;
}
</style>
