<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { api, type BackupInfo, type MonthlySummary, type Voucher, type VoucherStats } from "../api/client";
import StatCard from "../components/StatCard.vue";
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

async function load() {
  try {
    stats.value = await api.voucherStats();
    const list = await api.listVouchers({ page: 1, pageSize: 6 });
    recent.value = list.items;
    backups.value = await api.listBackups();
    monthly.value = await api.monthlySummary(REPORT_MONTHS);
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

// ---------- 月度趋势柱状图（手写 SVG，无第三方图表库） ----------

const CHART_W = 560;
const CHART_H = 190;
const PAD_L = 10;
const PAD_T = 22;
const PAD_B = 26;
const PLOT_H = CHART_H - PAD_T - PAD_B;

const maxCount = computed(() =>
  Math.max(1, ...monthly.value.map((m) => m.count)),
);

const slot = (CHART_W - PAD_L) / Math.max(1, monthly.value.length || REPORT_MONTHS);
const barW = Math.min(44, slot * 0.55);

function barX(i: number) {
  return PAD_L + slot * i + (slot - barW) / 2;
}

function barH(m: MonthlySummary) {
  return m.count === 0 ? 3 : Math.max(4, (m.count / maxCount.value) * PLOT_H);
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

onMounted(load);
</script>

<template>
  <div class="stat-grid" v-if="stats">
    <StatCard label="凭证总数" :value="stats.total" :delta="`本月新增 ${stats.thisMonth}`" />
    <StatCard label="草稿" :value="stats.draft" delta="待整理" />
    <StatCard label="已复核" :value="stats.reviewed" delta="可归档打印" />
    <StatCard label="已导出" :value="stats.exported" delta="已生成 PDF" />
    <StatCard label="附件图片" :value="stats.attachments" delta="含 OCR 识别" />
    <StatCard label="本地备份" :value="backups.length" :delta="backups[0] ? `最近 ${fmtDate(backups[0].createdAt)}` : '尚无备份'" />
  </div>

  <div class="callout info">
    <span class="ci">💡</span>
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
      </div>
      <div class="fluent-panel-body">
        <svg
          class="trend-svg"
          :viewBox="`0 0 ${CHART_W} ${CHART_H}`"
          role="img"
          aria-label="月度凭证张数趋势"
          preserveAspectRatio="xMidYMid meet"
        >
          <line class="axis" :x1="PAD_L" :y1="PAD_T + PLOT_H" :x2="CHART_W" :y2="PAD_T + PLOT_H" />
          <g v-for="(m, i) in monthly" :key="m.month">
            <rect
              class="bar"
              :class="{ zero: m.count === 0 }"
              :x="barX(i)"
              :y="barY(m)"
              :width="barW"
              :height="barH(m)"
              rx="3"
            >
              <title>{{ barTitle(m) }}</title>
            </rect>
            <text v-if="m.count > 0" class="bar-val" :x="barMid(i)" :y="barY(m) - 5">
              {{ m.count }}
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
          <div class="es-icon">📊</div>
          <div class="es-title">近 {{ REPORT_MONTHS }} 月暂无凭证</div>
          <div class="es-desc">导入图片后这里会出现统计图表</div>
        </div>
      </div>
    </div>
  </div>

  <div class="fluent-panel">
    <div class="fluent-panel-header">
      <h3>最近凭证</h3>
      <div style="display: flex; gap: 8px">
        <button class="btn btn-sm" :disabled="busy" @click="backupNow()">💾 立即备份</button>
        <button class="btn btn-sm" @click="router.push('/vouchers/new')">＋ 新建凭证</button>
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
          <div class="es-icon">🗂</div>
          <div class="es-title">还没有凭证</div>
          <div class="es-desc">拖入图片、或点「新建凭证」开始整理</div>
          <button class="btn btn-primary" @click="router.push('/vouchers/new')">＋ 新建凭证</button>
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
</style>
