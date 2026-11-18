<script setup lang="ts">
/**
 * 工作台（v0.27）：关键数据 + 分析图表 + 可下钻待办。
 *
 * B1 为静态原型：数字与图表先用占位/样例，B3 接 `dashboard_todos`、
 * `monthlySummary`、`topCounterparties`、`tagStats` 与 `data_health`。
 * 本页不设新建/导入入口——那是数据页的唯一入口。
 */
import { computed } from "vue";
import { useRouter } from "vue-router";
import { useInboxStore } from "../stores/inbox";
import { useModalRoute } from "../composables/useModalRoute";

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
  if (!t) return [];
  const rows: Todo[] = [
    {
      key: "ocr",
      label: "待识别",
      count: t.ocrPending,
      hint: "还有图片没跑识别",
      to: "/data?ocr=pending,failed",
    },
    {
      key: "amount",
      label: "缺金额",
      count: t.amountMissing,
      hint: "金额为空，核对前先补齐",
      to: "/data?amountEmpty=1",
    },
    {
      key: "stale",
      label: "陈旧草稿",
      count: t.staleDrafts,
      hint: `超过 ${t.staleDraftDays} 天未动`,
      to: "/data?stale=7",
    },
    {
      key: "dup",
      label: "疑似重复",
      count: t.dupPairs,
      hint: "同一笔可能录了两次",
      modal: "duplicates",
    },
    {
      key: "months",
      label: "未封账月份",
      count: t.unlockedMonths,
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

const trendPath =
  "M0,120 C40,110 70,96 110,100 C150,104 180,84 220,80 C260,76 290,62 330,66 C370,70 400,48 450,44 C500,40 540,30 600,22";
const trendArea = `${trendPath} L600,160 L0,160 Z`;

const customers = [
  { name: "凯诗蓝德", value: 128_600 },
  { name: "中天光伏", value: 86_400 },
  { name: "正泰安能", value: 62_800 },
  { name: "晴天科技", value: 41_200 },
  { name: "其他", value: 28_900 },
];

const categories = [
  { name: "组件", pct: 46 },
  { name: "逆变器", pct: 27 },
  { name: "支架", pct: 18 },
  { name: "服务", pct: 9 },
];

const money = (n: number) => `¥${(n / 10000).toFixed(1)} 万`;
</script>

<template>
  <div class="page">
    <div class="wb-hello">
      <h1 class="wb-greeting">{{ greeting }}</h1>
      <p class="wb-date">{{ dateText }} · 这是今天的账目概况</p>
    </div>

    <section class="wb-kpis" aria-label="本月关键数据">
      <button class="wb-kpi" type="button" @click="router.push('/data')">
        <span class="wb-kpi-label">本月条目</span>
        <span class="wb-kpi-value">—</span>
        <span class="wb-kpi-delta">较上月 —</span>
      </button>
      <button class="wb-kpi" type="button" @click="router.push('/data')">
        <span class="wb-kpi-label">本月金额</span>
        <span class="wb-kpi-value">—</span>
        <span class="wb-kpi-delta">较上月 —</span>
      </button>
      <button class="wb-kpi" type="button" @click="router.push('/data?booked=false')">
        <span class="wb-kpi-label">待入账</span>
        <span class="wb-kpi-value">—</span>
        <span class="wb-kpi-delta">台账已挂但未入账</span>
      </button>
      <button class="wb-kpi" type="button" @click="router.push('/data?view=exported')">
        <span class="wb-kpi-label">本月已导出</span>
        <span class="wb-kpi-value">—</span>
        <span class="wb-kpi-delta">已生成 PDF / 表格</span>
      </button>
    </section>

    <section class="wb-grid" aria-label="分析图表">
      <article class="wb-card wb-card--span2">
        <header class="wb-card-head">
          <h2 class="wb-card-title">近 12 个月</h2>
          <span class="wb-card-link">金额</span>
        </header>
        <div class="wb-chart">
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
        </div>
      </article>

      <article class="wb-card">
        <header class="wb-card-head">
          <h2 class="wb-card-title">Top 5 客户</h2>
          <span class="wb-card-link">近 12 个月</span>
        </header>
        <ul class="wb-rank">
          <li v-for="c in customers" :key="c.name">
            <span class="wb-rank-name">{{ c.name }}</span>
            <span class="wb-rank-value">{{ money(c.value) }}</span>
          </li>
        </ul>
      </article>

      <article class="wb-card">
        <header class="wb-card-head">
          <h2 class="wb-card-title">分类占比</h2>
          <span class="wb-card-link">近 12 个月</span>
        </header>
        <ul class="wb-rank">
          <li v-for="c in categories" :key="c.name">
            <span class="wb-rank-name">{{ c.name }}</span>
            <span class="wb-rank-value">{{ c.pct }}%</span>
          </li>
        </ul>
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
        <p v-else class="wb-empty">没有待处理事项，今天的账已经清干净了。</p>
      </article>

      <article class="wb-card">
        <header class="wb-card-head">
          <h2 class="wb-card-title">最近条目</h2>
          <button class="wb-card-link" type="button" @click="router.push('/data')">全部</button>
        </header>
        <p class="wb-empty">还没有条目。去数据页新建第一条。</p>
      </article>

      <article class="wb-card">
        <header class="wb-card-head">
          <h2 class="wb-card-title">最近导出</h2>
          <button class="wb-card-link" type="button" @click="open('export', { tab: 'history' })">
            导出历史
          </button>
        </header>
        <p class="wb-empty">还没有导出批次。</p>
      </article>
    </section>
  </div>
</template>
