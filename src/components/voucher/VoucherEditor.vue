<script setup lang="ts">
/**
 * 凭证编辑器（v0.19 从「编辑页」提取）—— 全应用**唯一**的凭证编辑界面。
 *
 * 背景（2026-09-28 用户决定）：原 `/vouchers/:id` 独立编辑页与资料库右栏「便捷修改」是两套
 * 实现，同字段两处可改 → 规则容易分叉、且"点进去/返回/重新筛选"丢上下文。
 * 现合并为：**编辑一律在资料库右栏**。本组件承载原编辑页的全部能力：
 * 7 字段（编号只读 / 日期 / 摘要 / 对方 / 金额 / 状态 / 标签 / 备注）、换图、重跑识别、
 * 裁剪、识别提取填入、修改历史、封账提示、疑似重复提示、⌘S / ⌘Enter 快捷键。
 * 新建向导仍保留独立页（`/vouchers/new`，见 VoucherFormPage）。
 *
 * 与宿主（LibraryPage）的分工：
 * - 宿主负责列表、选中、翻张与"下一张"落点；本组件只管**这一张**的编辑与保存；
 * - `detail` 由宿主传入（含附件/快照）；OCR、换图、裁剪会改库，组件把新详情 `emit("detail")`
 *   交由宿主更新，同时用**逐字段合并**刷新表单：用户已改的字段保留、未改的取新值
 *   （旧编辑页是无条件覆盖，会把用户正在输入的改动冲掉）。
 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useRouter } from "vue-router";
import { api, type Attachment, type ExtractedFields, type Voucher, type VoucherDetail } from "../../api/client";
import AttachmentPanel from "./AttachmentPanel.vue";
import DescriptionInput from "./DescriptionInput.vue";
import SnapshotPanel from "./SnapshotPanel.vue";
import OcrResultCard from "../OcrResultCard.vue";
import FluentIcon from "../FluentIcon.vue";
import { centsToYuanInput, parseTags, parseYuanToCents } from "../../composables/money";
import { pairKey } from "../../composables/dupReview";
import { useConfigStore } from "../../stores/config";
import { useDupStore } from "../../stores/dup";
import { toast, toastError } from "../../stores/toast";

const props = withDefaults(
  defineProps<{
    voucher: Voucher;
    detail: VoucherDetail | null;
    /** 紧凑布局（窄栏）：标签在左、控件在右 */
    dense?: boolean;
  }>(),
  { dense: false },
);

const emit = defineEmits<{
  /** 保存成功（宿主据此更新列表行与检视器） */
  saved: [v: Voucher];
  /** 附件/识别/裁剪等操作后的最新详情（宿主更新 detail） */
  detail: [d: VoucherDetail];
  /** 保存并下一张（宿主决定下一张是谁） */
  next: [];
}>();

const router = useRouter();
const configStore = useConfigStore();
const dupStore = useDupStore();

type FormState = {
  date: string;
  description: string;
  counterparty: string;
  amount: string;
  status: Voucher["status"];
  tags: string;
  note: string;
};

/** 当前基线（DB 侧状态）：脏判定与合并都以它为准 */
const baseline = ref<Voucher>(props.voucher);
const form = ref<FormState>(toForm(props.voucher));

function toForm(v: Voucher): FormState {
  return {
    date: v.voucherDate,
    description: v.description ?? "",
    counterparty: v.counterparty ?? "",
    amount: centsToYuanInput(v.amountCents),
    status: v.status,
    tags: (v.tags ?? []).join("、"),
    note: v.note ?? "",
  };
}

/** 金额按"分"比较，避免 1.10 与 1.1 这类文本差异被误判为脏 */
function amountCentsOf(text: string): { ok: boolean; cents: number | null } {
  const parsed = parseYuanToCents(text);
  return parsed.ok ? { ok: true, cents: parsed.cents } : { ok: false, cents: null };
}

/**
 * 合并外部刷新：逐字段判断"用户是否改过"（当前值 ≠ 旧基线值），
 * 改过的保留、没改的取新值——OCR/换图/裁剪回填时不会冲掉用户正在输入的内容。
 */
function mergeFrom(v: Voucher) {
  const prev = baseline.value;
  const cur = form.value;
  const fresh = toForm(v);
  const keep = <K extends keyof FormState>(key: K, prevText: string): FormState[K] =>
    (cur[key] as string) !== prevText ? cur[key] : fresh[key];
  const prevAmount = amountCentsOf(centsToYuanInput(prev.amountCents));
  form.value = {
    date: keep("date", prev.voucherDate),
    description: keep("description", prev.description ?? ""),
    counterparty: keep("counterparty", prev.counterparty ?? ""),
    amount: amountCentsOf(cur.amount).cents !== prevAmount.cents ? cur.amount : fresh.amount,
    status: cur.status !== prev.status ? cur.status : fresh.status,
    tags: keep("tags", (prev.tags ?? []).join("、")),
    note: keep("note", prev.note ?? ""),
  };
  baseline.value = v;
}

/** 强制覆盖（首次加载 / 切换凭证 / 保存成功后的规范化） */
function resetFrom(v: Voucher) {
  baseline.value = v;
  form.value = toForm(v);
}

const dirty = computed(() => {
  const b = baseline.value;
  const f = form.value;
  return (
    f.date !== b.voucherDate ||
    f.description.trim() !== (b.description ?? "") ||
    f.counterparty.trim() !== (b.counterparty ?? "") ||
    amountCentsOf(f.amount).cents !== b.amountCents ||
    f.status !== b.status ||
    parseTags(f.tags).join("、") !== (b.tags ?? []).join("、") ||
    f.note.trim() !== (b.note ?? "")
  );
});

/** 切换凭证：无条件重置（否则会把上一张的输入带过来） */
watch(
  () => props.voucher.id,
  () => resetFrom(props.voucher),
);
/** 同一张的 props 更新（保存回写/外部刷新）：走合并 */
watch(
  () => props.voucher,
  (v) => {
    if (v.id !== baseline.value.id) return;
    if (v.updatedAt !== baseline.value.updatedAt) mergeFrom(v);
  },
);

const descTemplates = computed<string[]>(() => configStore.config?.voucher?.descriptionTemplates ?? []);

// ---------- 期间锁（封账月不可改） ----------

const lockedPeriod = ref<string | null>(null);

async function loadLockState() {
  try {
    const locks = await api.listPeriodLocks();
    const period = form.value.date.slice(0, 7);
    lockedPeriod.value = locks.some((l) => l.period === period) ? period : null;
  } catch {
    lockedPeriod.value = null; // 辅助态，失败不打扰
  }
}

watch(() => form.value.date, () => loadLockState());
watch(() => props.voucher.id, () => loadLockState());

// ---------- 保存 ----------

const saving = ref(false);

/** 校验并保存；失败/无改动返回 null */
async function save(): Promise<Voucher | null> {
  if (saving.value) return null;
  if (!dirty.value) {
    toast.info("没有需要保存的修改");
    return null;
  }
  const parsed = parseYuanToCents(form.value.amount);
  if (!parsed.ok) {
    toast.warning(parsed.reason);
    return null;
  }
  saving.value = true;
  try {
    const oldNo = baseline.value.voucherNo;
    const saved = await api.updateVoucher(props.voucher.id, {
      voucherDate: form.value.date,
      description: form.value.description.trim(),
      counterparty: form.value.counterparty.trim(),
      amountCents: parsed.cents,
      clearAmount: parsed.cents == null,
      status: form.value.status,
      tags: parseTags(form.value.tags),
      note: form.value.note.trim(),
    });
    resetFrom(saved);
    if (saved.voucherNo !== oldNo) {
      toast.success(`已保存，档号按新日期重排：${oldNo} → ${saved.voucherNo}`);
    } else {
      toast.success("已保存修改");
    }
    emit("saved", saved);
    checkDuplicateHint(saved);
    return saved;
  } catch (e) {
    toastError(e);
    return null;
  } finally {
    saving.value = false;
  }
}

/** 疑似重复提示（与旧编辑页同口径）：金额为空不检测；已判定的对由后端排除；
 *  同一对本次会话只提示一次；toast 带「去核查」直达核查页并定位到这一对。 */
async function checkDuplicateHint(v: Voucher) {
  if (v.amountCents == null) return;
  try {
    const dup = await api.checkVoucherDuplicates(v.id);
    const fresh = dup.filter((d) => !dupStore.wasNotified(pairKey(v.id, d.voucher.id)));
    if (!fresh.length) return;
    fresh.forEach((d) => dupStore.markNotified(pairKey(v.id, d.voucher.id)));
    const names = fresh.map((d) => d.voucher.voucherNo).join("、");
    const strongest = fresh[0].evidence.level === "high" ? "（证据较强，建议优先核对）" : "";
    const first = fresh[0];
    toast.warning(
      `疑似重复：与凭证 ${names} 同日期同金额（${v.voucherDate} · ¥${((v.amountCents ?? 0) / 100).toFixed(2)}），请核对是否重复报销${strongest}`,
      {
        label: "去核查",
        onClick: () => router.push({ path: "/duplicates", query: { pair: pairKey(v.id, first.voucher.id) } }),
      },
    );
  } catch {
    // 提示是辅助能力，失败不打扰主流程
  }
}

/** 保存并下一张：无改动也直接翻张（连续核对场景字段本来就对） */
async function saveAndNext() {
  if (dirty.value) {
    const saved = await save();
    if (!saved) return;
  }
  emit("next");
}

// ---------- 图片区：重跑识别 / 换图 / 裁剪后刷新 ----------

const ocrBusy = ref(false);
const replacing = ref(false);

async function runOcr(a: Attachment) {
  ocrBusy.value = true;
  try {
    const detail = await api.runOcr(a.id);
    mergeFrom(detail.voucher);
    emit("detail", detail);
    toast.success("识别完成，已自动回填可识别的空字段");
  } catch (e) {
    toastError(e);
  } finally {
    ocrBusy.value = false;
  }
}

async function reRunOcrAfterCrop() {
  const a = props.detail?.attachments[0];
  if (!a) return;
  await runOcr(a);
}

async function replaceImage() {
  if (replacing.value) return;
  replacing.value = true;
  try {
    const detail = await api.pickAndReplaceImage(props.voucher.id);
    if (!detail) return; // 用户取消
    mergeFrom(detail.voucher);
    emit("detail", detail);
    toast.success("图片已更换（凭证不变）");
  } catch (e) {
    toastError(e);
  } finally {
    replacing.value = false;
  }
}

async function onCropSaved() {
  const detail = await api.getVoucher(props.voucher.id);
  mergeFrom(detail.voucher);
  emit("detail", detail);
  toast.success("图片已调整（覆盖增强版；原图不变）", {
    label: "重新识别",
    onClick: () => void reRunOcrAfterCrop(),
  });
}

// ---------- 识别提取预览（只读，不写库） ----------

const extracted = ref<ExtractedFields | null>(null);
const fieldsBusy = ref(false);

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

watch(
  () => props.detail?.attachments,
  (list) => void loadExtracted(list?.[0]),
);

function applyExtractedField(field: "date" | "amount" | "counterparty" | "merchant") {
  const f = extracted.value;
  if (!f) return;
  if (field === "date" && f.date) form.value.date = f.date;
  else if (field === "amount" && f.amountCents != null) form.value.amount = (f.amountCents / 100).toFixed(2);
  else if (field === "counterparty" && f.counterparty) form.value.counterparty = f.counterparty;
  else if (field === "merchant" && f.merchant) form.value.description = f.merchant;
}

/** 全部填入：金额仅"标签命中"自动填；低置信字段留待用户单点（与旧编辑页同策略） */
function applyAllExtracted() {
  const f = extracted.value;
  if (!f) return;
  const low = (c: "high" | "medium" | "low" | null) => c === "low";
  if (f.date) form.value.date = f.date;
  const amountTrusted = f.amountCents != null && f.amountSource === "labeled" && !low(f.confidence?.amount);
  if (amountTrusted) form.value.amount = (f.amountCents! / 100).toFixed(2);
  if (f.counterparty && !low(f.confidence?.counterparty)) form.value.counterparty = f.counterparty;
  if (f.merchant && !low(f.confidence?.merchant)) form.value.description = f.merchant;
  const skipped: string[] = [];
  if (f.amountCents != null && !amountTrusted) skipped.push("金额");
  if (f.counterparty && low(f.confidence?.counterparty)) skipped.push("对方账户");
  if (f.merchant && low(f.confidence?.merchant)) skipped.push("商家");
  toast.info(`已填入表单，核对后点「保存」生效${skipped.length ? `（${skipped.join("、")}为低置信未自动填，可单独点「填」）` : ""}`);
}

// ---------- 键盘动线（与旧编辑页一致；组件只在选中某张时挂载） ----------

function onKeydown(e: KeyboardEvent) {
  const mod = e.ctrlKey || e.metaKey;
  if (mod && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "s") {
    e.preventDefault();
    void save();
  } else if (mod && e.key === "Enter") {
    e.preventDefault();
    void saveAndNext();
  }
}

const dateEl = ref<HTMLInputElement | null>(null);
/** 供宿主在"展开编辑区"后把焦点送进第一个可编辑字段 */
defineExpose({ focusFirst: () => dateEl.value?.focus() });

onMounted(() => {
  window.addEventListener("keydown", onKeydown);
  loadLockState();
  void loadExtracted(props.detail?.attachments?.[0]);
});

onUnmounted(() => window.removeEventListener("keydown", onKeydown));
</script>

<template>
  <div class="editor" :class="{ 'editor-dense': dense }">
    <!-- 图片区与操作：换图 / 重跑识别 / 裁剪 / 大图查看（原编辑页能力） -->
    <AttachmentPanel
      v-if="detail"
      :attachments="detail.attachments"
      :ocr-busy="ocrBusy"
      :replacing="replacing"
      :locked-period="lockedPeriod"
      @run-ocr="runOcr"
      @replace-image="replaceImage"
      @crop-saved="onCropSaved"
    />

    <!-- 封账月提示（保存被禁用；查看/导出不受限） -->
    <div class="callout warning" v-if="lockedPeriod">
      <span class="ci"><FluentIcon name="lock" :size="18" /></span>
      <div class="ct">
        <strong>{{ lockedPeriod }} 已封账，该凭证不可修改。</strong>
        如确需调整，请先到「设置 → 凭证参数 → 期间管理」解锁（解锁意味着该月数据可能需要重新导出）。
      </div>
    </div>

    <div class="editor-form">
      <label class="ed-row" data-field="no">
        <span>凭证编号</span>
        <input class="input mono" :value="voucher.voucherNo" disabled />
      </label>
      <label class="ed-row" data-field="date">
        <span>凭证日期</span>
        <input ref="dateEl" v-model="form.date" type="date" class="input" />
      </label>
      <div class="ed-row" data-field="description">
        <span>摘要</span>
        <div class="ed-grow">
          <DescriptionInput v-model="form.description" :templates="descTemplates" />
        </div>
      </div>
      <label class="ed-row" data-field="counterparty">
        <span>对方账户</span>
        <input v-model="form.counterparty" class="input" placeholder="如：文一办公用品店" />
      </label>
      <label class="ed-row" data-field="amount">
        <span>金额（元）</span>
        <input v-model="form.amount" class="input" inputmode="decimal" placeholder="如：123.45，可留空" />
      </label>
      <label class="ed-row" data-field="status">
        <span>状态</span>
        <select v-model="form.status" class="input">
          <option value="draft">草稿</option>
          <option value="reviewed">已复核</option>
          <option value="exported">已导出</option>
          <option value="archived">已归档</option>
        </select>
      </label>
      <label class="ed-row" data-field="tags">
        <span>标签</span>
        <input v-model="form.tags" class="input" placeholder="多个标签用、或,分隔" />
      </label>
      <label class="ed-row" data-field="note">
        <span>备注</span>
        <textarea v-model="form.note" class="input" rows="3"></textarea>
      </label>
    </div>

    <div class="ed-ops">
      <span v-if="dirty && lockedPeriod" class="dirty-hint">● 已封账</span>
      <span v-else-if="dirty" class="dirty-hint">● 未保存</span>
      <span class="flex-1"></span>
      <button class="btn" :disabled="!dirty || saving" @click="resetFrom(voucher)">还原</button>
      <button class="btn" :disabled="saving || !!lockedPeriod" title="Ctrl/⌘+S" @click="save()">
        {{ saving ? "保存中…" : "保存" }}
      </button>
      <button class="btn btn-primary" :disabled="saving || !!lockedPeriod" title="Ctrl/⌘+Enter"
        @click="saveAndNext()">
        {{ saving ? "保存中…" : "保存并下一张" }}
      </button>
    </div>

    <!-- 识别提取结果（一键填入表单；落库仍由「保存」决定） -->
    <OcrResultCard :extracted="extracted" :fields-busy="fieldsBusy"
      @apply-field="applyExtractedField" @apply-all="applyAllExtracted" />

    <SnapshotPanel v-if="detail" :snapshots="detail.snapshots" />
  </div>
</template>

<style scoped>
.editor { display: flex; flex-direction: column; gap: var(--s-3); }
.editor-form { display: flex; flex-direction: column; gap: var(--s-2); }
.ed-row { display: grid; grid-template-columns: 76px 1fr; gap: var(--s-2); align-items: center; }
.ed-row > span { color: var(--label-tertiary); font: var(--type-caption); white-space: nowrap; }
.ed-row .input { width: 100%; }
.ed-grow { min-width: 0; }
.ed-ops { display: flex; align-items: center; gap: var(--s-2); flex-wrap: wrap; }
.flex-1 { flex: 1; }
.dirty-hint { color: var(--warning); font: var(--type-caption); white-space: nowrap; }
/* 备注是多行：标签对齐顶部更自然 */
.ed-row:has(textarea) { align-items: start; }
.ed-row:has(textarea) > span { padding-top: 6px; }
</style>
