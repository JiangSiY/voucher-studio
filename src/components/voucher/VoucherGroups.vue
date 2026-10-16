<script setup lang="ts">
/**
 * 右栏「所属事项组」区块（v0.22 关联批）。
 *
 * 「明细」用**组**承载：一个组＝一次付款/一笔事项，组内成员就是它的明细凭证。
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
  <section class="vgroups">
    <header class="rel-head">
      <h4>所属事项组</h4>
      <button class="btn btn-sm" :disabled="!voucherId" @click="creating = !creating">
        {{ creating ? "取消" : "＋ 新建组" }}
      </button>
    </header>

    <div v-if="creating" class="rel-row">
      <input
        v-model="title"
        class="input"
        placeholder="组标题（如：2026-08 差旅）"
        @keyup.enter="create"
      />
      <button class="btn btn-sm" @click="create">建立并加入</button>
    </div>

    <ul v-if="groups.length" class="rel-list">
      <li v-for="g in groups" :key="g.id">
        <strong>{{ g.title }}</strong>
        <span class="muted small">
          {{ g.members.length }} 张 · 组内合计 {{ fmtAmount(groupTotal(g)) }}（派生值，不改合计口径）
        </span>
        <ul class="rel-members">
          <li v-for="m in g.members" :key="m.id">
            <button class="link" @click="emit('focus', m.id)">{{ m.voucherNo }}</button>
            <button class="btn btn-sm" @click="remove(g, m)">移出</button>
          </li>
        </ul>
      </li>
    </ul>
    <p v-else class="muted small">
      还没有归组。一次付款或一笔事项下的多张凭证，放进同一个组就是它的「明细」。
    </p>

    <div v-if="all.length" class="rel-row">
      <select v-model.number="pick" class="input" title="加入已有组">
        <option :value="null">选择已有组…</option>
        <option v-for="g in all" :key="g.id" :value="g.id">{{ g.title }}</option>
      </select>
      <button class="btn btn-sm" :disabled="pick === null" @click="addTo">加入</button>
    </div>
  </section>
</template>
