<script setup lang="ts">
/**
 * 工作台（原「仪表盘」）—— v0.18 设计样张。
 *
 * 三件事与旧版不同（旧版的三处硬伤，见 `docs/UI-设计规范-macOS27.md` §3）：
 * 1. **统一口径**：顶部口径控制器（时间窗 + 状态）是**唯一真源**，KPI 与趋势/排行共用它；
 *    旧版统计卡走全库、状态分布只算 6 个月窗口、Top 是全历史前 10，且三处都不标注口径。
 * 2. **待办行动区置顶**：把库里已采但此前没有出口的数据（待识别、未填金额、陈旧草稿、
 *    未封账往月、备份新鲜度、疑似重复）变成**可点即达**的卡片，而不是只读数字。
 * 3. **口径标注**：每块标题右侧注明"窗口内 / 全库"，跨块差异一眼可见。
 *
 * 与后端的约定：`dashboard_overview` 的 `byStatus` 始终是**窗口内完整构成**（不随状态筛选收窄）；
 * 环比用**上一等长天数**窗口（不足时后端不给，前端不编造）。
 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useRouter } from "vue-router";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { safeListen } from "../composables/safeListen";
import {
  api,
  type BackupInfo,
  type DashboardOverview,
  type DashboardTodos,
  type ExportBatch,
  type ReconcileReport,
  type Voucher,
  type VoucherStats,
  type VoucherStatus,
} from "../api/client";
import FluentIcon from "../components/FluentIcon.vue";
import TrendPanel from "../components/dashboard/TrendPanel.vue";
import TopListsPanel from "../components/dashboard/TopListsPanel.vue";
import ReconcilePanel from "../components/dashboard/ReconcilePanel.vue";
import {
  RANGE_PRESETS,
  rangeLabel,
  rangeOfPreset,
  type DateRange,
  type RangePreset,
} from "../composables/dateRange";
import { STATUS_META, fmtDate } from "../composables/format";
import { useConfigStore } from "../stores/config";
import { useDupStore } from "../stores/dup";
import { toast, toastError } from "../stores/toast";

const router = useRouter();
const configStore = useConfigStore();
const dupStore = useDupStore();

const today = new Date();

// ---------- ① 口径控制器（唯一真源） ----------

const preset = ref<RangePreset>("thisMonth");
const range = ref<DateRange>(rangeOfPreset("thisMonth", today) as DateRange);
const status = ref<VoucherStatus | "all">("all");

const statusLabel = computed(() =>
  status.value === "all" ? "全部状态" : STATUS_META[status.value].label,
);
/** 口径说明（块标题旁标注用） */
const scopeLabel = computed(() => rangeLabel(range.value, statusLabel.value));

async function applyPreset(p: RangePreset) {
  preset.value = p;
  const r = rangeOfPreset(p, today);
  if (r) {
    range.value = r;
    await loadOverview();
  }
}

async function applyCustom(from: string, to: string) {
  if (!from || !to) return;
  preset.value = "custom";
  range.value = { dateFrom: from, dateTo: to };
  await loadOverview();
}

// ---------- ② 数据 ----------

const overview = ref<DashboardOverview | null>(null);
const todos = ref<DashboardTodos | null>(null);
const stats = ref<VoucherStats | null>(null);
const recent = ref<Voucher[]>([]);
const batches = ref<ExportBatch[]>([]);
const backups = ref<BackupInfo[]>([]);
const reconcileReport = ref<ReconcileReport | null>(null);
const reconcileBusy = ref(false);

async function loadOverview() {
  try {
    overview.value = await api.dashboardOverview(
      range.value.dateFrom,
      range.value.dateTo,
      status.value === "all" ? null : status.value,
    );
  } catch (e) {
    toastError(e);
  }
}

/** 待办与健康度：数据来自 `dashboard_todos`（轻量计数；库完整性走启动健康检查，不在此重算） */
async function loadTodos() {
  try {
    todos.value = await api.dashboardTodos();
    void dupStore.refresh(); // 侧栏徽标与待办卡同源
  } catch {
    // 待办是辅助信息，失败不打断工作台其余部分
    todos.value = null;
  }
}

/** 全量刷新：**并行**（旧版是 4 个串行 await，首屏被拉长） */
async function loadAll() {
  await Promise.all([loadOverview(), loadTodos(), loadRecentAndStats(), loadBatches()]);
}

async function loadRecentAndStats() {
  try {
    const [s, list, b] = await Promise.all([
      api.voucherStats(),
      api.listVouchers({ page: 1, pageSize: 6 }),
      api.listBackups(),
    ]);
    stats.value = s;
    recent.value = list.items;
    backups.value = b;
  } catch (e) {
    toastError(e);
  }
}

async function loadBatches() {
  try {
    batches.value = (await api.listExportBatches(5)).slice(0, 5);
  } catch {
    batches.value = [];
  }
}

async function backupNow() {
  try {
    const info = await api.runBackup();
    if (info.warnings?.length) {
      toast.warning(`备份完成：${info.fileName}，但副本存在问题：${info.warnings.join("；")}`);
    } else {
      toast.success(`备份完成：${info.fileName}`);
    }
    backups.value = await api.listBackups();
    await loadTodos();
  } catch (e) {
    toastError(e);
  }
}

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

// ---------- ③ 联动（下钻） ----------

/** 跳到凭证列表并带上筛选；列表页会回显"来自工作台"并可一键清除 */
function gotoVouchers(query: Record<string, string>) {
  router.push({ path: "/vouchers", query });
}

/** 环比：只在两个窗口都有数据时显示（不足两个窗口或上期为 0 → 不编造） */
const mom = computed<{ pct: number; up: boolean } | null>(() => {
  const o = overview.value;
  if (!o || o.prevAmountCents == null || o.prevCount == null) return null;
  if (o.prevCount === 0 || o.prevAmountCents === 0) return null;
  const pct = ((o.amountCents - o.prevAmountCents) / Math.abs(o.prevAmountCents)) * 100;
  return { pct, up: pct >= 0 };
});

/** 待办卡（0 值不显示；每张都可点即达） */
const todoCards = computed(() => {
  const t = todos.value;
  if (!t) return [];
  const cards: {
    key: string;
    icon: string;
    count: number;
    label: string;
    hint: string;
    run: () => void;
  }[] = [
    {
      key: "dup",
      icon: "duplicate",
      count: t.dupPairs,
      label: "疑似重复待核查",
      hint: t.dupHighPairs > 0 ? `其中 ${t.dupHighPairs} 对证据较强` : "同日期同金额",
      run: () => router.push("/duplicates"),
    },
    {
      key: "ocr",
      icon: "sparkle",
      count: t.ocrPending,
      label: "待识别附件",
      hint: "识别后自动填单",
      run: () => router.push("/settings?tab=ocr"),
    },
    {
      key: "amount",
      icon: "edit",
      count: t.amountMissing,
      label: "未填金额",
      hint: "金额为空的凭证",
      run: () => gotoVouchers({ status: "draft" }),
    },
    {
      key: "stale",
      icon: "info",
      count: t.staleDrafts,
      label: "陈旧草稿",
      hint: `${t.staleDraftDays} 天未更新`,
      run: () => gotoVouchers({ status: "draft" }),
    },
    {
      key: "unlock",
      icon: "lock",
      count: t.unlockedMonths,
      label: "未封账往月",
      hint: "有凭证但未锁定期间",
      run: () => router.push("/settings?tab=naming"),
    },
    {
      key: "backup",
      icon: "save",
      count: t.backupStale ? 1 : 0,
      label: "备份陈旧",
      hint: t.latestBackup ? `最近 ${fmtDate(t.latestBackup.modifiedAt)}` : "尚无备份",
      run: () => router.push("/restore"),
    },
  ];
  return cards.filter((c) => c.count > 0);
});

/** 备份提示（待办已过 24h 阈值时高亮；用最新的备份文件时间，不读 manifest） */
const backupHint = computed(() => {
  const b = todos.value?.latestBackup;
  if (!b) return "尚无备份，建议立即做一次";
  return `最近备份：${b.fileName}（${fmtDate(b.modifiedAt)}）`;
});

function statusOf(v: Voucher) {
  return STATUS_META[v.status];
}

// 拖放导入在任意页面全局生效；自动备份与疑似重复处置在别处完成后也需刷新
let unlistenImportDone: UnlistenFn | null = null;
let unlistenBackupDone: UnlistenFn | null = null;
let unlistenDupChanged: UnlistenFn | null = null;

onMounted(async () => {
  await loadAll();
  unlistenImportDone = await safeListen("import:done", () => loadAll());
  unlistenBackupDone = await safeListen("backup:done", () => loadAll());
  unlistenDupChanged = await safeListen("dup:changed", () => void loadTodos());
});

onUnmounted(() => {
  unlistenImportDone?.();
  unlistenBackupDone?.();
  unlistenDupChanged?.();
});
</script>

<template>
  <div class="workbench">
    <!-- ① 口径控制器：全屏唯一真源（时间窗 + 状态），所有区块共用 -->
    <div class="range-bar">
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
      <select
        class="range-status"
        :value="status"
        @change="applyCustom(range.dateFrom, range.dateTo); status = ($event.target as HTMLSelectElement).value as VoucherStatus | 'all'; loadOverview()"
      >
        <option value="all">全部状态</option>
        <option v-for="(meta, key) in STATUS_META" :key="key" :value="key">{{ meta.label }}</option>
      </select>
      <span class="scope">{{ scopeLabel }}（下方 KPI 与趋势共用此口径）</span>
      <span class="flex-1"></span>
      <button class="btn" title="立即备份" @click="backupNow">
        <FluentIcon name="save" :size="14" /> 备份
      </button>
      <button class="btn" :disabled="reconcileBusy" title="账单对账" @click="runReconcile">
        <FluentIcon name="chart" :size="14" /> 对账
      </button>
      <button class="btn btn-primary" @click="router.push('/vouchers/new')">
        <FluentIcon name="plus" :size="14" /> 新建
      </button>
    </div>

    <!-- ② KPI（口径 = 上方控制器；全库口径的项单独标注） -->
    <div class="kpi-grid">
      <button class="kpi" @click="gotoVouchers({ dateFrom: range.dateFrom, dateTo: range.dateTo })">
        <span class="kpi-label">窗口凭证</span>
        <span class="kpi-value">{{ overview?.count ?? "—" }}</span>
        <span class="kpi-foot">窗口内</span>
      </button>
      <button
        class="kpi"
        @click="gotoVouchers({ dateFrom: range.dateFrom, dateTo: range.dateTo })"
      >
        <span class="kpi-label">窗口金额</span>
        <span class="kpi-value">¥{{ ((overview?.amountCents ?? 0) / 100).toLocaleString("zh-CN", { minimumFractionDigits: 2, maximumFractionDigits: 2 }) }}</span>
        <span class="kpi-foot" :class="{ up: mom?.up, down: mom && !mom.up }">
          <template v-if="mom">环比 {{ mom.up ? "+" : "" }}{{ mom.pct.toFixed(1) }}%</template>
          <template v-else>环比：上一窗口不足两个 · 不显示</template>
        </span>
      </button>
      <button class="kpi" @click="gotoVouchers({})">
        <span class="kpi-label">全部凭证</span>
        <span class="kpi-value">{{ overview?.total ?? stats?.total ?? "—" }}</span>
        <span class="kpi-foot">全库</span>
      </button>
      <button class="kpi" @click="gotoVouchers({ status: 'draft' })">
        <span class="kpi-label">草稿</span>
        <span class="kpi-value">{{ stats?.draft ?? "—" }}</span>
        <span class="kpi-foot">全库</span>
      </button>
      <div class="kpi">
        <span class="kpi-label">附件</span>
        <span class="kpi-value">{{ overview?.attachments ?? stats?.attachments ?? "—" }}</span>
        <span class="kpi-foot">全库</span>
      </div>
    </div>

    <!-- ③ 待办与健康度（行动区：数值为 0 的项不显示） -->
    <section class="todos">
      <header class="section-head">
        <h2>待办与健康度</h2>
        <span class="scope">{{ todoCards.length ? `${todoCards.length} 项待处理` : "暂无待办" }}</span>
        <span class="flex-1"></span>
        <span class="backup-hint" :class="{ stale: todos?.backupStale }">{{ backupHint }}</span>
      </header>
      <div v-if="todoCards.length" class="todo-row">
        <button v-for="c in todoCards" :key="c.key" class="todo" @click="c.run()">
          <FluentIcon :name="c.icon" :size="15" />
          <span class="todo-count">{{ c.count }}</span>
          <span class="todo-text">
            <b>{{ c.label }}</b>
            <small>{{ c.hint }}</small>
          </span>
          <FluentIcon name="chevronRight" :size="13" />
        </button>
      </div>
      <p v-else class="empty-hint">
        没有待办：待识别、未填金额、陈旧草稿、未封账往月、备份陈旧与疑似重复都已处理完。
      </p>
    </section>

    <!-- ④ 趋势与排行（本样张沿用既有面板；批次 7 把它们接到同一口径控制器上） -->
    <section class="panel-row">
      <header class="section-head">
        <h2>趋势与构成</h2>
        <span class="scope">面板自带窗口（批次 7 接入上方口径）</span>
      </header>
      <TrendPanel />
    </section>

    <section class="panel-row">
      <header class="section-head">
        <h2>对方账户与标签</h2>
        <span class="scope">全历史 · 前 10</span>
      </header>
      <TopListsPanel />
    </section>

    <!-- ⑤ 最近活动 -->
    <div class="grid-activity">
      <section class="panel">
        <header class="section-head">
          <h2>最近凭证</h2>
          <span class="flex-1"></span>
          <button class="link" @click="gotoVouchers({})">查看全部</button>
        </header>
        <table v-if="recent.length" class="dtable">
          <tbody>
            <tr v-for="v in recent" :key="v.id" style="cursor: pointer" @click="router.push(`/vouchers/${v.id}`)">
              <td><span class="mono">{{ v.voucherNo }}</span></td>
              <td>{{ v.voucherDate }}</td>
              <td class="cell-desc">{{ v.description || "—" }}</td>
              <td class="num">{{ v.amountCents == null ? "—" : `¥${(v.amountCents / 100).toFixed(2)}` }}</td>
              <td><span class="tag" :class="statusOf(v).tag">{{ statusOf(v).label }}</span></td>
            </tr>
          </tbody>
        </table>
        <p v-else class="empty-hint">还没有凭证：把图片拖进窗口，或在「凭证列表」点「导入图片」。</p>
      </section>

      <section class="panel">
        <header class="section-head">
          <h2>最近导出</h2>
          <span class="flex-1"></span>
          <button class="link" @click="router.push('/exports')">导出历史</button>
        </header>
        <ul v-if="batches.length" class="activity-list">
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

    <!-- 对账结果（会话内保留；报告可打开） -->
    <ReconcilePanel v-if="reconcileReport" :report="reconcileReport" @close="reconcileReport = null" />

    <p class="footnote">
      数据全部保存在本机（{{ configStore.appInfo?.dataDir }}）。待办项的精确下钻筛选（未填金额 /
      待识别 / 从未导出）随批次 5 的查询扩维接入。
    </p>
  </div>
</template>

<style scoped>
/* 样张样式：只使用 `docs/UI-设计规范-macOS27.md` §1 的新语义令牌 */
.workbench { display: flex; flex-direction: column; gap: var(--s-4); }

.range-bar {
  display: flex; align-items: center; gap: var(--s-2);
  padding: var(--s-2) var(--s-3);
  border: 1px solid var(--separator);
  border-radius: var(--radius-panel);
  background: var(--bg-content);
}
.range-presets { display: inline-flex; border: 1px solid var(--separator); border-radius: var(--radius-control); overflow: hidden; }
.seg {
  height: var(--control-h-sm);
  padding: 0 var(--s-3);
  border: none;
  background: transparent;
  color: var(--label-secondary);
  font: var(--type-caption);
  cursor: default;
}
.seg + .seg { border-left: 1px solid var(--separator); }
.seg.active { background: var(--bg-selection); color: var(--label-primary); font-weight: 600; }
.range-status {
  height: var(--control-h-md);
  padding: 0 var(--s-2);
  border: 1px solid var(--separator);
  border-radius: var(--radius-control);
  background: var(--bg-control);
  color: var(--label-primary);
  font: var(--type-body);
}
.scope { color: var(--label-tertiary); font: var(--type-caption); }
.flex-1 { flex: 1; }

/* KPI 自适应列数：字号调大后 5 列会把金额挤到溢出——宁可换行也不要截断数字 */
.kpi-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: var(--s-3); }
.kpi {
  display: flex; flex-direction: column; gap: 2px;
  padding: var(--s-3);
  border: 1px solid var(--separator);
  border-radius: var(--radius-panel);
  background: var(--bg-content);
  text-align: left;
  cursor: default;
}
button.kpi:hover { background: var(--bg-selection); }
.kpi-label { color: var(--label-secondary); font: var(--type-caption); }
.kpi-value {
  color: var(--label-primary);
  font-size: 26px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  line-height: 1.25;
}
.kpi-foot { color: var(--label-tertiary); font: var(--type-caption); }
.kpi-foot.up { color: var(--success); }
.kpi-foot.down { color: var(--danger); }

.section-head { display: flex; align-items: baseline; gap: var(--s-2); }
.section-head h2 { font: var(--type-title); color: var(--label-primary); margin: 0; }
.backup-hint { color: var(--label-tertiary); font: var(--type-caption); }
.backup-hint.stale { color: var(--warning); }

.todos { display: flex; flex-direction: column; gap: var(--s-2); }
.todo-row { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: var(--s-2); }
.todo {
  display: flex; align-items: center; gap: var(--s-2);
  padding: var(--s-2) var(--s-3);
  border: 1px solid var(--separator);
  border-radius: var(--radius-panel);
  background: var(--bg-content);
  color: var(--label-secondary);
  text-align: left;
  cursor: default;
}
.todo:hover { background: var(--bg-selection); }
.todo-count { font-size: 17px; font-weight: 600; color: var(--label-primary); }
.todo-text { display: flex; flex-direction: column; flex: 1; min-width: 0; }
.todo-text b { font: var(--type-body); font-weight: 500; color: var(--label-primary); }
.todo-text small { font: var(--type-caption); color: var(--label-tertiary); }

/* 面板整行排布：趋势与排行各占一行（宽度充足 → 内部再分栏也不会溢出） */
.panel-row { display: flex; flex-direction: column; gap: var(--s-2); min-width: 0; }
.grid-activity { display: grid; grid-template-columns: minmax(0, 2fr) minmax(0, 1fr); gap: var(--s-4); }
.panel {
  padding: var(--s-3);
  border: 1px solid var(--separator);
  border-radius: var(--radius-panel);
  background: var(--bg-content);
}
.activity-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: var(--s-1); }
.activity-list li { display: flex; align-items: center; gap: var(--s-2); font: var(--type-body); }
.muted { color: var(--label-tertiary); font: var(--type-caption); }
.link { border: none; background: none; color: var(--accent); font: var(--type-caption); cursor: default; }
.link:hover { text-decoration: underline; }
.empty-hint { color: var(--label-tertiary); font: var(--type-body); margin: var(--s-2) 0; }
.cell-desc { max-width: 320px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.footnote { color: var(--label-tertiary); font: var(--type-caption); margin: 0; }

@media (max-width: 1100px) {
  .grid-activity { grid-template-columns: minmax(0, 1fr); }
}
</style>
