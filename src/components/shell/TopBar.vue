<script setup lang="ts">
/**
 * 顶部页签栏（v0.27）：固定四页签靠左，动作在右；不预留系统窗口按钮区。
 */
import { computed, onMounted, onUnmounted, ref } from "vue";
import { RouterLink, useRouter } from "vue-router";
import { isTauri } from "../../api/env";
import { useInboxStore } from "../../stores/inbox";
import { useSessionStore } from "../../stores/session";
import { useUiStore } from "../../stores/ui";
import { useModalRoute } from "../../composables/useModalRoute";
import FluentIcon from "../FluentIcon.vue";

const ui = useUiStore();
const inbox = useInboxStore();
const session = useSessionStore();
const router = useRouter();
const { modal, open } = useModalRoute();

const tabs = [
  { to: "/workbench", label: "工作台", short: "工作台" },
  { to: "/data", label: "数据", short: "数据" },
  { to: "/reconcile", label: "对账", short: "对账" },
  { to: "/accounting", label: "会计投影", short: "投影" },
] as const;

const backupStale = computed(() => inbox.todos?.backupStale === true);

const menuOpen = ref(false);
const actions = ref<HTMLElement | null>(null);

function onDocPointer(e: MouseEvent) {
  if (!menuOpen.value) return;
  if (actions.value && !actions.value.contains(e.target as Node)) menuOpen.value = false;
}

onMounted(() => document.addEventListener("mousedown", onDocPointer));
onUnmounted(() => document.removeEventListener("mousedown", onDocPointer));

async function logout() {
  menuOpen.value = false;
  await session.logout();
  await router.push("/login");
}
</script>

<template>
  <header class="topbar">
    <div class="topbar-brand">
      <span class="brand-mark" aria-hidden="true">凭</span>
      <span class="brand-name">凭证中心</span>
    </div>

    <nav class="tabstrip" aria-label="主导航">
      <RouterLink v-for="t in tabs" :key="t.to" :to="t.to" class="tab">
        <span class="tab-label">{{ t.label }}</span>
        <span class="tab-short" aria-hidden="true">{{ t.short }}</span>
      </RouterLink>
    </nav>

    <div ref="actions" class="shell-actions">
      <button
        class="shell-search"
        type="button"
        aria-label="搜索凭证"
        title="搜索凭证（⌘K）"
        @click="ui.openPalette()"
      >
        <FluentIcon name="search" :size="14" />
        <span>搜索</span>
        <kbd>⌘K</kbd>
      </button>
      <button
        class="shell-btn shell-search-compact"
        type="button"
        aria-label="搜索凭证"
        @click="ui.openPalette()"
      >
        <FluentIcon name="search" :size="16" />
      </button>

      <button
        class="shell-btn shell-btn-dot"
        type="button"
        :aria-label="backupStale ? '备份（备份已过期）' : '备份'"
        title="备份与恢复"
        @click="open('backup')"
      >
        <FluentIcon name="save" :size="16" />
        <span class="shell-label">备份</span>
        <span v-if="backupStale" class="shell-dot" aria-hidden="true"></span>
      </button>

      <button
        class="shell-btn"
        type="button"
        aria-label="设置"
        title="设置"
        :aria-current="modal === 'settings' ? 'page' : undefined"
        @click="open('settings')"
      >
        <FluentIcon name="settings" :size="16" />
        <span class="shell-label">设置</span>
      </button>

      <button
        class="avatar"
        type="button"
        aria-label="账户与帮助"
        aria-haspopup="menu"
        :aria-expanded="menuOpen"
        @click="menuOpen = !menuOpen"
      >
        <FluentIcon name="help" :size="16" />
      </button>

      <div v-if="menuOpen" class="shell-menu" role="menu">
        <button class="shell-menu-item" type="button" role="menuitem" @click="open('help')">
          <FluentIcon name="help" :size="14" /> 帮助与快捷键
        </button>
        <button
          v-if="!isTauri && session.authRequired"
          class="shell-menu-item"
          type="button"
          role="menuitem"
          @click="logout"
        >
          <FluentIcon name="arrowLeft" :size="14" /> 退出登录
        </button>
      </div>
    </div>
  </header>
</template>

<style scoped>
.tab-short {
  display: none;
}

.shell-search-compact {
  display: none;
}

.shell-btn-dot {
  position: relative;
}

.shell-dot {
  position: absolute;
  top: 6px;
  right: 4px;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--warning);
}

.shell-actions {
  position: relative;
}

.shell-menu {
  position: absolute;
  top: calc(100% + 6px);
  right: 0;
  z-index: 80;
  min-width: 176px;
  padding: 4px;
  border: 1px solid var(--separator);
  border-radius: var(--radius-panel);
  background: var(--bg-popover);
  box-shadow: var(--shadow-popover);
}

.shell-menu-item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 8px 10px;
  border: 0;
  border-radius: var(--radius-control);
  background: transparent;
  color: var(--text);
  font-size: var(--fs-caption);
  text-align: left;
  cursor: pointer;
}

.shell-menu-item:hover {
  background: var(--bg-selection);
}

@media (max-width: 767px) {
  .tab-label {
    display: none;
  }

  .tab-short {
    display: inline;
  }

  .shell-search-compact {
    display: inline-flex;
  }
}
</style>
