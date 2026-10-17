<script setup lang="ts">
/**
 * 洞察（资料库 → 洞察）—— v0.18：统计视图从首页移到资料库。
 *
 * 为什么单独成页：重构后首页（收件箱）只回答"现在该做什么"；"这个月花了多少、
 * 钱花在谁身上"属于**查数据**的场景，放在资料库这一组里、与凭证列表并列，
 * 才符合"流程在收件箱、数据在资料库"的分工。
 *
 * 口径说明（设计正本 §3）：页首的时间窗控制器是**唯一真源**，驱动趋势 / 月度透视 / 导出覆盖度；
 * 存储占用为全库口径并已标注。TopListsPanel（排行）2026-10-16 起跟随时间窗（此前恒全历史前 10，
 * 尚未接入该控制器。
 */
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import TrendPanel from "../components/dashboard/TrendPanel.vue";
import TopListsPanel from "../components/dashboard/TopListsPanel.vue";
import ReconcilePanel from "../components/dashboard/ReconcilePanel.vue";
import FluentIcon from "../components/FluentIcon.vue";
import { api, type BreakdownRow, type ExportCoverage, type ReconcileReport, type StorageUsage } from "../api/client";
import { toast, toastError } from "../stores/toast";
import { fmtAmount, fmtSize } from "../composables/format";
import {
  RANGE_PRESETS,
  rangeLabel,
  rangeOfPreset,
  type DateRange,
  type RangePreset,
} from "../composables/dateRange";

const router = useRouter();

// ---------- 统一口径控制器（全页唯一真源；与资料库共用同一套时间窗预设） ----------

const today = new Date();
const preset = ref<RangePreset>("last6");
const range = ref<DateRange>(rangeOfPreset("last6", today) as DateRange);
/** 预设 → 趋势面板的滚动月数（1 / 3 / 6；"今年"与"自定义"用日期区间，面板仍按月展示 12 个月） */
const trendMonths = computed(() => {
  if (preset.value === "thisMonth") return 1;
  if (preset.value === "last3") return 3;
  return 6;
});
const scopeLabel = computed(() => rangeLabel(range.value, "全部状态"));

function applyPreset(p: RangePreset) {
  preset.value = p;
  const r = rangeOfPreset(p, today);
  if (r) {
    range.value = r;
    void loadWindowed();
  }
}

// ---------- 月度透视（月份 × 对方 / 收支方向）与台账 ----------

const dim = ref<"counterparty" | "tag">("counterparty");
const breakdown = ref<BreakdownRow[]>([]);

// 台账（导出覆盖度 / 存储占用）
const coverage = ref<ExportCoverage | null>(null);
const storage = ref<StorageUsage | null>(null);
const storageBytesBusy = ref(false);
/** 失败终态：面板不再用「加载中…」掩盖失败（见 loadWindowed / loadLedgers 的 catch） */
const windowedFailed = ref(false);
const storageFailed = ref(false);

async function loadWindowed() {
  try {
    [coverage.value, breakdown.value] = await Promise.all([
      api.exportCoverage(range.value.dateFrom, range.value.dateTo),
      api.monthlyBreakdown(range.value.dateFrom, range.value.dateTo, dim.value, 5),
    ]);
    windowedFailed.value = false;
  } catch (e) {
    // 失败时必须有个终态：此前只有 toast，面板会永远停在「加载中…」，用户以为一直在跑
    windowedFailed.value = true;
    toastError(e);
  }
}

function switchDim(d: "counterparty" | "tag") {
  dim.value = d;
  void loadWindowed();
}

/**
 * 透视行 → 资料库（**精确**筛选）。
 *
 * 参数写成**一个显式字面量**（而不是拼在变量上）有两个原因：
 * ① 读代码的人一眼看清"这一行到底带什么条件"；
 * ② 静态守卫 `drilldownContract.test.ts` 靠解析这个字面量来校验
 *    "推出去的每个键，资料库都读得懂"——拼在变量上会让守卫失明（上一版正是如此，
 *    于是 `counterparty`/`tags` 漏读没被发现）。
 */
function drillDown(row: BreakdownRow) {
  const isUnfilled = row.key === "(未填)";
  void router.push({
    path: "/vouchers",
    query: {
      // 月份窗口（闭区间；月末用 -31 不影响字符串比较口径）
      dateFrom: `${row.month}-01`,
      dateTo: `${row.month}-31`,
      // 精确维度：未填行走 counterpartyEmpty；否则按当前维度用 tags 或 counterparty
      ...(isUnfilled ? { counterpartyEmpty: "1" } : {}),
      ...(!isUnfilled && dim.value === "tag" ? { tags: row.key } : {}),
      ...(!isUnfilled && dim.value === "counterparty" ? { counterparty: row.key } : {}),
    },
  });
}

async function loadLedgers() {
  try {
    storage.value = await api.storageUsage(false);
    storageFailed.value = false;
  } catch (e) {
    storageFailed.value = true;
    toastError(e);
  }
}

/** 增强版体积需逐文件 stat（DB 里刻意不落该列）→ 用户主动点开才算 */
async function loadEnhancedBytes() {
  storageBytesBusy.value = true;
  try {
    storage.value = await api.storageUsage(true);
  } catch (e) {
    toastError(e);
  } finally {
    storageBytesBusy.value = false;
  }
}

onMounted(async () => {
  await Promise.all([loadWindowed(), loadLedgers()]);
});

// 对账（只读：选账单 CSV 与库内凭证比对，报告仅在会话内保留）
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
/** 透视结果按月分组（后端已按金额倒序，这里只做展示分组） */
function monthsOf(rows: BreakdownRow[]): string[] {
  return [...new Set(rows.map((r) => r.month))].sort().reverse();
}
function rowsOf(rows: BreakdownRow[], month: string): BreakdownRow[] {
  return rows.filter((r) => r.month === month);
}
</script>

<template>
  <div class="insights">
    <header class="head">
      <h2>洞察</h2>
      <div class="range-presets">
        <button
          v-for="p in RANGE_PRESETS"
          :key="p.key"
          class="seg"
          :class="{ active: preset === p.key }"
          @click="applyPreset(p.key)"
        >
          {{ p.label }}
        </button>
      </div>
      <span class="scope">{{ scopeLabel }}（趋势 / 透视 / 覆盖度共用此口径；存储占用为全库）</span>
      <span class="flex-1"></span>
      <button class="btn" :disabled="reconcileBusy" title="选账单 CSV 与库内凭证对账（只读，不改数据）" @click="runReconcile">
        <FluentIcon name="chart" :size="14" /> {{ reconcileBusy ? "对账中…" : "账单对账" }}
      </button>
      <button class="btn" @click="router.push('/vouchers')">
        <FluentIcon name="list" :size="14" /> 去看凭证
      </button>
    </header>

    <!-- 台账：导出覆盖度 + 存储占用（批次 5 的只读聚合；每块标注口径） -->
    <div class="ledgers">
      <section class="panel">
        <header class="sh">
          <h3>导出覆盖度</h3>
          <span class="scope">全库 · 含归档</span>
        </header>
        <template v-if="coverage">
          <div class="kv">
            <div><dt>全部凭证</dt><dd>{{ coverage.total }}</dd></div>
            <div><dt>已导出过</dt><dd>{{ coverage.exported }}</dd></div>
            <div><dt>从未导出</dt><dd :class="{ warn: coverage.neverExported > 0 }">{{ coverage.neverExported }}</dd></div>
          </div>
          <ul v-if="coverage.byFormat.length" class="formats">
            <li v-for="f in coverage.byFormat" :key="f.format">
              <span class="tag tag-info">{{ f.format.toUpperCase() }}</span>
              <span class="muted">{{ f.batches }} 批 · {{ f.vouchers }} 张</span>
            </li>
          </ul>
          <button v-if="coverage.neverExported > 0" class="btn" @click="router.push({ path: '/vouchers', query: { exported: 'never' } })">
            查看从未导出的 {{ coverage.neverExported }} 张
          </button>
        </template>
        <p v-else-if="windowedFailed" class="muted">
          导出覆盖度加载失败。<button class="link" @click="loadWindowed()">重试</button>
        </p>
        <p v-else class="muted">加载中…</p>
      </section>

      <section class="panel">
        <header class="sh">
          <h3>存储占用</h3>
          <span class="scope">原图体积来自数据库；增强版体积需按需计算</span>
        </header>
        <template v-if="storage">
          <div class="kv">
            <div><dt>原图</dt><dd>{{ storage.originalCount }} 张 · {{ fmtSize(storage.originalBytes) }}</dd></div>
            <div>
              <dt>增强版</dt>
              <dd>
                {{ storage.enhancedCount }} 张
                <template v-if="storage.enhancedBytes != null"> · {{ fmtSize(storage.enhancedBytes) }}</template>
                <button v-else class="link" :disabled="storageBytesBusy" @click="loadEnhancedBytes()">
                  {{ storageBytesBusy ? "计算中…" : "（计算体积）" }}
                </button>
              </dd>
            </div>
            <div><dt>增强来源</dt><dd>扫描件 {{ storage.enhancedScan }} · 截图 {{ storage.enhancedScreenshot }} · 未识别 {{ storage.enhancedUnknown }}</dd></div>
            <div><dt>OCR 缓存</dt><dd>{{ storage.ocrCacheCount }} 条</dd></div>
          </div>
        </template>
        <p v-else-if="storageFailed" class="muted">
          存储占用加载失败。<button class="link" @click="loadLedgers()">重试</button>
        </p>
        <p v-else class="muted">加载中…</p>
      </section>
    </div>

    <!-- 月度透视：月份 × 对方/收支方向，点一行**精确下钻**到资料库（批次 5 的精确维度） -->
    <section class="panel">
      <header class="sh">
        <h3>月度透视</h3>
        <div class="segmented" role="tablist">
          <button class="seg-item" role="tab" :aria-selected="dim === 'counterparty'"
            :class="{ active: dim === 'counterparty' }" @click="switchDim('counterparty')">按对方账户</button>
          <button class="seg-item" role="tab" :aria-selected="dim === 'tag'"
            :class="{ active: dim === 'tag' }" @click="switchDim('tag')">按收支方向</button>
        </div>
        <span class="scope">每月前 5 项 · 按金额倒序</span>
      </header>
      <div v-if="breakdown.length" class="breakdown">
        <div v-for="m in monthsOf(breakdown)" :key="m" class="bd-month">
          <div class="bd-title">{{ m }}</div>
          <button v-for="row in rowsOf(breakdown, m)" :key="row.key" class="bd-row" @click="drillDown(row)">
            <span class="bd-key">{{ row.key }}</span>
            <span class="bd-count">{{ row.count }} 张</span>
            <span class="flex-1"></span>
            <span class="bd-amount">{{ fmtAmount(row.amountCents) }}</span>
            <FluentIcon name="chevronRight" :size="12" />
          </button>
        </div>
      </div>
      <p v-else class="muted">该口径下暂无数据。</p>
    </section>

    <TrendPanel :months="trendMonths" />
    <TopListsPanel :date-from="range.dateFrom" :date-to="range.dateTo" />
    <ReconcilePanel v-if="reconcileReport" :report="reconcileReport" @close="reconcileReport = null" />
  </div>
</template>

<style scoped>
.insights { display: flex; flex-direction: column; gap: var(--s-4); }
.head { display: flex; align-items: center; gap: var(--s-3); flex-wrap: wrap; }
.head h2 { font: var(--type-title); color: var(--label-primary); margin: 0; font-size: calc(20px * var(--font-scale)); }
.scope { font: var(--type-caption); color: var(--label-tertiary); }
.flex-1 { flex: 1; }
.ledgers { display: grid; grid-template-columns: repeat(auto-fit, minmax(340px, 1fr)); gap: var(--s-3); }
.panel { padding: var(--s-3); border: 1px solid var(--separator); border-radius: var(--radius-panel); background: var(--bg-content); display: flex; flex-direction: column; gap: var(--s-2); }
.sh { display: flex; align-items: baseline; gap: var(--s-2); }
.sh h3 { font: var(--type-title); color: var(--label-primary); margin: 0; }
.kv { display: flex; flex-direction: column; gap: 6px; }
.kv > div { display: grid; grid-template-columns: 84px 1fr; gap: var(--s-2); align-items: baseline; }
.kv dt { color: var(--label-tertiary); font: var(--type-caption); }
.kv dd { margin: 0; font: var(--type-body); color: var(--label-primary); font-variant-numeric: tabular-nums; }
.kv dd.warn { color: var(--warning); font-weight: 600; }
.formats { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
.formats li { display: flex; align-items: center; gap: var(--s-2); }
.muted { color: var(--label-tertiary); font: var(--type-caption); }
.link { border: none; background: none; color: var(--accent); font: var(--type-caption); cursor: default; }
.link:hover { text-decoration: underline; }
.range-presets { display: inline-flex; flex: 0 0 auto; border: 1px solid var(--separator); border-radius: var(--radius-control); overflow: hidden; }
.seg { height: var(--control-h-sm); padding: 0 var(--s-3); border: none; background: transparent; color: var(--label-secondary); font: var(--type-caption); flex: 0 0 auto; white-space: nowrap; cursor: default; }
.seg + .seg { border-left: 1px solid var(--separator); }
.seg.active { background: var(--bg-selection); color: var(--label-primary); font-weight: 600; }
.breakdown { display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: var(--s-3); }
.bd-month { display: flex; flex-direction: column; gap: 2px; }
.bd-title { font: var(--type-caption); font-weight: 600; color: var(--label-secondary); margin-bottom: 2px; }
.bd-row { display: flex; align-items: baseline; gap: var(--s-2); width: 100%; padding: 6px var(--s-2); border: none; border-radius: var(--radius-small); background: transparent; text-align: left; cursor: default; }
.bd-row:hover { background: var(--bg-selection); }
.bd-key { font: var(--type-body); color: var(--label-primary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 14em; }
.bd-count { font: var(--type-caption); color: var(--label-tertiary); white-space: nowrap; }
.bd-amount { font: var(--type-body); font-variant-numeric: tabular-nums; color: var(--label-primary); }
</style>
