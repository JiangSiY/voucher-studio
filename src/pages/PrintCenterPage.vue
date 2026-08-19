<script setup lang="ts">
/**
 * 打印中心（对应 UI 方案页面二 + 页面三）：
 * 左侧选凭证 + 打印选项，右侧实时预览（三种卡片风格 / 1-2-3 列布局），
 * 生成 PDF（Rust 后端 printpdf，A4、中文、省墨）
 */
import { computed, onMounted, ref } from "vue";
import { api, type ExportBatch, type Voucher } from "../api/client";
import EmptyState from "../components/EmptyState.vue";
import VoucherCard from "../components/VoucherCard.vue";
import { toast, toastError } from "../stores/toast";

const vouchers = ref<Voucher[]>([]);
const selected = ref<Set<number>>(new Set());

const title = ref("凭证整理单");
const cardStyle = ref<"standard" | "clean" | "ledger">("standard");
const columns = ref<1 | 2 | 3>(3);
const includeImages = ref(true);

const busy = ref(false);
const lastBatch = ref<ExportBatch | null>(null);

async function load() {
  try {
    const result = await api.listVouchers({ page: 1, pageSize: 100 });
    vouchers.value = result.items;
  } catch (e) {
    toastError(e);
  }
}

function toggle(id: number) {
  if (selected.value.has(id)) selected.value.delete(id);
  else selected.value.add(id);
}

const previewVouchers = computed(() => vouchers.value.filter((v) => selected.value.has(v.id)));

const cardsPerPage = computed(() => columns.value * 7);

async function generate() {
  if (!selected.value.size) {
    toast.warning("请先勾选要打印的凭证");
    return;
  }
  busy.value = true;
  try {
    lastBatch.value = await api.exportPdf([...selected.value], {
      title: title.value,
      cardStyle: cardStyle.value,
      columns: columns.value,
      includeImages: includeImages.value,
    });
    toast.success(`PDF 已生成：${lastBatch.value.batchNo}（${lastBatch.value.voucherCount} 张凭证）`);
  } catch (e) {
    toastError(e);
  } finally {
    busy.value = false;
  }
}

async function openLastPdf() {
  if (!lastBatch.value) return;
  try {
    await api.openPath(lastBatch.value.pdfPath);
  } catch (e) {
    toastError(e);
  }
}

onMounted(load);
</script>

<template>
  <div class="split" style="display: grid; grid-template-columns: 1fr 1fr; gap: 16px; align-items: start">
    <!-- 左：选择 + 选项 -->
    <div>
      <div class="fluent-panel">
        <div class="fluent-panel-header">
          <h3>选择凭证（已选 {{ selected.size }}）</h3>
        </div>
        <div class="fluent-panel-body" style="padding: 0; max-height: 340px; overflow-y: auto">
          <div class="dtable-wrap" v-if="vouchers.length">
            <table class="dtable">
              <thead>
                <tr>
                  <th style="width: 32px"></th>
                  <th>编号</th>
                  <th>日期</th>
                  <th>摘要</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="v in vouchers" :key="v.id">
                  <td><input type="checkbox" class="cb" :checked="selected.has(v.id)" @change="toggle(v.id)" /></td>
                  <td><span class="mono">{{ v.voucherNo }}</span></td>
                  <td>{{ v.voucherDate }}</td>
                  <td style="max-width: 160px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap">{{ v.description || "—" }}</td>
                </tr>
              </tbody>
            </table>
          </div>
          <EmptyState v-else icon="🗂" title="还没有凭证" desc="先到凭证列表页导入或新建" />
        </div>
      </div>

      <div class="fluent-panel">
        <div class="fluent-panel-header"><h3>打印选项</h3></div>
        <div class="fluent-panel-body">
          <div class="field">
            <label>整理单标题</label>
            <input v-model="title" class="input" />
          </div>
          <div class="field">
            <label>卡片风格</label>
            <div class="filter-chips">
              <button class="fchip" :class="{ active: cardStyle === 'standard' }" @click="cardStyle = 'standard'">标准式</button>
              <button class="fchip" :class="{ active: cardStyle === 'clean' }" @click="cardStyle = 'clean'">简洁式</button>
              <button class="fchip" :class="{ active: cardStyle === 'ledger' }" @click="cardStyle = 'ledger'">账簿式</button>
            </div>
            <div class="desc">以凭证编号为核心、文字为主、省墨设计，无二维码</div>
          </div>
          <div class="field">
            <label>每行列数</label>
            <div class="filter-chips">
              <button class="fchip" :class="{ active: columns === 3 }" @click="columns = 3">3 列 · 21 张/页</button>
              <button class="fchip" :class="{ active: columns === 2 }" @click="columns = 2">2 列 · 14 张/页</button>
              <button class="fchip" :class="{ active: columns === 1 }" @click="columns = 1">1 列 · 7 张/页</button>
            </div>
          </div>
          <div style="display: flex; align-items: center; justify-content: space-between; padding: 8px 0">
            <div>
              <div style="font-size: 13px; font-weight: 600">附带附件图片页</div>
              <div style="font-size: 12px; color: var(--text-3)">在凭证卡片后追加图片附录（每页 2×2）</div>
            </div>
            <label class="toggle">
              <input type="checkbox" v-model="includeImages" />
              <span class="slider"></span>
            </label>
          </div>
          <button class="btn btn-primary" style="width: 100%; justify-content: center; padding: 8px" :disabled="busy"
            @click="generate">
            {{ busy ? "生成中…" : "🖨 生成 PDF" }}
          </button>
          <button v-if="lastBatch" class="btn btn-subtle" style="width: 100%; justify-content: center; margin-top: 8px"
            @click="openLastPdf">
            打开上一次生成的 PDF（{{ lastBatch.batchNo }}）
          </button>
        </div>
      </div>
    </div>

    <!-- 右：实时预览 -->
    <div>
      <div class="print-area">
        <div style="text-align: center; font-weight: 700; font-size: 14px; margin-bottom: 8px">{{ title }}</div>
        <div v-if="previewVouchers.length" class="print-grid" :class="`cols-${columns}`">
          <VoucherCard v-for="v in previewVouchers.slice(0, cardsPerPage)" :key="v.id" :voucher="v"
            :style="cardStyle" :title="title" />
        </div>
        <div v-else style="text-align: center; color: #8e8e8e; padding: 48px 0; font-size: 13px">
          勾选左侧凭证后在此预览打印效果
        </div>
        <div v-if="previewVouchers.length > cardsPerPage" style="text-align: center; color: #8e8e8e; font-size: 11px; margin-top: 8px">
          仅预览第 1 页（共 {{ Math.ceil(previewVouchers.length / cardsPerPage) }} 页），PDF 中完整输出
        </div>
        <div v-if="includeImages && previewVouchers.length" style="text-align: center; color: #8e8e8e; font-size: 11px; margin-top: 4px">
          PDF 将在卡片后附带附件图片附录页
        </div>
      </div>
      <div style="font-size: 11px; color: var(--text-4); text-align: center; margin-top: 8px">
        预览仅供参考，实际打印以 PDF 输出为准
      </div>
    </div>
  </div>
</template>
