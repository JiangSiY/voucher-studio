<script setup lang="ts">
/**
 * 台账字段面板（Center v0.26）：把「这张凭证怎么入账」说清楚的那一组字段。
 *
 * 为什么单独成面板而不塞进「凭证字段」：
 * - 凭证字段是**档案层**（摘要/日期/金额/对方/备注），任何一张图都要填；
 * - 台账字段是**入账层**（单据类型/分类/账户/项目/票据/来源单据号），
 *   只有要对账、出分录、回写台账时才必须齐。两层混在一起，用户会以为每张都要填满。
 *
 * 面板职责：
 * 1. 编辑 + 保存 `voucher_ext`；
 * 2. 实时显示**还缺什么**（后端 `missingRequired` 同口径）；
 * 3. 显示归档名预览，并允许一键按台账公式改名（落盘 + 写库）。
 */
import { computed, onMounted, ref, watch } from "vue";
import { api, type BizDetail, type CategoryMap } from "../../api/client";
import {
  ACCOUNT_KEYS,
  DOC_TYPES,
  EMPTY_BIZ_FIELDS,
  type BizFields,
} from "../../composables/bizFields";
import { toast, toastError } from "../../stores/toast";

const props = defineProps<{
  voucherId: number;
  /** 紧凑布局（与编辑器一致） */
  dense?: boolean;
}>();

const emit = defineEmits<{
  /** 字段或归档名变动（宿主刷新详情/列表） */
  changed: [];
}>();

const loading = ref(true);
const saving = ref(false);
const renaming = ref(false);
const detail = ref<BizDetail | null>(null);
const form = ref<BizFields>({ ...EMPTY_BIZ_FIELDS });
const categories = ref<string[]>([]);
const open = ref(false);

const TICKET_TYPES = ["增值税专票", "增值税普票", "收据", "无票"];

const missing = computed(() => detail.value?.missing ?? []);
const ready = computed(() => missing.value.length === 0);
const archivePreview = computed(() => {
  if (!detail.value) return "";
  return detail.value.archiveName || detail.value.archiveError;
});
const isWarehouse = computed(() => form.value.docType === "仓库");
const isMeter = computed(() => form.value.docType === "电费");
const needsInvoice = computed(() => form.value.ticketType === "增值税专票");

/** 后端按「日期/金额/对方」判定必填，这里的对方可能写在凭证档案层，不在本面板 */
async function load() {
  loading.value = true;
  try {
    detail.value = await api.getBizFields(props.voucherId);
    form.value = { ...detail.value.fields };
  } catch (e) {
    toastError(e);
  } finally {
    loading.value = false;
  }
}

async function loadCategories() {
  try {
    const maps: CategoryMap[] = await api.glListCategoryMaps();
    categories.value = maps.map((m) => m.category);
  } catch {
    // 分类候选只是辅助，失败时退化成自由输入
  }
}

async function save() {
  if (saving.value) return;
  saving.value = true;
  try {
    detail.value = await api.saveBizFields(props.voucherId, form.value);
    form.value = { ...detail.value.fields };
    toast.success(
      detail.value.missing.length
        ? `已保存，还缺：${detail.value.missing.join("、")}`
        : "台账字段已保存，必填项齐全",
    );
    emit("changed");
  } catch (e) {
    toastError(e);
  } finally {
    saving.value = false;
  }
}

async function rename() {
  if (renaming.value) return;
  renaming.value = true;
  try {
    const r = await api.applyArchiveName(props.voucherId);
    const changed = r.files.filter((f) => f.oldName !== f.newName);
    toast.success(
      changed.length
        ? `已改名 ${changed.length} 个文件：${changed[0].newName}`
        : "文件已是目标名字，无需改名",
    );
    await load();
    emit("changed");
  } catch (e) {
    toastError(e);
  } finally {
    renaming.value = false;
  }
}

onMounted(() => {
  void load();
  void loadCategories();
});

watch(
  () => props.voucherId,
  () => void load(),
);
</script>

<template>
  <div class="fluent-panel">
    <div class="fluent-panel-header">
      <h3>
        台账字段
        <span v-if="!loading && !ready" class="lf-badge">{{ missing.length }} 项待补</span>
        <span v-else-if="!loading" class="lf-badge lf-badge-ok">已齐</span>
      </h3>
      <button class="lf-toggle" type="button" @click="open = !open">
        {{ open ? "收起" : "展开" }}
      </button>
    </div>
    <div v-if="open" class="fluent-panel-body">
      <p v-if="loading" class="lf-hint">读取中…</p>
      <template v-else>
        <div class="lf-grid" :class="{ dense }">
          <label class="lf-field">
            <span class="lf-label">单据类型</span>
            <select v-model="form.docType" class="input">
              <option value="">（未填）</option>
              <option v-for="t in DOC_TYPES" :key="t" :value="t">{{ t }}</option>
            </select>
          </label>
          <label class="lf-field">
            <span class="lf-label">分类</span>
            <input
              v-model="form.category"
              class="input"
              list="lf-categories"
              placeholder="如：施工外包"
            />
            <datalist id="lf-categories">
              <option v-for="c in categories" :key="c" :value="c" />
            </datalist>
          </label>
          <label class="lf-field">
            <span class="lf-label">结算账户</span>
            <select v-model="form.account" class="input">
              <option value="">（未填）</option>
              <option v-for="k in ACCOUNT_KEYS" :key="k" :value="k">{{ k }}</option>
            </select>
          </label>
          <label class="lf-field">
            <span class="lf-label">项目名称</span>
            <input v-model="form.projectName" class="input" placeholder="如：张帅-585kW-自投电站" />
          </label>
          <label class="lf-field">
            <span class="lf-label">票据类型</span>
            <select v-model="form.ticketType" class="input">
              <option value="">（无票/未填）</option>
              <option v-for="t in TICKET_TYPES" :key="t" :value="t">{{ t }}</option>
            </select>
          </label>
          <label class="lf-field">
            <span class="lf-label">
              发票号
              <span v-if="needsInvoice" class="lf-req">必填</span>
            </span>
            <input
              v-model="form.invoiceNo"
              class="input"
              :disabled="form.ticketType === '无票'"
              placeholder="专票必须填"
            />
          </label>
          <label class="lf-field">
            <span class="lf-label">
              来源单据号
              <span class="lf-req">归档名必填</span>
            </span>
            <input v-model="form.sourceDocRef" class="input" placeholder="V5 单据号 / 流水序号" />
          </label>
          <label v-if="isWarehouse" class="lf-field">
            <span class="lf-label">
              物料名称
              <span class="lf-req">仓库单必填</span>
            </span>
            <input v-model="form.materialName" class="input" />
          </label>
          <label v-if="isMeter" class="lf-field">
            <span class="lf-label">电表编号</span>
            <input
              v-model="form.meterNo"
              class="input"
              placeholder="如：表A-01（V5「5 电表」表号）"
            />
          </label>
          <label v-if="isMeter" class="lf-field">
            <span class="lf-label">
              电费月度
              <span class="lf-req">电费单必填</span>
            </span>
            <input v-model="form.meterMonth" class="input" type="month" placeholder="2026-10" />
          </label>
        </div>

        <div class="lf-row">
          <span class="lf-label">还缺</span>
          <span v-if="ready" class="lf-ok">必填项已齐全，可以复核</span>
          <span v-else class="lf-missing">{{ missing.join("、") }}</span>
        </div>

        <div class="lf-row">
          <span class="lf-label">归档名</span>
          <code class="lf-archive">{{ archivePreview || "（还不能算：字段没填全）" }}</code>
          <button
            class="btn"
            type="button"
            :disabled="renaming || !detail?.archiveName"
            :title="detail?.archiveError || '按台账公式给附件改名'"
            @click="rename()"
          >
            {{ renaming ? "改名中…" : "按台账改名" }}
          </button>
        </div>

        <div class="lf-ops">
          <button class="btn" type="button" :disabled="loading" @click="load()">还原</button>
          <button class="btn btn-primary" type="button" :disabled="saving" @click="save()">
            {{ saving ? "保存中…" : "保存台账字段" }}
          </button>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
.lf-badge {
  margin-left: 8px;
  padding: 1px 8px;
  border-radius: 999px;
  font-size: var(--fs-caption);
  font-weight: 500;
  background: var(--warning-soft);
  color: var(--warning);
}
.lf-badge-ok {
  background: var(--success-soft);
  color: var(--success);
}
.lf-toggle {
  border: none;
  background: none;
  color: var(--text-2);
  cursor: pointer;
  font-size: var(--fs-caption);
}
.lf-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px 16px;
}
.lf-grid.dense {
  grid-template-columns: minmax(0, 1fr);
}
.lf-field {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.lf-label {
  font-size: var(--fs-caption);
  color: var(--text-2);
}
.lf-req {
  margin-left: 6px;
  font-size: var(--fs-caption);
  color: var(--danger);
}
.lf-hint {
  color: var(--text-2);
  font-size: var(--fs-caption);
}
.lf-row {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 10px;
  flex-wrap: wrap;
}
.lf-missing {
  color: var(--danger);
  font-size: var(--fs-caption);
}
.lf-ok {
  color: var(--success);
  font-size: var(--fs-caption);
}
.lf-archive {
  padding: 2px 8px;
  border-radius: var(--radius-small);
  background: var(--bg-card-sunken);
  font-size: var(--fs-caption);
}
.lf-ops {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 12px;
}
</style>
