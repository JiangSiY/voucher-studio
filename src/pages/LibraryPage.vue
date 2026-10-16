<script setup lang="ts">
/**
 * 全部凭证（资料库）—— v0.18 重构：**双行列表（邮件式）+ 右侧检视器**；v0.19：**右栏即编辑器**。
 *
 * 与旧「凭证列表」的区别（用户要求的是结构重做，不是换皮）：
 * - 行形态从"10 列表格"改为**两行信息块**：上行 = 缩略图 + 摘要 + 金额（眼睛先看这两样），
 *   下行 = 编号 · 日期 · 对方 · 状态。信息层次按"核对时真正要看的顺序"排，而不是按字段表排。
 * - **选中即在右侧看大图与字段**（检视器），不再"点进去、返回再重新筛选"。
 * - v0.19（2026-09-28 用户决定）：原 `/vouchers/:id` 独立编辑页与右栏"便捷修改"合并为
 *   **右栏唯一编辑器**（`components/voucher/VoucherEditor.vue`，承载原编辑页全部能力）。
 *   双击列表行 / 回车 = **展开编辑区并聚焦**（不再跳页）；`/vouchers/:id` 改为重定向。
 *   新建向导仍保留独立页（`/vouchers/new`，多步流程）。
 *
 * 已实装：跨页选择集、按筛选全选、右栏完整编辑（7 字段 + 换图/识别/裁剪/提取填入/快照）、
 * `focus` 深链（其他页面跳来并聚焦某张）、**未保存修改的离开保护**（切凭证/深链跳走/离开本页前确认，
 * 默认「留在此页」；`beforeunload` 兜住刷新与关窗）。
 * 待补（审计 2026-09-27 登记）：列表虚拟滚动（量级上千时）。
 */
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { onBeforeRouteLeave, useRoute, useRouter } from "vue-router";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { safeListen } from "../composables/safeListen";
import { api, type VoucherQuery, type Voucher, type VoucherDetail, type VoucherStatus } from "../api/client";
import ConfirmDialog from "../components/ConfirmDialog.vue";
import EmptyState from "../components/EmptyState.vue";
import FluentIcon from "../components/FluentIcon.vue";
import VoucherEditor from "../components/voucher/VoucherEditor.vue";
import { useNeighborNav } from "../composables/neighborNav";
import { STATUS_META, fmtAmount, fmtDate } from "../composables/format";
import { parseTags } from "../composables/money";
import { candidatePaths, nextFallbackStep, pickCandidate } from "../composables/useAttachmentFallback";
import { toast, toastError } from "../stores/toast";

const route = useRoute();
const router = useRouter();

// ---------- 筛选（沿用既有 query 参数通道：仪表盘/收件箱下钻都用它） ----------

const status = ref<VoucherStatus | "all">("all");
const keyword = ref("");
const dateFrom = ref("");
const dateTo = ref("");

const items = ref<Voucher[]>([]);
const total = ref(0);
const page = ref(1);
const pageSize = 50;
const loading = ref(false);

/** 精确下钻筛选（对应收件箱"未填金额 / 待识别 / 从未导出"等待办；控件都在主筛选行里） */
const amountEmpty = ref(false);
const counterpartyEmpty = ref(false);
/** 陈旧天数（对应收件箱"陈旧草稿"待办；与 dashboard_todos 同口径） */
const staleDays = ref<number | null>(null);
/** 精确筛选：对方账户（整串相等）——洞察的月度透视下钻会带这个参数 */
const counterpartyExact = ref("");
/** 精确筛选：标签（逗号/顿号分隔，任一命中）——同上 */
const tagsText = ref("");
const ocrStatus = ref<string>("");
const exported = ref<string>("");
const sort = ref<string>("date_desc");

function query() {
  return {
    status: status.value === "all" ? null : status.value,
    keyword: keyword.value.trim() || null,
    dateFrom: dateFrom.value || null,
    dateTo: dateTo.value || null,
    amountEmpty: amountEmpty.value,
    counterpartyEmpty: counterpartyEmpty.value,
    staleDays: staleDays.value,
    counterparty: counterpartyExact.value.trim() || null,
    tags: parseTags(tagsText.value),
    ocrStatus: ocrStatus.value || null,
    exported: (exported.value || null) as "never" | "any" | null,
    sort: sort.value,
    page: page.value,
    pageSize,
  };
}

async function load() {
  loading.value = true;
  try {
    const res = await api.listVouchers(query());
    items.value = res.items;
    total.value = res.total;
    thumbs.value = res.thumbs;

    // 深链优先：其他页面"去看这一张"（处理完把它从 URL 摘掉，避免后续筛选变化时重复聚焦）
    if (pendingFocus != null) {
      const target = pendingFocus;
      pendingFocus = null;
      await showFocused(target);
      stripFocusFromUrl();
      return;
    }

    const stillHere = items.value.find((v) => v.id === selectedId.value);
    if (stillHere) {
      // 行数据可能已被保存更新：同步右栏展示对象（编辑器自身按 updatedAt 合并表单）
      shown.value = stillHere;
      return;
    }
    // 右栏展示的是"不在本页"的凭证（深链聚焦后用户改了筛选）：保留它，不抢选中
    if (shown.value && shown.value.id === selectedId.value) return;

    const first = items.value[0];
    if (first) await select(first);
    else {
      selectedId.value = null;
      shown.value = null;
      detail.value = null;
    }
  } catch (e) {
    toastError(e);
  } finally {
    loading.value = false;
  }
}

/** 把 `focus` 从 URL 摘掉（保留其余筛选键）：深链是一次性动作，不该留在地址栏里 */
function stripFocusFromUrl() {
  if (route.query.focus == null) return;
  const q = { ...route.query };
  delete q.focus;
  void router.replace({ query: q });
}

/** 从 URL 读筛选（收件箱/趋势下钻过来时带回显） */
function readQuery() {
  const q = route.query;
  status.value = (q.status as VoucherStatus) ?? "all";
  keyword.value = (q.keyword as string) ?? "";
  dateFrom.value = (q.dateFrom as string) ?? "";
  dateTo.value = (q.dateTo as string) ?? "";
  amountEmpty.value = q.amountEmpty === "1" || q.amountEmpty === "true";
  counterpartyEmpty.value = q.counterpartyEmpty === "1";
  const sd = Number(q.staleDays);
  staleDays.value = Number.isFinite(sd) && sd > 0 ? sd : null;
  ocrStatus.value = (q.ocrStatus as string) ?? "";
  exported.value = (q.exported as string) ?? "";
  sort.value = (q.sort as string) ?? "date_desc";
  // v0.18 修复：洞察-月度透视下钻会带 `counterparty=` / `tags=`，此前这里没读 → 列表不过滤
  counterpartyExact.value = (q.counterparty as string) ?? "";
  tagsText.value = (q.tags as string) ?? "";
  // v0.19：`focus` = 其他页面"去看这一张"（深链聚焦，不参与筛选；见 load() 里的处理）
  const f = Number(q.focus);
  pendingFocus = Number.isFinite(f) && f > 0 ? f : null;
}

const activeFilters = computed(() => {
  const out: { label: string; clear: () => void }[] = [];
  if (status.value !== "all") out.push({ label: `状态：${STATUS_META[status.value].label}`, clear: () => (status.value = "all") });
  if (keyword.value) out.push({ label: `关键词：${keyword.value}`, clear: () => (keyword.value = "") });
  if (dateFrom.value || dateTo.value)
    out.push({ label: `日期：${dateFrom.value || "…"} ~ ${dateTo.value || "…"}`, clear: () => ((dateFrom.value = ""), (dateTo.value = "")) });
  if (amountEmpty.value) out.push({ label: "金额未填", clear: () => (amountEmpty.value = false) });
  if (counterpartyEmpty.value)
    out.push({ label: "对方未填", clear: () => (counterpartyEmpty.value = false) });
  if (staleDays.value) out.push({ label: `超过 ${staleDays.value} 天未更新`, clear: () => (staleDays.value = null) });
  if (counterpartyExact.value)
    out.push({ label: `对方：${counterpartyExact.value}`, clear: () => (counterpartyExact.value = "") });
  if (tagsText.value)
    out.push({ label: `标签：${tagsText.value}`, clear: () => (tagsText.value = "") });
  if (ocrStatus.value)
    out.push({ label: `OCR：${ocrStatus.value === "pending" ? "待识别" : ocrStatus.value}`, clear: () => (ocrStatus.value = "") });
  if (exported.value)
    out.push({ label: exported.value === "never" ? "从未导出" : "已导出过", clear: () => (exported.value = "") });
  return out;
});

/** 是否有任何筛选生效（列表空态里的「清空全部筛选」据此显示：没筛选就不该劝用户清筛选） */
const hasAnyFilter = computed(() => activeFilters.value.length > 0);

// ---------- 缩略图与检视器 ----------
const thumbs = ref<Record<number, { filePath: string; enhancedPath: string | null; enhancedSource: string; fileAbs?: string | null; enhancedAbs?: string | null }>>({});
const thumbStep = ref<Record<number, number>>({});

function thumbUrl(v: Voucher): string | null {
  const t = thumbs.value[v.id];
  if (!t) return null;
  const abs = pickCandidate(t, thumbStep.value[v.id] ?? 0);
  return abs ? convertFileSrc(abs) : null;
}
function onThumbError(v: Voucher) {
  const t = thumbs.value[v.id];
  if (!t) return;
  const cur = thumbStep.value[v.id] ?? 0;
  thumbStep.value = { ...thumbStep.value, [v.id]: nextFallbackStep(cur, candidatePaths(t).length) };
}

const selectedId = ref<number | null>(null);
/** 右栏当前展示的凭证：通常来自本页列表；`focus` 深链指向不在本页的凭证时也在此 */
const shown = ref<Voucher | null>(null);
const detail = ref<VoucherDetail | null>(null);
/** 展开编辑区（窄栏 ⇄ 宽栏）：双击行/回车会展开并聚焦第一个字段 */
const wide = ref(false);
const editorRef = ref<InstanceType<typeof VoucherEditor> | null>(null);

/**
 * 上下文翻张（⌥←/→ 与右栏上的两个箭头）：按**整份筛选结果**翻，能跨页。
 * `query()` 是列表自己的筛选构造器——翻张与列表同口径，否则"下一张"会跳出眼前的筛选。
 */
const { neighbor: navNeighbor, positionLabel: navPosition, load: loadNeighbor, clear: clearNeighbor } =
  useNeighborNav({
    currentId: computed(() => shown.value?.id ?? null),
    query: computed(() => query() as VoucherQuery),
  });

/** 翻到相邻张：复用既有动线（页内直接选中；页外单独取 + 脏数据确认） */
async function goNeighbor(dir: "prev" | "next") {
  const n = navNeighbor.value;
  const id = dir === "prev" ? n?.prevId : n?.nextId;
  if (!id) return;
  await showFocused(id);
}
/** 待处理的 `focus` 深链目标（来自其他页面"去看这一张"） */
let pendingFocus: number | null = null;

// ---------- 右栏编辑器（v0.19：全应用唯一编辑处） ----------

/** 保存成功：就地更新列表行与右栏（不整页重载，保持滚动位置与选中项） */
function onEditorSaved(saved: Voucher) {
  const idx = items.value.findIndex((x) => x.id === saved.id);
  if (idx >= 0) items.value.splice(idx, 1, saved);
  if (shown.value?.id === saved.id) shown.value = saved;
  if (detail.value?.voucher.id === saved.id) detail.value = { ...detail.value, voucher: saved };
}

/** 编辑器内部操作（换图/识别/裁剪）产生的新详情 */
function onEditorDetail(d: VoucherDetail) {
  if (shown.value?.id !== d.voucher.id) return;
  detail.value = d;
}

/** 保存并下一张：本页下一行；到页尾且还有下一页则翻页取第一条 */
async function onEditorNext() {
  // 优先按上下文翻张：能跨页、且与筛选同口径（原先只在本页里找下一条，翻到页尾就卡住）
  if (navNeighbor.value?.nextId) {
    await goNeighbor("next");
    return;
  }
  const idx = items.value.findIndex((v) => v.id === selectedId.value);
  const next = idx >= 0 ? items.value[idx + 1] : items.value[0];
  if (next) {
    await select(next);
    return;
  }
  if (page.value < totalPages.value) {
    gotoPage(page.value + 1);
    await nextTick();
    const first = items.value[0];
    if (first) await select(first);
    return;
  }
  toast.info("当前筛选内没有更多凭证了");
}

/** 双击行 / 回车：展开编辑区并把焦点送进第一个可编辑字段（不再跳页） */
async function focusEditor(v: Voucher) {
  await select(v);
  wide.value = true;
  await nextTick();
  editorRef.value?.focusFirst();
}

function toggleWide() {
  wide.value = !wide.value;
  if (wide.value) void nextTick(() => editorRef.value?.focusFirst());
}

/** 本页列表里选中的那一行（用于行高亮；`focus` 深链指向不在本页时可能为 null） */
const selected = computed(() => items.value.find((v) => v.id === selectedId.value) ?? null);

/**
 * 选中一张：更新高亮、右栏展示对象，并拉取详情（附件/快照）。
 * 请求序号守卫（沿用 2026-09-23 审计 B3 的做法）：连续 ↑↓ / 翻张时会有多个 `get_voucher`
 * 在途，旧响应若无条件写入，会出现"字段是 A、附件是 B"的错配。
 */
let detailSeq = 0;

// ---------- 未保存修改的离开保护（v0.19：右栏是全应用唯一编辑处） ----------
/**
 * 三种离开方式都要拦：① 在列表里切到另一张；② `focus` 深链切到页外那张；③ 离开本页（路由/关窗/刷新）。
 * 旧实现只有 `resetFrom` 无条件覆盖——草稿说没就没，连个提示都没有。
 * 拦截时弹一次确认，**默认焦点在「留在此页」**（ConfirmDialog 首个可操作元素就是取消按钮）。
 */
type LeaveIntent =
  | { kind: "switch"; target: Voucher }
  | { kind: "focus"; id: number }
  | { kind: "route" };

const leaveIntent = ref<LeaveIntent | null>(null);
const leaveConfirmVisible = computed(() => leaveIntent.value !== null);
const leaveConfirmTitle = computed(() => {
  switch (leaveIntent.value?.kind) {
    case "switch":
      return "切换到另一张凭证？";
    case "focus":
      return "跳转到另一张凭证？";
    default:
      return "离开全部凭证？";
  }
});
let resolveRouteLeave: ((ok: boolean) => void) | null = null;

/** 编辑器里有未保存的修改吗（组件未挂载时视为干净） */
function editorDirty(): boolean {
  return editorRef.value?.isDirty() ?? false;
}

/** 切到另一张之前先问一句：目标与当前展示的是同一张则直接放行 */
function guardSwitch(v: Voucher): boolean {
  if (shown.value?.id === v.id || !editorDirty()) return true;
  leaveIntent.value = { kind: "switch", target: v };
  return false;
}

function confirmLeave() {
  const intent = leaveIntent.value;
  leaveIntent.value = null;
  if (!intent) return;
  if (intent.kind === "switch") {
    void doSelect(intent.target);
    return;
  }
  if (intent.kind === "focus") {
    void showFocusedOffPage(intent.id);
    return;
  }
  resolveRouteLeave?.(true);
  resolveRouteLeave = null;
}

function cancelLeave() {
  const intent = leaveIntent.value;
  leaveIntent.value = null;
  if (intent?.kind !== "route") return;
  resolveRouteLeave?.(false);
  resolveRouteLeave = null;
}

/** 路由守卫：脏数据时挂起导航，等用户在确认框里选（返回的 Promise 由 router 等待） */
onBeforeRouteLeave(() => {
  if (!editorDirty()) return true;
  return new Promise<boolean>((resolve) => {
    resolveRouteLeave = resolve;
    leaveIntent.value = { kind: "route" };
  });
});

/** 关窗/刷新兜底：浏览器与 webview 的 beforeunload（Tauri 的 `CloseRequested` 在 Rust 侧做关闭前备份） */
function onBeforeUnload(e: BeforeUnloadEvent) {
  if (!editorDirty()) return;
  e.preventDefault();
  e.returnValue = "";
}

onMounted(() => window.addEventListener("beforeunload", onBeforeUnload));
onUnmounted(() => {
  window.removeEventListener("beforeunload", onBeforeUnload);
  // 卸载时若还有挂起的路由确认（异常路径），放行以免导航永久卡住
  resolveRouteLeave?.(true);
  resolveRouteLeave = null;
});

async function doSelect(v: Voucher) {
  selectedId.value = v.id;
  shown.value = v;
  detail.value = null;
  const my = ++detailSeq;
  try {
    const d = await api.getVoucher(v.id);
    if (my !== detailSeq) return;
    detail.value = d;
    shown.value = d.voucher;
  } catch (e) {
    if (my !== detailSeq) return;
    toastError(e);
  }
}

async function select(v: Voucher) {
  if (!guardSwitch(v)) return;
  await doSelect(v);
}

// 当前显示的凭证一变就重拉翻张信息（换凭证、页外深链、删掉当前张都要跟）
watch(
  () => shown.value?.id ?? null,
  (id) => {
    if (id) void loadNeighbor();
    else clearNeighbor();
  },
);

/** `focus` 深链：目标可能不在本页（跨筛选/翻页跳来），此时单独取它并展示，不改列表 */ 
async function showFocused(id: number) {
  const inPage = items.value.find((v) => v.id === id);
  if (inPage) {
    await select(inPage);
    // 目标就在本页：滚动到该行，长列表里别让用户自己找
    await nextTick();
    document.querySelector(`[data-row-id="${id}"]`)?.scrollIntoView?.({ block: "nearest" });
    return;
  }
  if (editorDirty() && shown.value?.id !== id) {
    leaveIntent.value = { kind: "focus", id };
    return;
  }
  await showFocusedOffPage(id);
}

/** `focus` 深链指向页外凭证：单独取它并展示，不改列表（切走前同样过脏数据确认） */
async function showFocusedOffPage(id: number) {
  selectedId.value = id;
  const my = ++detailSeq;
  try {
    const d = await api.getVoucher(id);
    if (my !== detailSeq) return;
    shown.value = d.voucher;
    detail.value = d;
  } catch (e) {
    if (my !== detailSeq) return;
    toastError(e); // 例如凭证已被删除
  }
}

/** 键盘动线：↑↓ 移动选择（macOS 列表的常规操作）；回车 = 展开编辑区；⌘⇧E 切换展宽 */
function onKeydown(e: KeyboardEvent) {
  const mod = e.ctrlKey || e.metaKey;
  if (mod && e.shiftKey && e.key.toLowerCase() === "e") {
    e.preventDefault();
    toggleWide();
    return;
  }
  const tag = (e.target as HTMLElement)?.tagName;
  if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA") return;
  if (e.key === "Enter") {
    // 带 ⌘ 的 Enter 归右栏编辑器：⌘Enter = 保存并下一张，⌘⇧Enter = 复核通过并下一张。
    // 这里不加判断会把它们一起抢成"展开编辑区"。
    if (mod) return;
    const row = selected.value;
    if (row) {
      e.preventDefault();
      void focusEditor(row);
    }
    return;
  }
  // ⌥←/→ 上下文翻张（能跨页；↑↓ 只在本页内移动）。必须排在输入框早退**之后**：
  // macOS 文本框里 alt+方向键是"按词移动光标"，抢掉会很难用
  if (e.altKey && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
    e.preventDefault();
    void goNeighbor(e.key === "ArrowLeft" ? "prev" : "next");
    return;
  }
  if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
  const idx = items.value.findIndex((v) => v.id === selectedId.value);
  if (idx < 0) return;
  const next = e.key === "ArrowDown" ? Math.min(idx + 1, items.value.length - 1) : Math.max(idx - 1, 0);
  if (next !== idx) {
    e.preventDefault();
    void select(items.value[next]);
  }
}

// ---------- 批量操作（保持既有能力） ----------

// 跨页选择集：**不随翻页/筛选清空**（旧实现每次 load 都 clear，导致"勾了 3 页只剩最后 1 页"），
// 只由用户显式清除、或批量操作后重置。
// 2026-10-16：原「高级筛选」折叠行已删除——它的 6 个控件（日期起止 / OCR 状态 / 导出覆盖 /
// 金额未填 / 对方未填）与主筛选行是**同一批 v-model 的第二份**，同屏两份、改一处另一处跟着动；
// 而控制它展开的那个开关又没有 setter，等于死代码。逐项清除由主行的筛选 chip 负责，
// 「清空全部筛选」收敛到列表空态里的那一个按钮（`clearAllFilters`）。

function clearAllFilters() {
  staleDays.value = null;
  counterpartyExact.value = "";
  tagsText.value = "";
  dateFrom.value = "";
  dateTo.value = "";
  ocrStatus.value = "";
  exported.value = "";
  amountEmpty.value = false;
  counterpartyEmpty.value = false;
}

const selectedIds = ref<Set<number>>(new Set());
const confirmDelete = ref(false);
const selectingAll = ref(false);

/** 按**当前筛选**全选（走只读的 list_voucher_ids，不把 N 条对象拉回前端） */
async function selectAllMatching() {
  selectingAll.value = true;
  try {
    const ids = await api.listVoucherIds(query());
    selectedIds.value = new Set(ids);
    toast.info(`已选中当前筛选下的 ${ids.length} 张`);
  } catch (e) {
    toastError(e);
  } finally {
    selectingAll.value = false;
  }
}

function clearSelection() {
  selectedIds.value = new Set();
}
const batchStatus = ref<VoucherStatus>("reviewed");
/** 批量改状态的二次确认：原实现点「应用」立即生效，改错就是一整批；且选择集跨页/跨筛选保留，
    用户按下去时未必知道自己改的是哪些 */
const confirmBatchStatus = ref(false);

/** 已选里**不在当前页**的数量（跨页/跨筛选选中留下的），确认框里如实告知 */
const selectedOffPage = computed(() => {
  const onPage = new Set(items.value.map((v) => v.id));
  return [...selectedIds.value].filter((id) => !onPage.has(id)).length;
});

const batchStatusBody = computed(() => {
  const n = selectedIds.value.size;
  const off = selectedOffPage.value;
  const scope = off > 0 ? `（其中 ${off} 张不在当前列表里，是之前跨页或跨筛选选中的）` : "";
  return `将把已选 ${n} 张凭证的状态改为「${STATUS_META[batchStatus.value].label}」${scope}。改错需要一张张改回来。`;
});

function openBatchStatusConfirm() {
  if (!selectedIds.value.size) return;
  confirmBatchStatus.value = true;
}

/** 批量状态流转（单事务逐张快照，后端保证；迁自旧列表页） */
async function applyBatchStatus() {
  confirmBatchStatus.value = false;
  if (!selectedIds.value.size) return;
  try {
    const n = await api.batchUpdateStatus([...selectedIds.value], batchStatus.value);
    toast.success(`已把 ${n} 张改为「${STATUS_META[batchStatus.value].label}」`);
    clearSelection();
    await load();
  } catch (e) {
    toastError(e);
  }
}

function toggle(v: Voucher) {
  const s = new Set(selectedIds.value);
  if (s.has(v.id)) s.delete(v.id);
  else s.add(v.id);
  selectedIds.value = s;
}

async function removeSelected() {
  confirmDelete.value = false;
  try {
    const n = await api.deleteVouchers([...selectedIds.value]);
    toast.success(`已删除 ${n} 张`);
    clearSelection();
    detail.value = null;
    await load();
  } catch (e) {
    toastError(e);
  }
}

const totalPages = computed(() => Math.max(1, Math.ceil(total.value / pageSize)));
function gotoPage(p: number) {
  page.value = Math.min(Math.max(1, p), totalPages.value);
  void load();
}

let unlistenImport: UnlistenFn | null = null;
let queryWatcher: (() => void) | null = null;

onMounted(async () => {
  readQuery();
  await load();
  // 同一路由内 query 变化（再次从洞察/收件箱下钻）也要重读并重查——
  // 原来只在 onMounted 读一次，导致"已在列表页再点别的下钻"不生效
  queryWatcher = watch(
    () => route.query,
    () => {
      readQuery();
      page.value = 1;
      void load();
    },
  );
  unlistenImport = await safeListen("import:done", () => load());
  window.addEventListener("keydown", onKeydown);
});

onUnmounted(() => {
  unlistenImport?.();
  queryWatcher?.();
  window.removeEventListener("keydown", onKeydown);
});

// 筛选变化即重查（保留在 URL 里，便于返回时回显）
watch([status, dateFrom, dateTo, amountEmpty, counterpartyEmpty, counterpartyExact, tagsText, staleDays, ocrStatus, exported, sort], () => {
  page.value = 1;
  void load();
  // 筛选变了，"第 n / N 张"也跟着变；不重拉就会显示上一份筛选里的位置
  if (shown.value?.id) void loadNeighbor();
});
watch(keyword, () => {
  page.value = 1;
  void load();
  if (shown.value?.id) void loadNeighbor();
});
</script>

<template>
  <div class="library">
    <!-- 工具栏：筛选（保持 URL 参数通道）+ 批量动作 -->
    <div class="filter-bar">
      <!-- 搜索框用全局唯一实现（base.css 的 .search，高度走控件令牌，图标由 ::before 提供） -->
      <div class="search">
        <input v-model="keyword" placeholder="搜索编号 / 摘要 / 对方 / 备注" />
      </div>
      <select v-model="status" class="input" style="max-width: 160px">
        <option value="all">全部状态</option>
        <option v-for="(meta, key) in STATUS_META" :key="key" :value="key">{{ meta.label }}</option>
      </select>
      <input v-model="dateFrom" class="input" type="date" style="max-width: 160px" title="起始日期" />
      <input v-model="dateTo" class="input" type="date" style="max-width: 160px" title="结束日期" />
      <template v-for="f in activeFilters" :key="f.label">
        <button class="chip" :title="`清除筛选：${f.label}`" @click="f.clear()">
          {{ f.label }} <FluentIcon name="close" :size="12" />
        </button>
      </template>
      <select v-model="ocrStatus" class="input" style="max-width: 140px" title="按 OCR 状态筛选">
        <option value="">OCR 状态</option>
        <option value="pending,failed">待识别（含失败）</option>
        <option value="pending">仅待识别</option>
        <option value="failed">仅识别失败</option>
        <option value="done">已识别</option>
      </select>
      <select v-model="exported" class="input" style="max-width: 140px" title="按导出覆盖度筛选">
        <option value="">导出覆盖</option>
        <option value="never">从未导出</option>
        <option value="any">导出过</option>
      </select>
      <label class="inline-check" title="只看金额未填的凭证">
        <input v-model="amountEmpty" type="checkbox" /> 金额未填
      </label>
      <label class="inline-check" title="只看对方账户未填的凭证">
        <input v-model="counterpartyEmpty" type="checkbox" /> 对方未填
      </label>
      <input v-model="counterpartyExact" class="input" style="max-width: 170px"
        placeholder="对方账户（精确）" title="对方账户整串相等（区别于搜索框的模糊匹配）" />
      <input v-model="tagsText" class="input" style="max-width: 170px"
        placeholder="标签（逗号分隔）" title="标签精确匹配，多个用、或,分隔（任一命中）" />
      <select v-model="sort" class="input" style="max-width: 150px" title="排序">
        <option value="date_desc">日期（新→旧）</option>
        <option value="date_asc">日期（旧→新）</option>
        <option value="updated_desc">最近修改</option>
        <option value="amount_desc">金额（大→小）</option>
        <option value="amount_asc">金额（小→大）</option>
      </select>
      <span class="flex-1"></span>
      <span class="count">{{ total }} 张</span>
      <template v-if="selectedIds.size">
        <span class="sel-count">已选 {{ selectedIds.size }} 张</span>
        <select v-model="batchStatus" class="input" style="max-width: 150px" title="批量改状态">
          <option v-for="(meta, key) in STATUS_META" :key="key" :value="key">{{ meta.label }}</option>
        </select>
        <button class="btn" title="改状态前会先确认" @click="openBatchStatusConfirm()">应用</button>
        <button class="btn" @click="confirmDelete = true">
          <FluentIcon name="delete" :size="14" /> 删除
        </button>
        <button class="btn" title="清除选择" @click="clearSelection()">清除</button>
      </template>
      <button v-else class="btn" :disabled="selectingAll" title="按当前筛选选中全部（跨页生效）"
        @click="selectAllMatching()">
        {{ selectingAll ? "选择中…" : "全选筛选结果" }}
      </button>
      <button class="btn btn-primary" @click="router.push('/vouchers/new')">
        <FluentIcon name="plus" :size="14" /> 新建
      </button>
    </div>

    <!-- 主体：左列表（双行）+ 右编辑器（v0.19：右栏即编辑器；双击/回车展开） -->
    <div class="split" :class="{ 'split--wide': wide }">
      <section class="list-pane">
        <div v-if="loading" class="hint">加载中…</div>
        <EmptyState v-else-if="!items.length" icon="folder" title="没有符合条件的凭证"
          desc="试试清除筛选，或把图片拖进窗口导入。">
          <button v-if="hasAnyFilter" class="btn" @click="clearAllFilters()">清空全部筛选</button>
        </EmptyState>

        <ul v-else class="rows">
          <li
            v-for="v in items"
            :key="v.id"
            class="row"
            :class="{ sel: v.id === selectedId }"
            :data-row-id="v.id"
            @click="select(v)"
            @dblclick="focusEditor(v)"
          >
            <input class="row-check" type="checkbox" :checked="selectedIds.has(v.id)" @click.stop="toggle(v)" />
            <div class="row-thumb">
              <img v-if="thumbUrl(v)" :src="thumbUrl(v)!" :alt="v.voucherNo" loading="lazy" @error="onThumbError(v)" />
              <FluentIcon v-else name="image" :size="18" />
            </div>
            <div class="row-main">
              <div class="line1">
                <span class="desc">{{ v.description || "（无摘要）" }}</span>
                <span class="flex-1"></span>
                <span class="amount">{{ fmtAmount(v.amountCents) }}</span>
              </div>
              <div class="line2">
                <span class="mono">{{ v.voucherNo }}</span>
                <span class="dot">·</span>
                <span>{{ v.voucherDate }}</span>
                <span class="dot">·</span>
                <span class="party">{{ v.counterparty || "（无对方）" }}</span>
                <span class="flex-1"></span>
                <span class="tag" :class="STATUS_META[v.status].tag">{{ STATUS_META[v.status].label }}</span>
              </div>
            </div>
          </li>
        </ul>

        <div v-if="totalPages > 1" class="pager">
          <button class="btn" :disabled="page <= 1" @click="gotoPage(page - 1)">上一页</button>
          <span class="muted">第 {{ page }} / {{ totalPages }} 页</span>
          <button class="btn" :disabled="page >= totalPages" @click="gotoPage(page + 1)">下一页</button>
        </div>
      </section>

      <!-- 右栏：编辑器（v0.19 起全应用唯一的凭证编辑处；双击行/回车会展开） -->
      <aside class="inspector" v-if="shown">
        <header class="ins-head">
          <span class="mono">{{ shown.voucherNo }}</span>
          <span class="flex-1"></span>
          <!-- 上下文翻张（⌥←/→）：按**整份筛选结果**翻，能跨页（↑↓ 只在本页内移动）。
               顺序是后端定的日期序，与列表当前排序无关——title 里写明，免得误会。 -->
          <div class="ins-nav" role="group" aria-label="上下文翻张">
            <button class="btn-icon" :disabled="!navNeighbor?.prevId" title="上一张（⌥←）" aria-label="上一张"
              @click="goNeighbor('prev')">
              <FluentIcon name="chevronLeft" :size="14" />
            </button>
            <span class="ins-pos"
              :title="`按日期顺序翻张（当前筛选共 ${navNeighbor?.total ?? 0} 张，与列表排序无关）`">
              {{ navPosition }}
            </span>
            <button class="btn-icon" :disabled="!navNeighbor?.nextId" title="下一张（⌥→）" aria-label="下一张"
              @click="goNeighbor('next')">
              <FluentIcon name="chevronRight" :size="14" />
            </button>
          </div>
          <button class="btn" :title="wide ? '收起编辑区（⌘⇧E）' : '展开编辑区（⌘⇧E）'" @click="toggleWide()">
            <FluentIcon :name="wide ? 'flex-shrink' : 'display'" :size="14" /> {{ wide ? "收起" : "展开" }}
          </button>
        </header>

        <!-- 深链聚焦到"不在当前筛选结果里"的凭证：说明来源，避免误以为筛选坏了 -->
        <p v-if="!selected" class="ins-hint">
          该凭证不在当前筛选结果中（来自页外跳转）；
          <button class="link" @click="items[0] && select(items[0])">回到列表第一张</button>
        </p>

        <VoucherEditor
          ref="editorRef"
          :voucher="shown"
          :detail="detail"
          :dense="!wide"
          @saved="onEditorSaved"
          @detail="onEditorDetail"
          @next="onEditorNext"
        />

        <dl class="ins-meta">
          <div><dt>最后修改</dt><dd>{{ fmtDate(shown.updatedAt) }}</dd></div>
        </dl>
      </aside>
    </div>

    <ConfirmDialog
      :visible="confirmBatchStatus"
      title="批量修改状态？"
      :body="batchStatusBody"
      :confirm-text="`改为「${STATUS_META[batchStatus].label}」`"
      :danger="false"
      @confirm="applyBatchStatus"
      @cancel="confirmBatchStatus = false"
    />
    <ConfirmDialog
      :visible="confirmDelete"
      title="删除选中的凭证？"
      :body="`将删除 ${selectedIds.size} 张凭证及其图片文件，此操作不可恢复。`"
      confirm-text="删除"
      @confirm="removeSelected"
      @cancel="confirmDelete = false"
    />

    <!-- 未保存修改的离开保护：切凭证 / 跳走 / 离开本页前先问一句，「留在此页」是默认焦点 -->
    <ConfirmDialog
      :visible="leaveConfirmVisible"
      :title="leaveConfirmTitle"
      body="当前凭证有未保存的修改，继续将丢失这些改动。"
      confirm-text="放弃修改"
      cancel-text="留在此页"
      @confirm="confirmLeave"
      @cancel="cancelLeave"
    />
  </div>
</template>

<style scoped>
.library { display: flex; flex-direction: column; gap: var(--s-3); height: 100%; }

.filter-bar { display: flex; align-items: center; gap: var(--s-2); flex-wrap: wrap; }
.flex-1 { flex: 1; }
.count { font: var(--type-caption); color: var(--label-tertiary); }
.inline-check { display: inline-flex; align-items: center; gap: 4px; font: var(--type-caption); color: var(--label-secondary); }
.chip {
  display: inline-flex; align-items: center; gap: 4px;
  height: var(--control-h-sm);
  padding: 0 var(--s-2);
  border: 1px solid var(--separator);
  border-radius: 999px;
  background: var(--bg-selection);
  color: var(--accent);
  font: var(--type-caption);
  cursor: default;
}

.split { display: grid; grid-template-columns: minmax(0, 1.6fr) minmax(320px, 1fr); gap: var(--s-3); align-items: start; }
/* 展宽（⌘⇧E / 双击行）：编辑区取得大部分宽度，列表仍在左侧保留上下文 */
.split--wide { grid-template-columns: minmax(300px, 0.7fr) minmax(560px, 1.6fr); }
.list-pane { display: flex; flex-direction: column; gap: var(--s-2); min-width: 0; }
.hint { color: var(--label-tertiary); font: var(--type-body); padding: var(--s-4); }

.rows { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 2px; }
.row {
  display: flex; align-items: center; gap: var(--s-2);
  padding: var(--s-2) var(--s-3);
  border: 1px solid transparent;
  border-radius: var(--radius-panel);
  cursor: default;
}
.row:hover { background: var(--bg-selection); }
.row.sel { background: var(--bg-selection); border-color: var(--accent); }
.row-check { margin: 0; }
.row-thumb {
  width: 44px; height: 56px; flex-shrink: 0;
  display: flex; align-items: center; justify-content: center;
  border: 1px solid var(--separator);
  border-radius: var(--radius-small);
  background: var(--surface-2);
  color: var(--label-quaternary);
  overflow: hidden;
}
.row-thumb img { width: 100%; height: 100%; object-fit: cover; }
.row-main { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.line1 { display: flex; align-items: baseline; gap: var(--s-2); min-width: 0; }
.desc { font: var(--type-body); color: var(--label-primary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.amount { font: var(--type-body); font-weight: 600; color: var(--label-primary); font-variant-numeric: tabular-nums; }
.line2 { display: flex; align-items: center; gap: var(--s-2); font: var(--type-caption); color: var(--label-tertiary); min-width: 0; }
.line2 .party { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.dot { color: var(--label-quaternary); }
.mono { font-family: var(--font-mono); }

.pager { display: flex; align-items: center; gap: var(--s-3); justify-content: center; padding: var(--s-2); }
.muted { color: var(--label-tertiary); font: var(--type-caption); }

.inspector {
  display: flex; flex-direction: column; gap: var(--s-3);
  padding: var(--s-3);
  border: 1px solid var(--separator);
  border-radius: var(--radius-panel);
  background: var(--bg-content);
  position: sticky; top: 0;
  /* 右栏自成一个滚动区：字段多了（v0.19 起含图片/表单/快照）也不把整页拖长 */
  max-height: calc(100vh - 190px);
  overflow-y: auto;
}
.ins-head { display: flex; align-items: center; gap: var(--s-2); }

/* 上下文翻张：右栏头部右侧的「‹ 第 n/N 张 ›」。位置文案用等宽数字，翻页时宽度不跳。 */
.ins-nav { display: inline-flex; align-items: center; gap: 2px; }
.ins-pos {
  min-width: 84px;
  text-align: center;
  font: var(--type-caption);
  color: var(--label-tertiary);
  font-variant-numeric: tabular-nums;
}
.link { background: none; border: none; padding: 0; color: var(--accent); font: var(--type-caption); cursor: pointer; text-decoration: underline; }
.ins-meta { display: flex; flex-direction: column; gap: 6px; margin: 0; }
.ins-meta > div { display: grid; grid-template-columns: 64px 1fr; gap: var(--s-2); align-items: baseline; }
.ins-meta dt { color: var(--label-tertiary); font: var(--type-caption); white-space: nowrap; }
.ins-meta dd { margin: 0; font: var(--type-body); color: var(--label-primary); overflow-wrap: anywhere; }
.sel-count { font: var(--type-caption); color: var(--accent); white-space: nowrap; }
.ins-hint { margin: 0; font: var(--type-caption); color: var(--label-tertiary); }

@media (max-width: 1100px) {
  .split { grid-template-columns: minmax(0, 1fr); }
  .inspector { position: static; }
}
</style>
