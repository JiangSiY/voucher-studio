<script setup lang="ts">
/**
 * 台账页（v13）：月度清单导入 → 补附件 → 等入账，一批可回看可撤销。
 *
 * 为什么要单独一页而不是塞进「待处理」：待处理页回答的是"现在该干什么"（一次性待办），
 * 台账页回答的是"这个月的清单走到哪一步了"——待补附件几十张、要按月回看批次，
 * 塞进待办清单会把那里的两条真待办（待核对、待打印）淹掉。
 *
 * 三段的关系是流水线而不是并列的筛选：清单导入后落在「待补附件」，
 * 挂上票据后自动进「待入账」，人工在金蝶录完账回来勾「已入账」才离开这一页。
 */
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import {
  api,
  type CheckReport,
  type ImportBatch,
  type KeptVoucher,
  type LedgerCounts,
  type LedgerItem,
  type LedgerStage,
} from "../api/client";
import ConfirmDialog from "../components/ConfirmDialog.vue";
import EmptyState from "../components/EmptyState.vue";
import FluentIcon from "../components/FluentIcon.vue";
import LedgerImportDialog from "../components/ledger/LedgerImportDialog.vue";
import { fmtAmount, fmtDate } from "../composables/format";
import { useLedgerStore } from "../stores/ledger";
import { toast, toastError } from "../stores/toast";

const router = useRouter();
/** 侧栏徽标与待处理页那张卡同源；这里动作完顺带刷新它 */
const ledger = useLedgerStore();

type Tab = LedgerStage | "batches" | "check";
const tab = ref<Tab>("unattached");

const counts = ref<LedgerCounts | null>(null);
const items = ref<LedgerItem[]>([]);
const batches = ref<ImportBatch[]>([]);
const loading = ref(false);
const busyId = ref<number | null>(null);

const KIND_LABEL: Record<string, string> = {
  shoukuan: "收款",
  fukuan: "付款",
  jiti: "计提",
};
function kindLabel(kind: string): string {
  return KIND_LABEL[kind] ?? (kind || "—");
}

const TABS: { value: Tab; label: string }[] = [
  { value: "unattached", label: "待补附件" },
  { value: "unbooked", label: "待入账" },
  { value: "batches", label: "批次" },
  { value: "check", label: "对账" },
];

// ---- M4 对账：台账导出 CSV ↔ 凭证库 ----

const checkPath = ref("");
const checkReport = ref<CheckReport | null>(null);
const checking = ref(false);
/** 网页端上传会把文件落到服务端临时目录并加时间戳前缀，展示时去掉 */
const checkFileName = computed(() => {
  const name = checkPath.value.split("/").pop() ?? "";
  return name.replace(/^\d{8}-\d{6}-\d+-/, "");
});

/** 容差（元）：默认 0，只允许完全相等；填 0.01 可放过一分钱尾差 */
const checkTolerance = ref("0");

const CHECK_KIND_LABEL: Record<string, string> = {
  missing_voucher: "缺凭证",
  missing_ref: "编号缺失",
  duplicate_ref: "编号重复",
  amount_mismatch: "金额不符",
  date_mismatch: "日期不符",
  counterparty_mismatch: "对方不符",
  orphan_voucher: "多余凭证",
};

function checkKindLabel(kind: string): string {
  return CHECK_KIND_LABEL[kind] ?? kind;
}

/** 容差输入（元）→ 分；空/非法按 0 处理 */
function toleranceCents(): number {
  const n = Number(checkTolerance.value);
  return Number.isFinite(n) && n > 0 ? Math.round(n * 100) : 0;
}

async function chooseCheckCsv() {
  try {
    const path = await api.pickLedgerCsv();
    if (!path) return;
    checkPath.value = path;
    checkReport.value = null;
  } catch (e) {
    toastError(e);
  }
}

async function runCheck() {
  if (!checkPath.value) return;
  checking.value = true;
  try {
    checkReport.value = await api.ledgerCheck(checkPath.value, toleranceCents());
  } catch (e) {
    toastError(e);
  } finally {
    checking.value = false;
  }
}

async function exportCheck() {
  if (!checkPath.value) return;
  try {
    const path = await api.ledgerCheckExport(checkPath.value, toleranceCents());
    toast.success("差异表已导出");
    await api.openPath(path);
  } catch (e) {
    toastError(e);
  }
}

function tabCount(t: Tab): number {
  if (!counts.value) return 0;
  if (t === "unattached") return counts.value.unattached;
  if (t === "unbooked") return counts.value.unbooked;
  if (t === "check") return 0;
  return counts.value.activeBatches;
}

async function loadCounts() {
  try {
    counts.value = await api.ledgerCounts();
  } catch (e) {
    toastError(e);
  }
}

async function loadItems() {
  if (tab.value === "batches" || tab.value === "check") return;
  loading.value = true;
  try {
    items.value = await api.listLedgerItems(tab.value);
  } catch (e) {
    toastError(e);
    items.value = [];
  } finally {
    loading.value = false;
  }
}

async function loadBatches() {
  if (tab.value !== "batches") return;
  loading.value = true;
  try {
    batches.value = await api.listImportBatches(100);
  } catch (e) {
    toastError(e);
    batches.value = [];
  } finally {
    loading.value = false;
  }
}

async function reload() {
  await Promise.all([loadCounts(), loadItems(), loadBatches()]);
  void ledger.refresh(); // 侧栏「台账」徽标与待处理页那张卡跟着一起变
}

async function switchTab(t: Tab) {
  if (tab.value === t) return;
  tab.value = t;
  items.value = [];
  batches.value = [];
  await reload();
}

// ---------- 导入向导 ----------

const wizardVisible = ref(false);

async function onImportDone() {
  await reload();
  // 导完落在「待补附件」，用户接着就是挂票据（2026-11-14 定稿的动线）
  if (counts.value && counts.value.unattached > 0) await switchTab("unattached");
}

// ---------- 待补附件：挂票据 ----------

async function attach(item: LedgerItem) {
  busyId.value = item.voucherId;
  try {
    const summary = await api.pickAndAttachImage(item.voucherId, true);
    if (!summary) return; // 用户取消
    if (summary.created === 0 && (summary.skipped ?? 0) > 0) {
      toast.warning("这张票据已存在或无法识别，未挂上");
    } else {
      toast.success(`已挂上票据：${item.ledgerRef}`);
    }
    await reload();
  } catch (e) {
    toastError(e);
  } finally {
    busyId.value = null;
  }
}

// ---------- 待入账：勾「已入账」 ----------

async function markBooked(item: LedgerItem) {
  busyId.value = item.voucherId;
  try {
    await api.setLedgerBooked(item.voucherId, true);
    toast.success(`${item.ledgerRef} 已标记入账`);
    await reload();
  } catch (e) {
    toastError(e);
  } finally {
    busyId.value = null;
  }
}

function openVoucher(id: number) {
  router.push({ path: "/vouchers", query: { focus: String(id) } });
}

// ---------- 批次：撤销 ----------

const undoTarget = ref<ImportBatch | null>(null);
const undoResult = ref<{ removed: number; kept: KeptVoucher[] } | null>(null);
const undoing = ref(false);

const undoBody = computed(() => {
  const b = undoTarget.value;
  if (!b) return "";
  return `将删除这批（${kindLabel(b.kind)} · ${b.fileName || "清单"}）新建的 ${b.created} 张草稿凭证。已经挂上票据、已导出或已归档的凭证会保留，并列出保留原因。批次记录本身保留为「已撤销」。`;
});

async function doUndo() {
  const b = undoTarget.value;
  if (!b) return;
  undoing.value = true;
  try {
    const r = await api.undoImportBatch(b.id);
    undoResult.value = { removed: r.removed, kept: r.kept };
    toast.success(`已撤销：删除 ${r.removed} 张，保留 ${r.kept.length} 张`);
    await reload();
  } catch (e) {
    toastError(e);
  } finally {
    undoing.value = false;
    undoTarget.value = null;
  }
}

onMounted(reload);
</script>

<template>
  <div class="ledger-page">
    <!-- 概览 + 主操作 -->
    <div class="fluent-panel ledger-summary">
      <div class="sum-stats">
        <button
          class="sum-item"
          :class="{ on: tab === 'unattached' }"
          @click="switchTab('unattached')"
        >
          <span class="sum-label">待补附件</span>
          <strong class="sum-value">{{ counts?.unattached ?? 0 }}</strong>
        </button>
        <button class="sum-item" :class="{ on: tab === 'unbooked' }" @click="switchTab('unbooked')">
          <span class="sum-label">待入账</span>
          <strong class="sum-value">{{ counts?.unbooked ?? 0 }}</strong>
        </button>
        <button class="sum-item" :class="{ on: tab === 'batches' }" @click="switchTab('batches')">
          <span class="sum-label">导入批次</span>
          <strong class="sum-value">{{ counts?.activeBatches ?? 0 }}</strong>
        </button>
      </div>
      <button class="btn btn-primary" @click="wizardVisible = true">
        <FluentIcon name="upload" :size="14" />
        导入清单
      </button>
    </div>

    <div class="ledger-tabs">
      <div class="segmented" role="tablist" aria-label="台账阶段">
        <button
          v-for="t in TABS"
          :key="t.value"
          class="seg-item"
          role="tab"
          :aria-selected="tab === t.value"
          :class="{ active: tab === t.value }"
          @click="switchTab(t.value)"
        >
          {{ t.label }}
          <template v-if="tabCount(t.value)"> {{ tabCount(t.value) }}</template>
        </button>
      </div>
    </div>

    <!-- ===== 待补附件 / 待入账 ===== -->
    <div v-if="tab !== 'batches' && tab !== 'check'" class="fluent-panel">
      <div class="fluent-panel-body" style="padding: 0">
        <div v-if="items.length" class="dtable-wrap">
          <table class="dtable">
            <thead>
              <tr>
                <th>日期</th>
                <th>凭证号</th>
                <th>流水号</th>
                <th>类型</th>
                <th class="num">金额</th>
                <th>对方</th>
                <th>摘要</th>
                <th>操作</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="item in items" :key="item.voucherId">
                <td>{{ fmtDate(item.voucherDate) }}</td>
                <td class="mono">{{ item.voucherNo }}</td>
                <td class="mono">{{ item.ledgerRef }}</td>
                <td>{{ kindLabel(item.ledgerKind) }}</td>
                <td class="num">{{ fmtAmount(item.amountCents) }}</td>
                <td>{{ item.counterparty || "—" }}</td>
                <td class="desc">{{ item.description || "—" }}</td>
                <td class="ops">
                  <template v-if="tab === 'unattached'">
                    <button
                      class="btn btn-sm"
                      :disabled="busyId === item.voucherId"
                      @click="attach(item)"
                    >
                      挂附件
                    </button>
                  </template>
                  <template v-else>
                    <button
                      class="btn btn-sm"
                      :disabled="busyId === item.voucherId"
                      @click="markBooked(item)"
                    >
                      已入账
                    </button>
                  </template>
                  <button class="btn btn-sm" @click="openVoucher(item.voucherId)">查看</button>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <EmptyState
          v-else-if="!loading"
          :icon="tab === 'unattached' ? 'image' : 'checkCircle'"
          :title="tab === 'unattached' ? '没有待补附件的凭证' : '没有待入账的凭证'"
          :desc="
            tab === 'unattached'
              ? '导入清单后，每张凭证都等着挂上对应的票据扫描件。'
              : '待补附件的凭证挂上票据后，会出现在这里等金蝶入账。'
          "
        >
          <button class="btn btn-primary" @click="wizardVisible = true">导入清单</button>
        </EmptyState>
      </div>
    </div>

    <!-- ===== 对账（M4） ===== -->
    <template v-else-if="tab === 'check'">
      <div class="fluent-panel">
        <div class="fluent-panel-body check-bar">
          <button class="btn" :disabled="checking" @click="chooseCheckCsv()">
            <FluentIcon name="upload" :size="14" />
            选择台账 CSV
          </button>
          <span class="check-file" :title="checkPath">
            {{ checkFileName || "还没选文件（台账导出脚本产出的收款/付款/计提清单）" }}
          </span>
          <label class="check-tolerance">
            容差
            <input v-model="checkTolerance" class="input" inputmode="decimal" />
            元
          </label>
          <button class="btn btn-primary" :disabled="checking || !checkPath" @click="runCheck()">
            {{ checking ? "比对中…" : "开始比对" }}
          </button>
        </div>
      </div>

      <template v-if="checkReport">
        <div class="check-stats">
          <div class="check-stat">
            <span class="sum-label">台账行数</span>
            <strong class="sum-value">{{ checkReport.rows }}</strong>
          </div>
          <div class="check-stat ok">
            <span class="sum-label">完全一致</span>
            <strong class="sum-value">{{ checkReport.matched }}</strong>
          </div>
          <div class="check-stat" :class="{ bad: checkReport.missing }">
            <span class="sum-label">缺凭证</span>
            <strong class="sum-value">{{ checkReport.missing }}</strong>
          </div>
          <div class="check-stat" :class="{ bad: checkReport.mismatched }">
            <span class="sum-label">字段不符</span>
            <strong class="sum-value">{{ checkReport.mismatched }}</strong>
          </div>
          <div class="check-stat" :class="{ bad: checkReport.orphans }">
            <span class="sum-label">多余凭证</span>
            <strong class="sum-value">{{ checkReport.orphans }}</strong>
          </div>
          <div class="check-stat" :class="{ bad: checkReport.malformed }">
            <span class="sum-label">编号异常</span>
            <strong class="sum-value">{{ checkReport.malformed }}</strong>
          </div>
        </div>

        <div class="fluent-panel">
          <div class="fluent-panel-body" style="padding: 0">
            <div v-if="checkReport.issues.length" class="dtable-wrap">
              <table class="dtable">
                <thead>
                  <tr>
                    <th>问题</th>
                    <th>台账编号</th>
                    <th>凭证号</th>
                    <th>台账日期</th>
                    <th>凭证日期</th>
                    <th class="num">台账金额</th>
                    <th class="num">凭证金额</th>
                    <th>说明</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-for="(i, idx) in checkReport.issues" :key="idx">
                    <td>
                      <span
                        class="tag"
                        :class="i.kind === 'orphan_voucher' ? 'tag-muted' : 'tag-danger'"
                      >
                        {{ checkKindLabel(i.kind) }}
                      </span>
                    </td>
                    <td class="mono">{{ i.ledgerRef || "—" }}</td>
                    <td class="mono">{{ i.voucherNo || "—" }}</td>
                    <td>{{ i.ledgerDate ? fmtDate(i.ledgerDate) : "—" }}</td>
                    <td>{{ i.voucherDate ? fmtDate(i.voucherDate) : "—" }}</td>
                    <td class="num">{{ fmtAmount(i.ledgerAmountCents) }}</td>
                    <td class="num">{{ fmtAmount(i.voucherAmountCents) }}</td>
                    <td class="desc" :title="i.message">{{ i.message }}</td>
                  </tr>
                </tbody>
              </table>
            </div>
            <EmptyState
              v-else
              icon="checkCircle"
              title="两边完全对得上"
              :desc="`${checkReport.rows} 行台账记录都有对应凭证，金额、日期、对方一致。`"
            />
          </div>
        </div>

        <div class="check-actions">
          <span class="muted-hint">只读比对，不会改动凭证或台账；差异表可导出给台账那边核对。</span>
          <button class="btn" :disabled="!checkReport.issues.length" @click="exportCheck()">
            导出差异 CSV
          </button>
        </div>
      </template>
    </template>

    <!-- ===== 批次 ===== -->
    <div v-else class="fluent-panel">
      <div class="fluent-panel-body" style="padding: 0">
        <div v-if="batches.length" class="dtable-wrap">
          <table class="dtable">
            <thead>
              <tr>
                <th>导入时间</th>
                <th>类型</th>
                <th>文件名</th>
                <th class="num">新建</th>
                <th class="num">已存在</th>
                <th class="num">坏行</th>
                <th>状态</th>
                <th>操作</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="b in batches" :key="b.id" :class="{ undone: b.undoneAt }">
                <td>{{ fmtDate(b.createdAt) }}</td>
                <td>{{ kindLabel(b.kind) }}</td>
                <td class="desc" :title="b.filePath">{{ b.fileName || "—" }}</td>
                <td class="num">{{ b.created }}</td>
                <td class="num">{{ b.matched }}</td>
                <td class="num" :class="{ 'wz-bad': b.errorRows }">{{ b.errorRows }}</td>
                <td>
                  <span v-if="b.undoneAt" class="tag tag-muted">已撤销</span>
                  <span v-else class="tag tag-success">有效</span>
                </td>
                <td class="ops">
                  <button
                    v-if="!b.undoneAt && b.created > 0"
                    class="btn btn-sm"
                    :disabled="undoing"
                    @click="undoTarget = b"
                  >
                    撤销
                  </button>
                  <span v-else class="muted">—</span>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <EmptyState
          v-else-if="!loading"
          icon="upload"
          title="还没有导入过台账清单"
          desc="从台账导出的收款 / 付款 / 计提清单 CSV 可以从这里一次导进来。"
        >
          <button class="btn btn-primary" @click="wizardVisible = true">导入清单</button>
        </EmptyState>
      </div>
    </div>

    <LedgerImportDialog
      :visible="wizardVisible"
      @close="wizardVisible = false"
      @done="onImportDone"
    />

    <ConfirmDialog
      :visible="!!undoTarget"
      title="撤销这批导入？"
      :body="undoBody"
      confirm-text="撤销这批"
      :danger="true"
      @cancel="undoTarget = null"
      @confirm="doUndo"
    />

    <!-- 撤销结果：保留下来的凭证与原因（用户要知道"为什么没删干净"） -->
    <Teleport to="body">
      <div v-if="undoResult" class="dialog-mask" @click.self="undoResult = null">
        <div class="dialog" role="dialog" aria-modal="true" aria-label="撤销结果">
          <h3>撤销结果</h3>
          <div class="dialog-body">
            <p>
              已删除 {{ undoResult.removed }} 张草稿凭证；保留 {{ undoResult.kept.length }} 张。
            </p>
            <ul v-if="undoResult.kept.length" class="kept-list">
              <li v-for="k in undoResult.kept" :key="k.voucherNo">
                <span class="mono">{{ k.voucherNo }}</span>
                · {{ k.ledgerRef }} · {{ k.reason }}
              </li>
            </ul>
          </div>
          <div class="dialog-ops">
            <button class="btn btn-primary" @click="undoResult = null">知道了</button>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.ledger-summary {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--s-4);
  padding: var(--s-4) var(--s-5);
}

.sum-stats {
  display: flex;
  gap: var(--s-2);
}

.sum-item {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
  min-width: 96px;
  padding: var(--s-2) var(--s-3);
  border: 1px solid transparent;
  border-radius: var(--radius-small);
  background: transparent;
  cursor: default;
  text-align: left;
}

.sum-item:hover {
  background: var(--bg-selection);
}

.sum-item.on {
  background: var(--bg-card-sunken);
  border-color: var(--card-hairline);
}

.sum-label {
  font-size: var(--fs-caption);
  color: var(--label-secondary);
}

.sum-value {
  font: var(--type-metric);
  font-variant-numeric: tabular-nums;
}

.ledger-tabs {
  margin-bottom: var(--s-3);
}

.dtable td.num,
.dtable th.num {
  text-align: right;
}

.dtable .mono {
  font-family: var(--font-mono, ui-monospace, monospace);
  font-size: var(--fs-caption);
  white-space: nowrap;
}

.dtable .desc {
  max-width: 260px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.dtable .ops {
  display: flex;
  gap: var(--s-2);
}

.dtable .muted {
  color: var(--label-tertiary);
}

.dtable tr.undone td {
  color: var(--label-tertiary);
}

.wz-bad {
  color: var(--danger);
}

.check-bar {
  display: flex;
  align-items: center;
  gap: var(--s-3);
  flex-wrap: wrap;
}

.check-file {
  flex: 1;
  min-width: 220px;
  font-size: var(--fs-caption);
  color: var(--label-secondary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.check-tolerance {
  display: inline-flex;
  align-items: center;
  gap: var(--s-1);
  font-size: var(--fs-caption);
  color: var(--label-secondary);
}

.check-tolerance .input {
  width: 72px;
  text-align: right;
}

.check-stats {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(120px, 1fr));
  gap: var(--s-3);
  margin-bottom: var(--s-3);
}

.check-stat {
  display: flex;
  flex-direction: column;
  gap: var(--s-1);
  padding: var(--s-3) var(--s-4);
  border-radius: var(--radius-card);
  background: var(--bg-card);
  border: 1px solid var(--card-hairline);
}

.check-stat.ok .sum-value {
  color: var(--success);
}

.check-stat.bad .sum-value {
  color: var(--danger);
}

.check-actions {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--s-3);
  margin-top: var(--s-3);
}

.muted-hint {
  font-size: var(--fs-caption);
  color: var(--label-tertiary);
}

.kept-list {
  max-height: 200px;
  overflow: auto;
  margin: var(--s-2) 0 0;
  padding-left: var(--s-4);
  font-size: var(--fs-caption);
  color: var(--label-secondary);
}

/* 窄屏（<768）：概览卡改上下两行，按钮独占一行——否则「导入清单」被挤成一条蓝色细边
   （2026-11-18 真机走查踩到） */
@media (max-width: 767px) {
  .ledger-summary {
    flex-direction: column;
    align-items: stretch;
    gap: var(--s-3);
    padding: var(--s-3);
  }
  .sum-stats {
    justify-content: space-between;
  }
  .ledger-summary > .btn {
    width: 100%;
  }
}
</style>
