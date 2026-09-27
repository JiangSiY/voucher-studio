<script setup lang="ts">
/**
 * v0.13 全局命令面板（Ctrl+K）：页面跳转 + 快速动作 + 凭证搜索。
 * 手写组件：无第三方库；键盘 ↑↓ 导航、Enter 执行、Esc 关闭。
 * 搜索模式：输入命中命令名后仍无结果时自动搜凭证（复用全文搜索，含 OCR 文本）。
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useRouter } from "vue-router";
import { api, type SearchHit } from "../api/client";
import FluentIcon from "./FluentIcon.vue";
import { STATUS_META, fmtAmount } from "../composables/format";

const props = defineProps<{ visible: boolean }>();
const emit = defineEmits<{ (e: "close"): void }>();

const router = useRouter();

type Command = {
  key: string;
  label: string;
  hint: string;
  icon: string;
  run: () => void;
};

/** 快速动作（异步动作用 toast 反馈，面板即关） */
const commands: Command[] = [
  { key: "nav:inbox", label: "前往 待处理", hint: "今天要办的事（流程）", icon: "inbox", run: () => router.push("/inbox") },
  { key: "nav:list", label: "前往 全部凭证", hint: "筛选 / 批量操作", icon: "list", run: () => router.push("/vouchers") },
  { key: "nav:duplicates", label: "前往 重复核查", hint: "疑似重复的比对与处置", icon: "duplicate", run: () => router.push("/duplicates") },
  { key: "nav:new", label: "新建凭证", hint: "选图建卡 + OCR", icon: "plus", run: () => router.push("/vouchers/new") },
  { key: "nav:search", label: "前往 全文搜索", hint: "编号 / 摘要 / OCR 文本", icon: "search", run: () => router.push("/search") },
  { key: "nav:insights", label: "前往 洞察", hint: "趋势 / 排行 / 对账", icon: "chart", run: () => router.push("/insights") },
  { key: "nav:print", label: "前往 打印与导出", hint: "PDF / Excel / CSV", icon: "print", run: () => router.push("/print") },
  { key: "nav:exports", label: "前往 导出历史", hint: "批次反查", icon: "file", run: () => router.push("/exports") },
  { key: "nav:restore", label: "前往 备份与恢复", hint: "备份列表 / 演练", icon: "save", run: () => router.push("/restore") },
  { key: "nav:settings", label: "前往 设置", hint: "参数 / 密钥 / 期间管理", icon: "settings", run: () => router.push("/settings") },
  { key: "nav:help", label: "前往 帮助", hint: "使用说明", icon: "help", run: () => router.push("/help") },
];

const query = ref("");
const active = ref(0);
const inputEl = ref<HTMLInputElement | null>(null);
const busy = ref(false);

/** 命令过滤（关键词为空显示全部） */
const commandHits = computed(() => {
  const q = query.value.trim().toLowerCase();
  if (!q) return commands;
  return commands.filter(
    (c) => c.label.toLowerCase().includes(q) || c.hint.toLowerCase().includes(q),
  );
});

/** 凭证搜索（命令未命中时；防抖 250ms + 至少 2 字符避免全表扫） */
const voucherHits = ref<SearchHit[]>([]);
let searchTimer: ReturnType<typeof setTimeout> | null = null;

watch([query, () => props.visible], ([q, visible]) => {
  if (searchTimer) clearTimeout(searchTimer);
  voucherHits.value = [];
  active.value = 0;
  if (!visible || q.trim().length < 2 || commandHits.value.length > 0) return;
  searchTimer = setTimeout(async () => {
    busy.value = true;
    try {
      voucherHits.value = await api.searchVouchers(q.trim(), undefined, 6);
    } catch {
      voucherHits.value = []; // 搜索是辅助能力，失败静默
    } finally {
      busy.value = false;
    }
  }, 250);
});

/** 扁平结果集（命令在前、凭证在后）供键盘导航统一索引 */
const items = computed(() => {
  const cmds = commandHits.value.map((c) => ({
    kind: "cmd" as const,
    key: c.key,
    label: c.label,
    hint: c.hint,
    icon: c.icon,
    run: c.run,
  }));
  const vols = voucherHits.value.map((h) => ({
    kind: "voucher" as const,
    key: `v-${h.voucher.id}`,
    hit: h,
    run: () => router.push(`/vouchers/${h.voucher.id}`),
  }));
  return [...cmds, ...vols];
});

watch(items, () => {
  active.value = Math.min(active.value, Math.max(0, items.value.length - 1));
});

function move(delta: number) {
  const n = items.value.length;
  if (!n) return;
  active.value = (active.value + delta + n) % n;
}

function execute() {
  const it = items.value[active.value];
  if (it) {
    emit("close");
    it.run();
  }
}

function onKeydown(e: KeyboardEvent) {
  // Ctrl+K 开关由 App.vue 全局接管；本组件只处理面板可见时的按键
  if (!props.visible) return;
  if (e.key === "ArrowDown") {
    e.preventDefault();
    move(1);
  } else if (e.key === "ArrowUp") {
    e.preventDefault();
    move(-1);
  } else if (e.key === "Enter") {
    e.preventDefault();
    execute();
  } else if (e.key === "Escape") {
    e.preventDefault();
    emit("close");
  }
}

onMounted(() => window.addEventListener("keydown", onKeydown));
onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeydown);
  if (searchTimer) clearTimeout(searchTimer);
});

// 打开时自动聚焦输入框
watch(
  () => props.visible,
  async (v) => {
    if (v) {
      query.value = "";
      await nextTick();
      inputEl.value?.focus();
    }
  },
  { immediate: true },
);
</script>

<template>
  <Teleport to="body">
    <div v-if="visible" class="cp-mask" @click.self="emit('close')">
      <div class="cp-panel" role="dialog" aria-label="命令面板">
        <div class="cp-input-row">
          <FluentIcon name="search" :size="16" />
          <input ref="inputEl" v-model="query" class="cp-input"
            placeholder="输入页面 / 动作名，或直接搜凭证（编号、摘要、OCR 文本）…"
            @keydown="onKeydown" />
          <span class="cp-esc">Esc 关闭</span>
        </div>

        <div class="cp-list" v-if="items.length">
          <template v-for="(it, i) in items" :key="it.key">
            <button v-if="it.kind === 'cmd'" class="cp-item" :class="{ active: i === active }"
              @mouseenter="active = i" @click="execute()">
              <span class="cp-icon"><FluentIcon :name="it.icon" :size="15" /></span>
              <span class="cp-label">{{ it.label }}</span>
              <span class="cp-hint">{{ it.hint }}</span>
            </button>
            <button v-else class="cp-item" :class="{ active: i === active }"
              @mouseenter="active = i" @click="execute()">
              <span class="cp-icon"><FluentIcon name="image" :size="15" /></span>
              <span class="cp-label">
                <span class="mono">{{ it.hit.voucher.voucherNo }}</span>
                <span style="color: var(--text-3); margin-left: 6px">{{ it.hit.voucher.description || "（无摘要）" }}</span>
              </span>
              <span class="cp-hint">
                {{ it.hit.voucher.voucherDate }} · {{ fmtAmount(it.hit.voucher.amountCents) }} ·
                {{ STATUS_META[it.hit.voucher.status].label }}
              </span>
            </button>
          </template>
        </div>
        <div v-else class="cp-empty">
          {{ busy ? "搜索中…" : query.trim() ? "没有匹配的命令或凭证" : "↑↓ 选择 · Enter 执行 · Esc 关闭" }}
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.cp-mask {
  position: fixed;
  inset: 0;
  z-index: 300;
  background: rgba(0, 0, 0, 0.35);
  display: flex;
  align-items: flex-start;
  justify-content: center;
  padding-top: 12vh;
}

.cp-panel {
  width: min(560px, 92vw);
  background: var(--surface);
  border-radius: 10px;
  box-shadow: 0 16px 48px rgba(0, 0, 0, 0.3);
  overflow: hidden;
}

.cp-input-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 16px;
  border-bottom: 1px solid var(--stroke);
  color: var(--text-3);
}

.cp-input {
  flex: 1;
  border: none;
  outline: none;
  font-size: 14px;
  background: transparent;
  color: var(--text);
}

.cp-esc {
  font-size: 11px;
  color: var(--text-4);
  white-space: nowrap;
}

.cp-list {
  /* 随窗口高度伸缩（原为固定 320px：窗口变高列表不跟着长） */
  max-height: clamp(260px, 45vh, 520px);
  overflow-y: auto;
  padding: 6px;
}

.cp-item {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 8px 10px;
  border: none;
  border-radius: 6px;
  background: none;
  cursor: pointer;
  text-align: left;
  color: var(--text);
  font-size: 13px;
}

.cp-item.active {
  background: var(--accent-soft);
}

.cp-icon {
  color: var(--accent);
  display: inline-flex;
  flex-shrink: 0;
}

.cp-label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cp-hint {
  font-size: 12px;
  color: var(--text-3);
  flex-shrink: 0;
}

.cp-empty {
  padding: 28px 16px;
  text-align: center;
  font-size: 13px;
  color: var(--text-3);
}
</style>
