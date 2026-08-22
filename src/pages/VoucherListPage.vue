<script setup lang="ts">
/** 凭证列表（核心页）：筛选/搜索/分页/批量操作/导入/行内操作 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useRouter } from "vue-router";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useConfigStore } from "../stores/config";
import { api, type Voucher, type VoucherStatus, type VoucherThumb, type VoucherListResult } from "../api/client";
import ConfirmDialog from "../components/ConfirmDialog.vue";
import EmptyState from "../components/EmptyState.vue";
import { STATUS_META, fmtAmount } from "../composables/format";
import { toast, toastError } from "../stores/toast";

const router = useRouter();
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
const selected = ref<Set<number>>(new Set());
const loading = ref(false);

// D-3 翻页预取缓存：key = `${filter}|${keyword}|${pageSize}`，value = page → VoucherListResult
// hover › 200ms 时预取下一页；点击 › 时优先消费缓存，避免翻页闪烁
type ListKey = string;
const prefetched = ref<Map<ListKey, Map<number, VoucherListResult>>>(new Map());
let nextPagePrefetchTimer: ReturnType<typeof setTimeout> | null = null;
function listKey(): ListKey {
  return `${filter.value}|${keyword.value}|${pageSize}`;
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
    const result = await api.listVouchers({
      page: page.value,
      pageSize,
      status: filter.value === "all" ? null : filter.value,
      keyword: keyword.value || null,
    });
    // 越界回弹：删除末页数据后页码可能超出总页数，回到最后一页重取
    const pages = Math.max(1, Math.ceil(result.total / pageSize));
    if (page.value > pages) {
      page.value = pages;
      const retry = await api.listVouchers({
        page: page.value,
        pageSize,
        status: filter.value === "all" ? null : filter.value,
        keyword: keyword.value || null,
      });
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
      const data = await api.listVouchers({
        page: page.value + 1,
        pageSize,
        status: filter.value === "all" ? null : filter.value,
        keyword: keyword.value || null,
      });
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
    if (parts.length) toast.success(parts.join(" · "));
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
    toast.success(`PDF 已生成：${batch.batchNo}`);
  } catch (e) {
    toastError(e);
  }
}

async function exportSelected() {
  const ids = [...selected.value];
  if (!ids.length) return;
  try {
    const batch = await api.exportPdf(ids);
    toast.success(`已导出 ${batch.voucherCount} 张凭证 → ${batch.batchNo}`);
    load();
  } catch (e) {
    toastError(e);
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
let unlistenWxImported: UnlistenFn | null = null;

onMounted(async () => {
  load();
  unlistenImportDone = await listen("import:done", () => {
    // D-3：导入后总数变化 → 清预取缓存
    clearPrefetched();
    load();
  });
  // 微信监听为后台入库，用户停留本页时列表同样需要刷新
  unlistenWxImported = await listen("wechat:imported", () => {
    clearPrefetched();
    load();
  });
});

onUnmounted(() => {
  unlistenImportDone?.();
  unlistenWxImported?.();
  cancelPrefetchNextPage();
});
</script>

<template>
  <div class="fluent-panel">
    <div class="fluent-panel-header">
      <h3>凭证</h3>
      <div style="display: flex; gap: 8px">
        <button class="btn btn-sm" :disabled="importing" @click="importPicked">
          {{ importing ? "导入中…" : "🖼 导入图片" }}
        </button>
        <button class="btn btn-sm btn-primary" @click="router.push('/vouchers/new')">＋ 新建凭证</button>
      </div>
    </div>

    <div style="display: flex; gap: 12px; padding: 12px; border-bottom: 1px solid var(--stroke); flex-wrap: wrap; align-items: center">
      <div class="search">
        <input v-model="keyword" placeholder="搜索编号 / 摘要 / 备注…" @keyup.enter="search" />
      </div>
      <div class="filter-chips">
        <button v-for="f in filters" :key="f.key" class="fchip" :class="{ active: filter === f.key }"
          @click="switchFilter(f.key)">
          {{ f.label }}
        </button>
      </div>
    </div>

    <div style="padding: 0 16px" v-if="selected.size > 0">
      <div style="display: flex; align-items: center; justify-content: space-between; padding: 8px 0; background: var(--accent-soft); margin: 8px 0; border-radius: 4px; padding: 8px 12px">
        <span style="font-size: 13px; color: var(--accent); font-weight: 600">已选择 {{ selected.size }} 项</span>
        <div style="display: flex; gap: 8px">
          <button class="btn btn-sm" @click="exportSelected">📤 批量导出 PDF</button>
          <button class="btn btn-sm btn-danger" @click="askDelete([...selected])">🗑 删除选中</button>
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
            <tr v-for="v in items" :key="v.id" style="cursor: pointer" @click="router.push(`/vouchers/${v.id}`)">
              <td @click.stop><input type="checkbox" class="cb" :checked="selected.has(v.id)" @change="toggle(v.id)" /></td>
              <td>
                <div v-if="thumbUrl(v)" class="list-thumb" :title="thumbs[v.id]?.enhancedPath ? '✨ 增强版缩略图（点击行进入编辑页查看大图）' : '原图缩略图'">
                  <img :src="thumbUrl(v)!" :alt="v.voucherNo" loading="lazy" />
                  <span v-if="thumbs[v.id]?.enhancedSource === 'scan'" class="list-thumb-source" title="扫描件">扫</span>
                  <span v-else-if="thumbs[v.id]?.enhancedSource === 'screenshot'" class="list-thumb-source muted" title="截图">截</span>
                </div>
                <div v-else class="list-thumb list-thumb-empty" title="无附件">🖼</div>
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
                  <button class="icon-btn" title="编辑" @click="router.push(`/vouchers/${v.id}`)">✎</button>
                  <button class="icon-btn" title="导出 PDF" @click="exportOne(v)">🖨</button>
                  <button class="icon-btn danger" title="删除" @click="askDelete([v.id])">🗑</button>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
        <EmptyState v-else-if="!loading" icon="🗂" title="没有符合条件的凭证"
          desc="拖入图片快速建卡，或调整筛选条件">
          <button class="btn btn-primary" @click="router.push('/vouchers/new')">＋ 新建凭证</button>
        </EmptyState>
      </div>

      <div class="tbl-foot" style="padding: 12px 16px">
        <div class="info">共 {{ total }} 条记录</div>
        <div class="pager">
          <button :disabled="page <= 1" @mouseenter="cancelPrefetchNextPage" @click="page--; load()">‹</button>
          <button class="active">{{ page }}</button>
          <button :disabled="page >= totalPages()" @mouseenter="prefetchNextPage" @mouseleave="cancelPrefetchNextPage" @click="page++; load()">›</button>
        </div>
      </div>
    </div>
  </div>

  <ConfirmDialog :visible="confirmVisible" title="删除凭证" :body="confirmText" confirm-text="删除"
    @confirm="doDelete" @cancel="confirmVisible = false" />
</template>
