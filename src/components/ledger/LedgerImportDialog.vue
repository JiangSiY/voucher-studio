<script setup lang="ts">
/**
 * 台账清单导入向导（v13 `/ledger` 页的主操作）。
 *
 * 四步同一浮层：选类型 → 选文件 → **自动预演** → 写入 + 结果。
 *
 * 为什么预演是自动的而不是「先点预演再点写入」：台账导入一次 100 张，
 * 用户真正要看的是"这个文件会新建多少、命中多少、坏几行"，dry-run 几乎无成本，
 * 手动点一次纯属多一步（2026-11-14 与用户定稿的交互）。
 *
 * 为什么类型要手选、不按文件名猜：三张清单的表头不同（流水号 / 单据号），
 * 猜错会得到"缺必需列"的报错而不是清晰的选择——宁可多一次点击。
 */
import { computed, ref, watch } from "vue";
import { api, type ImportError, type ImportSummary, type LedgerKindName } from "../../api/client";
import FluentIcon from "../FluentIcon.vue";
import { useModal } from "../../composables/useModal";
import { toastError } from "../../stores/toast";

const props = defineProps<{ visible: boolean }>();
const emit = defineEmits<{ close: []; done: [batchId: number | null] }>();

const KINDS: { value: LedgerKindName; label: string; hint: string }[] = [
  { value: "shoukuan", label: "收款", hint: "流水号 · 收款清单.csv" },
  { value: "fukuan", label: "付款", hint: "流水号 · 付款清单.csv" },
  { value: "jiti", label: "计提", hint: "单据号 · 计提清单.csv" },
];

const kind = ref<LedgerKindName>("shoukuan");
const csvPath = ref("");
const fileName = computed(() => csvPath.value.split(/[\\/]/).pop() ?? "");
const preview = ref<ImportSummary | null>(null);
const result = ref<ImportSummary | null>(null);
const busy = ref<"pick" | "preview" | "commit" | null>(null);
const showErrors = ref(false);

const { panelEl } = useModal(
  () => props.visible,
  () => close(),
);

function reset() {
  csvPath.value = "";
  preview.value = null;
  result.value = null;
  busy.value = null;
  showErrors.value = false;
}

watch(
  () => props.visible,
  (v) => {
    if (v) reset();
  },
);

function close() {
  if (busy.value) return;
  emit("close");
}

/** 选文件 → 立刻预演（不写库） */
async function pick() {
  busy.value = "pick";
  try {
    const picked = await api.pickLedgerCsv();
    if (!picked) return;
    csvPath.value = picked;
    preview.value = null;
    result.value = null;
    busy.value = "preview";
    preview.value = await api.importLedgerCsv(picked, kind.value, true);
  } catch (e) {
    toastError(e);
    csvPath.value = "";
  } finally {
    busy.value = null;
  }
}

/** 换类型：已选过文件就按新类型重新预演（表头不同，结果会变） */
async function setKind(k: LedgerKindName) {
  if (kind.value === k) return;
  kind.value = k;
  if (!csvPath.value) return;
  busy.value = "preview";
  preview.value = null;
  result.value = null;
  try {
    preview.value = await api.importLedgerCsv(csvPath.value, k, true);
  } catch (e) {
    toastError(e);
  } finally {
    busy.value = null;
  }
}

const newCount = computed(() => preview.value?.created ?? 0);
const matchedCount = computed(() => preview.value?.matched ?? 0);
const errorList = computed<ImportError[]>(() => preview.value?.errors ?? []);

async function commit() {
  if (!csvPath.value || newCount.value <= 0) return;
  busy.value = "commit";
  try {
    result.value = await api.importLedgerCsv(csvPath.value, kind.value, false);
  } catch (e) {
    toastError(e);
  } finally {
    busy.value = null;
  }
}

function finish() {
  emit("done", result.value?.batchId ?? null);
  emit("close");
}
</script>

<template>
  <Teleport to="body">
    <div v-if="visible" class="dialog-mask" @click.self="close">
      <div
        ref="panelEl"
        class="dialog ledger-wizard"
        role="dialog"
        aria-modal="true"
        aria-label="导入台账清单"
        tabindex="-1"
      >
        <h3>导入台账清单</h3>

        <!-- ===== 结果态 ===== -->
        <div v-if="result" class="dialog-body">
          <p class="wz-line">
            已写入：
            <strong>{{ result.created }}</strong> 张新建 ·
            <strong>{{ result.matched }}</strong> 张已存在跳过 ·
            <strong>{{ result.skipped }}</strong> 行异常
          </p>
          <p class="wz-hint">新凭证是草稿态，还没有票据。下一步去「待补附件」把扫描件挂上即可。</p>
        </div>

        <!-- ===== 选择 / 预演态 ===== -->
        <div v-else class="dialog-body">
          <div class="segmented" role="tablist" aria-label="清单类型">
            <button
              v-for="k in KINDS"
              :key="k.value"
              class="seg-item"
              role="tab"
              :aria-selected="kind === k.value"
              :class="{ active: kind === k.value }"
              :disabled="!!busy"
              @click="setKind(k.value)"
            >
              {{ k.label }}
            </button>
          </div>
          <p class="wz-hint">
            {{ KINDS.find((k) => k.value === kind)?.hint }} · 与台账「流水号 /
            单据号」一一对应，重复导入不会重复建卡
          </p>

          <button class="btn wz-pick" :disabled="!!busy" @click="pick">
            <FluentIcon name="upload" :size="14" />
            {{ csvPath ? fileName : "选择 CSV 文件…" }}
          </button>

          <div v-if="busy === 'preview'" class="wz-hint">正在预演（不写库）…</div>
          <template v-else-if="preview">
            <div class="wz-preview">
              <span
                >新建 <strong>{{ newCount }}</strong></span
              >
              <span
                >已存在 <strong>{{ matchedCount }}</strong></span
              >
              <span :class="{ 'wz-bad': errorList.length }">
                坏行 <strong>{{ errorList.length }}</strong>
              </span>
            </div>
            <div v-if="newCount === 0" class="wz-hint">
              这份文件不会新建任何凭证——已经导过了，或全部行都有问题。
            </div>
            <div v-if="errorList.length" class="wz-errors">
              <button class="wz-link" @click="showErrors = !showErrors">
                {{ showErrors ? "收起" : "查看" }}逐行原因
              </button>
              <ul v-if="showErrors">
                <li v-for="err in errorList.slice(0, 50)" :key="err.line">
                  第 {{ err.line }} 行 · {{ err.ledgerRef || "（无流水号）" }} · {{ err.reason }}
                </li>
              </ul>
              <p class="wz-hint">错误明细同时落盘到 exports/import-errors-*.csv</p>
            </div>
          </template>
        </div>

        <div class="dialog-ops">
          <template v-if="result">
            <button class="btn" @click="finish">完成</button>
          </template>
          <template v-else>
            <button class="btn" :disabled="!!busy" @click="close">取消</button>
            <button
              class="btn btn-primary"
              :disabled="!!busy || newCount <= 0"
              data-autofocus
              @click="commit"
            >
              {{ busy === "commit" ? "写入中…" : `写入 ${newCount} 张` }}
            </button>
          </template>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.ledger-wizard {
  width: min(520px, 92vw);
}

.wz-line {
  margin: 0 0 var(--s-2);
}

.wz-hint {
  margin: var(--s-2) 0 0;
  font-size: var(--fs-caption);
  color: var(--label-secondary);
}

.wz-pick {
  margin-top: var(--s-3);
  width: 100%;
  justify-content: flex-start;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.wz-preview {
  display: flex;
  gap: var(--s-5);
  margin-top: var(--s-3);
  padding: var(--s-3) var(--s-4);
  border-radius: var(--radius-small);
  background: var(--bg-card-sunken);
  font-size: var(--fs-caption);
  color: var(--label-secondary);
}

.wz-preview strong {
  color: var(--label-primary);
}

.wz-bad strong {
  color: var(--danger);
}

.wz-errors {
  margin-top: var(--s-3);
  font-size: var(--fs-caption);
}

.wz-link {
  border: none;
  background: none;
  padding: 0;
  color: var(--accent);
  cursor: pointer;
  text-decoration: underline;
}

.wz-errors ul {
  max-height: 160px;
  overflow: auto;
  margin: var(--s-2) 0;
  padding-left: var(--s-4);
  color: var(--label-secondary);
}
</style>
