<script setup lang="ts">
/**
 * 洞察 → 趋势面板：月度趋势（**滚动窗口由外部口径控制器决定** / 年度切换 + 季度卡 + 年报导出）
 * 与状态分布（横向堆叠条 + 图例）双面板。自加载数据并监听 import:done 刷新。
 *
 * v0.18：`months` 由调用方（洞察页的时间窗控制器）传入——此前写死"近 6 月"，
 * 与同屏其它块的窗口口径不一致（旧仪表盘的核心病灶之一，见规范 §3）。
 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { safeListen } from "../../composables/safeListen";
import { useRouter } from "vue-router";
import { api, type MonthlySummary } from "../../api/client";
import FluentIcon from "../FluentIcon.vue";
import { STATUS_META, fmtAmount } from "../../composables/format";
import { toastError } from "../../stores/toast";

const props = withDefaults(
  defineProps<{
    /** 滚动窗口的月数（由外部口径控制器给出；1–24，后端同口径） */
    months?: number;
  }>(),
  { months: 6 },
);

const router = useRouter();

// ---------- v0.11：趋势视图切换（近 6 月滚动 | 自然年度 12 月） ----------

type TrendMode = "recent" | "yearly";
const trendMode = ref<TrendMode>("recent");
const trendYear = ref(new Date().getFullYear());
/** 年份选项：今年起倒推 5 年（更早年份通过导出历史查批次） */
const yearOptions = computed(() =>
  Array.from({ length: 5 }, (_, i) => new Date().getFullYear() - i),
);

const monthly = ref<MonthlySummary[]>([]);

/** 当前视图的窗口月数（滚动模式取外部口径，年度模式恒 12） */
const windowMonths = computed(() => (trendMode.value === "recent" ? props.months : 12));

// 外部窗口变化 → 重新拉取（仅在"滚动窗口"模式下生效；年度模式自成年份维度）
watch(
  () => props.months,
  () => {
    if (trendMode.value === "recent") void loadTrend();
  },
);

/** 拉取当前视图的月度数据（滚动窗口或某年 12 个月） */
async function loadTrend() {
  if (trendMode.value === "recent") {
    monthly.value = await api.monthlySummary(props.months);
  } else {
    monthly.value = await api.yearlySummary(trendYear.value);
  }
}

async function switchTrendMode(m: TrendMode) {
  if (trendMode.value === m) return;
  trendMode.value = m;
  try {
    await loadTrend();
  } catch (e) {
    toastError(e);
  }
}

async function switchTrendYear(y: number) {
  if (trendYear.value === y) return;
  trendYear.value = y;
  try {
    await loadTrend();
  } catch (e) {
    toastError(e);
  }
}

/** 季度汇总（年度视图卡片）：Q1–Q4 的张数与金额 */
const quarters = computed(() => {
  const out: { label: string; count: number; amountCents: number }[] = [];
  for (let q = 0; q < 4; q++) {
    const rows = monthly.value.slice(q * 3, q * 3 + 3);
    out.push({
      label: `Q${q + 1}`,
      count: rows.reduce((s, m) => s + m.count, 0),
      amountCents: rows.reduce((s, m) => s + m.amountCents, 0),
    });
  }
  return out;
});


// ---------- v0.9 动线：图表下钻（看数 → 看明细，不断链） ----------

/** 月度趋势柱 → 列表页按当月日期范围筛选 */
function gotoMonth(month: string) {
  const [y, m] = month.split("-");
  const lastDay = new Date(Number(y), Number(m), 0).getDate();
  router.push({
    path: "/vouchers",
    query: { dateFrom: `${y}-${m}-01`, dateTo: `${y}-${m}-${lastDay}` },
  });
}

/** v0.11 季度卡 → 列表页按该季度日期范围筛选 */
function gotoQuarter(q: number) {
  const y = trendYear.value;
  const fromM = (q - 1) * 3 + 1;
  const toM = q * 3;
  const lastDay = new Date(y, toM, 0).getDate();
  router.push({
    path: "/vouchers",
    query: {
      dateFrom: `${y}-${String(fromM).padStart(2, "0")}-01`,
      dateTo: `${y}-${String(toM).padStart(2, "0")}-${lastDay}`,
    },
  });
}

/** 状态分布段 → 列表页按状态筛选 */
function gotoStatus(k: string) {
  router.push(`/vouchers?status=${k}`);
}

// ---------- 月度趋势柱状图（手写 SVG，无第三方图表库；张数/金额双指标） ----------

const CHART_W = 560;
const CHART_H = 190;
const PAD_L = 10;
const PAD_T = 22;
const PAD_B = 26;
const PLOT_H = CHART_H - PAD_T - PAD_B;

type TrendMetric = "count" | "amount";
const trendMetric = ref<TrendMetric>("count");

function metricValue(m: MonthlySummary): number {
  return trendMetric.value === "count" ? m.count : m.amountCents;
}

const maxMetric = computed(() =>
  Math.max(1, ...monthly.value.map(metricValue)),
);

const slot = (CHART_W - PAD_L) / Math.max(1, monthly.value.length || windowMonths.value);
const barW = Math.min(44, slot * 0.55);

function barX(i: number) {
  return PAD_L + slot * i + (slot - barW) / 2;
}

function barH(m: MonthlySummary) {
  const v = metricValue(m);
  return v === 0 ? 3 : Math.max(4, (v / maxMetric.value) * PLOT_H);
}

function barY(m: MonthlySummary) {
  return PAD_T + PLOT_H - barH(m);
}

function barMid(i: number) {
  return barX(i) + barW / 2;
}

function monthLabel(m: string) {
  return `${parseInt(m.slice(5), 10)}月`;
}

/** 柱顶数值标签：张数为整数，金额为 ¥ 元（紧凑显示） */
function barValLabel(m: MonthlySummary) {
  if (trendMetric.value === "count") return String(m.count);
  return `¥${fmtAmount(m.amountCents)}`;
}

function barTitle(m: MonthlySummary) {
  return `${m.month} · ${m.count} 张 · ¥${fmtAmount(m.amountCents)}`;
}

// ---------- 状态分布（近 N 月合计，横向堆叠条 + 图例） ----------

const DIST_KEYS = ["draft", "reviewed", "exported", "archived"] as const;

const dist = computed(() => {
  const agg = { draft: 0, reviewed: 0, exported: 0, archived: 0 };
  for (const m of monthly.value) {
    agg.draft += m.draft;
    agg.reviewed += m.reviewed;
    agg.exported += m.exported;
    agg.archived += m.archived;
  }
  const total = agg.draft + agg.reviewed + agg.exported + agg.archived;
  const amount = monthly.value.reduce((s, m) => s + m.amountCents, 0);
  return { ...agg, total, amount };
});

function pct(n: number) {
  if (dist.value.total === 0) return "0%";
  return `${((n / dist.value.total) * 100).toFixed(1)}%`;
}

// 拖放导入在任意页面全局生效，新数据实时反映到趋势
let unlistenImportDone: UnlistenFn | null = null;

onMounted(async () => {
  try {
    await loadTrend();
  } catch (e) {
    toastError(e);
  }
  unlistenImportDone = await safeListen("import:done", () => loadTrend().catch(toastError));
});

onUnmounted(() => {
  unlistenImportDone?.();
});
</script>

<template>
  <div class="report-grid" v-if="monthly.length">
    <div class="fluent-panel">
      <div class="fluent-panel-header">
        <h3>{{ trendMode === "recent" ? `月度趋势（近 ${windowMonths} 月）` : `年度趋势（${trendYear} 年）` }}</h3>
        <div style="display: flex; gap: 8px; align-items: center">
          <!-- v0.11：滚动窗口 / 年度切换 + 年份选择 -->
          <select v-if="trendMode === 'yearly'" v-model.number="trendYear" class="input" style="width: 96px" title="切换年份" @change="switchTrendYear(trendYear)">
            <option v-for="y in yearOptions" :key="y" :value="y">{{ y }} 年</option>
          </select>
          <div class="metric-toggle" role="tablist" aria-label="趋势视图切换">
            <button role="tab" :class="{ active: trendMode === 'recent' }" @click="switchTrendMode('recent')">近 {{ windowMonths }} 月</button>
            <button role="tab" :class="{ active: trendMode === 'yearly' }" @click="switchTrendMode('yearly')">年度</button>
          </div>
          <!-- 功能深化 3：张数/金额双指标切换 -->
          <div class="metric-toggle" role="tablist" aria-label="趋势指标切换">
            <button
              role="tab"
              :class="{ active: trendMetric === 'count' }"
              @click="trendMetric = 'count'"
            >张数</button>
            <button
              role="tab"
              :class="{ active: trendMetric === 'amount' }"
              @click="trendMetric = 'amount'"
            >金额</button>
          </div>
        </div>
      </div>
      <div class="fluent-panel-body">
        <!-- v0.11：季度汇总（年度视图）——点击季度跳列表页该季度范围 -->
        <div v-if="trendMode === 'yearly'" class="quarter-grid">
          <button v-for="(q, qi) in quarters" :key="q.label" class="quarter-card" :disabled="!q.count"
            :title="q.count ? `查看 ${trendYear} 年 ${q.label} 明细` : '该季度无凭证'"
            @click="gotoQuarter(qi + 1)">
            <span class="q-label">{{ q.label }}</span>
            <span class="q-count">{{ q.count }} 张</span>
            <span class="q-amount">¥{{ fmtAmount(q.amountCents) }}</span>
          </button>
        </div>
        <svg
          class="trend-svg"
          :viewBox="`0 0 ${CHART_W} ${CHART_H}`"
          role="img"
          :aria-label="trendMetric === 'count' ? '月度凭证张数趋势' : '月度凭证金额趋势'"
          preserveAspectRatio="xMidYMid meet"
        >
          <line class="axis" :x1="PAD_L" :y1="PAD_T + PLOT_H" :x2="CHART_W" :y2="PAD_T + PLOT_H" />
          <g v-for="(m, i) in monthly" :key="m.month" style="cursor: pointer" @click="gotoMonth(m.month)">
            <rect
              class="bar"
              :class="{ zero: metricValue(m) === 0 }"
              :x="barX(i)"
              :y="barY(m)"
              :width="barW"
              :height="barH(m)"
              rx="3"
            >
              <title>{{ barTitle(m) }}（点击查看当月明细）</title>
            </rect>
            <text v-if="metricValue(m) > 0" class="bar-val" :x="barMid(i)" :y="barY(m) - 5">
              {{ barValLabel(m) }}
            </text>
            <text class="bar-label" :x="barMid(i)" :y="CHART_H - 8">
              {{ monthLabel(m.month) }}
            </text>
          </g>
        </svg>
        <!-- v0.11：年度报表导出（统计输出，不改凭证状态） -->
        <div v-if="trendMode === 'yearly'" style="display: flex; justify-content: flex-end; margin-top: 4px">
        </div>
      </div>
    </div>

    <div class="fluent-panel">
      <div class="fluent-panel-header">
        <h3>{{ trendMode === "recent" ? `状态分布（近 ${windowMonths} 月）` : `状态分布（${trendYear} 年）` }}</h3>
        <span class="dist-sum" v-if="dist.total">
          合计 {{ dist.total }} 张 · ¥{{ fmtAmount(dist.amount) }}
        </span>
      </div>
      <div class="fluent-panel-body">
        <template v-if="dist.total">
          <div class="dist-bar" role="img" aria-label="状态分布（点击段查看对应凭证）">
            <div
              v-for="k in DIST_KEYS"
              :key="k"
              class="seg seg-click"
              :class="`seg-${k}`"
              :style="{ width: pct(dist[k]) }"
              :title="`${STATUS_META[k].label} ${dist[k]} 张（${pct(dist[k])}）· 点击查看`"
              @click="gotoStatus(k)"
            />
          </div>
          <div class="dist-legend">
            <div v-for="k in DIST_KEYS" :key="k" class="legend-item legend-click"
              style="cursor: pointer" :title="`点击查看「${STATUS_META[k].label}」凭证`" @click="gotoStatus(k)">
              <span class="dot" :class="`dot-${k}`" />
              <span>{{ STATUS_META[k].label }}</span>
              <span class="legend-num">{{ dist[k] }}</span>
            </div>
          </div>
        </template>
        <div v-else class="empty-state" style="padding: 32px">
          <div class="es-icon"><FluentIcon name="chart" :size="42" /></div>
          <div class="es-title">{{ trendMode === "recent" ? `近 ${windowMonths} 月暂无凭证` : `${trendYear} 年暂无凭证` }}</div>
          <div class="es-desc">导入图片后这里会出现统计图表</div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* .report-grid / .quarter-grid 已收敛到 src/styles/base.css《窗口缩放适配》一节：
   本组件是 <style scoped>，其规则特异性高于 base.css，会让全局断点失效，故移出。 */

.quarter-card {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 2px;
  padding: 8px 4px;
  border: 1px solid var(--stroke);
  border-radius: 6px;
  background: transparent;
  cursor: pointer;
  transition: border-color 0.12s ease, background 0.12s ease;
}

.quarter-card:hover:not(:disabled) {
  border-color: var(--accent);
  background: var(--accent-soft);
}

.quarter-card:disabled {
  opacity: 0.45;
  cursor: default;
}

.q-label {
  font-size: var(--fs-caption);
  font-weight: 600;
  color: var(--text-2);
}

.q-count {
  font-size: var(--fs-caption);
  color: var(--text);
  font-variant-numeric: tabular-nums;
}

.q-amount {
  font-size: var(--fs-caption);
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
}

.trend-svg {
  width: 100%;
  height: auto;
  display: block;
}

.axis {
  stroke: var(--stroke);
  stroke-width: 1;
}

.bar {
  fill: var(--accent);
  transition: opacity 0.15s ease;
  cursor: pointer;
}

.bar:hover {
  opacity: 0.8;
}

.bar.zero {
  fill: var(--text-disabled);
  opacity: 0.35;
}

.bar-val {
  fill: var(--text);
  font-size: var(--fs-caption);
  text-anchor: middle;
}

.bar-label {
  fill: var(--text-disabled);
  font-size: var(--fs-caption);
  text-anchor: middle;
}

.dist-sum {
  font-size: var(--fs-caption);
  color: var(--text-disabled);
}

/* 功能深化 3：趋势指标切换（张数/金额） */
.metric-toggle {
  display: flex;
  gap: 0;
  border: 1px solid var(--stroke);
  border-radius: 4px;
  overflow: hidden;
}

.metric-toggle button {
  padding: 3px 12px;
  font-size: var(--fs-caption);
  border: none;
  background: transparent;
  color: var(--text-2);
  cursor: pointer;
  transition: background 0.12s ease, color 0.12s ease;
}

.metric-toggle button + button {
  border-left: 1px solid var(--stroke);
}

.metric-toggle button.active {
  background: var(--accent-soft);
  color: var(--accent);
  font-weight: 600;
}

.dist-bar {
  display: flex;
  height: 14px;
  border-radius: var(--radius-control);
  overflow: hidden;
  background: var(--stroke);
}

.seg {
  min-width: 0;
  transition: opacity 0.15s ease;
}

.seg:hover {
  opacity: 0.8;
}

.seg-click {
  cursor: pointer;
}

.seg-draft {
  background: var(--warning);
}

.seg-reviewed {
  background: var(--accent);
}

.seg-exported {
  background: var(--success);
}

.seg-archived {
  background: var(--text-disabled);
}

.dist-legend {
  display: flex;
  flex-wrap: wrap;
  gap: 12px 24px;
  margin-top: 16px;
}

.legend-item {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-caption);
  color: var(--text);
}

.legend-click:hover {
  color: var(--accent);
}

.legend-num {
  font-variant-numeric: tabular-nums;
  color: var(--text-3);
}

.dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  flex: none;
}

.dot-draft {
  background: var(--warning);
}

.dot-reviewed {
  background: var(--accent);
}

.dot-exported {
  background: var(--success);
}

.dot-archived {
  background: var(--text-disabled);
}
</style>
