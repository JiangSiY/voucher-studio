<script setup lang="ts">
/**
 * 设置 → 凭证参数：编号模板（实时校验预览）、常用摘要模板、期间管理（按月封账）。
 * draftCfg 为共享编辑缓冲（stores/settingsDraft，体检 #22 单向流改造）：
 * Tab 不再接收可变 cfg prop，而是读写共享 store 状态，保存由设置页整体提交。
 */
import { computed, onMounted, ref } from "vue";
import { storeToRefs } from "pinia";
import { api, errMsg, type PeriodLock } from "../../api/client";
import FluentIcon from "../FluentIcon.vue";
import ConfirmDialog from "../ConfirmDialog.vue";
import { toast, toastError } from "../../stores/toast";
import { useSettingsDraft } from "../../stores/settingsDraft";

const { cfg: draftCfg } = storeToRefs(useSettingsDraft());

// ---------- 编号模板校验与预览 ----------

const templatePreview = ref("");
const templateError = ref("");

async function validateTemplate() {
  templateError.value = "";
  try {
    // 后端返回归一化模板；预览用今日日期本地渲染全部令牌
    // （preview_voucher_no 基于已保存配置，编辑中的模板尚未保存）
    const normalized = await api.validateNamingTemplate(draftCfg.value.naming.template);
    const now = new Date();
    const y = now.getFullYear();
    const m = String(now.getMonth() + 1).padStart(2, "0");
    const d = String(now.getDate()).padStart(2, "0");
    templatePreview.value = normalized
      .replace(/\{date-\}/g, `${y}-${m}-${d}`)
      .replace(/\{date\}/g, `${y}${m}${d}`)
      .replace(/\{seq\}/g, "001")
      .replace(/\{n\}/g, "01");
  } catch (e) {
    templateError.value = errMsg(e);
    templatePreview.value = "";
  }
}

// ---------- v0.10：常用摘要模板 ----------

/** 每行一条 ↔ 数组互转；空行/首尾空白自动清理 */
const descTemplatesText = computed({
  get: () => (draftCfg.value.voucher?.descriptionTemplates ?? []).join("\n"),
  set: (v: string) => {
    if (!draftCfg.value.voucher) draftCfg.value.voucher = { descriptionTemplates: [] };
    draftCfg.value.voucher.descriptionTemplates = v
      .split("\n")
      .map((s) => s.trim())
      .filter(Boolean);
  },
});

// ---------- v0.11：会计期间管理（按月封账） ----------

const periodLocks = ref<PeriodLock[]>([]);
/** 解锁确认（二次确认对话框：解锁意味着该月凭证可再被修改） */
const unlockPending = ref<string | null>(null);

async function loadPeriodLocks() {
  try {
    periodLocks.value = await api.listPeriodLocks();
  } catch {
    periodLocks.value = [];
  }
}

/** 锁定某月（YYYY-MM）：该月凭证即刻禁止增删改 */
async function lockPeriodAction(period: string) {
  try {
    await api.lockPeriod(period);
    toast.success(`已锁定 ${period}：该月凭证不可增删改`);
    await loadPeriodLocks();
  } catch (e) {
    toastError(e);
  }
}

/** 解锁（经二次确认） */
async function unlockPeriodAction() {
  const p = unlockPending.value;
  if (!p) return;
  try {
    await api.unlockPeriod(p);
    toast.success(`已解锁 ${p}：该月凭证可正常修改`);
    unlockPending.value = null;
    await loadPeriodLocks();
  } catch (e) {
    toastError(e);
  }
}

/** 生成期间选项：近 18 个月（当月起倒推，覆盖常见补录窗口） */
const periodOptions = computed(() => {
  const now = new Date();
  const out: { period: string; label: string; locked: boolean }[] = [];
  for (let i = 0; i < 18; i++) {
    const d = new Date(now.getFullYear(), now.getMonth() - i, 1);
    const period = `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`;
    out.push({
      period,
      label: `${d.getFullYear()} 年 ${d.getMonth() + 1} 月`,
      locked: periodLocks.value.some((l) => l.period === period),
    });
  }
  return out;
});

onMounted(() => {
  validateTemplate();
  loadPeriodLocks();
});
</script>

<template>
  <div class="fluent-panel">
    <div class="fluent-panel-body">
      <div class="field">
        <label>凭证编号模板</label>
        <input v-model="draftCfg.naming.template" class="input mono" @input="validateTemplate" />
        <div class="desc">
          可用令牌：<code>{date}</code>（20260819）、<code>{date-}</code>（2026-08-19）、<code>{seq}</code>（001 起）、<code>{n}</code>（01 起）。
          默认 <code>{date-}-{n}</code> 生成形如 2026-08-19-01 的编号。
        </div>
        <div v-if="templateError" class="desc" style="color: var(--danger); display: flex; align-items: center; gap: 4px">
          <FluentIcon name="warning" :size="13" />{{ templateError }}
        </div>
        <div v-else-if="templatePreview" class="desc" style="color: var(--success); display: flex; align-items: center; gap: 4px">
          <FluentIcon name="check" :size="13" />保存后新凭证编号形如：{{ templatePreview }}
        </div>
      </div>
      <div class="callout info" style="margin: 0">
        <span class="ci"><FluentIcon name="bulb" :size="18" /></span>
        <div class="ct">修改模板只影响之后新建的凭证，已有编号不会变。</div>
      </div>
      <!-- v0.10：常用摘要模板（编辑页联想置顶项） -->
      <div class="field" style="border-top: 1px solid var(--stroke); padding-top: 16px; margin-top: 16px">
        <label>常用摘要模板（每行一条）</label>
        <textarea v-model="descTemplatesText" class="input" rows="5" style="resize: vertical"
          placeholder="如：&#10;办公用品采购&#10;差旅住宿费&#10;市内交通费"></textarea>
        <div class="desc">
          编辑凭证时聚焦「摘要」输入框，会联想这些模板（置顶）并合并历史高频摘要，点选即填，减少重复录入。
        </div>
      </div>

      <!-- v0.11：期间管理（按月封账） -->
      <div class="field" style="border-top: 1px solid var(--stroke); padding-top: 16px; margin-top: 16px">
        <label>期间管理（按月封账）</label>
        <div class="period-grid">
          <div v-for="p in periodOptions" :key="p.period" class="period-row" :class="{ locked: p.locked }">
            <span class="period-label">
              <FluentIcon v-if="p.locked" name="lock" :size="12" />
              {{ p.label }}
            </span>
            <button v-if="!p.locked" class="btn btn-sm" @click="lockPeriodAction(p.period)">锁定</button>
            <button v-else class="btn btn-sm btn-danger" @click="unlockPending = p.period">解锁</button>
          </div>
        </div>
        <div class="desc">
          锁定的月份内凭证禁止编辑 / 删除 / 批量操作 / 换图（防止封账后返工导致账实不符）。
          OCR 自动填单不受限（仅填空字段）。解锁需确认。
        </div>
      </div>

      <!-- 解锁二次确认 -->
      <ConfirmDialog :visible="unlockPending != null" title="解锁期间"
        :body="`确定解锁 ${unlockPending ?? ''} 吗？该月凭证将可再次修改，可能造成已导出报表与数据不一致。`"
        confirm-text="解锁" danger @confirm="unlockPeriodAction" @cancel="unlockPending = null" />
    </div>
  </div>
</template>

<style scoped>
/* v0.11 期间管理：近 18 个月锁定/解锁网格 */
.period-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
  gap: 6px;
  margin-bottom: 8px;
}

.period-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 6px 10px;
  border: 1px solid var(--stroke);
  border-radius: 6px;
}

.period-row.locked {
  border-color: var(--warning, #c88a17);
  background: rgba(200, 138, 23, 0.06);
}

.period-label {
  display: flex;
  align-items: center;
  gap: 5px;
  font-size: 13px;
  color: var(--text);
}

.period-row.locked .period-label {
  color: var(--warning, #a06a00);
  font-weight: 600;
}

/* 解锁按钮（危险加强版；与扫描矫正 Tab 保持一致） */
.btn.btn-danger {
  /* v0.18：不再自绘红色渐变（Fluent 时代遗留）——改用统一的危险按钮语言：
     文字红 + hover 淡红底，由 base.css 的 .btn-danger 提供 */
  background: var(--bg-control);
  border-color: var(--danger-soft);
  color: var(--danger);
}
.btn.btn-danger:hover:not(:disabled) {
  background: var(--danger-soft);
}
</style>
