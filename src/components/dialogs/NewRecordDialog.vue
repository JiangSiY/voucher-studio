<script setup lang="ts">
/**
 * v0.27：新建条目（数据为主、凭证必填）。
 *
 * 顺序刻意是「先填条目字段 → 再挂凭证」：凭证是必填属性，不是入口。
 * 网页端走 `/api/upload?mode=stage`，桌面端走 `pick_image_paths`（复制到 temp/uploads），
 * 最终都由 `create_record_with_attachment` 一条写路径落库。
 */
import { computed, reactive, ref, watch } from "vue";
import Dialog from "../ui/Dialog.vue";
import FluentIcon from "../FluentIcon.vue";
import { api, type NewVoucherInput, type Voucher } from "../../api/client";
import { isTauri } from "../../api/env";
import { parseYuanToCents } from "../../composables/money";
import { toast, toastError } from "../../stores/toast";

const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{ close: []; saved: [v: Voucher] }>();

const form = reactive({
  date: new Date().toISOString().slice(0, 10),
  counterparty: "",
  amount: "",
  description: "",
  category: "",
});
const paths = ref<string[]>([]);
const fileName = ref("");
const preview = ref("");
const busy = ref(false);
const fileInput = ref<HTMLInputElement | null>(null);

const canSave = computed(() => paths.value.length > 0 && !!form.date && !busy.value);

watch(
  () => props.open,
  (open) => {
    if (!open) reset();
  },
);

function reset() {
  form.date = new Date().toISOString().slice(0, 10);
  form.counterparty = "";
  form.amount = "";
  form.description = "";
  form.category = "";
  paths.value = [];
  fileName.value = "";
  if (preview.value) URL.revokeObjectURL(preview.value);
  preview.value = "";
  busy.value = false;
}

function close() {
  if (!busy.value) emit("close");
}

async function pick() {
  if (isTauri) {
    const picked = await api.pickImagePaths();
    if (picked?.length) {
      paths.value = picked.slice(0, 1);
      fileName.value = picked[0].split(/[\\/]/).pop() ?? "凭证";
    }
    return;
  }
  fileInput.value?.click();
}

async function onFile(e: Event) {
  const input = e.target as HTMLInputElement;
  const file = input.files?.[0];
  input.value = "";
  if (!file) return;
  fileName.value = file.name;
  if (preview.value) URL.revokeObjectURL(preview.value);
  preview.value = file.type.startsWith("image/") ? URL.createObjectURL(file) : "";
  busy.value = true;
  try {
    paths.value = await api.uploadStage([file]);
    if (!paths.value.length) throw new Error("上传失败，请重试");
  } catch (err) {
    paths.value = [];
    toastError(err);
  } finally {
    busy.value = false;
  }
}

async function save() {
  if (!paths.value.length) {
    toast.warning("请先选择凭证图片：凭证是必填项");
    return;
  }
  const parsed = parseYuanToCents(form.amount);
  if (!parsed.ok) {
    toast.warning(parsed.reason);
    return;
  }
  const payload: NewVoucherInput = {
    voucherDate: form.date,
    counterparty: form.counterparty.trim(),
    description: form.description.trim(),
    amountCents: parsed.cents,
    tags: form.category.trim() ? [form.category.trim()] : [],
  };
  busy.value = true;
  try {
    const detail = await api.createRecordWithAttachment(payload, paths.value, true);
    toast.success(`已新建 ${detail.voucher.voucherNo}`);
    emit("saved", detail.voucher);
    emit("close");
  } catch (e) {
    toastError(e);
  } finally {
    busy.value = false;
  }
}
</script>

<template>
  <Dialog :open="open" title="新建条目" narrow @close="close">
    <div class="new-record">
      <label class="data-field">
        <span>日期</span>
        <input v-model="form.date" class="input" type="date" />
      </label>
      <label class="data-field">
        <span>客户</span>
        <input v-model="form.counterparty" class="input" placeholder="对方名称" />
      </label>
      <label class="data-field">
        <span>金额（元）</span>
        <input v-model="form.amount" class="input" inputmode="decimal" placeholder="可留空" />
      </label>
      <label class="data-field">
        <span>摘要</span>
        <input v-model="form.description" class="input" placeholder="这笔是什么" />
      </label>
      <label class="data-field">
        <span>类别</span>
        <input v-model="form.category" class="input" placeholder="如：组件 / 服务" />
      </label>

      <div class="new-record-voucher">
        <div class="new-record-label">凭证图片 <span class="new-record-required">必填</span></div>
        <div v-if="preview" class="new-record-preview">
          <img :src="preview" alt="凭证预览" />
        </div>
        <div v-else-if="fileName" class="new-record-file">
          <FluentIcon name="file" :size="16" />
          <span>{{ fileName }}</span>
        </div>
        <div v-else class="new-record-empty">
          <FluentIcon name="image" :size="18" />
          <span>选择一张凭证图片；没有凭证不能入账 / 导出</span>
        </div>
        <button class="btn" type="button" :disabled="busy" @click="pick">
          <FluentIcon name="upload" :size="14" />
          <span>{{ paths.length ? "重新选择" : "选择凭证" }}</span>
        </button>
        <input
          ref="fileInput"
          class="sr-only"
          type="file"
          accept="image/*,application/pdf"
          @change="onFile"
        />
      </div>
    </div>

    <template #foot>
      <button class="btn" type="button" :disabled="busy" @click="close">取消</button>
      <button class="btn btn-primary" type="button" :disabled="!canSave" @click="save">
        {{ busy ? "保存中…" : "保存条目" }}
      </button>
    </template>
  </Dialog>
</template>

<style scoped>
.new-record {
  display: grid;
  gap: var(--s-3);
}

.new-record-voucher {
  display: grid;
  gap: var(--s-2);
  padding-top: var(--s-2);
  border-top: 1px solid var(--separator);
}

.new-record-label {
  font-size: var(--fs-caption);
  color: var(--text-2);
}

.new-record-required {
  color: var(--accent-strong);
}

.new-record-preview img {
  width: 100%;
  max-height: 220px;
  object-fit: contain;
  border-radius: var(--radius-control);
  background: var(--bg-card-sunken);
}

.new-record-file,
.new-record-empty {
  display: flex;
  align-items: center;
  gap: var(--s-2);
  min-height: 88px;
  padding: var(--s-3);
  border: 1px dashed var(--separator-strong);
  border-radius: var(--radius-control);
  color: var(--text-2);
  font-size: var(--fs-caption);
}
</style>
