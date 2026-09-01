<script setup lang="ts">
/**
 * 凭证编辑页。
 * 一图一凭证：新建 = 先选一张图片（导入即建卡 + OCR 自动填单），编辑 = 单图 + 更换图片（换图不换卡）。
 * 表单字段：编号/日期/摘要/对方账户/金额/状态/标签/备注 + 快照历史。
 * 拖放导入在 Rust 侧全局处理（App.vue 反馈）；本页新建模式监听完成事件跳转到新凭证。
 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, type Attachment, type ExtractedFields, type ImportSummary, type Voucher, type VoucherSnapshot } from "../api/client";
import ImageViewerDialog from "../components/ImageViewerDialog.vue";
import ConfidenceTag from "../components/ConfidenceTag.vue";
import { OCR_META, fmtDate } from "../composables/format";
import { useConfigStore } from "../stores/config";
import { toast, toastError } from "../stores/toast";

const route = useRoute();
const router = useRouter();
const configStore = useConfigStore();

const editId = computed(() => {
  const id = Number(route.params.id);
  return Number.isFinite(id) && id > 0 ? id : null;
});

// 表单状态
const voucherNo = ref("");
const voucherDate = ref(new Date().toISOString().slice(0, 10));
const description = ref("");
const counterparty = ref("");
const amountYuan = ref<string>("");
const status = ref<"draft" | "reviewed" | "exported" | "archived">("draft");
const tagsInput = ref("");
const note = ref("");

const attachments = ref<Attachment[]>([]);
const snapshots = ref<VoucherSnapshot[]>([]);
const saving = ref(false);
const ocrBusy = ref(false);
const replacing = ref(false);

const noPreview = ref("");
const attachmentsDir = computed(() =>
  configStore.appInfo ? `${configStore.appInfo.dataDir}\\attachments` : "",
);

async function refreshNoPreview() {
  try {
    noPreview.value = await api.previewVoucherNo(voucherDate.value);
  } catch {
    noPreview.value = "";
  }
}

/** 把凭证字段套用到表单 */
function applyForm(v: Voucher) {
  voucherNo.value = v.voucherNo;
  voucherDate.value = v.voucherDate;
  description.value = v.description;
  counterparty.value = v.counterparty;
  amountYuan.value = v.amountCents != null ? (v.amountCents / 100).toFixed(2) : "";
  status.value = v.status;
  tagsInput.value = v.tags.join(", ");
  note.value = v.note;
}

/** 把凭证详情套用到表单（编辑加载 / OCR 自动填单后共用） */
function applyDetail(detail: Awaited<ReturnType<typeof api.getVoucher>>) {
  applyForm(detail.voucher);
  attachments.value = detail.attachments;
  snapshots.value = detail.snapshots;
  // v0.7 S-2：识别文本就绪时拉取结构化提取预览（只读，不写库）
  loadExtracted(detail.attachments[0]);
}

// ---------- v0.7 S-2：OCR 识别结果卡片（提取预览 + 一键填入） ----------

const extracted = ref<ExtractedFields | null>(null);
const fieldsBusy = ref(false);

/** 从附件已识别文本抽取结构化字段（只读预览；失败静默，不打扰主流程） */
async function loadExtracted(a: Attachment | undefined) {
  extracted.value = null;
  if (!a || !a.ocrText) return;
  fieldsBusy.value = true;
  try {
    extracted.value = await api.ocrPreviewFields(a.id);
  } catch {
    // 提取预览是辅助功能，失败不影响编辑
  } finally {
    fieldsBusy.value = false;
  }
}

/** 提取字段是否可填入（至少一个字段非空才显示卡片） */
const hasExtracted = computed(() => {
  const f = extracted.value;
  return !!f && (f.date != null || f.amountCents != null || !!f.counterparty || !!f.merchant);
});

/** 单字段填入：只改表单值，不落库（用户点「保存」才生效） */
function applyExtractedField(field: "date" | "amount" | "counterparty" | "merchant") {
  const f = extracted.value;
  if (!f) return;
  if (field === "date" && f.date) voucherDate.value = f.date;
  else if (field === "amount" && f.amountCents != null) {
    amountYuan.value = (f.amountCents / 100).toFixed(2);
  } else if (field === "counterparty" && f.counterparty) {
    counterparty.value = f.counterparty;
  } else if (field === "merchant" && f.merchant) {
    // 商家/销售方作摘要素材（发票场景对方账户与商家是两个实体）
    description.value = f.merchant;
  }
}

/** 全部填入：四个字段一次性套用到表单。
 * 置信度策略：金额仅「标签命中（可信）」自动填；v5 版面置信度为 low 的字段
 * （启发式兜底：首行店名 / 全文唯一金额）不自动填——用户可在卡片上单字段点填。 */
function applyAllExtracted() {
  const f = extracted.value;
  if (!f) return;
  const low = (c: "high" | "medium" | "low" | null) => c === "low";
  if (f.date) voucherDate.value = f.date;
  const amountTrusted = f.amountCents != null && f.amountSource === "labeled" && !low(f.confidence?.amount);
  if (amountTrusted) amountYuan.value = (f.amountCents! / 100).toFixed(2);
  if (f.counterparty && !low(f.confidence?.counterparty)) counterparty.value = f.counterparty;
  if (f.merchant && !low(f.confidence?.merchant)) description.value = f.merchant;
  const skippedParts: string[] = [];
  if (f.amountCents != null && !amountTrusted) skippedParts.push("金额");
  if (f.counterparty && low(f.confidence?.counterparty)) skippedParts.push("对方账户");
  if (f.merchant && low(f.confidence?.merchant)) skippedParts.push("商家");
  const skipped = skippedParts.length ? `（${skippedParts.join("、")}为低置信未自动填，可单独点「填」）` : "";
  toast.info(`已填入表单，核对后点「保存」生效${skipped}`);
}

async function loadDetail() {
  if (!editId.value) {
    refreshNoPreview();
    return;
  }
  try {
    applyDetail(await api.getVoucher(editId.value));
  } catch (e) {
    toastError(e);
    router.replace("/vouchers");
  }
}

// ---------- 新建：选图即建卡（一图一凭证，Rust 侧对话框） ----------

const creating = ref(false);

async function createByImage() {
  if (creating.value) return;
  creating.value = true;
  try {
    const summary = await api.pickAndImport(true);
    if (!summary) return; // 用户取消
    reportImport(summary);
    if (summary.vouchers.length) {
      // 显式提示：建卡在选图这一刻完成，即将跳到编辑页显示图片
      toast.info(`已创建 ${summary.vouchers.length} 张凭证，正在打开第一张…`);
      // 多张时进入第一张；其余已在列表中
      router.replace(`/vouchers/${summary.vouchers[0].voucherId}`);
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

// ---------- 编辑：更换图片（换图不换卡，Rust 侧对话框） ----------

async function replaceImage() {
  if (!editId.value || replacing.value) return;
  replacing.value = true;
  try {
    const detail = await api.pickAndReplaceImage(editId.value);
    if (!detail) return; // 用户取消
    applyDetail(detail);
    toast.success("图片已更换（凭证不变）");
  } catch (e) {
    toastError(e);
  } finally {
    replacing.value = false;
  }
}

// ---------- 保存 ----------

async function save() {
  if (!editId.value) return;
  saving.value = true;
  try {
    // 金额严格校验：可选负号、最多两位小数（"12.34.56"/"1e3" 之类的宽松解析一律拒绝）
    const amt = amountYuan.value.trim();
    let cents: number | null = null;
    if (amt !== "") {
      if (!/^-?\d+(\.\d{1,2})?$/.test(amt)) {
        toast.warning("金额格式不正确：应为数字，最多两位小数（如 123.45）");
        return;
      }
      cents = Math.round(parseFloat(amt) * 100);
    }
    const tags = tagsInput.value.split(/[,，、]/).map((t) => t.trim()).filter(Boolean);

    const oldNo = voucherNo.value;
    const saved = await api.updateVoucher(editId.value, {
      voucherDate: voucherDate.value,
      description: description.value,
      counterparty: counterparty.value,
      amountCents: cents,
      clearAmount: cents == null,
      status: status.value,
      tags,
      note: note.value,
    });
    applyForm(saved);
    if (saved.voucherNo !== oldNo) {
      toast.success(`凭证已保存，档号按新日期重排：${oldNo} → ${saved.voucherNo}`);
    } else if (saved.voucherDate !== "" && oldNo.split("-").slice(0, 3).join("-") !== saved.voucherDate) {
      toast.success("凭证已保存（已导出/归档凭证编号锁定，档号未随日期变更）");
    } else {
      toast.success("凭证已保存");
    }
    loadDetail();
  } catch (e) {
    toastError(e);
  } finally {
    saving.value = false;
  }
}

// ---------- OCR（返回整份详情：字段可能被自动回填、附件可能被语义化重命名） ----------

async function runOcr(a: Attachment) {
  ocrBusy.value = true;
  try {
    applyDetail(await api.runOcr(a.id));
    toast.success("识别完成，已自动回填可识别的空字段");
  } catch (e) {
    toastError(e);
  } finally {
    ocrBusy.value = false;
  }
}

const SNAPSHOT_LABELS: Record<string, string> = {
  update: "修改前快照",
  status: "状态变更前快照",
  export: "导出前快照",
  delete: "删除前快照",
  ocr_fill: "OCR 填单前快照",
  replace_image: "换图前快照",
  renumber: "档号重排前快照",
};

// 拖放导入（Rust 侧全局处理）：新建模式下完成后跳到第一张新凭证
let unlistenImportDone: UnlistenFn | null = null;

// 路由参数变化时重载（同一组件实例从「新建 /vouchers/new」跳到「编辑 /vouchers/<id>」
// 时不重挂载，onMounted 只跑一次，必须显式监听 editId 变化）
watch(editId, () => loadDetail());

onMounted(async () => {
  loadDetail();
  unlistenImportDone = await listen<ImportSummary>("import:done", (e) => {
    if (!editId.value && e.payload.vouchers.length) {
      router.replace(`/vouchers/${e.payload.vouchers[0].voucherId}`);
    }
  });
});

onUnmounted(() => unlistenImportDone?.());

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

// ---------- D-1：编辑页缩略图优先用增强版，支持「原图/增强」切换 ----------

/** D-1：单图默认用增强版（如有）；查看器/查看大图也走同一规则 */
function attSrc(a: Attachment): string {
  // enhancedPath 始终指向 5 步矫正后的 JPEG；缺失时回退原图
  const rel = a.enhancedPath ?? a.filePath;
  return convertFileSrc(`${attachmentsDir.value}\\${rel.replace(/\//g, "\\")}`);
}

/** 切换该附件当前显示的图（编辑页用，per-attachment 状态） */
const useEnhanced = ref<Record<number, boolean>>({});
function thumbSrc(a: Attachment): string {
  // 默认 true（优先增强版），用户切换时改 map
  const preferEnhanced = useEnhanced.value[a.id] ?? true;
  const rel = preferEnhanced && a.enhancedPath ? a.enhancedPath : a.filePath;
  return convertFileSrc(`${attachmentsDir.value}\\${rel.replace(/\//g, "\\")}`);
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
  return convertFileSrc(`${attachmentsDir.value}\\${a.filePath.replace(/\//g, "\\")}`);
}
function compareClipRight(a: Attachment): string {
  // 增强版（thumbSrc）只显示左侧，比例 = compareSplit
  const p = (compareSplit.value[a.id] ?? 0.5) * 100;
  return `inset(0 ${100 - p}% 0 0)`;
}
</script>

<template>
  <!-- 新建模式：一图一凭证，先选图 -->
  <div v-if="!editId" class="form-grid">
    <div>
      <div class="fluent-panel">
        <div class="fluent-panel-header"><h3>新建凭证</h3></div>
        <div class="fluent-panel-body">
          <div class="callout info" style="margin: 0 0 16px">
            <span class="ci"><FluentIcon name="image" :size="18" /></span>
            <div class="ct">
              <strong>一张图片就是一张凭证。</strong>
              选择图片后系统会立即建卡 + OCR 识别回填日期 / 金额 / 对方账户（可再修改），然后自动跳到编辑页查看图片与完善字段。
            </div>
          </div>
          <div class="dropzone" style="padding: 48px 24px" @click="createByImage">
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
            <li>核对自动回填的日期、金额、对方账户，补充摘要与标签</li>
            <li>按天导出「凭证日报」PDF 归档</li>
          </ol>
        </div>
      </div>
    </div>
  </div>

  <!-- 编辑模式：单图 + 表单 -->
  <div v-else class="form-grid">
    <div>
      <div class="fluent-panel">
        <div class="fluent-panel-header">
          <h3>编辑凭证</h3>
        </div>
        <div class="fluent-panel-body">
          <div class="field">
            <label>凭证编号</label>
            <input v-model="voucherNo" class="input mono" disabled />
          </div>
          <div class="field">
            <label>凭证日期</label>
            <input v-model="voucherDate" type="date" class="input" />
          </div>
          <div class="field">
            <label>摘要</label>
            <input v-model="description" class="input" placeholder="如：八月办公用品采购" />
          </div>
          <div class="field">
            <label>对方账户</label>
            <input v-model="counterparty" class="input" placeholder="如：文一办公用品店（OCR 可自动识别）" />
          </div>
          <div class="field">
            <label>金额（元）</label>
            <input v-model="amountYuan" class="input" inputmode="decimal" placeholder="如：123.45，可留空" />
          </div>
          <div class="field">
            <label>状态</label>
            <select v-model="status" class="input">
              <option value="draft">草稿</option>
              <option value="reviewed">已复核</option>
              <option value="exported">已导出</option>
              <option value="archived">已归档</option>
            </select>
          </div>
          <div class="field">
            <label>标签</label>
            <input v-model="tagsInput" class="input" placeholder="多个标签用逗号分隔，如：办公, 报销" />
          </div>
          <div class="field">
            <label>备注</label>
            <textarea v-model="note" class="input" rows="3"></textarea>
          </div>
          <div style="display: flex; gap: 8px; justify-content: flex-end">
            <button class="btn" @click="router.push('/vouchers')">返回</button>
            <button class="btn btn-primary" :disabled="saving" @click="save">
              {{ saving ? "保存中…" : "保存" }}
            </button>
          </div>
        </div>
      </div>

      <div class="fluent-panel" v-if="snapshots.length">
        <div class="fluent-panel-header"><h3>修改历史（快照）</h3></div>
        <div class="fluent-panel-body">
          <div v-for="s in snapshots" :key="s.id" class="snapshot-item">
            <div class="sn-meta">
              <span>{{ SNAPSHOT_LABELS[s.reason] ?? s.reason }}</span>
              <span>{{ fmtDate(s.createdAt) }}</span>
            </div>
            <div style="color: var(--text-2)">
              {{ (s.payload as Record<string, unknown>).description || "（无摘要）" }}
            </div>
          </div>
        </div>
      </div>
    </div>

    <div>
      <div class="fluent-panel">
        <div class="fluent-panel-header">
          <h3>凭证图片（{{ attachments.length }}/1）</h3>
        </div>
        <div class="fluent-panel-body">
          <div v-if="attachments.length" class="att-grid">
            <div v-for="a in attachments" :key="a.id" class="att-card">
              <div class="att-img" :class="{ 'is-compare': isCompareOpen(a) }"
                title="点击查看大图（滚轮缩放 / 拖动平移）" @click="!isCompareOpen(a) && openViewer(a)">
                <!-- D-4：对比模式 - 底层原图（始终原图，不受 useEnhanced 影响） + 上层增强版用 clip-path 裁剪 -->
                <template v-if="isCompareOpen(a) && a.enhancedPath">
                  <img class="att-img-compare-base" :src="compareOriginalSrc(a)" :alt="a.fileName + ' (原图)'" />
                  <img class="att-img-compare-top" :src="thumbSrc(a)" :alt="a.fileName + ' (增强版)'"
                    :style="{ clipPath: compareClipRight(a) }" />
                  <!-- 中线 -->
                  <div class="att-compare-divider" :style="{ left: ((compareSplit[a.id] ?? 0.5) * 100) + '%' }"></div>
                  <!-- 滑块手柄 -->
                  <div class="att-compare-handle" :style="{ left: ((compareSplit[a.id] ?? 0.5) * 100) + '%' }">
                    <FluentIcon name="compare" :size="13" />
                  </div>
                  <!-- 角标：左=增强 右=原图 -->
                  <span class="att-compare-tag att-compare-tag-left"><FluentIcon name="sparkle" :size="10" /> 增强</span>
                  <span class="att-compare-tag att-compare-tag-right">原图</span>
                </template>
                <!-- 非对比模式：单图（thumbSrc 受 useEnhanced 切换） -->
                <template v-else>
                  <img :src="thumbSrc(a)" :alt="a.fileName" loading="lazy" />
                </template>
                <!-- v0.6 增强状态角标 -->
                <span v-if="a.enhancedPath && !isCompareOpen(a)" class="att-enhanced-badge" title="已通过 5 步算法矫正（用于 OCR / 查看器）">
                  <FluentIcon name="sparkle" :size="10" /> 已增强
                </span>
                <!-- D-1：原图/增强版切换器（无增强版时隐藏；对比模式下隐藏避免双控制） -->
                <button v-if="a.enhancedPath && !isCompareOpen(a)" class="att-thumb-toggle"
                  :title="(useEnhanced[a.id] ?? true) ? '点击查看原图' : '点击查看增强版'"
                  @click.stop="toggleThumbSource(a)">
                  {{ (useEnhanced[a.id] ?? true) ? "增强" : "原图" }}
                </button>
              </div>
              <div class="att-meta">
                <div class="att-name" :title="a.fileName">{{ a.fileName }}</div>
                <div style="display: flex; gap: 6px; flex-wrap: wrap; align-items: center">
                  <span class="tag" :class="OCR_META[a.ocrStatus].tag">{{ OCR_META[a.ocrStatus].label }}</span>
                  <span v-if="a.enhancedSource === 'screenshot'" class="tag tag-muted" title="截图（增强收益小）">截图</span>
                  <span v-else-if="a.enhancedSource === 'scan'" class="tag tag-info" title="扫描件 / 拍照">扫描件</span>
                </div>
                <div class="att-ops">
                  <button class="btn btn-sm" :disabled="ocrBusy" @click="runOcr(a)">
                    {{ ocrBusy ? "识别中…" : "重新识别" }}
                  </button>
                  <button class="btn btn-sm" :disabled="replacing" @click="replaceImage">
                    {{ replacing ? "更换中…" : "更换图片" }}
                  </button>
                  <!-- D-4：A/B 对比开关（无增强版不可用） -->
                  <button v-if="a.enhancedPath" class="btn btn-sm"
                    :class="{ 'btn-primary': isCompareOpen(a) }"
                    :title="isCompareOpen(a) ? '关闭 A/B 对比' : '打开 A/B 对比：拖动滑块看增强 vs 原图'"
                    @click="isCompareOpen(a) ? closeCompare(a) : openCompare(a)">
                    <FluentIcon :name="isCompareOpen(a) ? 'close' : 'compare'" :size="14" />
                    {{ isCompareOpen(a) ? "关闭对比" : "A/B 对比" }}
                  </button>
                </div>
                <!-- D-4：对比模式滑块行（仅打开时显示） -->
                <div v-if="isCompareOpen(a) && a.enhancedPath" class="att-compare-row">
                  <span class="att-compare-label"><FluentIcon name="sparkle" :size="10" /> 增强</span>
                  <input type="range" min="0" max="100" step="1"
                    class="att-compare-slider"
                    :value="Math.round((compareSplit[a.id] ?? 0.5) * 100)"
                    @input="setSplit(a, +($event.target as HTMLInputElement).value / 100)"
                    title="拖动调整：左=增强 / 右=原图" />
                  <span class="att-compare-label">原图</span>
                  <span class="att-compare-pct">{{ Math.round((compareSplit[a.id] ?? 0.5) * 100) }}%</span>
                </div>
                <div v-if="a.ocrText" class="ocr-text">{{ a.ocrText }}</div>
              </div>
            </div>
          </div>
          <div v-else class="dropzone" @click="replaceImage">
            <div class="dz-icon"><FluentIcon name="image" :size="32" /></div>
            <div style="font-weight: 600">点击选择图片补挂到这张凭证</div>
            <div style="font-size: 12px; margin-top: 4px">一图一凭证：每张凭证只关联一张图片</div>
          </div>
        </div>
      </div>

      <!-- v0.7 S-2：OCR 识别结果（结构化提取预览，一键填入表单） -->
      <div class="fluent-panel" v-if="hasExtracted || fieldsBusy">
        <div class="fluent-panel-header">
          <h3>识别结果</h3>
          <button class="btn btn-sm btn-primary" v-if="hasExtracted" @click="applyAllExtracted"><FluentIcon name="flash" :size="14" />全部填入</button>
        </div>
        <div class="fluent-panel-body">
          <div v-if="fieldsBusy" style="font-size: 13px; color: var(--text-2)">正在提取字段…</div>
          <template v-else-if="extracted">
            <div class="ext-row">
              <span class="ext-label">日期</span>
              <span class="ext-value mono">
                {{ extracted.date ?? "—" }}
                <ConfidenceTag :level="extracted.confidence?.date ?? null" />
              </span>
              <button v-if="extracted.date" class="btn btn-sm" @click="applyExtractedField('date')">填入</button>
            </div>
            <div class="ext-row">
              <span class="ext-label">金额</span>
              <span class="ext-value mono">
                {{ extracted.amountCents != null ? `¥${(extracted.amountCents / 100).toFixed(2)}` : "—" }}
                <!-- 置信度：标签命中可信 / 全文唯一兜底低置信 -->
                <span v-if="extracted.amountSource === 'labeled'" class="tag tag-success" title="命中「价税合计/合计」等标签">可信</span>
                <span v-else-if="extracted.amountSource === 'uniqueonly'" class="tag tag-warning" title="全文唯一金额兜底，无标签佐证，请人工核对">低置信</span>
              </span>
              <button v-if="extracted.amountCents != null" class="btn btn-sm" @click="applyExtractedField('amount')">填入</button>
            </div>
            <div class="ext-row">
              <span class="ext-label">对方账户</span>
              <span class="ext-value">
                {{ extracted.counterparty ?? "—" }}
                <ConfidenceTag :level="extracted.confidence?.counterparty ?? null" />
              </span>
              <button v-if="extracted.counterparty" class="btn btn-sm" @click="applyExtractedField('counterparty')">填入</button>
            </div>
            <div class="ext-row">
              <span class="ext-label">商家</span>
              <span class="ext-value">
                {{ extracted.merchant ?? "—" }}
                <ConfidenceTag :level="extracted.confidence?.merchant ?? null" />
              </span>
              <button v-if="extracted.merchant" class="btn btn-sm" title="商家名作为摘要填入" @click="applyExtractedField('merchant')">填入摘要</button>
            </div>
            <div class="ext-tip">提取自 OCR 识别文本；「填入」只改表单值，点「保存」才落库。</div>
          </template>
        </div>
      </div>
    </div>
  </div>

  <ImageViewerDialog :visible="viewerVisible" :src="viewerSrc" :file-name="viewerName"
    :attachment-id="viewerAttachmentId" @close="viewerVisible = false" />
</template>

<style scoped>
/* v0.7 S-2：识别结果卡片行（label / value / 填入按钮三段式） */
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
