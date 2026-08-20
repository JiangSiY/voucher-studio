<script setup lang="ts">
/** 凭证列表（核心页）：筛选/搜索/分页/批量操作/拖放导入/行内操作 */
import { onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { api, type Voucher, type VoucherStatus } from "../api/client";
import ConfirmDialog from "../components/ConfirmDialog.vue";
import EmptyState from "../components/EmptyState.vue";
import { STATUS_META, fmtAmount } from "../composables/format";
import { pickImages, useDropImport } from "../composables/useDropImport";
import { toast, toastError } from "../stores/toast";

const router = useRouter();

type Filter = "all" | VoucherStatus;
const filters: { key: Filter; label: string }[] = [
  { key: "all", label: "全部" },
  { key: "draft", label: "草稿" },
  { key: "reviewed", label: "已复核" },
  { key: "exported", label: "已导出" },
  { key: "archived", label: "已归档" },
];

const items = ref<Voucher[]>([]);
const total = ref(0);
const page = ref(1);
const pageSize = 20;
const filter = ref<Filter>("all");
const keyword = ref("");
const selected = ref<Set<number>>(new Set());
const loading = ref(false);

// 删除确认
const confirmVisible = ref(false);
const confirmText = ref("");
let pendingDelete: number[] = [];

async function load() {
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
    } else {
      items.value = result.items;
      total.value = result.total;
    }
    selected.value.clear();
  } catch (e) {
    toastError(e);
  } finally {
    loading.value = false;
  }
}

function switchFilter(f: Filter) {
  filter.value = f;
  page.value = 1;
  load();
}

function search() {
  page.value = 1;
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

async function importPicked() {
  const files = await pickImages();
  if (!files?.length) return;
  try {
    const summary = await api.importImages(files, null, false);
    toast.success(`已导入 ${summary.imported.length} 张到新凭证 ${summary.voucherNo}`);
    load();
  } catch (e) {
    toastError(e);
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
    load();
  } catch (e) {
    toastError(e);
  }
}

const { dragging, bind } = useDropImport(load);
onMounted(() => {
  bind();
  load();
});
</script>

<template>
  <div class="fluent-panel">
    <div class="fluent-panel-header">
      <h3>凭证</h3>
      <div style="display: flex; gap: 8px">
        <button class="btn btn-sm" @click="importPicked">🖼 导入图片</button>
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
            <tr v-for="v in items" :key="v.id">
              <td><input type="checkbox" class="cb" :checked="selected.has(v.id)" @change="toggle(v.id)" /></td>
              <td><span class="mono">{{ v.voucherNo }}</span></td>
              <td>{{ v.voucherDate }}</td>
              <td style="max-width: 220px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap" :title="v.description">{{ v.description || "—" }}</td>
              <td class="num">{{ fmtAmount(v.amountCents) }}</td>
              <td>
                <span v-for="t in v.tags.slice(0, 2)" :key="t" class="tag tag-muted" style="margin-right: 4px">{{ t }}</span>
                <span v-if="v.tags.length > 2" style="font-size: 11px; color: var(--text-4)">+{{ v.tags.length - 2 }}</span>
              </td>
              <td><span class="tag" :class="STATUS_META[v.status].tag">{{ STATUS_META[v.status].label }}</span></td>
              <td>
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
          <button :disabled="page <= 1" @click="page--; load()">‹</button>
          <button class="active">{{ page }}</button>
          <button :disabled="page >= totalPages()" @click="page++; load()">›</button>
        </div>
      </div>
    </div>
  </div>

  <div v-if="dragging" class="drop-overlay">松手导入图片 → 自动创建凭证</div>

  <ConfirmDialog :visible="confirmVisible" title="删除凭证" :body="confirmText" confirm-text="删除"
    @confirm="doDelete" @cancel="confirmVisible = false" />
</template>
