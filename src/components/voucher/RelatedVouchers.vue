<script setup lang="ts">
/**
 * 右栏「相关凭证」区块（v0.22 关联批；B 批重设计）。
 *
 * 版面规范（`docs/方案-编辑页重设计.md`）：与「识别结果 / 修改历史」同款 `fluent-panel`；
 * 标题行右侧**只放一个主按钮**（建立关联 / 取消）；展开后**所有控件在同一行**
 * （类型 / 搜索 / 搜索按钮 / 说明），候选列表在下一行，点候选即建立关联。
 *
 * 语义：只做**不带金额**的语义关系——`explains`（附件说明）与 `detail`（明细）；
 * 建立/解除都直接落库（两端凭证 + 审计留痕），**不改变任何金额合计口径**（关联 ≠ 抵消）。
 */
import { ref, watch } from "vue";

import { api } from "../../api/client";
import type { LinkedVoucher, Voucher, VoucherLinkKind } from "../../api/client";
import { fmtAmount } from "../../composables/format";
import { toastError } from "../../stores/toast";
import FluentIcon from "../FluentIcon.vue";

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
/** 已经点过「搜索」：用于区分"还没搜"与"搜了没有结果" */
const searched = ref(false);

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
  searched.value = false;
  if (!kw) {
    candidates.value = [];
    return;
  }
  try {
    const res = await api.listVouchers({ keyword: kw, page: 1, pageSize: 8 });
    candidates.value = res.items.filter((v) => v.id !== props.voucherId);
    searched.value = true;
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
    searched.value = false;
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
  <section class="fluent-panel">
    <header class="fluent-panel-header rel-head">
      <h3>
        相关凭证
        <span v-if="links.length" class="rel-badge">{{ links.length }} 条</span>
      </h3>
      <button
        class="btn btn-sm"
        :disabled="!voucherId"
        @click="
          picking = !picking;
          searched = false;
        "
      >
        {{ picking ? "取消" : "＋ 建立关联" }}
      </button>
    </header>

    <div class="fluent-panel-body rel-body">
      <!-- 第一行：类型 / 关键词 / 搜索按钮；第二行：说明（独占整行） -->
      <div v-if="picking" class="rel-pick">
        <div class="rel-pick-row">
          <select v-model="kind" class="input rel-kind" title="关系类型" aria-label="关系类型">
            <option value="explains">附件说明</option>
            <option value="detail">明细</option>
          </select>
          <input
            v-model="keyword"
            class="input rel-kw"
            placeholder="搜索凭证…"
            aria-label="搜索凭证"
            @keyup.enter="search"
          />
          <button class="btn" @click="search">搜索</button>
        </div>
        <input
          v-model="note"
          class="input rel-note"
          placeholder="说明（可选，例如：这笔付款对应的发票）"
          aria-label="关系说明"
        />
      </div>

      <ul v-if="candidates.length" class="rel-cand">
        <li v-for="c in candidates" :key="c.id">
          <button class="link" @click="link(c)">
            {{ c.voucherNo }} · {{ c.voucherDate }} · {{ fmtAmount(c.amountCents) }}
          </button>
          <span class="rel-cand-desc">{{ c.description }}</span>
        </li>
      </ul>

      <p v-if="searched && !candidates.length" class="rel-empty">
        没有找到匹配的凭证 —— 换个编号、摘要或对方关键词试试。
      </p>

      <ul v-if="links.length" class="rel-list">
        <li v-for="l in links" :key="l.linkId">
          <span class="rel-kind-tag">{{ KIND_LABEL[l.kind] }}</span>
          <button class="link rel-other" @click="emit('focus', l.other.id)">
            {{ l.other.voucherNo }} · {{ l.other.voucherDate }} · {{ fmtAmount(l.other.amountCents) }}
          </button>
          <span v-if="l.note" class="rel-note-text">{{ l.note }}</span>
          <button class="rel-unlink" title="解除关联" aria-label="解除关联" @click="unlink(l)">
            <FluentIcon name="close" :size="13" />
          </button>
        </li>
      </ul>
      <p v-else class="rel-empty">
        还没有关联。付款截图看不懂用途时，可以关联一张说明凭证，或把它归进一个事项组。
      </p>
    </div>
  </section>
</template>

<style scoped>
/* 标题右侧只放一个主按钮；标题内的条数徽标折叠时也看得见 */
.rel-head h3 {
  display: flex;
  align-items: baseline;
  gap: var(--s-2);
  margin: 0;
}
.rel-badge {
  font: var(--type-caption);
  color: var(--label-tertiary);
}
.rel-body {
  display: flex;
  flex-direction: column;
  gap: var(--s-2);
}
/* 建立关联的控件：**两行 flex**（2026-10-17 第二次修）。
   不要用 grid —— 跨列的「说明」会按 max-content 反向把 `auto` 列撑宽，
   于是类型下拉变超宽、关键词输入框被压成 0 宽（用户两次截图：一次看不见输入框、一次像两个「搜索」按钮）。
   flex 里只有关键词是 `flex:1`，类型与按钮 `flex:none`，说明单独一行，结构上不会再塌。 */
.rel-pick {
  display: flex;
  flex-direction: column;
  gap: var(--s-2);
}
.rel-pick-row {
  display: flex;
  align-items: center;
  gap: var(--s-2);
}
.rel-pick-row > .input,
.rel-pick-row > .btn {
  height: var(--control-h-md);
}
.rel-kind {
  flex: none;
  width: auto;
  max-width: 45%;
}
.rel-kw {
  flex: 1 1 auto;
  min-width: 0;
}
.rel-pick-row > .btn {
  flex: none;
}
/* 候选：一行一条，右侧给摘要 */
.rel-cand {
  list-style: none;
  margin: 0;
  padding: 0;
  border: 1px solid var(--separator);
  border-radius: var(--radius-control);
  max-height: 168px;
  overflow: auto;
}
.rel-cand li {
  display: flex;
  align-items: baseline;
  gap: var(--s-2);
  padding: 4px var(--s-2);
}
.rel-cand li + li {
  border-top: 1px solid var(--separator);
}
.rel-cand-desc {
  color: var(--label-tertiary);
  font: var(--type-caption);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
/* 已有关系：一行三段 + 行尾解除 */
.rel-list {
  list-style: none;
  margin: 0;
  padding: 0;
}
.rel-list li {
  display: flex;
  align-items: center;
  gap: var(--s-2);
  padding: 5px 0;
}
.rel-list li + li {
  border-top: 1px solid var(--separator);
}
.rel-kind-tag {
  flex: 0 0 auto;
  font: var(--type-caption);
  color: var(--label-secondary);
  background: var(--bg-selection);
  border-radius: 4px;
  padding: 1px 6px;
}
.rel-other {
  flex: 0 0 auto;
}
.rel-note-text {
  flex: 1 1 0;
  min-width: 0;
  color: var(--label-tertiary);
  font: var(--type-caption);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.rel-unlink {
  flex: 0 0 auto;
  background: none;
  border: 0;
  color: var(--label-tertiary);
  cursor: pointer;
  padding: 2px 4px;
  border-radius: 4px;
}
.rel-unlink:hover {
  color: var(--danger);
  background: var(--bg-selection);
}
.rel-empty {
  margin: 0;
  color: var(--label-tertiary);
  font: var(--type-caption);
}
</style>
