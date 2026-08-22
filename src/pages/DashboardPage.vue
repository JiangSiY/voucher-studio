<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useRouter } from "vue-router";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api, type BackupInfo, type CounterpartyRow, type MonthlySummary, type TagRow, type Voucher, type VoucherStats } from "../api/client";
import StatCard from "../components/StatCard.vue";
import FluentIcon from "../components/FluentIcon.vue";
import { STATUS_META, fmtAmount, fmtDate } from "../composables/format";
import { useConfigStore } from "../stores/config";
import { toast, toastError } from "../stores/toast";

const router = useRouter();
const configStore = useConfigStore();

const stats = ref<VoucherStats | null>(null);
const recent = ref<Voucher[]>([]);
const backups = ref<BackupInfo[]>([]);
const monthly = ref<MonthlySummary[]>([]);
const busy = ref(false);

const REPORT_MONTHS = 6;

// ---------- v0.7 R-1：Top 对方账户排行（按金额降序） ----------

const CP_TOP = 10;
const counterparties = ref<CounterpartyRow[]>([]);

const cpMaxAmount = computed(() =>
  Math.max(1, ...counterparties.value.map((c) => c.amountCents)),
);

/** 条形宽度：按最大金额归一化（最小 2% 保证可见） */
function cpBarWidth(c: CounterpartyRow): string {
  return `${Math.max(2, (c.amountCents / cpMaxAmount.value) * 100)}%`;
}

// ---------- 功能深化 3：标签分类统计（按金额降序，全部历史） ----------

const TAG_TOP = 10;
const tagRows = ref<TagRow[]>([]);

const tagMaxAmount = computed(() =>
  Math.max(1, ...tagRows.value.map((t) => t.amountCents)),
);

function tagBarWidth(t: TagRow): string {
  return `${Math.max(2, (t.amountCents / tagMaxAmount.value) * 100)}%`;
}

/** 标签显示名：空串 = 未分类 */
function tagLabel(t: TagRow): string {
  return t.tag || "未分类";
}

async function load() {
  try {
    stats.value = await api.voucherStats();
    const list = await api.listVouchers({ page: 1, pageSize: 6 });
    recent.value = list.items;
    backups.value = await api.listBackups();
    monthly.value = await api.monthlySummary(REPORT_MONTHS);
    counterparties.value = await api.topCounterparties(0, CP_TOP);
    tagRows.value = await api.tagStats(0, TAG_TOP);
  } catch (e) {
    toastError(e);
  }
}

async function backupNow() {
  busy.value = true;
  try {
    const info = await api.runBackup();
    toast.success(`备份完成：${info.fileName}`);
    backups.value = await api.listBackups();
  } catch (e) {
    toastError(e);
  } finally {
    busy.value = false;
  }
}

function statusOf(v: Voucher) {
  return STATUS_META[v.status];
}

// ---------- 月度趋势柱状图（手写 SVG，无第三方图表库；功能深化 3：张数/金额双指标） ----------

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

const slot = (CHART_W - PAD_L) / Math.max(1, monthly.value.length || REPORT_MONTHS);
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

// 拖放/微信导入在任意页面全局生效，概览停留期间入库也需实时刷新
let unlistenImportDone: UnlistenFn | null = null;
let unlistenWxImported: UnlistenFn | null = null;
// 自动备份在后台完成 → 刷新备份卡片与最近备份时间
let unlistenBackupDone: UnlistenFn | null = null;

onMounted(async () => {
  load();
  unlistenImportDone = await listen("import:done", () => load());
  unlistenWxImported = await listen("wechat:imported", () => load());
  unlistenBackupDone = await listen("backup:done", () => load());
});

onUnmounted(() => {
  unlistenImportDone?.();
  unlistenWxImported?.();
  unlistenBackupDone?.();
});
</script>

<template>
  <div class="stat-grid" v-if="stats">
    <!-- v0.7 R-2：状态卡片点击联动——跳转凭证列表并按状态预筛选 -->
    <div class="stat-click" title="查看全部凭证" @click="router.push('/vouchers')">
      <StatCard label="凭证总数" :value="stats.total" :delta="`本月新增 ${stats.thisMonth}`" />
    </div>
    <div class="stat-click" title="查看草稿凭证" @click="router.push('/vouchers?status=draft')">
      <StatCard label="草稿" :value="stats.draft" delta="待整理" />
    </div>
    <div class="stat-click" title="查看已复核凭证" @click="router.push('/vouchers?status=reviewed')">
      <StatCard label="已复核" :value="stats.reviewed" delta="可归档打印" />
    </div>
    <div class="stat-click" title="查看已导出凭证" @click="router.push('/vouchers?status=exported')">
      <StatCard label="已导出" :value="stats.exported" delta="已生成 PDF" />
    </div>
    <StatCard label="附件图片" :value="stats.attachments" delta="含 OCR 识别" />
    <StatCard label="本地备份" :value="backups.length" :delta="backups[0] ? `最近 ${fmtDate(backups[0].createdAt)}` : '尚无备份'" />
  </div>

  <div class="callout info">
    <span class="ci"><FluentIcon name="bulb" :size="18" /></span>
    <div class="ct">
      <strong>快捷入口：</strong>把凭证图片直接拖进「凭证列表」页面即可自动建卡入库；
      数据全部保存在本机（{{ configStore.appInfo?.dataDir }}），
      <span v-if="backups.length === 0" style="color: var(--warning)">建议先做一次备份。</span>
      <span v-else>最近备份 {{ fmtDate(backups[0].createdAt) }}。</span>
    </div>
  </div>

  <div class="report-grid" v-if="monthly.length">
    <div class="fluent-panel">
      <div class="fluent-panel-header">
        <h3>月度趋势（近 {{ REPORT_MONTHS }} 月）</h3>
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
      <div class="fluent-panel-body">
        <svg
          class="trend-svg"
          :viewBox="`0 0 ${CHART_W} ${CHART_H}`"
          role="img"
          :aria-label="trendMetric === 'count' ? '月度凭证张数趋势' : '月度凭证金额趋势'"
          preserveAspectRatio="xMidYMid meet"
        >
          <line class="axis" :x1="PAD_L" :y1="PAD_T + PLOT_H" :x2="CHART_W" :y2="PAD_T + PLOT_H" />
          <g v-for="(m, i) in monthly" :key="m.month">
            <rect
              class="bar"
              :class="{ zero: metricValue(m) === 0 }"
              :x="barX(i)"
              :y="barY(m)"
              :width="barW"
              :height="barH(m)"
              rx="3"
            >
              <title>{{ barTitle(m) }}</title>
            </rect>
            <text v-if="metricValue(m) > 0" class="bar-val" :x="barMid(i)" :y="barY(m) - 5">
              {{ barValLabel(m) }}
            </text>
            <text class="bar-label" :x="barMid(i)" :y="CHART_H - 8">
              {{ monthLabel(m.month) }}
            </text>
          </g>
        </svg>
      </div>
    </div>

    <div class="fluent-panel">
      <div class="fluent-panel-header">
        <h3>状态分布（近 {{ REPORT_MONTHS }} 月）</h3>
        <span class="dist-sum" v-if="dist.total">
          合计 {{ dist.total }} 张 · ¥{{ fmtAmount(dist.amount) }}
        </span>
      </div>
      <div class="fluent-panel-body">
        <template v-if="dist.total">
          <div class="dist-bar" role="img" aria-label="状态分布">
            <div
              v-for="k in DIST_KEYS"
              :key="k"
              class="seg"
              :class="`seg-${k}`"
              :style="{ width: pct(dist[k]) }"
              :title="`${STATUS_META[k].label} ${dist[k]} 张（${pct(dist[k])}）`"
            />
          </div>
          <div class="dist-legend">
            <div v-for="k in DIST_KEYS" :key="k" class="legend-item">
              <span class="dot" :class="`dot-${k}`" />
              <span>{{ STATUS_META[k].label }}</span>
              <span class="legend-num">{{ dist[k] }}</span>
            </div>
          </div>
        </template>
        <div v-else class="empty-state" style="padding: 32px">
          <div class="es-icon"><FluentIcon name="chart" :size="42" /></div>
          <div class="es-title">近 {{ REPORT_MONTHS }} 月暂无凭证</div>
          <div class="es-desc">导入图片后这里会出现统计图表</div>
        </div>
      </div>
    </div>
  </div>

  <!-- v0.7 R-1 + 功能深化 3：对方账户排行 / 标签分类统计（双栏，按金额降序，全部历史） -->
  <div class="report-grid" v-if="counterparties.length || tagRows.length">
    <div class="fluent-panel" v-if="counterparties.length">
      <div class="fluent-panel-header">
        <h3>Top 对方账户（按金额 · 全部历史）</h3>
        <span class="dist-sum">共 {{ counterparties.length }} 家 · 金额为凭证金额合计</span>
      </div>
      <div class="fluent-panel-body">
        <div v-for="(c, i) in counterparties" :key="c.name" class="cp-row">
          <span class="cp-rank" :class="{ 'cp-rank-top': i < 3 }">{{ i + 1 }}</span>
          <span class="cp-name" :title="c.name">{{ c.name }}</span>
          <div class="cp-bar-wrap">
            <div class="cp-bar" :style="{ width: cpBarWidth(c) }"
              :title="`${c.name} · ¥${fmtAmount(c.amountCents)} · ${c.count} 张`" />
          </div>
          <span class="cp-amount num" :title="`¥${fmtAmount(c.amountCents)}`">¥{{ fmtAmount(c.amountCents) }}</span>
          <span class="cp-count">{{ c.count }} 张</span>
        </div>
      </div>
    </div>

    <!-- 功能深化 3：标签分类统计（未分类固定末尾；一张多标签在每个标签下各计一次） -->
    <div class="fluent-panel" v-if="tagRows.length">
      <div class="fluent-panel-header">
        <h3>标签分类（按金额 · 全部历史）</h3>
        <span class="dist-sum">共 {{ tagRows.length }} 类 · 含未分类</span>
      </div>
      <div class="fluent-panel-body">
        <div v-for="(t, i) in tagRows" :key="t.tag || '__untagged__'" class="cp-row">
          <span class="cp-rank" :class="{ 'cp-rank-top': i < 3 && t.tag }">{{ t.tag ? i + 1 : "—" }}</span>
          <span class="cp-name" :class="{ 'cp-name-muted': !t.tag }" :title="t.tag ? t.tag : '没有任何标签的凭证'">
            {{ t.tag ? `# ${tagLabel(t)}` : tagLabel(t) }}
          </span>
          <div class="cp-bar-wrap">
            <div class="cp-bar" :class="{ 'cp-bar-muted': !t.tag }" :style="{ width: tagBarWidth(t) }"
              :title="`${tagLabel(t)} · ¥${fmtAmount(t.amountCents)} · ${t.count} 张`" />
          </div>
          <span class="cp-amount num" :title="`¥${fmtAmount(t.amountCents)}`">¥{{ fmtAmount(t.amountCents) }}</span>
          <span class="cp-count">{{ t.count }} 张</span>
        </div>
      </div>
    </div>
  </div>

  <div class="fluent-panel">
    <div class="fluent-panel-header">
      <h3>最近凭证</h3>
      <div style="display: flex; gap: 8px">
        <button class="btn btn-sm" :disabled="busy" @click="backupNow()"><FluentIcon v-if="!busy" name="save" :size="14" />立即备份</button>
        <button class="btn btn-sm" @click="router.push('/vouchers/new')"><FluentIcon name="plus" :size="14" />新建凭证</button>
      </div>
    </div>
    <div class="fluent-panel-body" style="padding: 0">
      <div class="dtable-wrap">
        <table class="dtable" v-if="recent.length">
          <thead>
            <tr>
              <th>凭证编号</th>
              <th>日期</th>
              <th>摘要</th>
              <th style="text-align: right">金额</th>
              <th>状态</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="v in recent" :key="v.id" style="cursor: pointer"
              @click="router.push(`/vouchers/${v.id}`)">
              <td><span class="mono">{{ v.voucherNo }}</span></td>
              <td>{{ v.voucherDate }}</td>
              <td>{{ v.description || "—" }}</td>
              <td class="num">{{ fmtAmount(v.amountCents) }}</td>
              <td><span class="tag" :class="statusOf(v)?.tag">{{ statusOf(v)?.label }}</span></td>
            </tr>
          </tbody>
        </table>
        <div v-else class="empty-state" style="padding: 40px">
          <div class="es-icon"><FluentIcon name="folder" :size="42" /></div>
          <div class="es-title">还没有凭证</div>
          <div class="es-desc">拖入图片、或点「新建凭证」开始整理</div>
          <button class="btn btn-primary" @click="router.push('/vouchers/new')"><FluentIcon name="plus" :size="14" />新建凭证</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.report-grid {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 16px;
  align-items: stretch;
}

/* v0.7 R-2：状态卡片可点击（hover 提示可跳转） */
.stat-click {
  cursor: pointer;
  border-radius: 8px;
  transition: transform 0.12s ease;
}

.stat-click:hover {
  transform: translateY(-1px);
}

@media (max-width: 960px) {
  .report-grid {
    grid-template-columns: 1fr;
  }
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
  font-size: 11px;
  text-anchor: middle;
}

.bar-label {
  fill: var(--text-disabled);
  font-size: 11px;
  text-anchor: middle;
}

.dist-sum {
  font-size: 12px;
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
  font-size: 12px;
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

/* 功能深化 3：标签面板中「未分类」行的弱化样式 */
.cp-name-muted {
  color: var(--text-3);
}

.cp-bar-muted {
  background: var(--text-disabled);
}

.dist-bar {
  display: flex;
  height: 14px;
  border-radius: 7px;
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
  font-size: 13px;
  color: var(--text);
}

.legend-num {
  color: var(--text-disabled);
  font-variant-numeric: tabular-nums;
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

/* v0.7 R-1：Top 对方账户排行行（排名 / 名称 / 条形 / 金额 / 张数） */
.cp-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 5px 0;
}

.cp-rank {
  flex: none;
  width: 22px;
  height: 22px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  border-radius: 50%;
  background: var(--stroke);
  color: var(--text-2);
  font-variant-numeric: tabular-nums;
}

.cp-rank-top {
  background: var(--accent-soft);
  color: var(--accent);
  font-weight: 600;
}

.cp-name {
  flex: none;
  width: 140px;
  font-size: 13px;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cp-bar-wrap {
  flex: 1;
  min-width: 0;
}

.cp-bar {
  height: 10px;
  border-radius: 5px;
  background: var(--accent);
  transition: opacity 0.15s ease;
}

.cp-bar:hover {
  opacity: 0.8;
}

.cp-amount {
  flex: none;
  width: 110px;
  font-size: 13px;
  color: var(--text);
  text-align: right;
  font-variant-numeric: tabular-nums;
}

.cp-count {
  flex: none;
  width: 52px;
  font-size: 12px;
  color: var(--text-4);
  text-align: right;
  font-variant-numeric: tabular-nums;
}
</style>
