<script setup lang="ts">
/**
 * 数据页（v0.27）：数据库视图 + 行内展开。
 *
 * 本页是全站唯一的新建 / 导入 / 打印导出入口；台账三段与「待入账」都落在视图 chip 上。
 * 行展开直接嵌 `VoucherEditor`（同一套校验/附件/台账字段），不再另造编辑面。
 */
import { computed, onMounted, reactive, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import FluentIcon from "../components/FluentIcon.vue";
import VoucherEditor from "../components/voucher/VoucherEditor.vue";
import { assetUrl } from "../api/asset";
import {
  api,
  type ImportBatch,
  type Voucher,
  type VoucherDetail,
  type VoucherQuery,
  type VoucherRowMeta,
  type VoucherStatus,
  type VoucherThumb,
} from "../api/client";
import { toast, toastError } from "../stores/toast";
import { useModalRoute } from "../composables/useModalRoute";
import NewRecordDialog from "../components/dialogs/NewRecordDialog.vue";

type View = "all" | "missing" | "legacy" | "booked" | "draft" | "month" | "batches";

const views: { key: View; label: string }[] = [
  { key: "all", label: "全部" },
  { key: "missing", label: "待补凭证" },
  { key: "legacy", label: "历史未补" },
  { key: "booked", label: "待入账" },
  { key: "draft", label: "待核对" },
  { key: "month", label: "本月" },
  { key: "batches", label: "导入批次" },
];

const statusLabel: Record<VoucherStatus, string> = {
  draft: "草稿",
  reviewed: "已复核",
  exported: "已导出",
  archived: "已归档",
};

const activeView = ref<View>("all");
const page = ref(1);
const pageSize = 50;
const total = ref(0);
const loading = ref(false);
const error = ref("");
const items = ref<Voucher[]>([]);
const thumbs = ref<Record<number, VoucherThumb>>({});
const meta = ref<Record<number, VoucherRowMeta>>({});
const batches = ref<ImportBatch[]>([]);
const batchId = ref<number | null>(null);
const expanded = ref<number | null>(null);
const details = reactive<Record<number, VoucherDetail>>({});
const detailBusy = ref(0);
const { open } = useModalRoute();
const route = useRoute();
const router = useRouter();
const newOpen = ref(route.query.new === "1");

function ymd(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

function monthRange(): { from: string; to: string } {
  const now = new Date();
  const first = new Date(now.getFullYear(), now.getMonth(), 1);
  const last = new Date(now.getFullYear(), now.getMonth() + 1, 0);
  return { from: ymd(first), to: ymd(last) };
}

const query = computed<VoucherQuery>(() => {
  const base: VoucherQuery = { page: page.value, pageSize, sort: "date_desc" };
  switch (activeView.value) {
    case "missing":
      return { ...base, noImage: true, attachmentRequired: true };
    case "legacy":
      return { ...base, noImage: true, attachmentRequired: false };
    case "booked":
      return { ...base, booked: false };
    case "draft":
      return { ...base, statuses: ["draft"] };
    case "month": {
      const { from, to } = monthRange();
      return { ...base, dateFrom: from, dateTo: to };
    }
    case "batches":
      return batchId.value === null ? { ...base, page: 1 } : { ...base, batchId: batchId.value };
    default:
      return base;
  }
});

async function load() {
  if (activeView.value === "batches" && batchId.value === null) {
    items.value = [];
    total.value = 0;
    return;
  }
  loading.value = true;
  error.value = "";
  try {
    const res = await api.listVouchers(query.value);
    items.value = res.items;
    thumbs.value = res.thumbs;
    meta.value = res.meta ?? {};
    total.value = res.total;
    await ensureFocus();
  } catch (e) {
    error.value = String(e);
    toastError(e);
  } finally {
    loading.value = false;
  }
}

/** 深链 ?focus=<id>：目标不在当前页时单独取回并置顶展开。 */
async function ensureFocus() {
  const id = Number(route.query.focus);
  if (!id || !Number.isFinite(id)) return;
  if (!items.value.some((r) => r.id === id)) {
    const d = await api.getVoucher(id);
    if (!d) return;
    items.value = [d.voucher, ...items.value];
    meta.value = { ...meta.value, [id]: d.rowMeta };
    if (d.attachments[0]) thumbs.value = { ...thumbs.value, [id]: d.attachments[0] };
  }
  expanded.value = id;
  await loadDetail(id);
}

async function loadBatches() {
  if (batches.value.length) return;
  try {
    batches.value = await api.listImportBatches(30);
  } catch (e) {
    toastError(e);
  }
}

function viewRows(): Voucher[] {
  if (activeView.value === "booked") {
    return items.value.filter((v) => !!thumbs.value[v.id] && !meta.value[v.id]?.booked);
  }
  return items.value;
}

const rows = computed(() => viewRows());

function metaOf(id: number): VoucherRowMeta {
  return meta.value[id] ?? { booked: false, batchId: null, attachmentRequired: true };
}

function amountText(cents: number | null): string {
  if (cents === null) return "—";
  return `¥${(cents / 100).toLocaleString("zh-CN", { maximumFractionDigits: 0 })}`;
}

function categoryOf(v: Voucher): string {
  return v.tags.find((t) => t !== "收入" && t !== "支出") ?? "—";
}

function thumbUrl(id: number): string {
  const t = thumbs.value[id];
  return t?.fileAbs ? assetUrl(t.fileAbs) : "";
}

function toggle(id: number) {
  if (expanded.value === id) {
    expanded.value = null;
    return;
  }
  expanded.value = id;
  void loadDetail(id);
}

async function loadDetail(id: number) {
  if (details[id]) return;
  detailBusy.value += 1;
  try {
    details[id] = await api.getVoucher(id);
  } catch (e) {
    toastError(e);
  } finally {
    detailBusy.value -= 1;
  }
}

function onSaved(v: Voucher) {
  const i = items.value.findIndex((x) => x.id === v.id);
  if (i >= 0) items.value[i] = v;
  details[v.id] = { ...details[v.id], voucher: v };
  void load();
}

function onDetail(d: VoucherDetail) {
  details[d.voucher.id] = d;
}

function setView(v: View) {
  activeView.value = v;
  page.value = 1;
  expanded.value = null;
  if (v === "batches") void loadBatches();
}

function goto(p: number) {
  page.value = Math.max(1, p);
  expanded.value = null;
  void load();
}

function later(what: string) {
  toast.info(`${what}将在 B2b 后续接入`);
}

async function importNow() {
  try {
    const summary = await api.pickAndImport(true);
    if (summary) {
      toast.success(`已导入 ${summary.vouchers.length} 张凭证`);
      await load();
    }
  } catch (e) {
    toastError(e);
  }
}

function openNew() {
  newOpen.value = true;
  void router.replace({ query: { ...route.query, new: "1" } });
}

function closeNew() {
  newOpen.value = false;
  const q = { ...route.query };
  delete q.new;
  void router.replace({ query: q });
}

async function onCreated(v: Voucher) {
  closeNew();
  await load();
  expanded.value = v.id;
  await loadDetail(v.id);
}

watch(
  () => route.query.new,
  (v) => {
    newOpen.value = v === "1";
  },
);

watch(
  () => route.query.focus,
  () => {
    void ensureFocus();
  },
);

onMounted(load);
watch(query, load);
</script>

<template>
  <div class="page page--wide">
    <div class="data-toolbar">
      <div class="data-chips" role="group" aria-label="视图">
        <button
          v-for="v in views"
          :key="v.key"
          class="chip"
          :class="{ 'is-active': activeView === v.key }"
          type="button"
          :aria-pressed="activeView === v.key"
          @click="setView(v.key)"
        >
          {{ v.label }}
        </button>
      </div>

      <div class="data-actions">
        <button class="btn btn-primary" type="button" @click="openNew">
          <FluentIcon name="plus" :size="14" />
          <span>新建条目</span>
        </button>
        <button class="btn" type="button" @click="importNow">
          <FluentIcon name="upload" :size="14" />
          <span>导入</span>
        </button>
        <button class="btn" type="button" @click="open('export')">
          <FluentIcon name="print" :size="14" />
          <span>打印导出</span>
        </button>
        <button
          class="btn btn-icon"
          type="button"
          aria-label="筛选"
          title="筛选"
          @click="later('筛选')"
        >
          <FluentIcon name="compare" :size="16" />
        </button>
        <button
          class="btn btn-icon"
          type="button"
          aria-label="列设置"
          title="列设置"
          @click="later('列设置')"
        >
          <FluentIcon name="list" :size="16" />
        </button>
      </div>
    </div>

    <div v-if="activeView === 'batches'" class="data-batchbar">
      <label class="data-field">
        <span>导入批次</span>
        <select v-model="batchId" class="input" @change="goto(1)">
          <option :value="null">请选择批次</option>
          <option v-for="b in batches" :key="b.id" :value="b.id">
            {{ b.createdAt.slice(0, 10) }} · {{ b.fileName }} · {{ b.created }} 条
          </option>
        </select>
      </label>
    </div>

    <div class="data-panel">
      <table class="data-table">
        <thead>
          <tr>
            <th class="data-check" scope="col"><span class="sr-only">选择</span></th>
            <th class="col-hide-sm" scope="col">编号</th>
            <th scope="col">日期</th>
            <th scope="col">客户</th>
            <th class="col-hide-sm" scope="col">摘要</th>
            <th class="col-hide-sm" scope="col">类别</th>
            <th class="data-amount" scope="col">金额</th>
            <th class="col-hide-sm" scope="col">凭证</th>
            <th scope="col">状态</th>
            <th class="col-hide-sm" scope="col">入账</th>
          </tr>
        </thead>
        <tbody>
          <template v-for="r in rows" :key="r.id">
            <tr
              class="data-row"
              :class="{ 'is-open': expanded === r.id }"
              tabindex="0"
              @click="toggle(r.id)"
              @keydown.enter="toggle(r.id)"
            >
              <td class="data-check" @click.stop>
                <input type="checkbox" :aria-label="`选择 ${r.voucherNo}`" />
              </td>
              <td class="data-no col-hide-sm">{{ r.voucherNo }}</td>
              <td>{{ r.voucherDate }}</td>
              <td>{{ r.counterparty || "—" }}</td>
              <td class="data-desc col-hide-sm">{{ r.description || "—" }}</td>
              <td class="col-hide-sm">{{ categoryOf(r) }}</td>
              <td class="data-amount">{{ amountText(r.amountCents) }}</td>
              <td class="col-hide-sm">
                <img
                  v-if="thumbUrl(r.id)"
                  :src="thumbUrl(r.id)"
                  class="data-thumb-img"
                  alt=""
                  loading="lazy"
                />
                <span
                  v-else-if="metaOf(r.id).attachmentRequired"
                  class="data-thumb data-thumb--missing"
                  >缺</span
                >
                <span v-else class="data-thumb" title="迁移豁免：历史无凭证条目">免</span>
              </td>
              <td>
                <span class="data-status" :data-status="r.status">{{ statusLabel[r.status] }}</span>
              </td>
              <td class="col-hide-sm">
                <span
                  class="data-booked"
                  :class="{ 'is-on': metaOf(r.id).booked }"
                  :aria-label="metaOf(r.id).booked ? '已入账' : '未入账'"
                >
                  {{ metaOf(r.id).booked ? "已入" : "未入" }}
                </span>
              </td>
            </tr>
            <tr v-if="expanded === r.id" class="data-expand">
              <td colspan="10">
                <div v-if="details[r.id]" class="data-expand-inner">
                  <VoucherEditor
                    :voucher="details[r.id].voucher"
                    :detail="details[r.id]"
                    @saved="onSaved"
                    @detail="onDetail"
                  />
                </div>
                <p v-else class="wb-empty">{{ detailBusy ? "正在读取…" : "展开失败，请重试。" }}</p>
              </td>
            </tr>
          </template>
          <tr v-if="!rows.length && !loading">
            <td colspan="10">
              <p class="wb-empty">
                {{
                  error || (activeView === "legacy" ? "没有历史未补凭证。" : "当前视图没有条目。")
                }}
              </p>
            </td>
          </tr>
        </tbody>
      </table>
      <div class="data-cards">
        <template v-for="r in rows" :key="`card-${r.id}`">
          <button class="data-card" type="button" @click="toggle(r.id)">
            <span class="data-card-top">
              <span>{{ r.voucherDate }}</span>
              <span class="data-status" :data-status="r.status">{{ statusLabel[r.status] }}</span>
            </span>
            <span class="data-card-main">
              <span class="data-card-party">{{ r.counterparty || "—" }}</span>
              <span class="data-card-amount">{{ amountText(r.amountCents) }}</span>
            </span>
            <span class="data-card-desc">{{ r.description || "—" }}</span>
            <span class="data-card-flags">
              <span v-if="thumbUrl(r.id)" class="data-card-flag is-on">有凭证</span>
              <span v-else-if="metaOf(r.id).attachmentRequired" class="data-card-flag is-missing"
                >缺凭证</span
              >
              <span v-else class="data-card-flag">历史豁免</span>
              <span class="data-card-flag">{{ metaOf(r.id).booked ? "已入账" : "未入账" }}</span>
            </span>
          </button>
          <div v-if="expanded === r.id" class="data-card-expand">
            <VoucherEditor
              v-if="details[r.id]"
              :voucher="details[r.id].voucher"
              :detail="details[r.id]"
              @saved="onSaved"
              @detail="onDetail"
            />
            <p v-else class="wb-empty">{{ detailBusy ? "正在读取…" : "展开失败，请重试。" }}</p>
          </div>
        </template>
      </div>
      <div class="data-pager">
        <span>共 {{ total }} 条 · 每页 {{ pageSize }}</span>
        <span class="data-pager-actions">
          <button class="btn btn-sm" type="button" :disabled="page <= 1" @click="goto(page - 1)">
            上一页
          </button>
          <span class="data-pager-page">第 {{ page }} 页</span>
          <button
            class="btn btn-sm"
            type="button"
            :disabled="page * pageSize >= total"
            @click="goto(page + 1)"
          >
            下一页
          </button>
        </span>
      </div>
    </div>
    <NewRecordDialog :open="newOpen" @close="closeNew" @saved="onCreated" />
  </div>
</template>

<style scoped>
.data-actions {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

.data-batchbar {
  margin-bottom: var(--s-3);
}

.data-no {
  font-variant-numeric: tabular-nums;
  color: var(--text-2);
  white-space: nowrap;
}

.data-desc {
  max-width: 260px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.data-booked {
  color: var(--text-3);
  font-size: var(--fs-caption);
}

.data-booked.is-on {
  color: var(--success);
}

.data-thumb-img {
  width: 28px;
  height: var(--control-h-lg);
  object-fit: cover;
  border-radius: var(--radius-small);
  border: 1px solid var(--card-hairline);
}

.data-cards {
  display: none;
}

.data-card {
  display: grid;
  gap: var(--s-1);
  width: 100%;
  padding: var(--s-3);
  text-align: left;
  background: var(--bg-card);
  border: 1px solid var(--card-hairline);
  border-radius: var(--radius-control);
  color: var(--text);
}

.data-card-top,
.data-card-main,
.data-card-flags {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--s-2);
}

.data-card-top {
  color: var(--text-3);
  font-size: var(--fs-caption);
}

.data-card-party {
  font-weight: 600;
}

.data-card-amount {
  font-variant-numeric: tabular-nums;
}

.data-card-desc {
  color: var(--text-2);
  font-size: var(--fs-caption);
}

.data-card-flags {
  justify-content: flex-start;
  flex-wrap: wrap;
}

.data-card-flag {
  padding: 0 var(--s-2);
  border-radius: var(--radius-pill);
  background: var(--surface-3);
  color: var(--text-2);
  font-size: var(--fs-caption);
}

.data-card-flag.is-missing {
  color: var(--accent-strong);
  background: var(--accent-soft);
}

.data-card-flag.is-on {
  color: var(--success);
}

.data-card-expand {
  padding: var(--s-2) 0;
}

@media (max-width: 767px) {
  .data-panel .data-table {
    display: none;
  }

  .data-cards {
    display: grid;
    gap: var(--s-2);
    padding: var(--s-2);
  }
}

.data-pager-actions {
  display: flex;
  align-items: center;
  gap: var(--s-2);
}

.data-pager-page {
  color: var(--text-3);
  font-size: var(--fs-caption);
}
</style>
