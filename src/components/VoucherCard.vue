<script setup lang="ts">
/**
 * 凭证卡片（打印预览）：三种风格严格对齐 UI 方案——
 * standard（深色头）/ clean（左色条）/ ledger（账簿式），无 QR 码。
 */
import { computed } from "vue";
import type { Voucher } from "../api/client";
import { fmtAmount } from "../composables/format";

const props = withDefaults(
  defineProps<{
    voucher: Pick<Voucher, "voucherNo" | "voucherDate" | "description" | "amountCents" | "tags" | "note">;
    style?: "standard" | "clean" | "ledger";
    title?: string;
  }>(),
  { style: "standard", title: "凭证整理单" },
);

/** 金额文本统一走 fmtAmount（千分位 + 固定两位小数）；打印件上空金额仍留白，故 null → 空串 */
const amount = computed(() =>
  props.voucher.amountCents == null ? "" : fmtAmount(props.voucher.amountCents),
);
</script>

<template>
  <!-- 标准式 -->
  <div v-if="style === 'standard'" class="vcard accent">
    <div class="vh">{{ title }}</div>
    <div class="vb">
      <div class="vinfo">
        <div class="vcode">{{ voucher.voucherNo }}</div>
        <div class="vdur">{{ voucher.voucherDate }}<template v-if="amount"> · ¥{{ amount }}</template></div>
        <div class="vexp">{{ voucher.description || "（无摘要）" }}</div>
      </div>
    </div>
    <div style="padding: 0 8px 4px">
      <div class="vnote">
        <template v-if="voucher.tags.length">{{ voucher.tags.join(" · ") }}<br /></template>
        {{ voucher.note || "记账原始凭证 · 归档留存" }}
      </div>
    </div>
  </div>

  <!-- 简洁式 -->
  <div v-else-if="style === 'clean'" class="vcard-clean">
    <div class="vcl-title">{{ title }}</div>
    <div class="vcl-code">{{ voucher.voucherNo }}</div>
    <div class="vcl-meta">
      {{ voucher.voucherDate }}<template v-if="amount"> · ¥{{ amount }}</template><br />
      {{ voucher.description || "（无摘要）" }}<br />
      {{ voucher.tags.join(" · ") }}
    </div>
  </div>

  <!-- 账簿式 -->
  <div v-else class="vcard-ledger">
    <div class="vll-head">
      <span class="num">{{ title }}</span>
      <span class="dt">{{ voucher.voucherDate }}</span>
    </div>
    <div class="vll-body">
      <div class="vll-code">{{ voucher.voucherNo }}</div>
      <div class="vll-rows">
        <div class="vll-row"><span class="k">摘要</span><span class="vv">{{ voucher.description || "—" }}</span></div>
        <div class="vll-row"><span class="k">金额</span><span class="vv">{{ amount ? `¥${amount}` : "—" }}</span></div>
        <div class="vll-row"><span class="k">标签</span><span class="vv">{{ voucher.tags.join("、") || "—" }}</span></div>
      </div>
    </div>
  </div>
</template>
