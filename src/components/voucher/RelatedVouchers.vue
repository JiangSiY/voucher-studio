<script setup lang="ts">
/**
 * 右栏「相关凭证」区块（v0.22 关联批）。
 *
 * 语义（方案 v2）：只做**不带金额**的语义关系——`explains`（附件说明）与 `detail`（明细）；
 * 建立/解除都直接落库（两端凭证 + 审计留痕），**不改变任何金额合计口径**（关联 ≠ 抵消）。
 * 关系表按 `a_id < b_id` 归一化落库，因此两个方向看到的都是同一条关系。
 */
import { ref, watch } from "vue";

import { api } from "../../api/client";
import type { LinkedVoucher, Voucher, VoucherLinkKind } from "../../api/client";
import { fmtAmount } from "../../composables/format";
import { toastError } from "../../stores/toast";

const props = defineProps<{ voucherId: number | null }>();
const emit = defineEmits<{ focus: [id: number] }>();

const KIND_LABEL: Record<VoucherLinkKind, string> = {
  explains: "附件说明",
  detail: "明细",
};

const links = ref<LinkedVoucher[]>([]);
const kind = ref<VoucherLinkKind>("explains");
const note = ref("");
const keyword = ref("");
const candidates = ref<Voucher[]>([]);
const picking = ref(false);

async function load() {
  if (!props.voucherId) {
    links.value = [];
    return;
  }
  try {
    links.value = await api.listVoucherLinks(props.voucherId);
  } catch (e) {
    toastError(e);
  }
}

watch(() => props.voucherId, load, { immediate: true });

async function search() {
  const kw = keyword.value.trim();
  if (!kw) {
    candidates.value = [];
    return;
  }
  try {
    const res = await api.listVouchers({ keyword: kw, page: 1, pageSize: 8 });
    candidates.value = res.items.filter((v) => v.id !== props.voucherId);
  } catch (e) {
    toastError(e);
  }
}

async function link(v: Voucher) {
  if (!props.voucherId) return;
  try {
    await api.linkVouchers(props.voucherId, v.id, kind.value, note.value.trim() || undefined);
    note.value = "";
    keyword.value = "";
    candidates.value = [];
    picking.value = false;
    await load();
  } catch (e) {
    toastError(e);
  }
}

async function unlink(l: LinkedVoucher) {
  if (!props.voucherId) return;
  try {
    await api.unlinkVouchers(props.voucherId, l.other.id, l.kind);
    await load();
  } catch (e) {
    toastError(e);
  }
}
</script>

<template>
  <section class="related">
    <header class="rel-head">
      <h4>相关凭证</h4>
      <button class="btn btn-sm" :disabled="!voucherId" @click="picking = !picking">
        {{ picking ? "取消" : "＋ 建立关联" }}
      </button>
    </header>

    <div v-if="picking" class="rel-pick">
      <div class="rel-row">
        <select v-model="kind" class="input" title="关系类型">
          <option value="explains">附件说明</option>
          <option value="detail">明细</option>
        </select>
        <input
          v-model="keyword"
          class="input"
          placeholder="搜索凭证（编号 / 摘要 / 对方）"
          @keyup.enter="search"
        />
        <button class="btn btn-sm" @click="search">搜索</button>
      </div>
      <input v-model="note" class="input" placeholder="说明（可选，例如：这笔付款对应的发票）" />
      <ul v-if="candidates.length" class="rel-cand">
        <li v-for="c in candidates" :key="c.id">
          <button class="link" @click="link(c)">
            {{ c.voucherNo }} · {{ c.voucherDate }} · {{ fmtAmount(c.amountCents) }}
          </button>
          <span class="muted small">{{ c.description }}</span>
        </li>
      </ul>
    </div>

    <ul v-if="links.length" class="rel-list">
      <li v-for="l in links" :key="l.linkId">
        <span class="rel-kind">{{ KIND_LABEL[l.kind] }}</span>
        <button class="link" @click="emit('focus', l.other.id)">
          {{ l.other.voucherNo }} · {{ l.other.voucherDate }} · {{ fmtAmount(l.other.amountCents) }}
        </button>
        <span v-if="l.note" class="muted small rel-note">{{ l.note }}</span>
        <button class="btn btn-sm" @click="unlink(l)">解除</button>
      </li>
    </ul>
    <p v-else class="muted small">
      还没有关联。付款截图看不懂用途时，可以关联一张说明凭证，或把它归进一个事项组。
    </p>
  </section>
</template>
