<script setup lang="ts">
/**
 * 凭证图片面板（v0.6/v0.12/D-1/D-4）：
 * 增强版优先展示 + 原图/增强切换 + A/B 对比滑块 + 大图查看器 + 手动裁剪/旋转。
 * 从 VoucherFormPage 提取；仅此面板聚焦图片展示与交互。
 * OCR 重跑 / 换图 / 裁剪后刷新由父组件编排（裁剪落库后 emit cropSaved 交由父级刷新 + 提示）。
 */
import { ref } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, type Attachment } from "../../api/client";
import { OCR_META } from "../../composables/format";
import { toastError } from "../../stores/toast";
import ImageViewerDialog from "../ImageViewerDialog.vue";
import CropDialog from "../CropDialog.vue";
import FluentIcon from "../FluentIcon.vue";

defineProps<{
  attachments: Attachment[];
  ocrBusy: boolean;
  replacing: boolean;
  lockedPeriod: string | null;
  /** 右栏未展宽（B 批）：操作按钮收成纯图标，避免换行成两排 */
  dense?: boolean;
}>();

const emit = defineEmits<{
  runOcr: [a: Attachment];
  replaceImage: [];
  /** 裁剪已落库：父组件负责刷新详情 + 展示「重新识别」提示 */
  cropSaved: [];
}>();

// 图片地址一律用后端下发的绝对路径（`fileAbs` / `enhancedAbs`）。
// 不要在前端拼路径：拼分隔符曾在 macOS 上拼出不存在的路径，被 asset protocol 全部拒绝。

/** 把一个绝对路径转成 webview 可用的 asset URL（空值 → 占位空串） */
function assetUrl(abs?: string | null): string {
  return abs ? convertFileSrc(abs) : "";
}

// ---------- D-1：编辑页缩略图优先用增强版，支持「原图/增强」切换 ----------

/** D-1：单图默认用增强版（如有）；查看器/查看大图也走同一规则 */
function attSrc(a: Attachment): string {
  // enhancedAbs 由后端保证「指向真实存在的文件」；缺失时回退原图
  return assetUrl(a.enhancedAbs ?? a.fileAbs);
}

/** 切换该附件当前显示的图（编辑页用，per-attachment 状态） */
const useEnhanced = ref<Record<number, boolean>>({});
function thumbSrc(a: Attachment): string {
  // 默认 true（优先增强版），用户切换时改 map
  const preferEnhanced = useEnhanced.value[a.id] ?? true;
  return assetUrl(preferEnhanced && a.enhancedAbs ? a.enhancedAbs : a.fileAbs);
}
function toggleThumbSource(a: Attachment) {
  useEnhanced.value[a.id] = !(useEnhanced.value[a.id] ?? true);
}

// ---------- D-4：增强版 vs 原图 A/B 对比滑块 ----------
// 用 `clip-path: inset(0 (1-x)*100% 0 0)` 把"已显示"区裁出来
// slider=0.5 → 增强版占左半、原图层占右半（用户能直接看到差异）
// 仅当 a.enhancedPath 存在时启用
const compareOpen = ref<Record<number, boolean>>({});
const compareSplit = ref<Record<number, number>>({}); // 0..1，0.5 = 中线
function isCompareOpen(a: Attachment): boolean {
  return compareOpen.value[a.id] ?? false;
}
function openCompare(a: Attachment) {
  compareOpen.value[a.id] = true;
  if (compareSplit.value[a.id] === undefined) {
    compareSplit.value[a.id] = 0.5; // 默认中线
  }
}
function closeCompare(a: Attachment) {
  compareOpen.value[a.id] = false;
}
function setSplit(a: Attachment, v: number) {
  compareSplit.value[a.id] = Math.max(0, Math.min(1, v));
}
function compareOriginalSrc(a: Attachment): string {
  // 原图层 = always 原图（不受 useEnhanced 切换影响）
  return assetUrl(a.fileAbs);
}
function compareClipRight(a: Attachment): string {
  // 增强版（thumbSrc）只显示左侧，比例 = compareSplit
  const p = (compareSplit.value[a.id] ?? 0.5) * 100;
  return `inset(0 ${100 - p}% 0 0)`;
}

// ---------- 图片查看器（缩放/平移/旋转，核对 OCR 回填时看细节） ----------

const viewerVisible = ref(false);
const viewerSrc = ref("");
const viewerName = ref("");
const viewerAttachmentId = ref<number | null>(null);

function openViewer(a: Attachment) {
  viewerSrc.value = attSrc(a);
  viewerName.value = a.fileName;
  viewerAttachmentId.value = a.id;
  viewerVisible.value = true;
}

// ---------- v0.12：手动裁剪 / 旋转（自动矫正失败的人工补救） ----------

const cropVisible = ref(false);
const cropAttachmentId = ref<number | null>(null);
const cropSrc = ref("");
const cropFileName = ref("");
const cropDialog = ref<InstanceType<typeof CropDialog> | null>(null);

function openCrop(a: Attachment) {
  // 基准 = 原图（手动裁剪替代自动增强，不叠加）
  cropSrc.value = assetUrl(a.fileAbs);
  cropFileName.value = a.fileName;
  cropAttachmentId.value = a.id;
  cropVisible.value = true;
}

async function onCropSave(
  rect: { x: number; y: number; w: number; h: number },
  rotationDeg: number,
) {
  if (!cropAttachmentId.value) return;
  try {
    await api.cropAttachment(cropAttachmentId.value, rect, rotationDeg);
    cropVisible.value = false;
    emit("cropSaved"); // 父组件刷新详情 + 展示「重新识别」提示
  } catch (e) {
    toastError(e);
    cropDialog.value?.done();
  }
}
</script>

<template>
  <div class="fluent-panel">
    <div class="fluent-panel-header">
      <h3>凭证图片（{{ attachments.length }}/1）</h3>
    </div>
    <div class="fluent-panel-body">
      <div v-if="attachments.length" class="att-grid">
        <div v-for="a in attachments" :key="a.id" class="att-card">
          <div
            class="att-img"
            :class="{ 'is-compare': isCompareOpen(a) }"
            title="点击查看大图（滚轮缩放 / 拖动平移）"
            @click="!isCompareOpen(a) && openViewer(a)"
          >
            <!-- D-4：对比模式 - 底层原图（始终原图，不受 useEnhanced 影响） + 上层增强版用 clip-path 裁剪 -->
            <template v-if="isCompareOpen(a) && a.enhancedPath">
              <img
                class="att-img-compare-base"
                :src="compareOriginalSrc(a)"
                :alt="a.fileName + ' (原图)'"
              />
              <img
                class="att-img-compare-top"
                :src="thumbSrc(a)"
                :alt="a.fileName + ' (增强版)'"
                :style="{ clipPath: compareClipRight(a) }"
              />
              <!-- 中线 -->
              <div
                class="att-compare-divider"
                :style="{ left: (compareSplit[a.id] ?? 0.5) * 100 + '%' }"
              ></div>
              <!-- 滑块手柄 -->
              <div
                class="att-compare-handle"
                :style="{ left: (compareSplit[a.id] ?? 0.5) * 100 + '%' }"
              >
                <FluentIcon name="compare" :size="14" />
              </div>
              <!-- 角标：左=增强 右=原图 -->
              <span class="att-compare-tag att-compare-tag-left"
                ><FluentIcon name="sparkle" :size="12" /> 增强</span
              >
              <span class="att-compare-tag att-compare-tag-right">原图</span>
            </template>
            <!-- 非对比模式：单图（thumbSrc 受 useEnhanced 切换） -->
            <template v-else>
              <img :src="thumbSrc(a)" :alt="a.fileName" loading="lazy" />
            </template>
            <!-- v0.6 增强状态角标 -->
            <span
              v-if="a.enhancedPath && !isCompareOpen(a)"
              class="att-enhanced-badge"
              title="已通过 5 步算法矫正（用于 OCR / 查看器）"
            >
              <FluentIcon name="sparkle" :size="12" /> 已增强
            </span>
            <!-- D-1：原图/增强版切换器（无增强版时隐藏；对比模式下隐藏避免双控制） -->
            <button
              v-if="a.enhancedPath && !isCompareOpen(a)"
              class="att-thumb-toggle"
              :title="(useEnhanced[a.id] ?? true) ? '点击查看原图' : '点击查看增强版'"
              @click.stop="toggleThumbSource(a)"
            >
              {{ (useEnhanced[a.id] ?? true) ? "增强" : "原图" }}
            </button>
          </div>
          <div class="att-meta">
            <div class="att-name" :title="a.fileName">{{ a.fileName }}</div>
            <div style="display: flex; gap: 6px; flex-wrap: wrap; align-items: center">
              <span class="tag" :class="OCR_META[a.ocrStatus].tag">{{
                OCR_META[a.ocrStatus].label
              }}</span>
              <span
                v-if="a.enhancedSource === 'screenshot'"
                class="tag tag-muted"
                title="截图（增强收益小）"
                >截图</span
              >
              <span
                v-else-if="a.enhancedSource === 'scan'"
                class="tag tag-info"
                title="扫描件 / 拍照"
                >扫描件</span
              >
            </div>
            <div class="att-ops" :class="{ isDense: dense }">
              <button
                class="btn btn-sm"
                :disabled="ocrBusy"
                :title="ocrBusy ? '识别中…' : '重新识别（重跑本地 OCR）'"
                aria-label="重新识别"
                @click="emit('runOcr', a)"
              >
                <FluentIcon class="ops-icon" name="refresh" :size="14" />
                <span class="ops-label">{{ ocrBusy ? "识别中…" : "重新识别" }}</span>
              </button>
              <button
                class="btn btn-sm"
                :disabled="replacing"
                :title="replacing ? '更换中…' : '更换图片'"
                aria-label="更换图片"
                @click="emit('replaceImage')"
              >
                <FluentIcon class="ops-icon" name="image" :size="14" />
                <span class="ops-label">{{ replacing ? "更换中…" : "更换图片" }}</span>
              </button>
              <!-- v0.12：手动裁剪/旋转（自动矫正失败的人工补救；覆盖增强版，原图不动） -->
              <button
                class="btn btn-sm"
                :disabled="!!lockedPeriod"
                :title="
                  lockedPeriod ? '该月已封账，不可调整' : '手动裁剪 / 旋转（覆盖增强版，原图不变）'
                "
                aria-label="手动裁剪或旋转"
                @click="openCrop(a)"
              >
                <FluentIcon name="edit" :size="14" /><span class="ops-label">调整</span>
              </button>
              <!-- D-4：A/B 对比开关（无增强版不可用） -->
              <button
                v-if="a.enhancedPath"
                class="btn btn-sm"
                :class="{ 'btn-primary': isCompareOpen(a) }"
                :title="
                  isCompareOpen(a) ? '关闭 A/B 对比' : '打开 A/B 对比：拖动滑块看增强 vs 原图'
                "
                :aria-label="isCompareOpen(a) ? '关闭 A/B 对比' : '打开 A/B 对比'"
                @click="isCompareOpen(a) ? closeCompare(a) : openCompare(a)"
              >
                <FluentIcon :name="isCompareOpen(a) ? 'close' : 'compare'" :size="14" />
                <span class="ops-label">{{ isCompareOpen(a) ? "关闭对比" : "A/B 对比" }}</span>
              </button>
            </div>
            <!-- D-4：对比模式滑块行（仅打开时显示） -->
            <div v-if="isCompareOpen(a) && a.enhancedPath" class="att-compare-row">
              <span class="att-compare-label"><FluentIcon name="sparkle" :size="12" /> 增强</span>
              <input
                type="range"
                min="0"
                max="100"
                step="1"
                class="att-compare-slider"
                :value="Math.round((compareSplit[a.id] ?? 0.5) * 100)"
                @input="setSplit(a, +($event.target as HTMLInputElement).value / 100)"
                title="拖动调整：左=增强 / 右=原图"
              />
              <span class="att-compare-label">原图</span>
              <span class="att-compare-pct"
                >{{ Math.round((compareSplit[a.id] ?? 0.5) * 100) }}%</span
              >
            </div>
            <div v-if="a.ocrText" class="ocr-text">{{ a.ocrText }}</div>
          </div>
        </div>
      </div>
      <div
        v-else
        class="dropzone"
        role="button"
        tabindex="0"
        aria-label="选择图片补挂到这张凭证"
        @click="emit('replaceImage')"
        @keydown.enter.prevent="emit('replaceImage')"
        @keydown.space.prevent="emit('replaceImage')"
      >
        <div class="dz-icon"><FluentIcon name="image" :size="24" /></div>
        <div style="font-weight: 600">点击选择图片补挂到这张凭证</div>
        <div style="font-size: var(--fs-caption); margin-top: 4px">
          一图一凭证：每张凭证只关联一张图片
        </div>
      </div>
    </div>
  </div>

  <ImageViewerDialog
    :visible="viewerVisible"
    :src="viewerSrc"
    :file-name="viewerName"
    :attachment-id="viewerAttachmentId"
    @close="viewerVisible = false"
  />

  <!-- v0.12：手动裁剪 / 旋转对话框（保存 → crop_attachment → emit cropSaved；可一键重新识别） -->
  <CropDialog
    ref="cropDialog"
    :visible="cropVisible"
    :attachment-id="cropAttachmentId"
    :src="cropSrc"
    :file-name="cropFileName"
    @close="cropVisible = false"
    @save="onCropSave"
  />
</template>
