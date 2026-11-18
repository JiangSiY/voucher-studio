<script setup lang="ts">
/**
 * 对账页（v0.27）：项目台账 CSV 与条目库比对。
 *
 * 只读：`ledger_check` 产出六类差异，`ledger_check_export` 落盘差异表；
 * 两边数据都不改。本页是对账的唯一入口，工作台不再放「台账比对」行。
 */
import { computed, ref } from "vue";
import FluentIcon from "../components/FluentIcon.vue";
import type { CheckReport } from "../api/client";
import { api } from "../api/client";
import { toast, toastError } from "../stores/toast";

const KIND_LABEL: Record<string, string> = {
  missing_voucher: "缺凭证",
  missing_ref: "编号缺失",
  duplicate_ref: "编号重复",
  amount_mismatch: "金额不符",
  date_mismatch: "日期不符",
  counterparty_mismatch: "对方不符",
  orphan_voucher: "多余凭证",
};

const csvPath = ref("");
const report = ref<CheckReport | null>(null);
const checking = ref(false);
const exporting = ref(false);
const tolerance = ref("0");

const fileName = computed(() => csvPath.value.split("/").pop() ?? "");

function toleranceCents(): number {
  const n = Number(tolerance.value);
  return Number.isFinite(n) && n > 0 ? Math.round(n * 100) : 0;
}

function fmtMoney(cents: number | null): string {
  if (cents == null) return "—";
  return `¥${(cents / 100).toLocaleString("zh-CN", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  })}`;
}

async function pick() {
  try {
    const path = await api.pickLedgerCsv();
    if (!path) return;
    csvPath.value = path;
    report.value = null;
  } catch (e) {
    toastError(e);
  }
}

async function run() {
  if (!csvPath.value) return;
  checking.value = true;
  try {
    report.value = await api.ledgerCheck(csvPath.value, toleranceCents());
    const r = report.value;
    if (r.issues.length) toast.info(`比对完成：${r.issues.length} 条差异。`);
    else toast.success("两边完全对得上。");
  } catch (e) {
    toastError(e);
  } finally {
    checking.value = false;
  }
}

async function exportDiff() {
  if (!csvPath.value) return;
  exporting.value = true;
  try {
    const path = await api.ledgerCheckExport(csvPath.value, toleranceCents());
    await api.openPath(path);
    toast.success("差异表已导出");
  } catch (e) {
    toastError(e);
  } finally {
    exporting.value = false;
  }
}

const stats = computed(() => {
  const r = report.value;
  if (!r) return [];
  return [
    { label: "台账行", value: r.rows, tone: "" },
    { label: "完全一致", value: r.matched, tone: "ok" },
    { label: "缺凭证", value: r.missing, tone: r.missing ? "bad" : "" },
    { label: "字段不符", value: r.mismatched, tone: r.mismatched ? "bad" : "" },
    { label: "多余凭证", value: r.orphans, tone: r.orphans ? "bad" : "" },
    { label: "编号异常", value: r.malformed, tone: r.malformed ? "bad" : "" },
  ];
});
</script>

<template>
  <div class="page">
    <div class="page-head">
      <div>
        <h1 class="page-title">台账比对</h1>
        <p class="page-sub">把项目台账 CSV 与条目库对齐，只列差异，不改任何一边的数据。</p>
      </div>
      <div class="page-actions">
        <button class="btn" type="button" :disabled="checking" @click="pick">
          <FluentIcon name="folder" :size="14" />
          <span class="rec-file">{{ fileName || "选择台账 CSV" }}</span>
        </button>
        <button class="btn btn-primary" type="button" :disabled="checking || !csvPath" @click="run">
          <FluentIcon name="compare" :size="14" />
          <span>{{ checking ? "比对中…" : "重新比对" }}</span>
        </button>
      </div>
    </div>

    <div class="rec-bar">
      <span class="rec-path" :title="csvPath">
        {{ fileName || "还没选文件（台账导出脚本产出的收款 / 付款 / 计提清单）" }}
      </span>
      <label class="rec-tolerance">
        容差
        <input v-model="tolerance" class="input" inputmode="decimal" aria-label="金额容差（元）" />
        元
      </label>
    </div>

    <template v-if="report">
      <section class="rec-stats" aria-label="比对摘要">
        <div v-for="s in stats" :key="s.label" class="rec-stat" :class="s.tone">
          <span class="rec-stat-label">{{ s.label }}</span>
          <strong class="rec-stat-value">{{ s.value }}</strong>
        </div>
      </section>

      <section class="data-panel">
        <div class="data-toolbar">
          <div class="data-chips">
            <span class="chip is-active">差异 {{ report.issues.length }}</span>
            <span class="chip">容差 {{ (report.toleranceCents / 100).toFixed(2) }} 元</span>
            <span class="chip">{{ report.checkedAt.replace("T", " ").slice(0, 16) }}</span>
          </div>
          <div class="data-actions">
            <button class="btn" type="button" :disabled="exporting" @click="exportDiff">
              <FluentIcon name="download" :size="14" />
              <span>{{ exporting ? "导出中…" : "导出差异" }}</span>
            </button>
          </div>
        </div>
        <div v-if="report.issues.length" class="rec-table-wrap">
          <table class="data-table rec-table">
            <thead>
              <tr>
                <th scope="col">差异类型</th>
                <th scope="col">台账编号</th>
                <th scope="col">凭证号</th>
                <th scope="col">台账日期</th>
                <th scope="col">凭证日期</th>
                <th scope="col" class="data-amount">台账金额</th>
                <th scope="col" class="data-amount">凭证金额</th>
                <th scope="col">说明</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="(i, idx) in report.issues" :key="idx" class="data-row">
                <td>
                  <span
                    class="data-status"
                    :data-status="i.kind === 'orphan_voucher' ? 'archived' : 'draft'"
                  >
                    {{ KIND_LABEL[i.kind] ?? i.kind }}
                  </span>
                </td>
                <td class="rec-mono">{{ i.ledgerRef || "—" }}</td>
                <td class="rec-mono">{{ i.voucherNo || "—" }}</td>
                <td>{{ i.ledgerDate || "—" }}</td>
                <td>{{ i.voucherDate || "—" }}</td>
                <td class="data-amount">{{ fmtMoney(i.ledgerAmountCents) }}</td>
                <td class="data-amount">{{ fmtMoney(i.voucherAmountCents) }}</td>
                <td class="rec-message">{{ i.message }}</td>
              </tr>
            </tbody>
          </table>
        </div>
        <p v-else class="wb-empty rec-empty">
          两边完全对得上：{{ report.rows }} 行台账记录都有对应条目，金额、日期、对方一致。
        </p>
      </section>
    </template>

    <section v-else class="wb-card">
      <p class="wb-empty">
        {{ checking ? "正在比对…" : "还没有比对。先选择一份台账 CSV，再点「重新比对」。" }}
      </p>
    </section>
  </div>
</template>

<style scoped>
.rec-file {
  max-width: 240px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rec-bar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--s-3);
  margin-bottom: var(--s-3);
}

.rec-path {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  color: var(--text-3);
  font-size: var(--fs-caption);
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rec-tolerance {
  display: inline-flex;
  align-items: center;
  gap: var(--s-2);
  color: var(--text-2);
  font-size: var(--fs-caption);
}

.rec-tolerance .input {
  width: 80px;
}

.rec-stats {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(132px, 1fr));
  gap: var(--s-2);
  margin-bottom: var(--s-3);
}

.rec-stat {
  display: flex;
  flex-direction: column;
  gap: var(--s-1);
  padding: var(--s-3) var(--s-4);
  border: 1px solid var(--separator);
  border-radius: var(--radius-panel);
  background: var(--bg-card);
}

.rec-stat-label {
  color: var(--text-3);
  font-size: var(--fs-caption);
}

.rec-stat-value {
  font-family: var(--font-display);
  font-size: var(--fs-metric);
  font-variant-numeric: tabular-nums;
}

.rec-stat.ok .rec-stat-value {
  color: var(--success);
}

.rec-stat.bad .rec-stat-value {
  color: var(--danger);
}

.rec-table-wrap {
  overflow-x: auto;
}

.rec-mono {
  font-family: var(--font-mono, ui-monospace, monospace);
  font-size: var(--fs-caption);
}

.rec-message {
  max-width: 260px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.rec-empty {
  padding: var(--s-6);
}

@media (max-width: 767px) {
  .rec-table-wrap {
    overflow-x: auto;
  }

  .rec-table {
    min-width: 720px;
  }
}
</style>
