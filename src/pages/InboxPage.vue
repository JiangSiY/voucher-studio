<script setup lang="ts">
/**
 * 待处理（收件箱）—— v0.18 收件箱驱动重构的首页。
 *
 * 与旧「仪表盘」的根本区别（用户明确要求"重构，不是在旧布局上增删改"）：
 * - **问题不是"这个月花了多少"，而是"现在该做什么"**：页面就是一条流水线——
 *   待识别 → 待核对 → 待打印/导出 → 疑似重复处置 —— 每个阶段一张卡，能就地推进，
 *   不必先想"这功能在哪个页面"，也不用来回跳页再重新筛选。
 * - 统计（趋势/排行）**降级为资料库的视图**，不再占据首屏；首屏留给待办。
 * - 阶段卡的数字与侧栏徽标同源（`inboxStore`），处理完即刷新。
 *
 * 数据来源：`dashboard_todos`（待办/健康度，轻量计数）+ `voucher_stats`（全库口径的
 * 阶段计数）+ `duplicate_pending_count`（疑似重复轻量计数）。
 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useRouter } from "vue-router";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { safeListen } from "../composables/safeListen";
import { api, type ExportBatch, type Voucher, type VoucherStats } from "../api/client";
import EmptyState from "../components/EmptyState.vue";
import FluentIcon from "../components/FluentIcon.vue";
import { STATUS_META, fmtDate } from "../composables/format";
import { useInboxStore } from "../stores/inbox";
import { toast, toastError } from "../stores/toast";

const router = useRouter();
const inbox = useInboxStore();

const stats = ref<VoucherStats | null>(null);
const recentDrafts = ref<Voucher[]>([]);
const batches = ref<ExportBatch[]>([]);
const busy = ref<"ocr" | null>(null);

/** 流水线阶段：每个阶段一句话说明"为什么在这里、点了会怎样" */
const stages = computed(() => {
  const t = inbox.todos;
  const s = stats.value;
  return [
    {
      key: "ocr",
      icon: "sparkle",
      title: "待识别",
      count: t?.ocrPending ?? 0,
      unit: "张附件",
      why: t?.ocrPending ? "识别后可自动填单（日期/金额/对方/摘要）" : "没有等待识别的图片",
      action: {
        label: busy.value === "ocr" ? "识别中…" : "开始识别",
        run: runOcr,
        disabled: !t?.ocrPending || busy.value === "ocr",
      },
      secondary: null as { label: string; run: () => void } | null,
    },
    {
      key: "review",
      icon: "checkCircle",
      title: "待核对",
      count: s?.draft ?? 0,
      unit: "张草稿",
      why: s?.draft ? "草稿尚未复核；核对后可送打印导出" : "没有待核对的草稿",
      action: {
        label: "继续核对",
        run: () => router.push({ path: "/vouchers", query: { status: "draft" } }),
        disabled: !s?.draft,
      },
      secondary: recentDrafts.value.length
        ? { label: `最近一张：${recentDrafts.value[0].voucherNo}`, run: () => openVoucher(recentDrafts.value[0].id) }
        : null,
    },
    {
      key: "print",
      icon: "print",
      title: "待打印 / 导出",
      count: stats.value?.reviewed ?? 0,
      unit: "张已复核",
      why: stats.value?.reviewed ? "已复核但还没出 PDF/表格，可批量打印归档" : "没有待打印的凭证",
      action: {
        label: "去打印",
        run: () => router.push("/print"),
        disabled: !stats.value?.reviewed,
      },
      secondary: null,
    },
    {
      key: "dup",
      icon: "duplicate",
      title: "疑似重复待处置",
      count: inbox.todos?.dupPairs ?? 0,
      unit: "对",
      why: inbox.todos?.dupHighPairs
        ? `其中 ${inbox.todos.dupHighPairs} 对证据较强（同图或同对方同摘要）`
        : "同日期同金额的对，需人工判定",
      action: {
        label: "去核查",
        run: () => router.push("/duplicates"),
        disabled: !inbox.todos?.dupPairs,
      },
      secondary: null,
    },
  ];
});

/** 健康项（不阻断流程，但会影响数据质量） */
const health = computed(() => {
  const t = inbox.todos;
  if (!t) return [];
  const items: { key: string; label: string; count: number; hint: string; run: () => void }[] = [
    {
      key: "amount",
      label: "未填金额",
      count: t.amountMissing,
      hint: "金额为空的凭证（不参与统计与重复判定）",
      run: () => router.push({ path: "/vouchers", query: { status: "draft" } }),
    },
    {
      key: "stale",
      label: `超过 ${t.staleDraftDays} 天没动的草稿`,
      count: t.staleDrafts,
      hint: "长期挂着的草稿，建议复核或删除",
      run: () => router.push({ path: "/vouchers", query: { status: "draft" } }),
    },
    {
      key: "unlock",
      label: "未封账的往月",
      count: t.unlockedMonths,
      hint: "有凭证但未锁定会计期间",
      run: () => router.push("/settings?tab=naming"),
    },
    {
      key: "backup",
      label: "备份",
      count: t.backupStale ? 1 : 0,
      hint: t.latestBackup
        ? `最近备份 ${fmtDate(t.latestBackup.modifiedAt)}`
        : "尚无任何备份",
      run: () => router.push("/restore"),
    },
  ];
  return items;
});

/** 本地全部处理完了？给一个明确的"清零"状态，而不是空白页 */
const allClear = computed(() => {
  const t = inbox.todos;
  if (!t) return false;
  return (
    t.ocrPending === 0 &&
    t.dupPairs === 0 &&
    (stats.value?.draft ?? 0) === 0 &&
    (stats.value?.reviewed ?? 0) === 0
  );
});

async function runOcr() {
  busy.value = "ocr";
  try {
    const done = await api.runOcrPending(50);
    toast.success(done ? `已识别 ${done} 张` : "没有可识别的附件");
    await load();
  } catch (e) {
    toastError(e);
  } finally {
    busy.value = null;
  }
}

function openVoucher(id: number) {
  router.push(`/vouchers/${id}`);
}

async function load() {
  await Promise.all([
    inbox.refresh(),
    (async () => {
      try {
        const [s, drafts, bs] = await Promise.all([
          api.voucherStats(),
          api.listVouchers({ status: "draft", page: 1, pageSize: 3 }),
          api.listExportBatches(3),
        ]);
        stats.value = s;
        recentDrafts.value = drafts.items;
        batches.value = bs.slice(0, 3);
      } catch (e) {
        toastError(e);
      }
    })(),
  ]);
}

let unlistenImport: UnlistenFn | null = null;
let unlistenDup: UnlistenFn | null = null;
let unlistenBackup: UnlistenFn | null = null;

onMounted(async () => {
  await load();
  unlistenImport = await safeListen("import:done", () => load());
  unlistenDup = await safeListen("dup:changed", () => load());
  unlistenBackup = await safeListen("backup:done", () => load());
});

onUnmounted(() => {
  unlistenImport?.();
  unlistenDup?.();
  unlistenBackup?.();
});
</script>

<template>
  <div class="inbox">
    <!-- 一句话现状：今天有几件事 -->
    <header class="inbox-head">
      <h2>待处理</h2>
      <span class="scope">
        <template v-if="inbox.pendingCount">有 {{ inbox.pendingCount }} 件事等着办</template>
        <template v-else>暂时没有待办</template>
      </span>
      <span class="flex-1"></span>
      <button class="btn" title="立即备份" @click="router.push('/restore')">
        <FluentIcon name="save" :size="14" /> 备份
      </button>
      <button class="btn btn-primary" @click="router.push('/vouchers/new')">
        <FluentIcon name="plus" :size="14" /> 新建凭证
      </button>
    </header>

    <!-- 流水线：四个阶段，能就地推进 -->
    <section class="stages">
      <article v-for="s in stages" :key="s.key" class="stage" :class="{ done: s.count === 0 }">
        <div class="stage-top">
          <FluentIcon :name="s.icon" :size="18" />
          <span class="stage-title">{{ s.title }}</span>
          <span class="flex-1"></span>
          <span class="stage-count">{{ s.count }}</span>
          <span class="stage-unit">{{ s.unit }}</span>
        </div>
        <p class="stage-why">{{ s.why }}</p>
        <div class="stage-ops">
          <button class="btn btn-primary" :disabled="s.action.disabled" @click="s.action.run()">
            {{ s.action.label }}
          </button>
          <button v-if="s.secondary" class="btn" @click="s.secondary.run()">{{ s.secondary.label }}</button>
        </div>
      </article>
    </section>

    <!-- 全部处理完：明确说清"没有待办"，并给下一步建议 -->
    <EmptyState
      v-if="allClear"
      icon="checkCircle"
      title="待办都清空了"
      desc="没有待识别的图片、待核对的草稿、待打印的已复核凭证，也没有未处置的疑似重复。可以去「资料库」查看趋势与排行。"
    >
      <button class="btn" @click="router.push('/vouchers')">打开资料库</button>
    </EmptyState>

    <!-- 健康项：不阻断流程，但影响数据质量 -->
    <section v-if="health.some((h) => h.count > 0)" class="health">
      <h3>数据质量</h3>
      <ul>
        <li v-for="h in health.filter((x) => x.count > 0)" :key="h.key">
          <button class="health-item" @click="h.run()">
            <span class="health-count">{{ h.count }}</span>
            <span class="health-text">
              <b>{{ h.label }}</b>
              <small>{{ h.hint }}</small>
            </span>
            <FluentIcon name="chevronRight" :size="13" />
          </button>
        </li>
      </ul>
    </section>

    <!-- 最近活动（次要信息，放最后） -->
    <div class="activity">
      <section class="panel">
        <header class="section-head">
          <h3>最近凭证</h3>
          <span class="flex-1"></span>
          <button class="link" @click="router.push('/vouchers')">全部</button>
        </header>
        <ul v-if="recentDrafts.length" class="mini-list">
          <li v-for="v in recentDrafts" :key="v.id" @click="openVoucher(v.id)">
            <span class="mono">{{ v.voucherNo }}</span>
            <span class="muted">{{ v.description || "（无摘要）" }}</span>
            <span class="flex-1"></span>
            <span class="tag" :class="STATUS_META[v.status].tag">{{ STATUS_META[v.status].label }}</span>
          </li>
        </ul>
        <p v-else class="empty-hint">没有草稿。</p>
      </section>

      <section class="panel">
        <header class="section-head">
          <h3>最近导出</h3>
          <span class="flex-1"></span>
          <button class="link" @click="router.push('/exports')">导出历史</button>
        </header>
        <ul v-if="batches.length" class="mini-list">
          <li v-for="b in batches" :key="b.id">
            <span class="mono">{{ b.batchNo }}</span>
            <span class="tag tag-info">{{ b.format?.toUpperCase() ?? "PDF" }}</span>
            <span class="muted">{{ b.voucherCount }} 张</span>
            <span class="flex-1"></span>
            <span class="muted">{{ fmtDate(b.createdAt) }}</span>
          </li>
        </ul>
        <p v-else class="empty-hint">还没有导出批次。</p>
      </section>
    </div>
  </div>
</template>

<style scoped>
/* 收件箱：首屏只讲"该做什么"；样式全部使用设计正本 §1 的语义令牌 */
.inbox { display: flex; flex-direction: column; gap: var(--s-4); }

.inbox-head { display: flex; align-items: center; gap: var(--s-3); flex-wrap: wrap; }
.inbox-head h2 { font: var(--type-title); color: var(--label-primary); margin: 0; font-size: calc(20px * var(--font-scale)); }
.flex-1 { flex: 1; }

.stages { display: grid; grid-template-columns: repeat(auto-fit, minmax(300px, 1fr)); gap: var(--s-3); }
.stage {
  display: flex; flex-direction: column; gap: var(--s-2);
  padding: var(--s-4);
  border: 1px solid var(--separator);
  border-radius: var(--radius-panel);
  background: var(--bg-content);
}
/* 已清零的阶段弱化，但不隐藏——用户要能确认"确实没有" */
.stage.done { opacity: 0.55; }
.stage-top { display: flex; align-items: baseline; gap: var(--s-2); color: var(--label-secondary); }
.stage-title { font: var(--type-body); font-weight: 600; color: var(--label-primary); }
.stage-count { font-size: calc(26px * var(--font-scale)); font-weight: 600; color: var(--label-primary); font-variant-numeric: tabular-nums; }
.stage-unit { font: var(--type-caption); color: var(--label-tertiary); }
.stage-why { margin: 0; font: var(--type-caption); color: var(--label-secondary); }
.stage-ops { display: flex; gap: var(--s-2); flex-wrap: wrap; margin-top: auto; }

.health h3, .section-head h3 { font: var(--type-title); color: var(--label-primary); margin: 0; }
.health ul { list-style: none; margin: var(--s-2) 0 0; padding: 0; display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: var(--s-2); }
.health-item {
  display: flex; align-items: center; gap: var(--s-2); width: 100%;
  padding: var(--s-2) var(--s-3);
  border: 1px solid var(--separator);
  border-radius: var(--radius-panel);
  background: var(--bg-content);
  text-align: left; cursor: default;
}
.health-item:hover { background: var(--bg-selection); }
.health-count { font-size: calc(18px * var(--font-scale)); font-weight: 600; color: var(--label-primary); font-variant-numeric: tabular-nums; }
.health-text { display: flex; flex-direction: column; flex: 1; min-width: 0; }
.health-text b { font: var(--type-body); font-weight: 500; color: var(--label-primary); }
.health-text small { font: var(--type-caption); color: var(--label-tertiary); }

.activity { display: grid; grid-template-columns: repeat(auto-fit, minmax(320px, 1fr)); gap: var(--s-3); }
.panel { padding: var(--s-3); border: 1px solid var(--separator); border-radius: var(--radius-panel); background: var(--bg-content); }
.section-head { display: flex; align-items: baseline; gap: var(--s-2); }
.mini-list { list-style: none; margin: var(--s-2) 0 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
.mini-list li { display: flex; align-items: center; gap: var(--s-2); padding: 6px var(--s-2); border-radius: var(--radius-small); font: var(--type-body); cursor: default; }
.mini-list li:hover { background: var(--bg-selection); }
.muted { color: var(--label-tertiary); font: var(--type-caption); }
.link { border: none; background: none; color: var(--accent); font: var(--type-caption); cursor: default; }
.link:hover { text-decoration: underline; }
.empty-hint { color: var(--label-tertiary); font: var(--type-body); margin: var(--s-2) 0; }
</style>
