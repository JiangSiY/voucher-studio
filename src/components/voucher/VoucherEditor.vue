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
import { fmtAmount } from "../../composables/format";
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
  /** 未保存状态变化（宿主在右栏头部显示「● 未保存」；B 批：把状态提示从底部提到头部） */
  dirty: [v: boolean];
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

/** 状态分组按钮（顺序＝流程顺序；用户要求"便于操作"） */
const STATUS_OPTIONS: { value: Voucher["status"]; label: string }[] = [
  { value: "draft", label: "草稿" },
  { value: "reviewed", label: "已复核" },
  { value: "exported", label: "已导出" },
  { value: "archived", label: "已归档" },
];

/** 收支方向（用户只用标签表达金额正负）——存进 `tags`，非方向的旧标签原样保留 */
const DIRECTION_TAGS = ["收入", "支出"];
const DIRECTION_OPTIONS: { value: string; label: string }[] = [
  { value: "", label: "未标" },
  { value: "收入", label: "收入" },
  { value: "支出", label: "支出" },
];

/** 标签串 → 数组（与 `parseTags` 同口径：中英文逗号/顿号分隔，去空去重保序） */
function parseTagList(s: string): string[] {
  const out: string[] = [];
  for (const raw of s.split(/[、,，]/)) {
    const t = raw.trim();
    if (t && !out.includes(t)) out.push(t);
  }
  return out;
}
function joinTagList(list: string[]): string {
  return list.join("、");
}

const otherTags = computed(() =>
  parseTagList(form.value.tags).filter((t) => !DIRECTION_TAGS.includes(t)),
);
const direction = computed(
  () => parseTagList(form.value.tags).find((t) => DIRECTION_TAGS.includes(t)) ?? "",
);
/** 切换方向：先剔除旧方向标签，再按需追加新方向（其他标签保持原样） */
function setDirection(d: string) {
  form.value.tags = joinTagList([...otherTags.value, ...(d ? [d] : [])]);
}

/** ←/→ 在分组按钮间移动（Roving tabindex：只有选中项可 Tab 进入，方向键移动并切换） */
function onStatusKeydown(e: KeyboardEvent) {
  const delta = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
  if (!delta) return;
  e.preventDefault();
  const i = STATUS_OPTIONS.findIndex((o) => o.value === form.value.status);
  const next = (i + delta + STATUS_OPTIONS.length) % STATUS_OPTIONS.length;
  form.value.status = STATUS_OPTIONS[next].value;
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
      `疑似重复：与凭证 ${names} 同日期同金额（${v.voucherDate} · ¥${fmtAmount(v.amountCents)}），请核对是否重复报销${strongest}`,
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

/**
 * ⌘⇧Enter「复核通过并下一张」（2026-10-16 用户要求实装；规范 §2.3 的规划项）：
 * 把状态置**已复核**并落盘，然后翻下一张。
 * - 无改动且已经是「已复核」→ 不写库，直接翻（连续核对最常见的路径）
 * - 有改动（含只是改状态）→ 走同一条 `save()`（校验、留痕、重复提示都复用，不另起保存逻辑）
 * - 校验失败（如金额格式错）→ 停在本张，绝不跳过
 */
async function reviewAndNext() {
  if (form.value.status !== "reviewed") form.value.status = "reviewed";
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

/** 全部填入：与后端自动填单同政策（金额放开——「唯一兜底」档经评测≈98% 正确，
 *  2026-10-09 用户拍板；日期/对方/商家 Low 档留待单点）。 */
function applyAllExtracted() {
  const f = extracted.value;
  if (!f) return;
  const low = (c: "high" | "medium" | "low" | null) => c === "low";
  if (f.date && !low(f.confidence?.date)) form.value.date = f.date;
  if (f.amountCents != null) form.value.amount = (f.amountCents / 100).toFixed(2);
  if (f.counterparty && !low(f.confidence?.counterparty)) form.value.counterparty = f.counterparty;
  if (f.merchant && !low(f.confidence?.merchant)) form.value.description = f.merchant;
  const skipped: string[] = [];
  if (f.date && low(f.confidence?.date)) skipped.push("日期");
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
    return;
  }
  // ⌘⇧Enter 必须排在 ⌘Enter 之前：原实现只判 `mod && Enter`、没有 shift 判断，
  // ⌘⇧Enter 一直被 ⌘Enter 分支吞掉（规范 §2.3 记着这笔账）
  if (mod && e.shiftKey && !e.altKey && e.key === "Enter") {
    e.preventDefault();
    void reviewAndNext();
    return;
  }
  if (mod && !e.shiftKey && !e.altKey && e.key === "Enter") {
    e.preventDefault();
    void saveAndNext();
  }
}

const dateEl = ref<HTMLInputElement | null>(null);
/** 供宿主在"展开编辑区"后把焦点送进第一个可编辑字段 */
/** 暴露给宿主：① 聚焦首个字段；② 上报未保存修改（资料库据此在切换凭证/离开页面前弹确认，v0.19） */
/** 未保存状态广播给宿主（右栏头部的小圆点） */
watch(dirty, (v) => emit("dirty", v), { immediate: true });

defineExpose({ focusFirst: () => dateEl.value?.focus(), isDirty: () => dirty.value });

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
      :dense="dense"
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

    <!-- 字段顺序（B 批）：摘要 → 日期 → 金额 → 状态 → 对方 → 标签 → 备注。
         只读的「凭证编号」已移出表单——右栏头部本来就显示编号，白占一行；
         最常改的三件事（摘要/日期金额/状态）因此落在首屏。 -->
    <div class="editor-form">
      <div class="ed-row" data-field="description">
        <span>摘要</span>
        <div class="ed-grow">
          <!-- 本组件已提供「摘要」标签（保证与其它行同列对齐），故隐藏其自带的那一枚 -->
          <DescriptionInput v-model="form.description" :templates="descTemplates" class="desc-no-label" />
        </div>
      </div>
      <!-- 日期 + 金额同排：**按控件利用率决定**——
           右栏常规档可用宽度 ≈ 296px（扣掉标签列 ≈ 212px），一个日期框只吃掉 ~130px，
           若并排每格只剩 ~100px（原生日期框会挤到看不清）；展宽档 ≈536px（值格 ≈452px），
           并排后每格 ~222px 既放得下又不浪费。故：**展宽并排、常规堆叠**（堆叠时字段仍各占整行）。 -->
      <div class="ed-row ed-pair" data-field="date-amount">
        <span class="ed-pair-labels">
          <!-- 常规档（堆叠）：外标签**双行**，与两个输入行逐行对齐（用户 2026-10-17 反馈） -->
          <span class="ed-pair-col">日期</span>
          <span class="ed-pair-col">金额（元）</span>
        </span>
        <div class="ed-pair-fields">
          <label class="ed-pair-item" data-field="date">
            <span class="ed-pair-label">日期</span>
            <input ref="dateEl" v-model="form.date" type="date" class="input" />
          </label>
          <label class="ed-pair-item" data-field="amount">
            <span class="ed-pair-label">金额（元）</span>
            <input v-model="form.amount" class="input" inputmode="decimal" placeholder="可留空" />
          </label>
        </div>
      </div>
      <!-- 状态分组按钮（用户要求"便于操作"）：一眼看到全部状态、单击即切；
           role=radiogroup + aria-checked，支持 ←/→ 键盘切换（Roving tabindex）。 -->
      <div class="ed-row" data-field="status">
        <span>状态</span>
        <div class="ed-seg" role="radiogroup" aria-label="凭证状态" @keydown="onStatusKeydown">
          <button
            v-for="opt in STATUS_OPTIONS"
            :key="opt.value"
            type="button"
            class="ed-seg-btn"
            role="radio"
            :aria-checked="form.status === opt.value"
            :tabindex="form.status === opt.value ? 0 : -1"
            :data-status="opt.value"
            @click="form.status = opt.value"
          >
            {{ opt.label }}
          </button>
        </div>
      </div>
      <!-- 对方账户 + 收支方向同排（与"日期/金额"同一套条件并排规则）。
           用户只用标签表达金额正负，故「标签」改为**收支方向分段按钮**（省控件、单击即切）；
           原有非方向标签不丢——仍在数据里，并在下方以小字只读展示。 -->
      <div class="ed-row ed-pair" data-field="counterparty-direction">
        <span class="ed-pair-labels">
          <span class="ed-pair-col">对方账户</span>
          <span class="ed-pair-col">收支方向</span>
        </span>
        <div class="ed-pair-fields">
          <label class="ed-pair-item" data-field="counterparty">
            <span class="ed-pair-label">对方账户</span>
            <input v-model="form.counterparty" class="input" placeholder="如：文一办公用品店" />
          </label>
          <div class="ed-pair-item" data-field="tags">
            <span class="ed-pair-label">收支方向</span>
            <div class="ed-seg" role="radiogroup" aria-label="收支方向">
              <button
                v-for="opt in DIRECTION_OPTIONS"
                :key="opt.label"
                type="button"
                class="ed-seg-btn"
                role="radio"
                :aria-checked="direction === opt.value"
                :tabindex="direction === opt.value ? 0 : -1"
                :data-direction="opt.value || 'none'"
                @click="setDirection(opt.value)"
              >
                {{ opt.label }}
              </button>
            </div>
            <span v-if="otherTags.length" class="ed-pair-hint" title="这些标签仍在数据里，可在筛选/洞察中按标签查找">
              其他标签：{{ otherTags.join("、") }}
            </span>
          </div>
        </div>
      </div>
      <label class="ed-row" data-field="note">
        <span>备注</span>
        <textarea v-model="form.note" class="input ed-note" rows="2"></textarea>
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

    <!-- 关系与组（v0.22 B 批：由宿主插入，位置在**核心字段之后、识别结果之前**——
         用户反馈"相关凭证/所属事项组太靠下"，这里给它们一个稳定的落点） -->
    <slot name="relations" />

    <!-- 识别提取结果（一键填入表单；落库仍由「保存」决定；默认折叠） -->
    <OcrResultCard :extracted="extracted" :fields-busy="fieldsBusy"
      @apply-field="applyExtractedField" @apply-all="applyAllExtracted" />

    <SnapshotPanel v-if="detail" :snapshots="detail.snapshots" />
  </div>
</template>

<style scoped>
.editor { display: flex; flex-direction: column; gap: var(--s-3); }

/* 操作条吸底（B 批，用户选择「吸底」）：字段改完不用再往下滚去找「保存」。
   背景与右栏一致，避免滚动时文字从底下透出来。 */
.editor > .ed-ops {
  position: sticky;
  bottom: 0;
  z-index: 2;
  padding: var(--s-2) 0;
  background: var(--bg-content);
  border-top: 1px solid var(--separator);
}

/* 备注自适应高度（B 批，用户选择「自适应」）：2 行起步，随内容长高，最多 6 行后内部滚动 */
.ed-note {
  field-sizing: content;
  min-height: calc(2 * 1.5em + 12px);
  max-height: calc(6 * 1.5em + 12px);
  resize: vertical;
}

/* 表单是**一个**网格：标签列全局共用一条（宽度 = 最宽标签），因此每行的输入框左边缘对齐。
   反面案例：把 `max-content` 写在每一行上 → 每行各算各的宽度（"状态"两字窄、"金额（元）"五字宽），
   输入框左边缘参差不齐（用户 2026-09-28 截图报）。
   标签列用 max-content 而非固定 px：字号可调（默认已偏大），固定值会让长标签溢出压到输入框。 */
.editor-form {
  display: grid;
  grid-template-columns: max-content minmax(0, 1fr);
  gap: var(--s-2) var(--s-3);
  align-items: center;
}
/* 行壳不参与布局：让行内的标签与控件直接落进上面那条网格 */
.ed-row { display: contents; }
/* 日期 + 金额：常规档堆叠（各占整行），展宽档并排（两个半宽） */
/* 成对行标签（v0.22，用户 2026-10-17 两次反馈后的定稿）：
   - **两档都用双行标签**（`日期` / `金额（元）`；`对方账户` / `收支方向`）——
     常规档下每行正对它的输入框；展宽档下把标签列压到最窄（不再出现「对方账户 / 收支方向」这种长行，
     用户："展开后把这两个长标签做成两行显示，减少标签占的宽度"）；
   - 展宽档字段并排时，字段内子标签显示（就近标注是哪一格）；常规档隐藏（避免与外标签重复）。
   两档共用同一层 DOM，切档只切显示（不重建输入框、不丢焦点/草稿）。*/
.ed-pair > span {
  align-self: start;
}
.ed-pair-labels {
  display: grid;
  grid-auto-rows: var(--control-h-md);
  gap: 4px; /* **必须与 .ed-pair-fields 同值**：逐行对齐靠它，不是靠估算 margin */
  align-content: start;
}
.ed-pair-col {
  display: flex;
  align-items: center;
  color: var(--label-tertiary);
  font: var(--type-caption);
  white-space: nowrap;
}
/* 内部子标签：默认（堆叠）不显示，避免与外标签双行重复 */
.ed-pair-item .ed-pair-label {
  display: none;
}
/* 展宽档：字段并排，故字段内子标签显示（外标签仍是双行，标签列保持最窄）。
   此时外标签**压缩成紧凑两行**（行高按文字、行距 2px）：字段那边因带子标签高 ~52px，
   若外标签仍按 32px 行高 + 8px 行距排（共 72px），就会比输入框高出 20px、看着"间距大、对不上"，
   并把它下面的行整体压低。紧凑两行 ≈38px，与字段行贴齐（用户 2026-10-17 反馈「间距缩小一些」）。 */
.editor:not(.editor-dense) .ed-pair-item .ed-pair-label {
  display: block;
}
.editor:not(.editor-dense) .ed-pair-labels {
  grid-auto-rows: auto;
  gap: 2px;
}
.ed-pair-hint {
  color: var(--label-tertiary);
  font: var(--type-caption);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
/* 组内间距 4px（比行与行之间的 8px 更紧）：两个字段读起来是"一组"，
   且标签网格用同一个值 → 逐行对齐不会被破坏（用户 2026-10-17：间距缩小一些、
   让两组标签与输入框更好对齐匹配）。 */
.ed-pair-fields {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: 4px;
}
.editor:not(.editor-dense) .ed-pair-fields {
  grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
}
.ed-pair-item {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}
.ed-pair-label {
  color: var(--label-tertiary);
  font: var(--type-caption);
  white-space: nowrap;
}

/* 状态分组按钮：一眼看到全部状态、单击即切（替代原下拉） */
.ed-seg {
  display: inline-flex;
  align-items: center;
  border: 1px solid var(--separator-strong);
  border-radius: var(--radius-control);
  overflow: hidden;
  height: var(--control-h-md);
  width: 100%;
}
.ed-seg-btn {
  flex: 1 1 0;
  min-width: 0;
  height: 100%;
  padding: 0 6px;
  background: transparent;
  border: 0;
  border-left: 1px solid var(--separator);
  color: var(--label-secondary);
  font: var(--type-caption);
  cursor: pointer;
  white-space: nowrap;
}
.ed-seg-btn:first-child {
  border-left: 0;
}
.ed-seg-btn:hover {
  background: var(--bg-selection);
}
.ed-seg-btn[aria-checked="true"] {
  background: var(--accent-soft);
  color: var(--accent);
  font-weight: 600;
}
.ed-seg-btn:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: -2px;
}

/* 备注是多行控件：标签对齐顶部更自然（display:contents 后按 DOM 选择器定位即可） */
.ed-row[data-field="note"] > span { align-self: start; margin-top: 7px; }
.ed-row > span { color: var(--label-tertiary); font: var(--type-caption); white-space: nowrap; }
.ed-row .input { width: 100%; }
.ed-grow { min-width: 0; }
.ed-ops { display: flex; align-items: center; gap: var(--s-2); flex-wrap: wrap; }
.flex-1 { flex: 1; }
.dirty-hint { color: var(--warning); font: var(--type-caption); white-space: nowrap; }
/* 摘要行隐藏子组件自带标签（见模板注释）。
   DescriptionInput 的根元素本身就是 `.field`，类名会落到同一个元素上，故用后代 label 选择器 */
.desc-no-label :deep(label) { display: none; }
</style>
