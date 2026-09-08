<script setup lang="ts">
/**
 * 凭证编辑页。
 * 一图一凭证：新建 = 先选一张图片（导入即建卡 + OCR 自动填单），编辑 = 单图 + 更换图片（换图不换卡）。
 * 表单字段：编号/日期/摘要/对方账户/金额/状态/标签/备注 + 快照历史。
 * 拖放导入在 Rust 侧全局处理（App.vue 反馈）；本页新建模式监听完成事件跳转到新凭证。
 */
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { useRoute, useRouter, onBeforeRouteLeave } from "vue-router";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, type Attachment, type ExtractedFields, type ImportSummary, type Voucher, type VoucherSnapshot } from "../api/client";
import ImageViewerDialog from "../components/ImageViewerDialog.vue";
import ConfirmDialog from "../components/ConfirmDialog.vue";
import CropDialog from "../components/CropDialog.vue";
import OcrResultCard from "../components/OcrResultCard.vue";
import { OCR_META, fmtDate } from "../composables/format";
import { useDirtyGuard } from "../composables/dirtyGuard";
import { parseNavContext, useNeighborNav } from "../composables/neighborNav";
import { useDescriptionSuggest } from "../composables/descriptionSuggest";
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

// ---------- v0.9 编辑动线：脏数据守卫 + 上下文导航（组合式函数提取） ----------

/** 表单当前值的序列化（与基线比对判断 dirty；编号只读不入基线） */
function formSnapshot(): string {
  return JSON.stringify([
    voucherDate.value, description.value, counterparty.value,
    amountYuan.value, status.value, tagsInput.value, note.value,
  ]);
}

const guard = useDirtyGuard({
  snapshot: formSnapshot,
  enabled: () => editId.value != null,
  navigate: (p) => router.push(p),
});
const { dirty, leaveConfirmVisible, leaveConfirmText } = guard;
const { leaveTo, confirmLeave, cancelLeave } = guard;

// 侧栏等其他入口的路由离开（按钮触发的已走 leaveTo，不会到这里带 dirty）
onBeforeRouteLeave((to, _from, next) => next(guard.beforeRouteLeave(to)));

/** 列表页带来的筛选上下文（query 参数；无上下文入口 → 按全量顺序导航） */
const navCtx = computed(() => parseNavContext(route.query));
const fromSearch = computed(() => navCtx.value.fromSearch);
const ctxStatus = computed(() => navCtx.value.status);
const ctxKeyword = computed(() => navCtx.value.keyword);
const { neighbor, positionLabel, loadNeighbor } = useNeighborNav({
  editId,
  ctx: navCtx,
});

/** 把凭证详情套用到表单（编辑加载 / OCR 自动填单后共用） */
function applyDetail(detail: Awaited<ReturnType<typeof api.getVoucher>>) {
  applyForm(detail.voucher);
  attachments.value = detail.attachments;
  snapshots.value = detail.snapshots;
  guard.markClean(); // DB 侧状态 = 新基线（脏状态清零）
  // v0.7 S-2：识别文本就绪时拉取结构化提取预览（只读，不写库）
  loadExtracted(detail.attachments[0]);
}

/** 跳到相邻凭证（保留筛选上下文；脏数据走确认） */
function gotoVoucher(id: number) {
  leaveTo(() => {
    router.push({ path: `/vouchers/${id}`, query: { ...route.query } });
  });
}

function goNeighbor(dir: "prev" | "next") {
  const n = neighbor.value;
  if (!n) return;
  const target = dir === "prev" ? n.prevId : n.nextId;
  if (target == null) {
    toast.info(dir === "prev" ? "已是第一张" : "已是最后一张");
    return;
  }
  gotoVoucher(target);
}

/** 返回（来源感知：搜索页进入回搜索页恢复关键词；否则回列表并还原完整筛选） */
function goBack() {
  leaveTo(() => {
    if (fromSearch.value) {
      router.push({ path: "/search", query: ctxKeyword.value ? { keyword: ctxKeyword.value } : {} });
      return;
    }
    const c = navCtx.value;
    const query: Record<string, string> = {};
    if (c.status) query.status = c.status;
    if (c.keyword) query.keyword = c.keyword;
    if (c.dateFrom) query.dateFrom = c.dateFrom;
    if (c.dateTo) query.dateTo = c.dateTo;
    router.push({ path: "/vouchers", query });
  });
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
    loadNeighbor();
    loadLockState(); // v0.11：期间锁定态（保存前提示）
  } catch (e) {
    toastError(e);
    router.replace("/vouchers");
  }
}

// ---------- v0.11：期间锁定态（封账月凭证只读提示） ----------

const lockedPeriod = ref<string | null>(null);

async function loadLockState() {
  try {
    const locks = await api.listPeriodLocks();
    const period = voucherDate.value.slice(0, 7);
    lockedPeriod.value = locks.some((l) => l.period === period) ? period : null;
  } catch {
    lockedPeriod.value = null; // 辅助态，失败不打扰
  }
}

/** 表单内改日期 → 重估锁定态（改到封账月将无法保存，提前提示） */
watch(voucherDate, () => {
  if (editId.value) loadLockState();
});

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

// ---------- v0.10：摘要快速录入（模板 + 历史高频联想） ----------

const descTemplates = computed<string[]>(
  () => configStore.config?.voucher?.descriptionTemplates ?? [],
);
const { load: loadDescHistory, suggest: suggestDesc } = useDescriptionSuggest({
  templates: descTemplates,
});
const descFocused = ref(false);
const descHighlight = ref(-1);

/** 联想列表：聚焦时展示；键盘 ↑↓ 移动高亮（Enter 选中，Esc 关闭） */
const descSuggestions = computed(() => suggestDesc(description.value));

function onDescFocus() {
  descFocused.value = true;
  loadDescHistory();
}

function onDescBlur() {
  // 延迟收起：点击下拉项时 mousedown 先于 blur 触发，直接关会吃掉点击
  setTimeout(() => {
    descFocused.value = false;
    descHighlight.value = -1;
  }, 150);
}

function pickDescription(text: string) {
  description.value = text;
  descFocused.value = false;
  descHighlight.value = -1;
}

function onDescKeydown(e: KeyboardEvent) {
  if (!descFocused.value || !descSuggestions.value.length) return;
  if (e.key === "ArrowDown") {
    e.preventDefault();
    descHighlight.value = (descHighlight.value + 1) % descSuggestions.value.length;
  } else if (e.key === "ArrowUp") {
    e.preventDefault();
    descHighlight.value =
      descHighlight.value <= 0 ? descSuggestions.value.length - 1 : descHighlight.value - 1;
  } else if (e.key === "Enter" && descHighlight.value >= 0) {
    e.preventDefault();
    pickDescription(descSuggestions.value[descHighlight.value].description);
  } else if (e.key === "Escape") {
    descFocused.value = false;
    descHighlight.value = -1;
  }
}

// ---------- 保存 ----------

/** 校验并保存，返回保存后的凭证（失败 / 无改动返回 null） */
async function save(): Promise<Voucher | null> {
  if (!editId.value || saving.value) return null;
  if (!dirty.value) {
    toast.info("没有需要保存的修改");
    return null;
  }
  saving.value = true;
  try {
    // 金额严格校验：可选负号、最多两位小数（"12.34.56"/"1e3" 之类的宽松解析一律拒绝）
    const amt = amountYuan.value.trim();
    let cents: number | null = null;
    if (amt !== "") {
      if (!/^-?\d+(\.\d{1,2})?$/.test(amt)) {
        toast.warning("金额格式不正确：应为数字，最多两位小数（如 123.45）");
        return null;
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
    guard.markClean(); // 保存成功 = 新基线
    if (saved.voucherNo !== oldNo) {
      toast.success(`凭证已保存，档号按新日期重排：${oldNo} → ${saved.voucherNo}`);
    } else if (saved.voucherDate !== "" && oldNo.split("-").slice(0, 3).join("-") !== saved.voucherDate) {
      toast.success("凭证已保存（已导出/归档凭证编号锁定，档号未随日期变更）");
    } else {
      toast.success("凭证已保存");
    }
    // v0.10：保存后疑似重复检测（同日期同金额的其他凭证；仅提示，由用户判断）
    checkDuplicateHint(saved);
    return saved;
  } catch (e) {
    toastError(e);
    return null;
  } finally {
    saving.value = false;
  }
}

/** 疑似重复提示：金额为空不检测（空金额无判定意义）；失败静默 */
async function checkDuplicateHint(v: Voucher) {
  if (v.amountCents == null) return;
  try {
    const dup = await api.checkVoucherDuplicates(v.id);
    if (dup.length) {
      toast.warning(
        `疑似重复：与凭证 ${dup.map((d) => d.voucherNo).join("、")} 同日期同金额（${v.voucherDate} · ￥${(v.amountCents / 100).toFixed(2)}），请核对是否重复报销`,
      );
    }
  } catch {
    // 提示是辅助能力，失败不打扰主流程
  }
}

/** 保存（留在当前页，刷新详情与导航）——Ctrl+S */
async function saveOnly() {
  const saved = await save();
  if (saved) await loadDetail();
}

/** 翻到下一张（已保存或无改动状态直接跳）。返回是否真的跳走了。 */
async function jumpNext(): Promise<boolean> {
  await loadNeighbor(); // 用最新键位取邻居（保存改日期后位置会变）
  const next = neighbor.value?.nextId;
  if (next == null) {
    toast.info("当前筛选内没有更多凭证了");
    return false;
  }
  guard.armNav(); // 已保存或无改动，直接放行
  await router.push({ path: `/vouchers/${next}`, query: { ...route.query } });
  return true;
}

/** 保存并跳到下一张（连续核对主操作）——Ctrl+Enter。
 * 无改动直接翻张（核对场景字段本来就对，也要能连续推进）。 */
async function saveAndNext() {
  if (!dirty.value) {
    await jumpNext();
    return;
  }
  const saved = await save();
  if (!saved) return;
  const jumped = await jumpNext();
  if (!jumped) await loadDetail(); // 留在末张：刷新快照列表
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

// ---------- v0.9 键盘动线：Ctrl+S 保存 / Ctrl+Enter 保存并下一张 / Alt+←→ 翻张 ----------

function onKeydown(e: KeyboardEvent) {
  if (!editId.value) return;
  const mod = e.ctrlKey || e.metaKey;
  if (mod && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "s") {
    e.preventDefault();
    saveOnly();
  } else if (mod && e.key === "Enter") {
    e.preventDefault();
    saveAndNext();
  } else if (e.altKey && !mod && e.key === "ArrowLeft") {
    e.preventDefault();
    goNeighbor("prev");
  } else if (e.altKey && !mod && e.key === "ArrowRight") {
    e.preventDefault();
    goNeighbor("next");
  }
}

onMounted(async () => {
  loadDetail();
  window.addEventListener("keydown", onKeydown);
  unlistenImportDone = await listen<ImportSummary>("import:done", (e) => {
    if (!editId.value && e.payload.vouchers.length) {
      router.replace(`/vouchers/${e.payload.vouchers[0].voucherId}`);
    }
  });
});

onUnmounted(() => {
  unlistenImportDone?.();
  window.removeEventListener("keydown", onKeydown);
});

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
  cropSrc.value = convertFileSrc(`${attachmentsDir.value}\\${a.filePath.replace(/\//g, "\\")}`);
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
    if (editId.value) await loadDetail();
    toast.success("图片已调整（覆盖增强版；原图不变）", {
      label: "重新识别",
      onClick: () => void reRunOcrAfterCrop(),
    });
  } catch (e) {
    toastError(e);
    cropDialog.value?.done();
  }
}

/** 裁剪后重跑 OCR（run_ocr 会读最新增强版；缓存按增强图内容区分） */
async function reRunOcrAfterCrop() {
  const a = attachments.value[0];
  if (!a || !editId.value) return;
  ocrBusy.value = true;
  try {
    applyDetail(await api.runOcr(a.id));
    toast.success("已按调整后的图片重新识别并回填可识别的空字段");
  } catch (e) {
    toastError(e);
  } finally {
    ocrBusy.value = false;
  }
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
          <!-- v0.9 上下文导航：同筛选下的上一张/下一张（列表跳转携带筛选，无上下文则全量顺序） -->
          <div class="vnav" v-if="neighbor">
            <button class="btn btn-sm" :disabled="!neighbor.prevId" title="上一张（Alt+←）" @click="goNeighbor('prev')">
              <FluentIcon name="chevronLeft" :size="13" />上一张
            </button>
            <span class="vnav-pos" :title="ctxKeyword ? `筛选：${ctxKeyword}` : '当前列表筛选'">{{ positionLabel }}</span>
            <button class="btn btn-sm" :disabled="!neighbor.nextId" title="下一张（Alt+→）" @click="goNeighbor('next')">
              下一张<FluentIcon name="chevronRight" :size="13" />
            </button>
          </div>
        </div>
        <div class="fluent-panel-body">
          <!-- v0.11：封账月锁定提示（保存被禁用；查看/导出不受限） -->
          <div class="callout warning" v-if="lockedPeriod" style="margin: 0 0 16px">
            <span class="ci"><FluentIcon name="lock" :size="18" /></span>
            <div class="ct">
              <strong>{{ lockedPeriod }} 已封账，该凭证不可修改。</strong>
              如确需调整，请先到「设置 → 凭证参数 → 期间管理」解锁（解锁意味着该月数据可能需要重新导出）。
            </div>
          </div>
          <div class="field">
            <label>凭证编号</label>
            <input v-model="voucherNo" class="input mono" disabled />
          </div>
          <div class="field">
            <label>凭证日期</label>
            <input v-model="voucherDate" type="date" class="input" />
          </div>
          <div class="field desc-suggest">
            <label>摘要</label>
            <input v-model="description" class="input" placeholder="如：八月办公用品采购（聚焦可联想常用摘要）"
              @focus="onDescFocus" @blur="onDescBlur" @keydown="onDescKeydown" />
            <!-- v0.10 摘要联想：模板（置顶标 pin）+ 历史高频（带次数） -->
            <div v-if="descFocused && descSuggestions.length" class="desc-dropdown">
              <button v-for="(s, i) in descSuggestions" :key="s.description" type="button"
                class="desc-option" :class="{ active: i === descHighlight }"
                @mousedown.prevent="pickDescription(s.description)"
                @mousemove="descHighlight = i">
                <FluentIcon v-if="s.pinned" name="pin" :size="11" />
                <span class="desc-text">{{ s.description }}</span>
                <span v-if="s.count != null" class="desc-count">{{ s.count }} 次</span>
              </button>
            </div>
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
          <div style="display: flex; gap: 8px; justify-content: flex-end; align-items: center">
            <span v-if="dirty && lockedPeriod" class="dirty-hint" title="该月已封账，无法保存">● 已封账</span>
            <span v-else-if="dirty" class="dirty-hint" title="有未保存的修改">● 未保存</span>
            <button class="btn" @click="goBack">返回</button>
            <button class="btn" :disabled="saving || !!lockedPeriod" title="Ctrl+S" @click="saveOnly">
              {{ saving ? "保存中…" : "保存" }}
            </button>
            <button class="btn btn-primary" :disabled="saving || !!lockedPeriod" title="Ctrl+Enter" @click="saveAndNext">
              {{ saving ? "保存中…" : "保存并下一张" }}
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
                  <!-- v0.12：手动裁剪/旋转（自动矫正失败的人工补救；覆盖增强版，原图不动） -->
                  <button class="btn btn-sm" :disabled="!!lockedPeriod"
                    :title="lockedPeriod ? '该月已封账，不可调整' : '手动裁剪 / 旋转（覆盖增强版，原图不变）'"
                    @click="openCrop(a)">
                    <FluentIcon name="edit" :size="14" />调整
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

      <!-- v0.7 S-2：OCR 识别结果（结构化提取预览，一键填入表单）；v0.9 提取为子组件 -->
      <OcrResultCard :extracted="extracted" :fields-busy="fieldsBusy"
        @apply-field="applyExtractedField" @apply-all="applyAllExtracted" />
    </div>
  </div>

  <ImageViewerDialog :visible="viewerVisible" :src="viewerSrc" :file-name="viewerName"
    :attachment-id="viewerAttachmentId" @close="viewerVisible = false" />

  <!-- v0.12：手动裁剪 / 旋转对话框（保存 → crop_attachment → 刷新详情；可一键重新识别） -->
  <CropDialog ref="cropDialog" :visible="cropVisible" :attachment-id="cropAttachmentId"
    :src="cropSrc" :file-name="cropFileName"
    @close="cropVisible = false" @save="onCropSave" />

  <!-- v0.9：未保存离开确认（翻张 / 返回 / 侧栏路由统一走这里） -->
  <ConfirmDialog :visible="leaveConfirmVisible" title="未保存的修改" :body="leaveConfirmText" confirm-text="离开"
    @confirm="confirmLeave" @cancel="cancelLeave" />
</template>

<style scoped>
/* v0.9 上下文导航条（面板头右侧：上一张 / 位置 / 下一张） */
.vnav {
  display: flex;
  align-items: center;
  gap: 8px;
}

.vnav-pos {
  font-size: 12px;
  color: var(--text-3);
  min-width: 64px;
  text-align: center;
  white-space: nowrap;
}

/* v0.9 脏状态提示（保存按钮左侧） */
.dirty-hint {
  font-size: 12px;
  color: var(--warning);
  white-space: nowrap;
}

/* v0.10 摘要联想下拉（模板置顶 + 历史高频） */
.desc-suggest {
  position: relative;
}

.desc-dropdown {
  position: absolute;
  left: 0;
  right: 0;
  top: 100%;
  z-index: 30;
  margin-top: 4px;
  background: var(--bg-1, #fff);
  border: 1px solid var(--border);
  border-radius: 6px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.12);
  max-height: 240px;
  overflow-y: auto;
  padding: 4px;
}

.desc-option {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 6px 8px;
  border: none;
  border-radius: 4px;
  background: none;
  cursor: pointer;
  font-size: 13px;
  text-align: left;
  color: var(--text-1);
}

.desc-option.active {
  background: var(--bg-3, rgba(0, 0, 0, 0.06));
}

.desc-text {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.desc-count {
  font-size: 11px;
  color: var(--text-3);
  flex-shrink: 0;
}
</style>
