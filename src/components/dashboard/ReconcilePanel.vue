<script setup lang="ts">
/**
 * 仪表盘 → 对账结果面板（v0.13）：✅ 匹配 / 🔵 账单无凭证 / 🔴 凭证无账单
 * 三格概览 + 差异明细表；报告由父页触发对账后传入，关闭事件回传。
 */
import { useRouter } from "vue-router";
import { api, type ReconcileReport } from "../../api/client";
import FluentIcon from "../FluentIcon.vue";
import { fmtAmount } from "../../composables/format";

defineProps<{ report: ReconcileReport }>();
const emit = defineEmits<{ close: [] }>();
const router = useRouter();
</script>

<template>
  <div class="fluent-panel">
    <div class="fluent-panel-header">
      <h3>对账结果（{{ report.dateFrom }} ~ {{ report.dateTo }} · 账单 {{ report.billCount }} 笔）</h3>
      <div style="display: flex; gap: 8px">
        <button v-if="report.reportPath" class="btn btn-sm"
          @click="api.openPath(report.reportPath!)"><FluentIcon name="file" :size="14" />打开报告 CSV</button>
        <button class="btn btn-sm" @click="emit('close')">关闭</button>
      </div>
    </div>
    <div class="fluent-panel-body" style="padding: 0">
      <div class="recap-grid">
        <div class="recap-cell ok">
          <div class="rc-num">{{ report.matched.length }}</div>
          <div class="rc-label">✅ 匹配</div>
        </div>
        <div class="recap-cell warn">
          <div class="rc-num">{{ report.billOnly.length }}</div>
          <div class="rc-label">🔵 账单无凭证（漏拍图）</div>
        </div>
        <div class="recap-cell warn">
          <div class="rc-num">{{ report.voucherOnly.length }}</div>
          <div class="rc-label">🔴 凭证无账单（金额/日期不符或现金）</div>
        </div>
      </div>

      <div class="dtable-wrap" v-if="report.billOnly.length || report.voucherOnly.length">
        <table class="dtable">
          <thead>
            <tr>
              <th>类型</th><th>日期</th><th>金额</th><th>对方 / 摘要</th><th>说明</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="b in report.billOnly" :key="`bo-${b.lineNo}`">
              <td><span class="tag tag-info">🔵 账单无凭证</span></td>
              <td>{{ b.date }}</td>
              <td class="num">{{ fmtAmount(b.amountCents) }}</td>
              <td style="max-width: 160px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap">{{ b.counterparty || "—" }}</td>
              <td style="max-width: 200px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap">{{ b.subject || "—" }}（{{ b.direction }}，需补拍图或录入）</td>
            </tr>
            <tr v-for="v in report.voucherOnly" :key="`vo-${v.id}`" style="cursor: pointer"
              @click="router.push(`/vouchers/${v.id}`)">
              <td><span class="tag tag-muted">🔴 凭证无账单</span></td>
              <td>{{ v.voucherDate }}</td>
              <td class="num">{{ fmtAmount(v.amountCents) }}</td>
              <td style="max-width: 160px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap">{{ v.counterparty || "—" }}</td>
              <td style="max-width: 200px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap">
                {{ v.description || "（无摘要）" }}（现金交易或金额/日期与账单不符，点击核对）
              </td>
            </tr>
          </tbody>
        </table>
      </div>
      <div v-else class="empty-state" style="padding: 28px">
        <div class="es-icon"><FluentIcon name="checkCircle" :size="36" /></div>
        <div class="es-title">账单与凭证全部匹配</div>
        <div class="es-desc">范围内没有差异项</div>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* v0.13 对账结果概览（三格：匹配 / 账单无凭证 / 凭证无账单） */
.recap-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 10px;
  padding: 14px 16px 6px;
}

.recap-cell {
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 10px 6px;
  border-radius: 8px;
  border: 1px solid var(--stroke);
}

.recap-cell.ok {
  border-color: var(--success, #107c10);
  background: rgba(16, 124, 16, 0.06);
}

.recap-cell.warn {
  border-color: var(--warning, #c88a17);
  background: rgba(200, 138, 23, 0.06);
}

.rc-num {
  font-size: 22px;
  font-weight: 700;
  font-variant-numeric: tabular-nums;
}

.rc-label {
  font-size: 12px;
  color: var(--text-3);
  margin-top: 2px;
}
</style>
