<script setup lang="ts">
/** 全文搜索：编号/摘要/备注/标签/附件 OCR 文本 + v0.11 组合筛选（日期/金额范围） */
import { onMounted, onUnmounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api, type SearchFilters, type SearchHit } from "../api/client";
import EmptyState from "../components/EmptyState.vue";
import FluentIcon from "../components/FluentIcon.vue";
import { STATUS_META, fmtAmount } from "../composables/format";
import { toast, toastError } from "../stores/toast";

const route = useRoute();
const router = useRouter();
const keyword = ref("");
const hits = ref<SearchHit[]>([]);
const searched = ref(false);
const busy = ref(false);

// v0.11 组合筛选（金额单位：元，发送前转分）
const filterDateFrom = ref("");
const filterDateTo = ref("");
const filterAmountFrom = ref("");
const filterAmountTo = ref("");

/** 元字符串 → 分；非法输入返回 false（提示后中止） */
function yuanToCents(s: string, out: { cents?: number }): boolean {
  const t = s.trim();
  if (!t) return true; // 未填：不过滤
  if (!/^-?\d+(\.\d{1,2})?$/.test(t)) {
    toast.warning("金额范围格式不正确：应为数字，最多两位小数（如 100 或 123.45）");
    return false;
  }
  out.cents = Math.round(parseFloat(t) * 100);
  return true;
}

/** 当前筛选条件（金额元 → 分；空值不携带）。返回 null = 输入非法，中止搜索 */
function buildFilters(): SearchFilters | null | undefined {
  const from: { cents?: number } = {};
  const to: { cents?: number } = {};
  if (!yuanToCents(filterAmountFrom.value, from) || !yuanToCents(filterAmountTo.value, to)) {
    return null;
  }
  const filters: SearchFilters = {};
  if (filterDateFrom.value) filters.dateFrom = filterDateFrom.value;
  if (filterDateTo.value) filters.dateTo = filterDateTo.value;
  if (from.cents !== undefined) filters.amountFrom = from.cents;
  if (to.cents !== undefined) filters.amountTo = to.cents;
  return Object.keys(filters).length ? filters : undefined;
}

/** 有筛选条件（用于「清除筛选」按钮显隐与提示） */
const hasFilters = () =>
  Boolean(filterDateFrom.value || filterDateTo.value || filterAmountFrom.value || filterAmountTo.value);

function clearFilters() {
  filterDateFrom.value = "";
  filterDateTo.value = "";
  filterAmountFrom.value = "";
  filterAmountTo.value = "";
  if (searched.value && keyword.value.trim()) search();
}

async function search() {
  const kw = keyword.value.trim();
  if (!kw) {
    toast.info("请输入关键词（可搭配日期 / 金额范围筛选）");
    return;
  }
  const filters = buildFilters();
  if (filters === null) return; // 金额输入非法
  busy.value = true;
  try {
    hits.value = await api.searchVouchers(kw, filters, 100);
    searched.value = true;
  } catch (e) {
    toastError(e);
  } finally {
    busy.value = false;
  }
}

/** 结果行 → 编辑页（带搜索上下文：编辑页返回回搜索页、翻张按搜索结果顺序） */
function openHit(id: number) {
  const q: Record<string, string> = { keyword: keyword.value.trim(), from: "search" };
  router.push({ path: `/vouchers/${id}`, query: q });
}

// 已出结果后发生拖放入库，自动重跑当前关键词，避免结果陈旧
let unlistenImportDone: UnlistenFn | null = null;

onMounted(async () => {
  // 动线：从编辑页返回搜索页时带 keyword（恢复关键词并自动重搜，接续核对）
  const q = route.query.keyword;
  if (typeof q === "string" && q.trim()) {
    keyword.value = q.trim();
    search();
  }
  unlistenImportDone = await listen("import:done", () => {
    if (searched.value && keyword.value.trim()) search();
  });
});

onUnmounted(() => {
  unlistenImportDone?.();
});
</script>

<template>
  <div class="fluent-panel">
    <div class="fluent-panel-body" style="padding: 16px">
      <div style="display: flex; gap: 12px">
        <div class="search">
          <input v-model="keyword" placeholder="输入关键词：编号、摘要、备注、标签或图片上的文字（OCR）…"
            @keyup.enter="search" autofocus />
        </div>
        <button class="btn btn-primary" :disabled="busy" @click="search">{{ busy ? "搜索中…" : "搜索" }}</button>
      </div>
      <!-- v0.11 组合筛选：日期范围 + 金额范围（与关键词 AND 组合） -->
      <div style="display: flex; flex-wrap: wrap; gap: 8px; align-items: center; margin-top: 12px">
        <input type="date" v-model="filterDateFrom" class="input" style="width: 130px" title="起始日期" @keyup.enter="search" />
        <span style="color: var(--text-3); font-size: 12px">至</span>
        <input type="date" v-model="filterDateTo" class="input" style="width: 130px" title="结束日期" @keyup.enter="search" />
        <span style="color: var(--text-3); font-size: 12px; margin-left: 4px">金额</span>
        <input v-model="filterAmountFrom" class="input" style="width: 110px" inputmode="decimal"
          placeholder="≥ 元" title="金额下限（元）" @keyup.enter="search" />
        <span style="color: var(--text-3); font-size: 12px">—</span>
        <input v-model="filterAmountTo" class="input" style="width: 110px" inputmode="decimal"
          placeholder="≤ 元" title="金额上限（元）" @keyup.enter="search" />
        <button v-if="hasFilters()" class="btn btn-subtle" style="padding: 4px 10px; font-size: 12px" @click="clearFilters">清除筛选</button>
      </div>
      <div v-if="hasFilters()" style="font-size: 12px; color: var(--text-3); margin-top: 6px">
        筛选条件与关键词同时生效（AND）；金额按凭证金额精确匹配，空金额凭证在设定金额范围时不出现
      </div>
    </div>
  </div>

  <div class="fluent-panel" v-if="searched">
    <div class="fluent-panel-header">
      <h3>搜索结果（{{ hits.length }}）</h3>
      <span v-if="keyword.trim()" style="font-size: 12px; color: var(--text-3)">关键词：{{ keyword.trim() }} · 点击进入编辑，返回仍在搜索页</span>
    </div>
    <div class="fluent-panel-body" style="padding: 0">
      <div class="dtable-wrap" v-if="hits.length">
        <table class="dtable">
          <thead>
            <tr>
              <th>凭证编号</th>
              <th>日期</th>
              <th>摘要</th>
              <th style="text-align: right">金额</th>
              <th>状态</th>
              <th>命中位置</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="h in hits" :key="h.voucher.id" style="cursor: pointer"
              @click="openHit(h.voucher.id)">
              <td><span class="mono">{{ h.voucher.voucherNo }}</span></td>
              <td>{{ h.voucher.voucherDate }}</td>
              <td>{{ h.voucher.description || "—" }}</td>
              <td class="num">{{ fmtAmount(h.voucher.amountCents) }}</td>
              <td><span class="tag" :class="STATUS_META[h.voucher.status].tag">{{ STATUS_META[h.voucher.status].label }}</span></td>
              <td>
                <span v-for="m in h.matchedIn" :key="m" class="tag tag-info" style="margin-right: 4px">{{ m }}</span>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <EmptyState v-else icon="search" title="没有找到相关凭证"
        desc="换个关键词试试；图片上的文字需要先完成 OCR 识别才可被搜到" />
    </div>
  </div>

  <div v-else class="empty-state">
    <div class="es-icon"><FluentIcon name="search" :size="42" /></div>
    <div class="es-title">全文搜索</div>
    <div class="es-desc">支持搜索凭证编号、摘要、备注、标签，以及附件图片上的文字（OCR 识别结果）</div>
  </div>
</template>
