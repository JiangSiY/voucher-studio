<script setup lang="ts">
/**
 * 会计投影页（v0.26）：把台账里的收付款变成金蝶能直接抄的分录。
 *
 * 为什么是「投影」而不是「记账」：
 * 这一页**不产生新的会计事实**，只是把已经存在的凭证按科目映射投影成借贷两列。
 * 所以它允许反复预演、反复覆盖草稿——投影算错不会污染业务数据，重算一遍就好。
 *
 * 四步对应四个决定：
 * 1. 预演：先看分录对不对（借贷是否平、有没有缺映射），不写库；
 * 2. 写入草稿：落 `gl_entries`（status='draft'），可重算覆盖；
 * 3. 导出：KIS 引入 CSV / 抄单纸 xlsx / V5 凭证分录 JSON，三者同源；
 * 4. 科目映射：分类找不到科目就整张不出——这里把缺口补上。
 */
import { computed, onMounted, ref } from "vue";
import {
  api,
  type CategoryMap,
  type AccountMap,
  type EntryVoucher,
  type GlEntry,
  type PeriodPlan,
  type PostResult,
} from "../api/client";
import EmptyState from "../components/EmptyState.vue";
import FluentIcon from "../components/FluentIcon.vue";
import { fmtAmount } from "../composables/format";
import { toast, toastError } from "../stores/toast";

type Tab = "plan" | "entries" | "maps";
const tab = ref<Tab>("plan");

function monthOf(d: Date) {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
}
const period = ref(monthOf(new Date()));

const plan = ref<PeriodPlan | null>(null);
const posting = ref(false);
const lastPost = ref<PostResult | null>(null);
const entries = ref<GlEntry[]>([]);
const categoryMaps = ref<CategoryMap[]>([]);
const accountMaps = ref<AccountMap[]>([]);
const busy = ref(false);
const exporting = ref(false);

const vouchers = computed<EntryVoucher[]>(() => {
  const p = plan.value;
  if (!p) return [];
  return [...p.receipts, ...p.payments, ...p.accruals];
});
// 后端不序列化合计（只给逐张凭证），合计在前端加——少一处两边各算一遍的口径漂移
const totals = computed(() => {
  const list = vouchers.value;
  return {
    debit: list.reduce((n, v) => n + v.debitCents, 0),
    credit: list.reduce((n, v) => n + v.creditCents, 0),
    vouchers: list.length,
  };
});
const balanced = computed(() => totals.value.debit === totals.value.credit);
const issues = computed(() => plan.value?.issues ?? []);

async function preview() {
  if (busy.value) return;
  busy.value = true;
  try {
    plan.value = await api.glBuildPeriod(period.value);
    lastPost.value = null;
    if (!plan.value.issues.length) {
      toast.success(
        `已预演：${totals.value.vouchers} 张凭证，借贷各 ${fmtAmount(totals.value.debit)}`,
      );
    }
  } catch (e) {
    toastError(e);
  } finally {
    busy.value = false;
  }
}

async function post() {
  if (posting.value) return;
  posting.value = true;
  try {
    lastPost.value = await api.glPostPeriod(period.value);
    toast.success(
      `已写入草稿：${lastPost.value.vouchers ?? 0} 张，${lastPost.value.lines ?? 0} 行分录`,
    );
    await loadEntries();
  } catch (e) {
    toastError(e);
  } finally {
    posting.value = false;
  }
}

async function loadEntries() {
  try {
    entries.value = await api.glListEntries(period.value);
  } catch (e) {
    toastError(e);
  }
}

async function loadMaps() {
  try {
    [categoryMaps.value, accountMaps.value] = await Promise.all([
      api.glListCategoryMaps(),
      api.glListAccountMaps(),
    ]);
  } catch (e) {
    toastError(e);
  }
}

async function exportOne(kind: "kis" | "copy" | "v5") {
  if (exporting.value) return;
  exporting.value = true;
  try {
    const path =
      kind === "kis"
        ? await api.glExportKis(period.value)
        : kind === "copy"
          ? await api.glExportCopySheet(period.value)
          : await api.glExportV5Entries(period.value);
    const label = kind === "kis" ? "KIS 引入 CSV" : kind === "copy" ? "抄单纸" : "V5 凭证分录";
    toast.success(`${label} 已生成`, {
      label: "打开",
      onClick: () => void api.openPath(path),
    });
  } catch (e) {
    toastError(e);
  } finally {
    exporting.value = false;
  }
}

async function saveCategoryMap(m: CategoryMap) {
  try {
    await api.glUpsertCategoryMap(m);
    toast.success(`已保存映射：${m.category} → ${m.accountCode}`);
    await loadMaps();
  } catch (e) {
    toastError(e);
  }
}

async function saveAccountMap(m: AccountMap) {
  try {
    await api.glUpsertAccountMap(m);
    toast.success(`已保存账户映射：${m.accountKey} → ${m.accountCode}`);
    await loadMaps();
  } catch (e) {
    toastError(e);
  }
}

function switchTab(next: Tab) {
  tab.value = next;
  if (next === "entries") void loadEntries();
  if (next === "maps" && !categoryMaps.value.length) void loadMaps();
}

onMounted(() => {
  void preview();
});
</script>

<template>
  <div class="page">
    <!-- 标题由工作台头部提供（route meta），这里只放本期选择与主操作，避免同一标题出现两次 -->
    <header class="page-head">
      <p class="page-sub">本期收付款按科目映射折成借贷分录，导出后金蝶可直接抄</p>
      <div class="head-ops">
        <input v-model="period" type="month" class="input" :disabled="busy" />
        <button class="btn" :disabled="busy" @click="preview()">预演</button>
        <button class="btn btn-primary" :disabled="busy || !plan" @click="post()">
          {{ posting ? "写入中…" : "写入草稿" }}
        </button>
      </div>
    </header>

    <div class="tabs">
      <button class="tab" :class="{ active: tab === 'plan' }" @click="switchTab('plan')">
        预演
      </button>
      <button class="tab" :class="{ active: tab === 'entries' }" @click="switchTab('entries')">
        已落草稿
      </button>
      <button class="tab" :class="{ active: tab === 'maps' }" @click="switchTab('maps')">
        科目映射
      </button>
    </div>

    <!-- 预演 -->
    <template v-if="tab === 'plan'">
      <div v-if="issues.length" class="callout warning">
        <span class="ci"><FluentIcon name="warning" :size="18" /></span>
        <div class="ct">
          <strong>{{ issues.length }} 个问题挡着入账</strong>
          <ul class="issue-list">
            <li v-for="(it, i) in issues" :key="i">{{ it }}</li>
          </ul>
          缺映射的分类不会出分录——去「科目映射」补上再预演。
        </div>
      </div>

      <div v-if="plan" class="summary">
        <div class="stat">
          <span class="k">凭证张数</span><span class="v">{{ totals.vouchers }}</span>
        </div>
        <div class="stat">
          <span class="k">借方合计</span><span class="v">{{ fmtAmount(totals.debit) }}</span>
        </div>
        <div class="stat">
          <span class="k">贷方合计</span><span class="v">{{ fmtAmount(totals.credit) }}</span>
        </div>
        <div class="stat" :class="balanced ? 'ok' : 'bad'">
          <span class="k">借贷</span>
          <span class="v">{{
            balanced ? "平" : `差 ${fmtAmount(totals.debit - totals.credit)}`
          }}</span>
        </div>
      </div>

      <div class="ops">
        <button class="btn" :disabled="exporting || !plan" @click="exportOne('kis')">
          导出 KIS 引入 CSV
        </button>
        <button class="btn" :disabled="exporting || !plan" @click="exportOne('copy')">
          导出抄单纸
        </button>
        <button class="btn" :disabled="exporting || !plan" @click="exportOne('v5')">
          导出 V5 凭证分录
        </button>
      </div>

      <EmptyState
        v-if="!vouchers.length"
        title="这个月还没有可投影的凭证"
        hint="先把台账清单导入并补上附件"
      >
        <template #icon><FluentIcon name="chart" :size="24" /></template>
      </EmptyState>

      <section v-for="(v, vi) in vouchers" :key="vi" class="voucher-card">
        <header class="vc-head">
          <strong>{{ v.summary }}</strong>
          <span class="vc-meta">{{ v.entryDate }} · 借贷 {{ fmtAmount(v.debitCents) }}</span>
        </header>
        <table class="lines">
          <thead>
            <tr>
              <th>摘要</th>
              <th>科目</th>
              <th class="num">借方</th>
              <th class="num">贷方</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(l, i) in v.lines" :key="i">
              <td>{{ l.summary }}</td>
              <td>{{ l.accountCode }} {{ l.accountName }}</td>
              <td class="num">{{ l.debitCents ? fmtAmount(l.debitCents) : "" }}</td>
              <td class="num">{{ l.creditCents ? fmtAmount(l.creditCents) : "" }}</td>
            </tr>
          </tbody>
        </table>
      </section>
    </template>

    <!-- 已落草稿 -->
    <template v-else-if="tab === 'entries'">
      <EmptyState
        v-if="!entries.length"
        title="还没有落库的分录"
        hint="先在「预演」里确认无误，再点「写入草稿」"
      >
        <template #icon><FluentIcon name="list" :size="24" /></template>
      </EmptyState>
      <table v-else class="lines wide">
        <thead>
          <tr>
            <th>凭证字号</th>
            <th>日期</th>
            <th>摘要</th>
            <th>科目</th>
            <th class="num">借方</th>
            <th class="num">贷方</th>
            <th>项目</th>
            <th>状态</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="e in entries" :key="e.id">
            <td>{{ e.voucherNo }}</td>
            <td>{{ e.entryDate }}</td>
            <td>{{ e.summary }}</td>
            <td>{{ e.accountCode }} {{ e.accountName }}</td>
            <td class="num">{{ e.debitCents ? fmtAmount(e.debitCents) : "" }}</td>
            <td class="num">{{ e.creditCents ? fmtAmount(e.creditCents) : "" }}</td>
            <td>{{ e.projectKey }}</td>
            <td>{{ e.status }}</td>
          </tr>
        </tbody>
      </table>
    </template>

    <!-- 科目映射 -->
    <template v-else>
      <h2 class="section-title">分类 → 科目</h2>
      <table class="lines wide">
        <thead>
          <tr>
            <th>分类</th>
            <th>科目代码</th>
            <th>科目名称</th>
            <th>方向</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="m in categoryMaps" :key="m.category">
            <td>{{ m.category }}</td>
            <td><input v-model="m.accountCode" class="input sm" /></td>
            <td><input v-model="m.accountName" class="input sm" /></td>
            <td>
              <select v-model="m.direction" class="input sm">
                <option value="借">借</option>
                <option value="贷">贷</option>
              </select>
            </td>
            <td><button class="btn sm" @click="saveCategoryMap(m)">保存</button></td>
          </tr>
        </tbody>
      </table>

      <h2 class="section-title">结算账户 → 科目</h2>
      <table class="lines wide">
        <thead>
          <tr>
            <th>账户</th>
            <th>科目代码</th>
            <th>科目名称</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="m in accountMaps" :key="m.accountKey">
            <td>{{ m.accountKey }}</td>
            <td><input v-model="m.accountCode" class="input sm" /></td>
            <td><input v-model="m.accountName" class="input sm" /></td>
            <td><button class="btn sm" @click="saveAccountMap(m)">保存</button></td>
          </tr>
        </tbody>
      </table>
    </template>
  </div>
</template>

<style scoped>
.page-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
}
.page-sub {
  color: var(--text-2);
  font-size: var(--fs-caption);
  margin-top: 4px;
}
.head-ops {
  display: flex;
  gap: 8px;
  align-items: center;
}
.tabs {
  display: flex;
  gap: 4px;
  margin: 16px 0 12px;
  border-bottom: 1px solid var(--separator);
}
.tab {
  border: none;
  background: none;
  padding: 8px 12px;
  cursor: pointer;
  color: var(--text-2);
  border-bottom: 2px solid transparent;
}
.tab.active {
  color: var(--text);
  border-bottom-color: var(--accent);
  font-weight: 600;
}
.summary {
  display: flex;
  gap: 12px;
  flex-wrap: wrap;
  margin-bottom: 12px;
}
.stat {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 10px 16px;
  border-radius: var(--radius-card);
  background: var(--bg-card);
}
.stat .k {
  font-size: var(--fs-caption);
  color: var(--text-2);
}
.stat .v {
  font-size: var(--fs-title);
  font-weight: 600;
}
.stat.ok .v {
  color: var(--success);
}
.stat.bad .v {
  color: var(--danger);
}
.ops {
  display: flex;
  gap: 8px;
  margin: 12px 0 16px;
}
.issue-list {
  margin: 6px 0 6px 20px;
}
.voucher-card {
  border: 1px solid var(--separator);
  border-radius: var(--radius-card);
  margin-bottom: 16px;
  overflow: hidden;
  background: var(--bg-card);
}
.vc-head {
  display: flex;
  justify-content: space-between;
  padding: 10px 16px;
  border-bottom: 1px solid var(--separator);
}
.vc-meta {
  color: var(--text-2);
  font-size: var(--fs-caption);
}
.lines {
  width: 100%;
  border-collapse: collapse;
  font-size: var(--fs-caption);
}
.lines th,
.lines td {
  padding: 6px 16px;
  text-align: left;
  border-bottom: 1px solid var(--separator);
}
.lines th {
  color: var(--text-2);
  font-weight: 500;
}
.lines .num {
  text-align: right;
  font-variant-numeric: tabular-nums;
}
.lines.wide {
  margin-bottom: 20px;
  background: var(--bg-card);
}
.input.sm {
  height: 28px;
  padding: 0 8px;
}
.btn.sm {
  height: 28px;
  padding: 0 10px;
}
.section-title {
  font-size: var(--fs-title);
  margin: 16px 0 8px;
}

/* 窄屏（<768）：头部改上下两段、操作按钮换行、统计卡两列、页签横向滚动
   ——否则副标题被挤成一行一个字（2026-11-18 真机走查踩到） */
@media (max-width: 767px) {
  .page-head {
    flex-direction: column;
    align-items: stretch;
    gap: 8px;
  }
  .head-ops {
    flex-wrap: wrap;
  }
  .head-ops .btn-primary {
    flex: 1;
  }
  .tabs {
    overflow-x: auto;
    scrollbar-width: none;
  }
  .tabs::-webkit-scrollbar {
    display: none;
  }
  .summary {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
  .ops {
    flex-wrap: wrap;
  }
}
</style>
