<script setup lang="ts">
/**
 * 设置 → OCR：引擎选择（本地/云端）、本地模型状态、批量识别待处理附件。
 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api, type AppConfig, type LocalOcrStatus } from "../../api/client";
import FluentIcon from "../FluentIcon.vue";
import { toast, toastError } from "../../stores/toast";

defineProps<{ cfg: AppConfig }>();

const localOcr = ref<LocalOcrStatus | null>(null);

// 批量识别待处理附件（列表页导入未跑 OCR 的、历史失败件等），成功后自动填单（只填空字段）
const ocrPendingBusy = ref(false);
const ocrProgress = ref<{ done: number; total: number } | null>(null);

const ocrProgressPct = computed(() => {
  const p = ocrProgress.value;
  if (!p || p.total === 0) return 0;
  return Math.round((p.done / p.total) * 100);
});

async function runOcrPending() {
  if (ocrPendingBusy.value) return;
  ocrPendingBusy.value = true;
  ocrProgress.value = null;
  try {
    const n = await api.runOcrPending(500);
    if (n > 0) {
      toast.success(`批量识别完成：${n} 张附件已识别并自动填单`);
    } else {
      toast.info?.("没有待识别的附件");
    }
  } catch (e) {
    toastError(e);
  } finally {
    ocrPendingBusy.value = false;
    ocrProgress.value = null;
  }
}

// 批量识别进度（后端逐张推送 ocr:progress 事件）
let unlistenOcrProgress: UnlistenFn | null = null;

onMounted(async () => {
  try {
    localOcr.value = await api.getLocalOcrStatus();
  } catch (e) {
    toastError(e);
  }
  unlistenOcrProgress = await listen<{ done: number; total: number }>("ocr:progress", (e) => {
    if (ocrPendingBusy.value) ocrProgress.value = e.payload;
  });
});

onUnmounted(() => {
  unlistenOcrProgress?.();
});
</script>

<template>
  <div class="fluent-panel">
    <div class="fluent-panel-body">
      <div style="display: flex; align-items: center; justify-content: space-between; padding: 8px 0 16px">
        <div>
          <div style="font-size: 13px; font-weight: 600">启用 OCR</div>
          <div style="font-size: 12px; color: var(--text-3)">关闭后导入图片不再自动识别文字</div>
        </div>
        <label class="toggle">
          <input type="checkbox" v-model="cfg.ocr.enabled" />
          <span class="slider"></span>
        </label>
      </div>
      <div class="field">
        <label>识别引擎</label>
        <select v-model="cfg.ocr.engine" class="input" style="max-width: 280px">
          <option value="local" :disabled="!localOcr?.installed">本地（离线 · 推荐）</option>
          <option value="cloud">云端（HTTP API）</option>
        </select>
        <div class="desc">本地引擎完全离线运行，数据不出本机（PP-OCRv4，Spike B 实测字符准确率 99.9%）。</div>
      </div>
      <div v-if="localOcr?.installed" class="callout success" style="margin: 0 0 16px">
        <span class="ci"><FluentIcon name="checkCircle" :size="18" /></span>
        <div class="ct">
          <strong>本地模型已安装</strong>
          <span v-if="localOcr.files.length">（{{ localOcr.files.length }} 个文件，约 {{ (localOcr.totalSize / 1048576).toFixed(1) }} MB）</span>
          ：RapidOCR PP-OCRv4 检测 + 方向分类 + 识别，首次识别时加载约 0.2 秒。
        </div>
      </div>
      <div v-else class="callout warning" style="margin: 0 0 16px">
        <span class="ci"><FluentIcon name="warning" :size="18" /></span>
        <div class="ct">
          <strong>本地模型未安装</strong>——将模型文件（ch_PP-OCRv4_det_infer.onnx 等 3 个）放入
          <code>{{ localOcr?.modelsDir || "数据目录/models" }}</code> 后重启应用即可启用。
        </div>
      </div>
      <template v-if="cfg.ocr.engine === 'cloud'">
        <div class="field">
          <label>API 地址</label>
          <input v-model="cfg.ocr.apiUrl" class="input mono" placeholder="https://api.example.com/ocr" />
          <div class="desc">POST JSON：{image_base64, model}，返回 {text}；API Key 在「密钥管理」页设置。</div>
        </div>
        <div class="field">
          <label>模型名（可选）</label>
          <input v-model="cfg.ocr.model" class="input" placeholder="如：voucher-ocr-v1" />
        </div>
      </template>
      <div class="field" style="border-top: 1px solid var(--stroke); padding-top: 16px">
        <label>批量识别</label>
        <div style="display: flex; gap: 8px; align-items: center">
          <button class="btn" :disabled="ocrPendingBusy" @click="runOcrPending">
            {{ ocrPendingBusy ? "识别中…" : "识别全部待处理" }}
          </button>
          <span style="font-size: 12px; color: var(--text-3)">
            处理所有待识别附件（列表页导入、识别失败件），识别后自动填单（只填空字段）
          </span>
        </div>
        <div v-if="ocrProgress" class="ocr-progress">
          <div class="ocr-progress-track">
            <div class="ocr-progress-fill" :style="{ width: `${ocrProgressPct}%` }"></div>
          </div>
          <span class="ocr-progress-text">
            {{ ocrProgress.done }} / {{ ocrProgress.total }}（{{ ocrProgressPct }}%）
          </span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 批量识别进度条（Fluent 细条样式） */
.ocr-progress {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 8px;
}

.ocr-progress-track {
  flex: 1;
  height: 4px;
  border-radius: 2px;
  background: var(--stroke);
  overflow: hidden;
}

.ocr-progress-fill {
  height: 100%;
  border-radius: 2px;
  background: var(--accent);
  transition: width 0.2s ease;
}

.ocr-progress-text {
  font-size: 12px;
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
</style>
