<script setup lang="ts">
/**
 * 工作台（v0.27）：关键数据 + 分析图表 + 可下钻待办。
 *
 * B3 起全部接真实数据：`dashboard_overview`、`monthly_summary`、
 * `top_counterparties`、`tag_stats`、`data_health`、`dashboard_todos`、
 * `list_vouchers` 与 `list_export_batches`。
 * 本页不设新建/导入入口——那是数据页的唯一入口。
 */
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { api } from "../api/client";
import type {
  CounterpartyRow,
  DashboardOverview,
  DataHealth,
  ExportBatch,
  ExportCoverage,
  MonthlySummary,
  TagRow,
  Voucher,
} from "../api/client";
import { useInboxStore } from "../stores/inbox";
import { useModalRoute } from "../composables/useModalRoute";
import { toastError } from "../stores/toast";

const router = useRouter();
const inbox = useInboxStore();
const { open } = useModalRoute();

const now = new Date();
const hour = now.getHours();
const greeting = hour < 6 ? "夜深了" : hour < 12 ? "早上好" : hour < 18 ? "下午好" : "晚上好";
const dateText = now.toLocaleDateString("zh-CN", {
  month: "long",
  day: "numeric",
  weekday: "long",
});

const loading = ref(true);
const overview = ref<DashboardOverview | null>(null);
const months = ref<MonthlySummary[]>([]);
const customers = ref<CounterpartyRow[]>([]);
const categories = ref<TagRow[]>([]);
const health = ref<DataHealth | null>(null);
const recent = ref<Voucher[]>([]);
const exports = ref<ExportBatch[]>([]);
const bookedCount = ref(0);
const coverage = ref<ExportCoverage | null>(null);

function ymd(d: Date): string {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

function monthBounds(): { from: string; to: string } {
  const first = new Date(now.getFullYear(), now.getMonth(), 1);
  const last = new Date(now.getFullYear(), now.getMonth() + 1, 0);
  return { from: ymd(first), to: ymd(last) };
}

async function load() {
  const { from, to } = monthBounds();
  const results = await Promise.allSettled([
    api.dashboardOverview(from, to, null),
    api.monthlySummary(12),
    api.topCounterparties(12, 5),
    api.tagStats(12, 5),
    api.dataHealth(),
    api.listVouchers({ page: 1, pageSize: 5, sort: "updated_desc" }),
    api.listExportBatches(5),
    api.listVouchers({ page: 1, pageSize: 1, booked: false, noImage: false }),
    api.exportCoverage(from, to),
  ]);
  if (results[0].status === "fulfilled") overview.value = results[0].value;
  if (results[1].status === "fulfilled") months.value = results[1].value;
  if (results[2].status === "fulfilled") customers.value = results[2].value;
  if (results[3].status === "fulfilled") categories.value = results[3].value;
  if (results[4].status === "fulfilled") health.value = results[4].value;
  if (results[5].status === "fulfilled") recent.value = results[5].value.items;
  if (results[6].status === "fulfilled") exports.value = results[6].value;
  if (results[7].status === "fulfilled") bookedCount.value = results[7].value.total;
  if (results[8].status === "fulfilled") coverage.value = results[8].value;
  const failed = results.find((r) => r.status === "rejected");
  if (failed && failed.status === "rejected") toastError(failed.reason);
  loading.value = false;
}

onMounted(() => {
  void inbox.refresh();
  void load();
});

interface Todo {
  key: string;
  label: string;
  count: number;
  hint: string;
  to?: string;
  modal?: "duplicates";
}

const todos = computed<Todo[]>(() => {
  const t = inbox.todos;
  const h = health.value;
  const rows: Todo[] = [
    {
      key: "missing",
      label: "待补凭证",
      count: h?.noImageRequired ?? 0,
      hint: "新条目必须挂凭证，补齐后才能入账",
      to: "/data?view=missing",
    },
    {
      key: "legacy",
      label: "历史未补",
      count: h?.noImageOptional ?? 0,
      hint: "迁移豁免的说明/备忘条目，可补图",
      to: "/data?view=legacy",
    },
    {
      key: "ocr",
      label: "待识别",
      count: t?.ocrPending ?? 0,
      hint: "还有图片没跑识别",
      to: "/data?ocr=pending,failed",
    },
    {
      key: "amount",
      label: "缺金额",
      count: t?.amountMissing ?? 0,
      hint: "金额为空，核对前先补齐",
      to: "/data?amountEmpty=1",
    },
    {
      key: "stale",
      label: "陈旧草稿",
      count: t?.staleDrafts ?? 0,
      hint: `超过 ${t?.staleDraftDays ?? 7} 天未动`,
      to: "/data?stale=7",
    },
    {
      key: "dup",
      label: "疑似重复",
      count: t?.dupPairs ?? 0,
      hint: "同一笔可能录了两次",
      modal: "duplicates",
    },
    {
      key: "months",
      label: "未封账月份",
      count: t?.unlockedMonths ?? 0,
      hint: "封账后投影才稳定",
      to: "/accounting",
    },
  ];
  return rows.filter((r) => r.count > 0);
});

function go(t: Todo) {
  if (t.modal) open(t.modal);
  else if (t.to) void router.push(t.to);
}

function fmtMoney(cents: number | null | undefined): string {
  if (cents == null) return "—";
  return `¥${(cents / 100).toLocaleString("zh-CN", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  })}`;
}

function fmtShortMoney(cents: number): string {
  if (cents >= 100_000_00) return `¥${(cents / 100 / 10000).toFixed(1)} 万`;
  return fmtMoney(cents);
}

const kpis = computed(() => {
  const ov = overview.value;
  const count = ov?.count ?? 0;
  const amount = ov?.amountCents ?? 0;
  const prevCount = ov?.prevCount ?? null;
  const prevAmount = ov?.prevAmountCents ?? null;
  const countDelta =
    prevCount == null
      ? "较上月 —"
      : `较上月 ${count - prevCount >= 0 ? "+" : ""}${count - prevCount}`;
  let amountDelta = "较上月 —";
  if (prevAmount && prevAmount > 0) {
    const pct = ((amount - prevAmount) / prevAmount) * 100;
    amountDelta = `较上月 ${pct >= 0 ? "+" : ""}${pct.toFixed(1)}%`;
  }
  return [
    {
      key: "count",
      label: "本月条目",
      value: String(count),
      delta: countDelta,
      to: "/data?view=month",
    },
    {
      key: "amount",
      label: "本月金额",
      value: fmtMoney(amount),
      delta: amountDelta,
      to: "/data?view=month",
    },
    {
      key: "booked",
      label: "待入账",
      value: String(bookedCount.value),
      delta: "台账已挂但未入账",
      to: "/data?view=booked",
    },
    {
      key: "exported",
      label: "本月已导出",
      value: String(coverage.value?.exported ?? 0),
      delta: "已生成 PDF / 表格",
      to: "/data?exported=any",
    },
  ];
});

const trendPath = computed(() => {
  const vals = months.value.map((m) => m.amountCents);
  if (vals.length < 2) return "";
  const max = Math.max(...vals, 1);
  return vals
    .map((v, i) => {
      const x = (i / (vals.length - 1)) * 600;
      const y = 150 - (v / max) * 130;
      return `${i === 0 ? "M" : "L"}${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
});

const trendArea = computed(() => (trendPath.value ? `${trendPath.value} L600,160 L0,160 Z` : ""));

const customerRows = computed(() =>
  customers.value.map((c) => ({ name: c.name || "未填", value: fmtShortMoney(c.amountCents) })),
);

const categoryRows = computed(() => {
  const sum = categories.value.reduce((a, c) => a + c.amountCents, 0);
  if (!sum) return [];
  return categories.value.map((c) => ({
    name: c.tag || "未分类",
    pct: Math.round((c.amountCents / sum) * 100),
  }));
});

const recentRows = computed(() =>
  recent.value.map((v) => ({
    id: v.id,
    date: v.voucherDate,
    main: v.counterparty || v.description || "（未填）",
    amount: fmtMoney(v.amountCents),
    status: v.status,
  })),
);
</script>

<template>
  <div class="page">
    <div class="wb-hello">
      <h1 class="wb-greeting">{{ greeting }}</h1>
      <p class="wb-date">{{ dateText }} · 这是今天的账目概况</p>
    </div>

    <section class="wb-kpis" aria-label="本月关键数据">
      <button
        v-for="k in kpis"
        :key="k.key"
        class="wb-kpi"
        type="button"
        @click="router.push(k.to)"
      >
        <span class="wb-kpi-label">{{ k.label }}</span>
        <span class="wb-kpi-value">{{ loading ? "…" : k.value }}</span>
        <span class="wb-kpi-delta">{{ k.delta }}</span>
      </button>
    </section>

    <section class="wb-grid" aria-label="分析图表">
      <article class="wb-card wb-card--span2">
        <header class="wb-card-head">
          <h2 class="wb-card-title">近 12 个月</h2>
          <span class="wb-card-link">金额</span>
        </header>
        <div v-if="trendPath" class="wb-chart">
          <svg
            viewBox="0 0 600 160"
            preserveAspectRatio="none"
            role="img"
            aria-label="近 12 个月金额趋势"
          >
            <defs>
              <linearGradient id="wb-trend" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stop-color="var(--accent)" stop-opacity=".22" />
                <stop offset="100%" stop-color="var(--accent)" stop-opacity="0" />
              </linearGradient>
            </defs>
            <path :d="trendArea" fill="url(#wb-trend)" />
            <path :d="trendPath" fill="none" stroke="var(--accent)" stroke-width="2" />
          </svg>
          <div class="wb-chart-axis">
            <span>{{ months[0]?.month.slice(5) }}</span>
            <span>{{ months[months.length - 1]?.month.slice(5) }}</span>
          </div>
        </div>
        <p v-else class="wb-empty">{{ loading ? "正在读取…" : "还没有足够的月度数据。" }}</p>
      </article>

      <article class="wb-card">
        <header class="wb-card-head">
          <h2 class="wb-card-title">Top 5 客户</h2>
          <span class="wb-card-link">近 12 个月</span>
        </header>
        <ul v-if="customerRows.length" class="wb-rank">
          <li v-for="c in customerRows" :key="c.name">
            <span class="wb-rank-name">{{ c.name }}</span>
            <span class="wb-rank-value">{{ c.value }}</span>
          </li>
        </ul>
        <p v-else class="wb-empty">{{ loading ? "正在读取…" : "还没有客户数据。" }}</p>
      </article>

      <article class="wb-card">
        <header class="wb-card-head">
          <h2 class="wb-card-title">分类占比</h2>
          <span class="wb-card-link">近 12 个月</span>
        </header>
        <ul v-if="categoryRows.length" class="wb-rank">
          <li v-for="c in categoryRows" :key="c.name">
            <span class="wb-rank-name">{{ c.name }}</span>
            <span class="wb-rank-value">{{ c.pct }}%</span>
          </li>
        </ul>
        <p v-else class="wb-empty">{{ loading ? "正在读取…" : "还没有分类数据。" }}</p>
      </article>
    </section>

    <section class="wb-grid" aria-label="待办与最近">
      <article class="wb-card">
        <header class="wb-card-head">
          <h2 class="wb-card-title">该处理的事</h2>
        </header>
        <ul v-if="todos.length" class="wb-todos">
          <li v-for="t in todos" :key="t.key" class="wb-todo">
            <span class="wb-todo-main">
              <span class="wb-todo-label">{{ t.label }}</span>
              <span class="wb-todo-count">{{ t.count }}</span>
              <span class="wb-todo-hint">{{ t.hint }}</span>
            </span>
            <button class="wb-todo-action" type="button" @click="go(t)">去处理</button>
          </li>
        </ul>
        <p v-else class="wb-empty">
          {{ loading ? "正在读取…" : "没有待处理事项，今天的账已经清干净了。" }}
        </p>
      </article>

      <article class="wb-card">
        <header class="wb-card-head">
          <h2 class="wb-card-title">最近条目</h2>
          <button class="wb-card-link" type="button" @click="router.push('/data')">全部</button>
        </header>
        <ul v-if="recentRows.length" class="wb-recent">
          <li v-for="r in recentRows" :key="r.id">
            <button type="button" class="wb-recent-row" @click="router.push(`/data?focus=${r.id}`)">
              <span class="wb-recent-date">{{ r.date }}</span>
              <span class="wb-recent-main">{{ r.main }}</span>
              <span class="wb-recent-amount">{{ r.amount }}</span>
            </button>
          </li>
        </ul>
        <p v-else class="wb-empty">
          {{ loading ? "正在读取…" : "还没有条目。去数据页新建第一条。" }}
        </p>
      </article>

      <article class="wb-card">
        <header class="wb-card-head">
          <h2 class="wb-card-title">最近导出</h2>
          <button class="wb-card-link" type="button" @click="open('export', { tab: 'history' })">
            导出历史
          </button>
        </header>
        <ul v-if="exports.length" class="wb-recent">
          <li v-for="b in exports" :key="b.id">
            <span class="wb-recent-row">
              <span class="wb-recent-date">{{ b.createdAt.slice(0, 10) }}</span>
              <span class="wb-recent-main">{{ b.batchNo }}</span>
              <span class="wb-recent-amount">{{ b.voucherCount }} 条</span>
            </span>
          </li>
        </ul>
        <p v-else class="wb-empty">
          {{ loading ? "正在读取…" : "还没有导出批次。" }}
        </p>
      </article>
    </section>
  </div>
</template>
