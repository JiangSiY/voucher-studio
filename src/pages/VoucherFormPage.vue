<script setup lang="ts">
/** 凭证编辑页（新建/编辑）：表单 + 附件管理（缩略图/OCR/删除）+ 快照历史 */
import { computed, onMounted, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, type Attachment, type VoucherSnapshot } from "../api/client";
import ConfirmDialog from "../components/ConfirmDialog.vue";
import { OCR_META, fmtDate } from "../composables/format";
import { pickImages, useDropImport } from "../composables/useDropImport";
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
const amountYuan = ref<string>("");
const status = ref<"draft" | "reviewed" | "exported" | "archived">("draft");
const tagsInput = ref("");
const note = ref("");

const attachments = ref<Attachment[]>([]);
const snapshots = ref<VoucherSnapshot[]>([]);
const saving = ref(false);
const ocrBusy = ref<number | null>(null);

// 删除附件确认
const confirmVisible = ref(false);
let pendingAttachment: Attachment | null = null;

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

async function loadDetail() {
  if (!editId.value) {
    refreshNoPreview();
    return;
  }
  try {
    const detail = await api.getVoucher(editId.value);
    const v = detail.voucher;
    voucherNo.value = v.voucherNo;
    voucherDate.value = v.voucherDate;
    description.value = v.description;
    amountYuan.value = v.amountCents != null ? (v.amountCents / 100).toFixed(2) : "";
    status.value = v.status;
    tagsInput.value = v.tags.join(", ");
    note.value = v.note;
    attachments.value = detail.attachments;
    snapshots.value = detail.snapshots;
  } catch (e) {
    toastError(e);
    router.replace("/vouchers");
  }
}

async function importToVoucher() {
  const files = await pickImages();
  if (!files?.length) return;
  try {
    const summary = await api.importImages(files, editId.value, false);
    if (summary.imported.length) toast.success(`已附加 ${summary.imported.length} 张图片`);
    if (summary.skippedDuplicates.length) toast.info(`跳过重复图片 ${summary.skippedDuplicates.length} 张`);
    if (!editId.value && summary.voucherId) {
      router.replace(`/vouchers/${summary.voucherId}`);
    } else {
      loadDetail();
    }
  } catch (e) {
    toastError(e);
  }
}

const { dragging, bind } = useDropImport(() => loadDetail(), editId.value);

async function save() {
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

    if (editId.value) {
      await api.updateVoucher(editId.value, {
        voucherNo: voucherNo.value.trim() || undefined,
        voucherDate: voucherDate.value,
        description: description.value,
        amountCents: cents,
        clearAmount: cents == null,
        status: status.value,
        tags,
        note: note.value,
      });
      toast.success("凭证已保存");
    } else {
      const created = await api.createVoucher({
        voucherNo: voucherNo.value.trim(),
        voucherDate: voucherDate.value,
        description: description.value,
        amountCents: cents,
        status: status.value,
        tags,
        note: note.value,
      });
      toast.success(`凭证 ${created.voucherNo} 已创建`);
      router.replace(`/vouchers/${created.id}`);
    }
    loadDetail();
  } catch (e) {
    toastError(e);
  } finally {
    saving.value = false;
  }
}

async function runOcr(a: Attachment) {
  ocrBusy.value = a.id;
  try {
    const updated = await api.runOcr(a.id);
    attachments.value = attachments.value.map((x) => (x.id === updated.id ? updated : x));
    toast.success("识别完成");
  } catch (e) {
    toastError(e);
  } finally {
    ocrBusy.value = null;
  }
}

function askDeleteAttachment(a: Attachment) {
  pendingAttachment = a;
  confirmVisible.value = true;
}

async function doDeleteAttachment() {
  confirmVisible.value = false;
  if (!pendingAttachment) return;
  try {
    await api.deleteAttachment(pendingAttachment.id);
    attachments.value = attachments.value.filter((x) => x.id !== pendingAttachment!.id);
    toast.success("附件已删除");
  } catch (e) {
    toastError(e);
  }
}

onMounted(() => {
  bind();
  loadDetail();
});
</script>

<template>
  <div class="form-grid">
    <div>
      <div class="fluent-panel">
        <div class="fluent-panel-header">
          <h3>{{ editId ? "编辑凭证" : "新建凭证" }}</h3>
        </div>
        <div class="fluent-panel-body">
          <div class="field">
            <label>凭证编号</label>
            <input v-model="voucherNo" class="input mono" :placeholder="editId ? '' : noPreview || '自动生成'"
              :disabled="!!editId" />
            <div class="desc" v-if="!editId">留空自动生成：{{ noPreview || "…" }}（规则可在设置中修改）</div>
          </div>
          <div class="field">
            <label>凭证日期</label>
            <input v-model="voucherDate" type="date" class="input" @change="refreshNoPreview" />
          </div>
          <div class="field">
            <label>摘要</label>
            <input v-model="description" class="input" placeholder="如：八月办公用品采购" />
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
              <span>{{ s.reason === "update" ? "修改前快照" : s.reason === "status" ? "状态变更前快照" : s.reason === "export" ? "导出前快照" : s.reason === "delete" ? "删除前快照" : s.reason }}</span>
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
          <h3>附件图片（{{ attachments.length }}）</h3>
          <button class="btn btn-sm" @click="importToVoucher">＋ 添加图片</button>
        </div>
        <div class="fluent-panel-body">
          <div v-if="attachments.length" class="att-grid">
            <div v-for="a in attachments" :key="a.id" class="att-card">
              <div class="att-img">
                <img :src="attSrc(a)" :alt="a.fileName" loading="lazy" />
              </div>
              <div class="att-meta">
                <div class="att-name" :title="a.fileName">{{ a.fileName }}</div>
                <span class="tag" :class="OCR_META[a.ocrStatus].tag">{{ OCR_META[a.ocrStatus].label }}</span>
                <div class="att-ops">
                  <button class="btn btn-sm" :disabled="ocrBusy === a.id" @click="runOcr(a)">
                    {{ ocrBusy === a.id ? "识别中…" : "识别" }}
                  </button>
                  <button class="btn btn-sm btn-danger" @click="askDeleteAttachment(a)">删除</button>
                </div>
                <div v-if="a.ocrText" class="ocr-text">{{ a.ocrText }}</div>
              </div>
            </div>
          </div>
          <div v-else class="dropzone" @click="importToVoucher">
            <div class="dz-icon">🖼</div>
            <div style="font-weight: 600">拖放图片到这里，或点击选择</div>
            <div style="font-size: 12px; margin-top: 4px">支持 jpg / png / bmp / webp，相同图片自动去重</div>
          </div>
        </div>
      </div>
    </div>
  </div>

  <div v-if="dragging" class="drop-overlay">松手附加图片到{{ editId ? "当前凭证" : "新凭证" }}</div>

  <ConfirmDialog :visible="confirmVisible" title="删除附件" body="确定删除这张附件图片吗？（磁盘文件将一并移除）"
    confirm-text="删除" @confirm="doDeleteAttachment" @cancel="confirmVisible = false" />
</template>
