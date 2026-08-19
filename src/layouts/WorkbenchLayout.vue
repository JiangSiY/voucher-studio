<script setup lang="ts">
import { computed } from "vue";
import { useRoute } from "vue-router";
import { useConfigStore } from "../stores/config";

const route = useRoute();
const configStore = useConfigStore();

const version = computed(() => configStore.appInfo?.version ?? "…");

const navGroups = [
  {
    group: "概览",
    items: [
      { to: "/dashboard", icon: "▦", label: "仪表盘" },
      { to: "/search", icon: "🔍", label: "搜索" },
    ],
  },
  {
    group: "凭证管理",
    items: [
      { to: "/vouchers", icon: "📋", label: "凭证列表" },
      { to: "/print", icon: "🖨", label: "打印中心" },
      { to: "/exports", icon: "📊", label: "导出历史" },
    ],
  },
  {
    group: "数据与设置",
    items: [
      { to: "/settings", icon: "⚙", label: "设置" },
      { to: "/restore", icon: "💾", label: "备份与恢复" },
      { to: "/help", icon: "?", label: "帮助" },
    ],
  },
];

const pageTitle = computed(() => (route.meta.title as string) ?? "");
</script>

<template>
  <div class="app-shell">
    <aside class="sidebar">
      <div class="sidebar-header">
        <div class="sidebar-logo">
          <span class="badge">VS</span>
          <span>
            Voucher Studio
            <br />
            <small style="font-weight: 400; font-size: 11px; color: var(--text-3)">凭证整理打印</small>
          </span>
        </div>
      </div>
      <nav class="sidebar-nav">
        <template v-for="g in navGroups" :key="g.group">
          <div class="nav-group">{{ g.group }}</div>
          <router-link v-for="item in g.items" :key="item.to" :to="item.to">
            <span class="nav-icon">{{ item.icon }}</span>
            {{ item.label }}
          </router-link>
        </template>
      </nav>
      <div class="sidebar-footer">v{{ version }} · 数据本地存储</div>
    </aside>

    <div class="main-area">
      <div class="main-scroll">
        <div class="page">
          <h1 class="page-title">{{ pageTitle }}</h1>
          <router-view />
        </div>
      </div>
    </div>
  </div>
</template>
