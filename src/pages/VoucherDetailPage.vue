<script setup lang="ts">
/**
 * 凭证详情（v0.26 P1）——手机端的"整页编辑器"。
 *
 * 为什么需要它：`/vouchers` 是"左列表 + 右编辑器"的双栏形态，在窄屏上只能上下堆叠；
 * 点一行后编辑器落在列表下方，等于"翻过整屏去找表单"。手机改用整页路由：
 * 列表点一行 → 这里；返回 → 回到列表（带筛选，浏览上下文不丢）。
 *
 * 宽屏不重复维护两套交互：宽屏访问该路由会 `replace` 回 `/vouchers?focus=<id>`，
 * 编辑器仍只有 `VoucherEditor` 一处实现（v0.19 的既定点，不改）。
 */
import { computed, onMounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { api, type Voucher, type VoucherDetail } from "../api/client";
import VoucherEditor from "../components/voucher/VoucherEditor.vue";
import { useBreakpoint } from "../composables/useBreakpoint";
import { STATUS_META } from "../composables/format";
import { toastError } from "../stores/toast";

const route = useRoute();
const router = useRouter();
const { isWide } = useBreakpoint();

const voucherId = computed(() => Number(route.params.id ?? 0));
const voucher = ref<Voucher | null>(null);
const detail = ref<VoucherDetail | null>(null);
const dirty = ref(false);
const loading = ref(true);

const statusMeta = computed(() => (voucher.value ? STATUS_META[voucher.value.status] : null));

async function load() {
  if (!Number.isFinite(voucherId.value) || voucherId.value <= 0) {
    router.replace("/vouchers");
    return;
  }
  loading.value = true;
  try {
    const d = await api.getVoucher(voucherId.value);
    detail.value = d;
    voucher.value = d.voucher;
  } catch (e) {
    toastError(e);
    router.replace("/vouchers");
  } finally {
    loading.value = false;
  }
}

/** 宽屏回到双栏（`focus` 深链是既有通道），避免同一张凭证有两套界面 */
function ensureWide() {
  if (isWide.value && voucherId.value > 0) {
    router.replace({ path: "/vouchers", query: { focus: String(voucherId.value) } });
  }
}

onMounted(async () => {
  ensureWide();
  await load();
});

watch(isWide, ensureWide);
watch(voucherId, load);

function onSaved(v: Voucher) {
  voucher.value = v;
}

function onDetail(d: VoucherDetail) {
  detail.value = d;
  voucher.value = d.voucher;
}

/** 手机端没有"下一张"的列表上下文，这里退回列表（保留筛选由浏览器历史承担） */
function onNext() {
  router.back();
}
</script>

<template>
  <div class="detail-page">
    <header class="detail-head">
      <div class="head-main">
        <div class="head-title mono">{{ voucher?.voucherNo ?? "凭证详情" }}</div>
        <div class="head-sub">
          <span v-if="voucher">{{ voucher.voucherDate }}</span>
          <span v-if="statusMeta" class="tag" :class="statusMeta.tag">{{ statusMeta.label }}</span>
          <span v-if="dirty" class="dirty">● 未保存</span>
        </div>
      </div>
    </header>

    <p v-if="loading" class="muted pad">正在读取…</p>
    <template v-else-if="voucher">
      <VoucherEditor
        :voucher="voucher"
        :detail="detail"
        dense
        @saved="onSaved"
        @detail="onDetail"
        @next="onNext"
        @dirty="dirty = $event"
      />
    </template>
    <p v-else class="muted pad">这张凭证不存在或已被删除。</p>
  </div>
</template>

<style scoped>
.detail-page {
  /* 手机固定底栏（56 保存条 + 56 tab bar）会盖住内容尾部 */
  padding-bottom: calc(var(--tabbar-h) * 2);
  display: flex;
  flex-direction: column;
  gap: var(--s-3);
  min-width: 0;
}
.detail-head {
  display: flex;
  align-items: center;
  gap: var(--s-2);
  padding: var(--s-2) var(--s-1);
}
.head-main {
  min-width: 0;
}
.head-title {
  font: var(--type-title);
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.head-sub {
  display: flex;
  align-items: center;
  gap: var(--s-2);
  font: var(--type-caption);
  color: var(--label-secondary);
}
.dirty {
  color: var(--warning);
}
.muted {
  color: var(--label-secondary);
  font: var(--type-body);
}
.pad {
  padding: var(--s-4);
}
</style>
