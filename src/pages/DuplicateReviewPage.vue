<script setup lang="ts">
/**
 * v0.16 疑似重复核查页（`/duplicates`）。
 *
 * 背景（2026-09-23 用户报"很鸡肋"）：旧版只检测不处理——仪表盘面板与保存提示每次
 * 重算、永远重现，用户无处处置。本页给出行业通行的处置闭环：
 * **可疑队列 → 旁列对比（证据/置信度）→ 二元判定（不是重复 / 确认重复并删除）→
 * 留痕**。判定口径不变（同日期 + 同金额，金额非空）；已判定的对不再出现在队列与提示里。
 *
 * 术语：本页处理的是**疑似重复**（同日同金额）。同一张图（内容重复、导入期 MD5 阻断）
 * 与编号冲突（`voucherNo` UNIQUE）都另有入口，不在此列。
 */
import { computed, onMounted, onBeforeUnmount, ref } from "vue";
import { useRoute, useRouter } from "vue-router";
import { convertFileSrc } from "@tauri-apps/api/core";
import {
  api,
  type Attachment,
  type DupDecision,
  type PendingSummary,
  type ReviewGroup,
  type ReviewPair,
  type Voucher,
} from "../api/client";
import ConfirmDialog from "../components/ConfirmDialog.vue";
import EmptyState from "../components/EmptyState.vue";
import FluentIcon from "../components/FluentIcon.vue";
import {
  compareRows,
  decisionSummary,
  decisionTag,
  diffCount,
  evidenceMeta,
  evidenceReasons,
  groupHeadline,
  pairKey,
} from "../composables/dupReview";
import { fmtAmount, fmtDate } from "../composables/format";
import { safeListen } from "../composables/safeListen";
import { toast, toastError } from "../stores/toast";
import {
  candidatePaths,
  nextFallbackStep,
  pickCandidate,
} from "../composables/useAttachmentFallback";

const route = useRoute();
const router = useRouter();

const tab = ref<"pending" | "handled">("pending");
const loading = ref(false);
const groups = ref<ReviewGroup[]>([]);
const summary = ref<PendingSummary>({ groups: 0, pairs: 0 });
const decisions = ref<DupDecision[]>([]);

/** 本次会话内"稍后再说"的对：只影响排序（沉到组尾），不写库 */
const skipped = ref<Set<string>>(new Set());
const busy = ref(false);
const note = ref("");

async function load() {
  loading.value = true;
  try {
    const [queue, stat] = await Promise.all([
      api.scanDuplicateReview(),
      api.duplicatePendingSummary(),
    ]);
    groups.value = queue;
    summary.value = stat;
  } catch (e) {
    toastError(e);
    groups.value = [];
  } finally {
    loading.value = false;
  }
}

async function loadHandled() {
  try {
    decisions.value = await api.listDuplicateDecisions(200);
  } catch (e) {
    toastError(e);
    decisions.value = [];
  }
}

/** 组内对：本次会话"稍后"的沉到末尾，其余保持后端的置信度排序 */
const orderedPairs = (g: ReviewGroup): ReviewPair[] => {
  const later = g.pending.filter((p) => skipped.value.has(pairKey(p.a.id, p.b.id)));
  const now = g.pending.filter((p) => !skipped.value.has(pairKey(p.a.id, p.b.id)));
  return [...now, ...later];
};

function skip(p: ReviewPair) {
  const k = pairKey(p.a.id, p.b.id);
  const s = new Set(skipped.value);
  s.add(k);
  skipped.value = s;
  closeCompare();
  toast.info("已放到该组末尾，稍后仍可处理（本次会话有效）");
}

// ---------- 旁列对比：选中一对 → 取两张的附件（缓存） ----------

interface Selection {
  group: ReviewGroup;
  pair: ReviewPair;
}

const sel = ref<Selection | null>(null);
const attachments = ref<Record<number, Attachment[]>>({});
/** 图片回退阶梯：key = `${voucherId}:${side}`（增强版 → 原图 → 占位） */
const imgStep = ref<Record<string, number>>({});

async function ensureAttachments(v: Voucher) {
  if (attachments.value[v.id]) return;
  try {
    const detail = await api.getVoucher(v.id);
    attachments.value = { ...attachments.value, [v.id]: detail.attachments };
  } catch {
    // 取不到附件不影响判定（可凭字段与标题判断）；界面显示占位
    attachments.value = { ...attachments.value, [v.id]: [] };
  }
}

async function selectPair(group: ReviewGroup, pair: ReviewPair) {
  sel.value = { group, pair };
  note.value = "";
  await Promise.all([ensureAttachments(pair.a), ensureAttachments(pair.b)]);
}

function closeCompare() {
  sel.value = null;
  note.value = "";
}

function imgUrl(v: Voucher, side: "a" | "b"): string | null {
  const list = attachments.value[v.id] ?? [];
  if (!list.length) return null;
  const key = `${v.id}:${side}`;
  const abs = pickCandidate(list[0], imgStep.value[key] ?? 0);
  return abs ? convertFileSrc(abs) : null;
}

function onImgError(v: Voucher, side: "a" | "b") {
  const list = attachments.value[v.id] ?? [];
  if (!list.length) return;
  const key = `${v.id}:${side}`;
  const total = candidatePaths(list[0]).length;
  imgStep.value = { ...imgStep.value, [key]: nextFallbackStep(imgStep.value[key] ?? 0, total) };
}

const rows = computed(() => (sel.value ? compareRows(sel.value.pair.a, sel.value.pair.b) : []));
const diffs = computed(() => diffCount(rows.value));

// ---------- 三个动作 ----------

async function markNotDuplicate() {
  if (!sel.value) return;
  const { a, b } = sel.value.pair;
  busy.value = true;
  try {
    await api.resolveDuplicatePair(a.id, b.id, note.value.trim());
    toast.success(`已标记"不是重复"：${a.voucherNo} / ${b.voucherNo}`);
    closeCompare();
    await load();
  } catch (e) {
    toastError(e);
  } finally {
    busy.value = false;
  }
}

const confirmRemove = ref<{ keep: Voucher; remove: Voucher } | null>(null);

function askRemove(keep: Voucher, remove: Voucher) {
  confirmRemove.value = { keep, remove };
}

const removeBody = computed(() => {
  const c = confirmRemove.value;
  if (!c) return "";
  const desc = [
    c.remove.description || "（无摘要）",
    c.remove.counterparty || "（无对方）",
    fmtAmount(c.remove.amountCents),
  ].join(" · ");
  return `将保留「${c.keep.voucherNo}」，删除「${c.remove.voucherNo}」（${desc}）。删除会移除该凭证及其图片文件，凭证本身不可恢复；被删凭证的完整快照会记入"已处理"留痕，日后可追溯。`;
});

async function doRemove() {
  const c = confirmRemove.value;
  if (!c) return;
  confirmRemove.value = null;
  busy.value = true;
  try {
    await api.removeDuplicateVoucher(c.keep.id, c.remove.id, note.value.trim());
    toast.success(
      `已删除重复凭证「${c.remove.voucherNo}」，已保留「${c.keep.voucherNo}」（可在"已处理"查看留痕）`,
    );
    closeCompare();
    await load();
    await loadHandled();
  } catch (e) {
    toastError(e);
  } finally {
    busy.value = false;
  }
}

// ---------- 已处理：留痕查看 ----------

const traceVisible = ref(false);
const trace = ref<DupDecision | null>(null);

function showTrace(d: DupDecision) {
  trace.value = d;
  traceVisible.value = true;
}

function openVoucher(id: number | null | undefined) {
  if (id == null) return;
  // v0.19：编辑在资料库右栏——带 focus 深链过去并聚焦该张
  router.push({ path: "/vouchers", query: { focus: String(id) } });
}

// ---------- 深链：?pair=a-b（保存提示的「去核查」直接定位到那一对） ----------

function pairFromQuery() {
  const raw = String(route.query.pair ?? "");
  const m = raw.match(/^(\d+)-(\d+)$/);
  if (!m) return;
  const [x, y] = [Number(m[1]), Number(m[2])];
  for (const g of groups.value) {
    for (const p of g.pending) {
      if (pairKey(p.a.id, p.b.id) === pairKey(x, y)) {
        void selectPair(g, p);
        return;
      }
    }
  }
  toast.info("这一对已经没有待核查项了（可能刚被处置）");
}

let unlisten: (() => void) | null = null;

onMounted(async () => {
  await load();
  pairFromQuery();
  // 别处（编辑页等）处置后刷新本页；注册失败只告警（safeListen）
  unlisten = await safeListen("dup:changed", () => {
    void load();
  });
});

onBeforeUnmount(() => {
  unlisten?.();
});
</script>

<template>
  <div class="dup-page">
    <!-- 页头：口径说明（"同日期同金额"是判定口径本身，先说清楚） -->
    <div class="fluent-panel">
      <div class="fluent-panel-header">
        <h3>疑似重复核查</h3>
        <span class="dup-sub">
          判定口径：<strong>凭证日期相同 且 金额相同</strong>（金额为空的凭证不参与）。
          这是<strong>提示</strong>，不阻断任何操作——同日两笔相同金额的正常业务同样会被列出。
        </span>
      </div>
      <div class="fluent-panel-body dup-bar">
        <div class="dup-stat">
          <span class="dup-num">{{ summary.groups }}</span> 组待核查 ·
          <span class="dup-num">{{ summary.pairs }}</span> 对未判定
        </div>
        <div class="dup-tabs">
          <button
            class="btn"
            :class="{ 'btn-primary': tab === 'pending' }"
            @click="tab = 'pending'"
          >
            待处理<span v-if="summary.pairs">（{{ summary.pairs }}）</span>
          </button>
          <button
            class="btn"
            :class="{ 'btn-primary': tab === 'handled' }"
            @click="
              tab = 'handled';
              loadHandled();
            "
          >
            已处理<span v-if="decisions.length">（{{ decisions.length }}）</span>
          </button>
          <button
            class="icon-btn"
            title="刷新"
            @click="
              load();
              if (tab === 'handled') loadHandled();
            "
          >
            <FluentIcon name="refresh" :size="14" />
          </button>
        </div>
      </div>
    </div>

    <!-- 待处理：按组列出 -->
    <template v-if="tab === 'pending'">
      <div v-if="loading" class="dup-loading">加载中…</div>
      <EmptyState
        v-else-if="!groups.length"
        icon="check"
        title="没有待核查的疑似重复"
        desc="所有同日期同金额的对都已处置（标记过「不是重复」或已确认删除的不再列出）。新导入的凭证若与旧凭证同日期同金额，会在这里重新出现。"
      />

      <div
        v-for="g in groups"
        :key="`${g.voucherDate}-${g.amountCents}`"
        class="fluent-panel dup-group"
      >
        <div class="fluent-panel-header">
          <h3>{{ groupHeadline(g) }}</h3>
          <span class="dup-progress">
            {{ g.pending.length + g.decidedPairs }} 对中已判定 {{ g.decidedPairs }} 对 · 剩
            {{ g.pending.length }} 对
          </span>
        </div>
        <div class="fluent-panel-body" style="padding: 0">
          <div
            v-for="p in orderedPairs(g)"
            :key="pairKey(p.a.id, p.b.id)"
            class="dup-pair"
            :class="{
              active: sel && pairKey(sel.pair.a.id, sel.pair.b.id) === pairKey(p.a.id, p.b.id),
            }"
          >
            <span
              class="tag"
              :class="evidenceMeta(p.evidence.level).tag"
              :title="evidenceMeta(p.evidence.level).advice"
            >
              置信度 {{ evidenceMeta(p.evidence.level).label }}
            </span>
            <div class="dup-pair-main">
              <a class="mono dup-link" href="#" @click.prevent="openVoucher(p.a.id)">{{
                p.a.voucherNo
              }}</a>
              <span class="dup-vs">↔</span>
              <a class="mono dup-link" href="#" @click.prevent="openVoucher(p.b.id)">{{
                p.b.voucherNo
              }}</a>
              <span class="dup-why">{{ evidenceReasons(p.evidence).join(" · ") }}</span>
            </div>
            <button class="btn btn-primary" :disabled="busy" @click="selectPair(g, p)">
              <FluentIcon name="zoomIn" :size="14" /> 核对
            </button>
            <button
              class="btn"
              :disabled="busy"
              title="放到该组末尾，稍后再处理（本次会话有效）"
              @click="skip(p)"
            >
              稍后
            </button>
            <span v-if="skipped.has(pairKey(p.a.id, p.b.id))" class="dup-later">已稍后</span>
          </div>
        </div>
      </div>
    </template>

    <!-- 已处理：处置历史（含留痕） -->
    <div v-else class="fluent-panel">
      <div class="fluent-panel-header"><h3>处置历史</h3></div>
      <div class="fluent-panel-body" style="padding: 0">
        <div v-if="decisions.length" class="dtable-wrap">
          <table class="dtable">
            <thead>
              <tr>
                <th>处置时间</th>
                <th>结论</th>
                <th>涉及凭证</th>
                <th>备注</th>
                <th>操作</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="d in decisions" :key="d.id">
                <td>{{ fmtDate(d.createdAt) }}</td>
                <td>
                  <span class="tag" :class="decisionTag(d).tag">{{ decisionTag(d).label }}</span>
                </td>
                <td>{{ decisionSummary(d) }}</td>
                <td
                  style="
                    max-width: 260px;
                    overflow: hidden;
                    text-overflow: ellipsis;
                    white-space: nowrap;
                  "
                  :title="d.note"
                >
                  {{ d.note || "—" }}
                </td>
                <td>
                  <div class="action-row">
                    <button
                      v-if="d.decision === 'duplicate_removed'"
                      class="icon-btn"
                      title="查看留痕（被删凭证快照）"
                      @click="showTrace(d)"
                    >
                      <FluentIcon name="file" :size="14" />
                    </button>
                    <button
                      v-if="d.keepId != null"
                      class="icon-btn"
                      title="打开保留的凭证"
                      @click="openVoucher(d.keepId)"
                    >
                      <FluentIcon name="list" :size="14" />
                    </button>
                  </div>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
        <EmptyState
          v-else
          icon="list"
          title="还没有处置记录"
          desc="在「待处理」里标记「不是重复」或确认重复删除后，处置记录（含留痕）会显示在这里"
        />
      </div>
    </div>

    <!-- 旁列对比（选中一对后出现） -->
    <div v-if="sel" class="fluent-panel dup-compare">
      <div class="fluent-panel-header">
        <h3>核对这一对</h3>
        <span class="dup-sub">
          {{ evidenceReasons(sel.pair.evidence).join(" · ") }} · {{ rows.length }} 项字段中
          {{ diffs }} 项不同
        </span>
        <button class="icon-btn" title="收起" @click="closeCompare">
          <FluentIcon name="close" :size="14" />
        </button>
      </div>
      <div class="fluent-panel-body">
        <div class="dup-sides">
          <div v-for="(v, side) in [sel.pair.a, sel.pair.b]" :key="v.id" class="dup-side">
            <div class="dup-side-head">
              <span class="mono">{{ v.voucherNo }}</span>
              <span class="tag" :class="sel.pair.evidence.sameImage ? 'tag-danger' : ''">
                {{ side === 0 ? "左" : "右" }}
              </span>
            </div>
            <div class="dup-img">
              <img
                v-if="imgUrl(v, side === 0 ? 'a' : 'b')"
                :src="imgUrl(v, side === 0 ? 'a' : 'b')!"
                :alt="v.voucherNo"
                @error="onImgError(v, side === 0 ? 'a' : 'b')"
              />
              <div v-else class="dup-img-empty">
                <FluentIcon name="image" :size="24" /><span>无附件或图片不可显示</span>
              </div>
            </div>
          </div>
        </div>

        <!-- 字段对照：差异高亮 -->
        <table class="dtable dup-diff">
          <thead>
            <tr>
              <th>字段</th>
              <th>{{ sel.pair.a.voucherNo }}</th>
              <th>{{ sel.pair.b.voucherNo }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="r in rows" :key="r.key" :class="{ 'dup-differ': !r.same }">
              <td>{{ r.label }}</td>
              <td>{{ r.left }}</td>
              <td>{{ r.right }}</td>
            </tr>
          </tbody>
        </table>

        <div class="dup-actions">
          <input
            class="input"
            v-model="note"
            placeholder="备注（可选，记入留痕：如「供应商重复上传」「同一天两笔正常付款」）"
            style="flex: 1"
            :disabled="busy"
          />
          <button class="btn" :disabled="busy" @click="markNotDuplicate">
            <FluentIcon name="check" :size="14" /> 不是重复
          </button>
          <button
            class="btn btn-danger"
            :disabled="busy"
            @click="askRemove(sel.pair.a, sel.pair.b)"
          >
            <FluentIcon name="delete" :size="14" /> 保留左 · 删除右
          </button>
          <button
            class="btn btn-danger"
            :disabled="busy"
            @click="askRemove(sel.pair.b, sel.pair.a)"
          >
            <FluentIcon name="delete" :size="14" /> 保留右 · 删除左
          </button>
          <button class="btn" :disabled="busy" @click="skip(sel.pair)">稍后再说</button>
        </div>
        <div class="dup-hint">
          「不是重复」只记一条判定，<strong>不改动凭证</strong>（封账月份也能标记）；
          「删除」会移除凭证与图片文件，<strong>不可恢复</strong>，但会留下被删凭证的完整快照（已处理页可查）。
        </div>
      </div>
    </div>

    <!-- 删除二次确认 -->
    <ConfirmDialog
      :visible="confirmRemove !== null"
      title="确认是同一笔业务的重复凭证？"
      :body="removeBody"
      confirm-text="确认重复并删除"
      :danger="true"
      @confirm="doRemove"
      @cancel="confirmRemove = null"
    />

    <!-- 留痕查看 -->
    <div v-if="traceVisible" class="dialog-mask" @click.self="traceVisible = false">
      <div class="dialog" role="dialog" aria-modal="true" aria-label="被删凭证留痕">
        <h3>留痕 · 被删凭证快照</h3>
        <div class="dialog-body">
          <div v-if="trace" class="dup-trace">
            <div class="dup-trace-row">
              <span>结论</span><b>{{ decisionSummary(trace) }}</b>
            </div>
            <div class="dup-trace-row">
              <span>处置时间</span><b>{{ fmtDate(trace.createdAt) }}</b>
            </div>
            <div class="dup-trace-row">
              <span>备注</span><b>{{ trace.note || "—" }}</b>
            </div>
            <div class="dup-trace-row">
              <span>被删凭证编号</span
              ><b class="mono">{{ trace.removedPayload?.voucher?.voucherNo ?? "—" }}</b>
            </div>
            <div class="dup-trace-row">
              <span>日期</span><b>{{ trace.removedPayload?.voucher?.voucherDate ?? "—" }}</b>
            </div>
            <div class="dup-trace-row">
              <span>金额</span
              ><b>{{ fmtAmount(trace.removedPayload?.voucher?.amountCents ?? null) }}</b>
            </div>
            <div class="dup-trace-row">
              <span>摘要</span><b>{{ trace.removedPayload?.voucher?.description || "—" }}</b>
            </div>
            <div class="dup-trace-row">
              <span>对方账户</span><b>{{ trace.removedPayload?.voucher?.counterparty || "—" }}</b>
            </div>
            <div class="dup-trace-row" v-if="trace.removedPayload?.attachments?.length">
              <span>附件</span>
              <b>
                <span v-for="att in trace.removedPayload.attachments" :key="att.id" class="mono">
                  {{ att.fileName }}（{{ att.fileHash.slice(0, 12) }}…）
                </span>
              </b>
            </div>
            <div class="dup-trace-row" v-else><span>附件</span><b>—（无附件记录）</b></div>
          </div>
        </div>
        <div class="dialog-ops">
          <button class="btn" @click="traceVisible = false">关闭</button>
          <button
            v-if="trace?.removedPayload?.keptVoucherId != null"
            class="btn btn-primary"
            @click="
              traceVisible = false;
              openVoucher(trace!.removedPayload!.keptVoucherId);
            "
          >
            打开保留的凭证
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.dup-page {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.dup-sub {
  font-size: var(--fs-caption);
  color: var(--text-3);
  font-weight: 400;
  max-width: 62ch;
}
.dup-bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
.dup-stat {
  font-size: var(--fs-caption);
  color: var(--text-2);
}
.dup-num {
  font-weight: 600;
  color: var(--text);
}
.dup-tabs {
  display: flex;
  gap: 8px;
  align-items: center;
}
.dup-loading {
  padding: 24px;
  text-align: center;
  color: var(--text-3);
}

.dup-group .fluent-panel-body {
  padding: 0;
}
.dup-progress {
  font-size: var(--fs-caption);
  color: var(--text-3);
}
.dup-pair {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border-bottom: 1px solid var(--border-subtle, transparent);
}
.dup-pair:last-child {
  border-bottom: none;
}
.dup-pair.active {
  background: var(--surface-2, transparent);
}
.dup-pair-main {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.dup-link {
  color: var(--accent, inherit);
  text-decoration: none;
}
.dup-vs {
  color: var(--text-3);
}
.dup-why {
  font-size: var(--fs-caption);
  color: var(--text-3);
}
.dup-later {
  font-size: var(--fs-caption);
  color: var(--text-3);
}

.dup-sides {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
  margin-bottom: 12px;
}
.dup-side {
  border: 1px solid var(--stroke);
  border-radius: var(--radius-control);
  padding: 8px;
}
.dup-side-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 6px;
}
.dup-img {
  height: 220px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--surface-2, rgba(0, 0, 0, 0.02));
  border-radius: var(--radius-control);
  overflow: hidden;
}
.dup-img img {
  max-width: 100%;
  max-height: 100%;
  object-fit: contain;
}
.dup-img-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  color: var(--text-3);
  font-size: var(--fs-caption);
}

.dup-diff {
  margin-bottom: 12px;
}
.dup-differ td {
  background: var(--warning-bg, rgba(255, 196, 0, 0.12));
}
.dup-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
.dup-hint {
  font-size: var(--fs-caption);
  color: var(--text-3);
  margin-top: 8px;
}
.dup-trace {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.dup-trace-row {
  display: grid;
  grid-template-columns: 96px 1fr;
  gap: 8px;
  font-size: var(--fs-caption);
}
.dup-trace-row > span {
  color: var(--text-3);
}
</style>
