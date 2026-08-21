<script setup lang="ts">
/**
 * 凭证编辑页。
 * 一图一凭证：新建 = 先选一张图片（导入即建卡 + OCR 自动填单），编辑 = 单图 + 更换图片（换图不换卡）。
 * 表单字段：编号/日期/摘要/对方账户/金额/状态/标签/备注 + 快照历史。
 * 拖放导入在 Rust 侧全局处理（App.vue 反馈）；本页新建模式监听完成事件跳转到新凭证。
 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, type Attachment, type ImportSummary, type Voucher, type VoucherSnapshot } from "../api/client";
import ImageViewerDialog from "../components/ImageViewerDialog.vue";
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

function attSrc(a: Attachment): string {
  return convertFileSrc(`${attachmentsDir.value}\\${a.filePath.replace(/\//g, "\\")}`);
}

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
</script>

<template>
  <!-- 新建模式：一图一凭证，先选图 -->
  <div v-if="!editId" class="form-grid">
    <div>
      <div class="fluent-panel">
        <div class="fluent-panel-header"><h3>新建凭证</h3></div>
        <div class="fluent-panel-body">
          <div class="callout info" style="margin: 0 0 16px">
            <span class="ci">🖼</span>
            <div class="ct">
              <strong>一张图片就是一张凭证。</strong>
              选择图片即自动建卡，并立即 OCR 识别回填日期 / 金额 / 对方账户（可再修改）。
            </div>
          </div>
          <div class="dropzone" style="padding: 48px 24px" @click="createByImage">
            <div class="dz-icon">🖼</div>
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
            <li>选择 / 拖放 / 微信监听导入图片</li>
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
              <div class="att-img" title="点击查看大图（滚轮缩放 / 拖动平移）" @click="openViewer(a)">
                <img :src="attSrc(a)" :alt="a.fileName" loading="lazy" />
              </div>
              <div class="att-meta">
                <div class="att-name" :title="a.fileName">{{ a.fileName }}</div>
                <span class="tag" :class="OCR_META[a.ocrStatus].tag">{{ OCR_META[a.ocrStatus].label }}</span>
                <div class="att-ops">
                  <button class="btn btn-sm" :disabled="ocrBusy" @click="runOcr(a)">
                    {{ ocrBusy ? "识别中…" : "重新识别" }}
                  </button>
                  <button class="btn btn-sm" :disabled="replacing" @click="replaceImage">
                    {{ replacing ? "更换中…" : "更换图片" }}
                  </button>
                </div>
                <div v-if="a.ocrText" class="ocr-text">{{ a.ocrText }}</div>
              </div>
            </div>
          </div>
          <div v-else class="dropzone" @click="replaceImage">
            <div class="dz-icon">🖼</div>
            <div style="font-weight: 600">点击选择图片补挂到这张凭证</div>
            <div style="font-size: 12px; margin-top: 4px">一图一凭证：每张凭证只关联一张图片</div>
          </div>
        </div>
      </div>
    </div>
  </div>

  <ImageViewerDialog :visible="viewerVisible" :src="viewerSrc" :file-name="viewerName"
    :attachment-id="viewerAttachmentId" @close="viewerVisible = false" />
</template>
