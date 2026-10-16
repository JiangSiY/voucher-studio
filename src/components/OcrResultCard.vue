<script setup lang="ts">
/**
 * OCR 识别结果卡片（v0.9 从 VoucherFormPage 提取）：
 * 结构化提取预览（日期/金额/对方账户/商家）+ 置信度标签 + 单字段/全部填入。
 * 「填入」只通知父组件改表单值，落库仍由父组件「保存」决定。
 */
import { computed } from "vue";
import type { ExtractedFields } from "../api/client";
import CollapsiblePanel from "./CollapsiblePanel.vue";
import ConfidenceTag from "./ConfidenceTag.vue";
import FluentIcon from "./FluentIcon.vue";
import { fmtAmount } from "../composables/format";

const props = defineProps<{
  extracted: ExtractedFields | null;
  fieldsBusy: boolean;
}>();

const emit = defineEmits<{
  applyField: [field: "date" | "amount" | "counterparty" | "merchant"];
  applyAll: [];
}>();

/** 徽标：识别到几项（折叠时也能看出有没有内容） */
const extractedCount = computed(() => {
  const f = props.extracted;
  if (!f) return 0;
  return [f.date, f.amountCents, f.counterparty, f.merchant].filter(
    (v) => v != null && v !== "",
  ).length;
});
const badge = computed(() => (extractedCount.value ? `识别到 ${extractedCount.value} 项` : ""));

/** 提取字段是否可填入（至少一个字段非空才显示卡片） */
const hasExtracted = computed(() => {
  const f = props.extracted;
  return !!f && (f.date != null || f.amountCents != null || !!f.counterparty || !!f.merchant);
});
</script>

<template>
  <CollapsiblePanel v-if="hasExtracted || fieldsBusy" title="识别结果" :badge="badge">
    <template #actions>
      <button class="btn btn-sm btn-primary" v-if="hasExtracted" @click="emit('applyAll')">
        <FluentIcon name="flash" :size="14" />全部填入
      </button>
    </template>
    <div>
      <div v-if="fieldsBusy" style="font-size: 13px; color: var(--text-2)">正在提取字段…</div>
      <template v-else-if="extracted">
        <div class="ext-row">
          <span class="ext-label">日期</span>
          <span class="ext-value mono">
            {{ extracted.date ?? "—" }}
            <ConfidenceTag :level="extracted.confidence?.date ?? null" />
          </span>
          <button v-if="extracted.date" class="btn btn-sm" @click="emit('applyField', 'date')">填入</button>
        </div>
        <div class="ext-row">
          <span class="ext-label">金额</span>
          <span class="ext-value mono">
            {{ extracted.amountCents != null ? `¥${fmtAmount(extracted.amountCents)}` : "—" }}
            <!-- 置信度：标签命中可信 / 全文唯一兜底低置信 -->
            <span v-if="extracted.amountSource === 'labeled'" class="tag tag-success" title="命中「价税合计/合计」等标签">可信</span>
            <span v-else-if="extracted.amountSource === 'uniqueonly'" class="tag tag-warning" title="全文唯一金额兜底，无标签佐证，请人工核对">低置信</span>
          </span>
          <button v-if="extracted.amountCents != null" class="btn btn-sm" @click="emit('applyField', 'amount')">填入</button>
        </div>
        <div class="ext-row">
          <span class="ext-label">对方账户</span>
          <span class="ext-value">
            {{ extracted.counterparty ?? "—" }}
            <ConfidenceTag :level="extracted.confidence?.counterparty ?? null" />
          </span>
          <button v-if="extracted.counterparty" class="btn btn-sm" @click="emit('applyField', 'counterparty')">填入</button>
        </div>
        <div class="ext-row">
          <span class="ext-label">商家</span>
          <span class="ext-value">
            {{ extracted.merchant ?? "—" }}
            <ConfidenceTag :level="extracted.confidence?.merchant ?? null" />
          </span>
          <button v-if="extracted.merchant" class="btn btn-sm" title="商家名作为摘要填入" @click="emit('applyField', 'merchant')">填入摘要</button>
        </div>
        <div class="ext-tip">提取自 OCR 识别文本；「填入」只改表单值，点「保存」才落库。</div>
      </template>
    </div>
  </CollapsiblePanel>
</template>

<style scoped>
/* 识别结果卡片行（label / value / 填入按钮三段式） */
.ext-row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 0;
  border-bottom: 1px solid var(--stroke);
}

.ext-row:last-of-type {
  border-bottom: none;
}

.ext-label {
  flex: none;
  width: 72px;
  font-size: 13px;
  color: var(--text-2);
}

.ext-value {
  flex: 1;
  min-width: 0;
  font-size: 13px;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  display: flex;
  align-items: center;
  gap: 6px;
}

.ext-value .tag {
  flex: none;
}

.ext-tip {
  margin-top: 8px;
  font-size: 12px;
  color: var(--text-4);
}
</style>
