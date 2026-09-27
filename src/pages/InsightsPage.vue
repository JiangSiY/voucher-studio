<script setup lang="ts">
/**
 * 洞察（资料库 → 洞察）—— v0.18：统计视图从首页移到资料库。
 *
 * 为什么单独成页：重构后首页（收件箱）只回答"现在该做什么"；"这个月花了多少、
 * 钱花在谁身上"属于**查数据**的场景，放在资料库这一组里、与凭证列表并列，
 * 才符合"流程在收件箱、数据在资料库"的分工。
 *
 * 口径说明（设计正本 §3）：本页各块**各自标注口径**；统一时间窗控制器在与面板
 * 的内部窗口合并后再上线（面板当前自带窗口：月度 6 个月 / 排行全历史前 10）。
 */
import { ref } from "vue";
import { useRouter } from "vue-router";
import TrendPanel from "../components/dashboard/TrendPanel.vue";
import TopListsPanel from "../components/dashboard/TopListsPanel.vue";
import ReconcilePanel from "../components/dashboard/ReconcilePanel.vue";
import FluentIcon from "../components/FluentIcon.vue";
import { api, type ReconcileReport } from "../api/client";
import { toast, toastError } from "../stores/toast";

const router = useRouter();
const reconcileReport = ref<ReconcileReport | null>(null);
const reconcileBusy = ref(false);

async function runReconcile() {
  if (reconcileBusy.value) return;
  reconcileBusy.value = true;
  try {
    const report = await api.pickAndReconcile();
    if (!report) return; // 用户取消
    reconcileReport.value = report;
    toast.info(
      `对账完成：✅ 匹配 ${report.matched.length} · 🔵 账单无凭证 ${report.billOnly.length} · 🔴 凭证无账单 ${report.voucherOnly.length}`,
    );
  } catch (e) {
    toastError(e);
  } finally {
    reconcileBusy.value = false;
  }
}
</script>

<template>
  <div class="insights">
    <header class="head">
      <h2>洞察</h2>
      <span class="scope">趋势与排行各块自带窗口（统一口径控制器随后端透视接口一并接入）</span>
      <span class="flex-1"></span>
      <button class="btn" :disabled="reconcileBusy" title="选账单 CSV 与库内凭证对账（只读，不改数据）" @click="runReconcile">
        <FluentIcon name="chart" :size="14" /> {{ reconcileBusy ? "对账中…" : "账单对账" }}
      </button>
      <button class="btn" @click="router.push('/vouchers')">
        <FluentIcon name="list" :size="14" /> 去看凭证
      </button>
    </header>

    <TrendPanel />
    <TopListsPanel />
    <ReconcilePanel v-if="reconcileReport" :report="reconcileReport" @close="reconcileReport = null" />
  </div>
</template>

<style scoped>
.insights { display: flex; flex-direction: column; gap: var(--s-4); }
.head { display: flex; align-items: center; gap: var(--s-3); flex-wrap: wrap; }
.head h2 { font: var(--type-title); color: var(--label-primary); margin: 0; font-size: calc(20px * var(--font-scale)); }
.scope { font: var(--type-caption); color: var(--label-tertiary); }
.flex-1 { flex: 1; }
</style>
