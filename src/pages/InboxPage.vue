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

/**
 * **待办清单（单一来源）**：把流水线阶段与数据质量项合成一份列表，
 * **count 为 0 的一律不渲染**（用户要求："以待办显示，完成之后就隐藏，提高空间利用率"）。
 * 顺序固定（先流程、后质量），便于形成肌肉记忆。
 */
const todoItems = computed(() => {
  const t = inbox.todos;
  const s = stats.value;
  type Item = {
    key: string;
    icon: string;
    title: string;
    count: number;
    unit: string;
    hint: string;
    action: { label: string; run: () => void; disabled?: boolean };
    secondary?: { label: string; run: () => void };
  };
  const all: Item[] = [
    {
      key: "ocr",
      icon: "sparkle",
      title: "待识别",
      count: t?.ocrPending ?? 0,
      unit: "张",
      hint: "识别后可自动填单（日期 / 金额 / 对方 / 摘要）",
      action: { label: busy.value === "ocr" ? "识别中…" : "开始识别", run: runOcr, disabled: busy.value === "ocr" },
      // 精确下钻：只看"待识别/失败"的附件所在凭证（批次 5 的 ocrStatus 维度）
      secondary: { label: "查看待识别", run: () => router.push({ path: "/vouchers", query: { ocrStatus: "pending" } }) },
    },
    {
      key: "review",
      icon: "checkCircle",
      title: "待核对",
      count: s?.draft ?? 0,
      unit: "张草稿",
      hint: recentDrafts.value.length
        ? `最近一张：${recentDrafts.value[0].voucherNo}（点击右侧"继续核对"进入核对序列）`
        : "草稿尚未复核",
      action: { label: "继续核对", run: () => router.push({ path: "/vouchers", query: { status: "draft" } }) },
    },
    {
      key: "print",
      icon: "print",
      title: "待打印 / 导出",
      count: s?.reviewed ?? 0,
      unit: "张已复核",
      hint: "已复核但还没出 PDF / 表格",
      action: { label: "去打印", run: () => router.push("/print") },
    },
    {
      key: "dup",
      icon: "duplicate",
      title: "疑似重复待处置",
      count: t?.dupPairs ?? 0,
      unit: "对",
      hint: t?.dupHighPairs ? `其中 ${t.dupHighPairs} 对证据较强（同图或同对方同摘要）` : "同日期同金额，需人工判定",
      action: { label: "去核查", run: () => router.push("/duplicates") },
    },
    {
      key: "amount",
      icon: "edit",
      title: "未填金额",
      count: t?.amountMissing ?? 0,
      unit: "张",
      hint: "金额为空（不参与统计与重复判定）",
      // 精确下钻：只列"金额为空"的凭证（批次 5 的 amountEmpty 维度）
      action: { label: "去补填", run: () => router.push({ path: "/vouchers", query: { amountEmpty: "1" } }) },
    },
    {
      key: "stale",
      icon: "info",
      title: `超过 ${t?.staleDraftDays ?? 7} 天没动的草稿`,
      count: t?.staleDrafts ?? 0,
      unit: "张",
      hint: "长期挂着的草稿，建议复核或删除",
      action: { label: "去看看", run: () => router.push({ path: "/vouchers", query: { status: "draft", sort: "updated_desc" } }) },
    },
    {
      key: "unlock",
      icon: "lock",
      title: "未封账的往月",
      count: t?.unlockedMonths ?? 0,
      unit: "个月",
      hint: "有凭证但未锁定会计期间",
      action: { label: "去封账", run: () => router.push("/settings?tab=naming") },
    },
    {
      key: "backup",
      icon: "save",
      title: "该备份了",
      count: t?.backupStale ? 1 : 0,
      unit: "项",
      hint: t?.latestBackup ? `最近备份 ${fmtDate(t.latestBackup.modifiedAt)}` : "尚无任何备份",
      action: { label: "去备份", run: () => router.push("/restore") },
    },
  ];
  return all.filter((i) => i.count > 0);
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

    <!-- 待办清单：count 为 0 的项不渲染；全部清零时整块消失（空间让给资料） -->
    <section v-if="todoItems.length" class="todos">
      <ul>
        <li v-for="i in todoItems" :key="i.key">
          <button class="todo" :disabled="i.action.disabled" @click="i.action.run()">
            <span class="todo-icon"><FluentIcon :name="i.icon" :size="17" /></span>
            <span class="todo-title">{{ i.title }}</span>
            <span class="todo-count">{{ i.count }}<small>{{ i.unit }}</small></span>
            <span class="todo-hint">{{ i.hint }}</span>
            <span class="flex-1"></span>
            <span v-if="i.secondary" class="todo-secondary" @click.stop="i.secondary.run()">
              {{ i.secondary.label }}
            </span>
            <span class="todo-action">{{ i.action.label }}</span>
            <FluentIcon name="chevronRight" :size="13" />
          </button>
        </li>
      </ul>
    </section>

    <!-- 全部处理完：只留一条细横幅，不占版面 -->
    <div v-else-if="inbox.todos" class="all-clear">
      <FluentIcon name="checkCircle" :size="17" />
      <span>待办都清空了 —— 没有待识别的图片、待核对的草稿、待打印的凭证，也没有未处置的疑似重复。</span>
      <span class="flex-1"></span>
      <button class="link" @click="router.push('/vouchers')">打开资料库</button>
    </div>

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

/* 待办行：一行一件，高 44px 左右——比卡片网格省下大量纵向空间 */
.todos ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
.todo {
  display: flex; align-items: center; gap: var(--s-3); width: 100%;
  padding: 10px var(--s-3);
  border: 1px solid transparent;
  border-radius: var(--radius-control);
  background: var(--bg-content);
  text-align: left; cursor: default;
}
.todo:hover { background: var(--bg-selection); border-color: var(--separator); }
.todo:disabled { opacity: 0.6; }
.todo-icon { color: var(--label-secondary); display: inline-flex; }
.todo-title { font: var(--type-body); font-weight: 600; color: var(--label-primary); white-space: nowrap; }
.todo-count { font: var(--type-body); font-weight: 600; color: var(--label-primary); font-variant-numeric: tabular-nums; white-space: nowrap; }
.todo-count small { margin-left: 3px; font: var(--type-caption); font-weight: 400; color: var(--label-tertiary); }
.todo-hint { font: var(--type-caption); color: var(--label-tertiary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
.todo-action { font: var(--type-caption); color: var(--accent); white-space: nowrap; }
.todo-secondary { font: var(--type-caption); color: var(--label-tertiary); white-space: nowrap; }
.todo-secondary:hover { color: var(--accent); text-decoration: underline; }

.all-clear {
  display: flex; align-items: center; gap: var(--s-2);
  padding: var(--s-2) var(--s-3);
  border: 1px solid var(--separator);
  border-radius: var(--radius-control);
  background: var(--bg-content);
  color: var(--label-secondary);
  font: var(--type-body);
}

.section-head h3 { font: var(--type-title); color: var(--label-primary); margin: 0; }

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
