<script setup lang="ts">
/**
 * 右栏「所属事项组」区块（v0.22 关联批；B 批重设计）。
 *
 * 「明细」用**组**承载：一个组＝一次付款/一笔事项，组内成员就是它的明细凭证。
 * 版面同 `RelatedVouchers`：标题行一个主按钮；新建/加入的控件各占**一行**且按钮与输入同行。
 * 纪律：**组内合计只是派生展示值**，不参与任何导出/统计口径（关联 ≠ 抵消）。
 */
import { ref, watch } from "vue";

import { api } from "../../api/client";
import type { Voucher, VoucherGroup } from "../../api/client";
import { fmtAmount } from "../../composables/format";
import { toastError } from "../../stores/toast";

const props = defineProps<{ voucherId: number | null }>();
const emit = defineEmits<{ focus: [id: number] }>();

const groups = ref<VoucherGroup[]>([]);
const all = ref<VoucherGroup[]>([]);
const creating = ref(false);
const title = ref("");
const pick = ref<number | null>(null);

async function load() {
  if (!props.voucherId) {
    groups.value = [];
    return;
  }
  try {
    groups.value = await api.voucherGroupsOf(props.voucherId);
    all.value = await api.listVoucherGroups();
  } catch (e) {
    toastError(e);
  }
}

watch(() => props.voucherId, load, { immediate: true });

async function create() {
  if (!props.voucherId || !title.value.trim()) return;
  try {
    const g = await api.createVoucherGroup(title.value.trim());
    await api.addVouchersToGroup(g.id, [props.voucherId]);
    title.value = "";
    creating.value = false;
    await load();
  } catch (e) {
    toastError(e);
  }
}

async function addTo() {
  if (!props.voucherId || pick.value === null) return;
  try {
    await api.addVouchersToGroup(pick.value, [props.voucherId]);
    await load();
  } catch (e) {
    toastError(e);
  }
}

async function remove(g: VoucherGroup, v: Voucher) {
  try {
    await api.removeVoucherFromGroup(g.id, v.id);
    await load();
  } catch (e) {
    toastError(e);
  }
}

/** 组内合计（派生值，仅展示） */
function groupTotal(g: VoucherGroup): number {
  return g.members.reduce((sum, m) => sum + (m.amountCents ?? 0), 0);
}
</script>

<template>
  <section class="fluent-panel">
    <header class="fluent-panel-header vg-head">
      <h3>
        所属事项组
        <span v-if="groups.length" class="vg-badge">{{ groups.length }} 个</span>
      </h3>
      <button class="btn btn-sm" :disabled="!voucherId" @click="creating = !creating">
        {{ creating ? "取消" : "＋ 新建组" }}
      </button>
    </header>

    <div class="fluent-panel-body vg-body">
      <!-- 新建：输入与按钮同一行 -->
      <div v-if="creating" class="vg-row">
        <input
          v-model="title"
          class="input"
          placeholder="组标题（如：2026-08 差旅）"
          aria-label="组标题"
          @keyup.enter="create"
        />
        <button class="btn" @click="create">建立并加入</button>
      </div>

      <ul v-if="groups.length" class="vg-list">
        <li v-for="g in groups" :key="g.id">
          <div class="vg-item-head">
            <strong class="vg-title">{{ g.title }}</strong>
            <span class="vg-sum">
              {{ g.members.length }} 张 · 组内合计 {{ fmtAmount(groupTotal(g)) }}（派生值）
            </span>
          </div>
          <ul class="vg-members">
            <li v-for="m in g.members" :key="m.id">
              <button class="link" @click="emit('focus', m.id)">{{ m.voucherNo }}</button>
              <span class="vg-member-desc">{{ m.description }}</span>
              <button
                class="vg-remove"
                title="移出该组"
                aria-label="移出该组"
                @click="remove(g, m)"
              >
                移出
              </button>
            </li>
          </ul>
        </li>
      </ul>
      <p v-else class="vg-empty">
        还没有归组。一次付款或一笔事项下的多张凭证，放进同一个组就是它的「明细」。
      </p>

      <!-- 加入已有组：下拉与按钮同一行 -->
      <div v-if="all.length" class="vg-row">
        <select v-model.number="pick" class="input" title="加入已有组" aria-label="加入已有组">
          <option :value="null">选择已有组…</option>
          <option v-for="g in all" :key="g.id" :value="g.id">{{ g.title }}</option>
        </select>
        <button class="btn" :disabled="pick === null" @click="addTo">加入</button>
      </div>
    </div>
  </section>
</template>

<style scoped>
.vg-head h3 {
  display: flex;
  align-items: baseline;
  gap: var(--s-2);
  margin: 0;
}
.vg-badge {
  font: var(--type-caption);
  color: var(--label-tertiary);
}
.vg-body {
  display: flex;
  flex-direction: column;
  gap: var(--s-2);
}
/* 输入与按钮同行 */
/* 输入与按钮**同一行同一高度**（行内按钮用 `.btn`（md = 32px）与 `.input` 对齐；
   此前用 `.btn-sm`（26px）导致"按钮和输入框大小不一致"，用户 2026-10-16 截图反馈）。 */
.vg-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: center;
  gap: var(--s-2);
}
.vg-row > .input,
.vg-row > .btn {
  height: var(--control-h-md);
}
.vg-list {
  list-style: none;
  margin: 0;
  padding: 0;
}
.vg-list > li + li {
  margin-top: var(--s-2);
  padding-top: var(--s-2);
  border-top: 1px solid var(--separator);
}
.vg-item-head {
  display: flex;
  align-items: baseline;
  gap: var(--s-2);
  flex-wrap: wrap;
}
.vg-title {
  font: var(--type-body);
}
.vg-sum {
  color: var(--label-tertiary);
  font: var(--type-caption);
}
.vg-members {
  list-style: none;
  margin: 4px 0 0;
  padding: 0 0 0 var(--s-3);
}
.vg-members li {
  display: flex;
  align-items: center;
  gap: var(--s-2);
  padding: 4px 0;
}
.vg-member-desc {
  flex: 1 1 0;
  min-width: 0;
  color: var(--label-tertiary);
  font: var(--type-caption);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.vg-remove {
  flex: 0 0 auto;
  background: none;
  border: 0;
  color: var(--label-tertiary);
  font: var(--type-caption);
  cursor: pointer;
  padding: 2px 4px;
  border-radius: var(--radius-small);
}
.vg-remove:hover {
  color: var(--danger);
  background: var(--bg-selection);
}
.vg-empty {
  margin: 0;
  color: var(--label-tertiary);
  font: var(--type-caption);
}
</style>
