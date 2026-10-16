<script setup lang="ts">
/**
 * 新建凭证（向导）—— v0.19 起本页**只负责新建**。
 *
 * 编辑已并入资料库右栏（见 `components/voucher/VoucherEditor.vue` 的文件头注释：
 * 原先"独立编辑页 + 右栏便捷修改"两套实现，同字段两处可改、跳页丢上下文，2026-09-28 合并）。
 *
 * 为什么新建仍保留独立页：它是"选图 → 拖拽导入 → 建卡 + OCR → 确认"的多步流程，
 * 而且建卡之前列表里还没有这条记录，"选中即在右栏编辑"的模型不适用。
 */
import { onMounted, onUnmounted, ref } from "vue";
import { useRouter } from "vue-router";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api, type ImportSummary } from "../api/client";
import FluentIcon from "../components/FluentIcon.vue";
import { toast, toastError } from "../stores/toast";

const router = useRouter();
const creating = ref(false);
const noPreview = ref("");

// 本地日期（不能用 toISOString：那是 UTC，UTC+8 环境下本地 00:00–08:00 会得到「昨天」，
// 编号预览也跟着取错日期档号）。2026-09-23 审计 B16。
function todayLocal(): string {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

async function refreshNoPreview() {
  try {
    noPreview.value = await api.previewVoucherNo(todayLocal());
  } catch {
    noPreview.value = "";
  }
}

/** 建卡完成 → 去资料库并聚焦该张（编辑一律在右栏做） */
function openInLibrary(voucherId: number) {
  router.push({ path: "/vouchers", query: { focus: String(voucherId) } });
}

async function createByImage() {
  if (creating.value) return;
  creating.value = true;
  try {
    const summary = await api.pickAndImport(true);
    if (!summary) return; // 用户取消
    reportImport(summary);
    if (summary.vouchers.length) {
      // 显式提示：建卡在选图这一刻完成，即将去资料库聚焦显示（多张时进第一张，其余已在列表）
      toast.info(`已创建 ${summary.vouchers.length} 张凭证，正在打开第一张…`);
      openInLibrary(summary.vouchers[0].voucherId);
    }
  } catch (e) {
    toastError(e);
  } finally {
    creating.value = false;
  }
}

function reportImport(summary: ImportSummary) {
  const parts: string[] = [];
  if (summary.vouchers.length) parts.push(`已创建 ${summary.vouchers.length} 张凭证（一图一凭证）`);
  if (summary.skippedDuplicates.length) parts.push(`跳过重复 ${summary.skippedDuplicates.length} 张`);
  if (summary.failed.length) parts.push(`失败 ${summary.failed.length} 张`);
  if (parts.length) toast.success(parts.join(" · "));
}

// 拖放导入（Rust 侧全局处理）：完成后去资料库聚焦第一张新凭证
let unlistenImportDone: UnlistenFn | null = null;

onMounted(async () => {
  void refreshNoPreview();
  unlistenImportDone = await listen<ImportSummary>("import:done", (e) => {
    if (e.payload.vouchers.length) openInLibrary(e.payload.vouchers[0].voucherId);
  });
});

onUnmounted(() => unlistenImportDone?.());
</script>

<template>
  <div class="form-grid">
    <div>
      <div class="fluent-panel">
        <div class="fluent-panel-header"><h3>新建凭证</h3></div>
        <div class="fluent-panel-body">
          <div class="callout info" style="margin: 0 0 16px">
            <span class="ci"><FluentIcon name="image" :size="18" /></span>
            <div class="ct">
              <strong>一张图片就是一张凭证。</strong>
              选择图片后系统会立即建卡 + OCR 识别回填日期 / 金额 / 对方账户（可再修改），
              然后自动跳到资料库并聚焦这一张——在右侧编辑区核对与完善字段。
            </div>
          </div>
          <!-- 键盘可达：role=button + Tab 聚焦 + Enter/Space 触发（内含 div，故不用真 <button> 以免非法嵌套） -->
          <div class="dropzone" style="padding: 48px 24px" role="button" tabindex="0"
            aria-label="选择图片创建凭证" @click="createByImage"
            @keydown.enter.prevent="createByImage" @keydown.space.prevent="createByImage">
            <div class="dz-icon"><FluentIcon name="image" :size="32" /></div>
            <div style="font-weight: 600">点击选择图片，或拖放到此处</div>
            <div style="font-size: 12px; margin-top: 4px">
              支持 jpg / png / bmp / webp；多张图片会各自创建凭证，相同图片自动去重
            </div>
          </div>
          <div class="field" style="margin-top: 16px">
            <label>新凭证编号预览</label>
            <input :value="noPreview || '…'" class="input mono" disabled />
            <div class="desc">编号规则可在设置中修改；导入后自动取号。</div>
          </div>
        </div>
      </div>
    </div>
    <div>
      <div class="fluent-panel">
        <div class="fluent-panel-header"><h3>录入流程</h3></div>
        <div class="fluent-panel-body">
          <ol style="padding-left: 20px; display: flex; flex-direction: column; gap: 10px; font-size: 13px; color: var(--text-2)">
            <li>选择 / 拖放导入图片（支持批量，一图一凭证）</li>
            <li>自动建卡并 OCR 识别（离线引擎，数据不出本机）</li>
            <li>在资料库右侧编辑区核对自动回填的日期、金额、对方账户，补充摘要与标签</li>
            <li>按天导出「凭证日报」PDF 归档</li>
          </ol>
        </div>
      </div>
    </div>
  </div>
</template>
