<script setup lang="ts">
/** 全文搜索：编号/摘要/备注/标签/附件 OCR 文本 */
import { ref } from "vue";
import { useRouter } from "vue-router";
import { api, type SearchHit } from "../api/client";
import EmptyState from "../components/EmptyState.vue";
import { STATUS_META, fmtAmount } from "../composables/format";
import { toastError } from "../stores/toast";

const router = useRouter();
const keyword = ref("");
const hits = ref<SearchHit[]>([]);
const searched = ref(false);
const busy = ref(false);

async function search() {
  const kw = keyword.value.trim();
  if (!kw) return;
  busy.value = true;
  try {
    hits.value = await api.searchVouchers(kw, 100);
    searched.value = true;
  } catch (e) {
    toastError(e);
  } finally {
    busy.value = false;
  }
}
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
    </div>
  </div>

  <div class="fluent-panel" v-if="searched">
    <div class="fluent-panel-header">
      <h3>搜索结果（{{ hits.length }}）</h3>
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
              @click="router.push(`/vouchers/${h.voucher.id}`)">
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
      <EmptyState v-else icon="🔍" title="没有找到相关凭证"
        desc="换个关键词试试；图片上的文字需要先完成 OCR 识别才可被搜到" />
    </div>
  </div>

  <div v-else class="empty-state">
    <div class="es-icon">🔍</div>
    <div class="es-title">全文搜索</div>
    <div class="es-desc">支持搜索凭证编号、摘要、备注、标签，以及附件图片上的文字（OCR 识别结果）</div>
  </div>
</template>
