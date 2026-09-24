<script setup lang="ts">
import { computed } from "vue";
import { useRoute } from "vue-router";
import { useConfigStore } from "../stores/config";
import FluentIcon from "../components/FluentIcon.vue";
import SidebarResizeHandle from "../components/layout/SidebarResizeHandle.vue";

const route = useRoute();
const configStore = useConfigStore();

// 阅读型页面（表单 / 设置 / 帮助）保留 1100px 上限；宽内容页（列表/仪表盘/打印中心）铺满
const isReading = computed(() => route.meta.reading === true);

const version = computed(() => configStore.appInfo?.version ?? "…");

const navGroups = [
  {
    group: "概览",
    items: [
      { to: "/dashboard", icon: "dashboard", label: "仪表盘" },
      { to: "/search", icon: "search", label: "搜索" },
    ],
  },
  {
    group: "凭证管理",
    items: [
      { to: "/vouchers", icon: "list", label: "凭证列表" },
      { to: "/duplicates", icon: "duplicate", label: "重复核查" },
      { to: "/print", icon: "print", label: "打印中心" },
      { to: "/exports", icon: "chart", label: "导出历史" },
    ],
  },
  {
    group: "数据与设置",
    items: [
      { to: "/settings", icon: "settings", label: "设置" },
      { to: "/restore", icon: "save", label: "备份与恢复" },
      { to: "/help", icon: "help", label: "帮助" },
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
          <router-link v-for="item in g.items" :key="item.to" :to="item.to" :title="item.label">
            <span class="nav-icon"><FluentIcon :name="item.icon" :size="17" /></span>
            <span class="nav-label">{{ item.label }}</span>
          </router-link>
        </template>
      </nav>
      <div class="sidebar-footer">v{{ version }} · 数据本地存储</div>
    </aside>

    <!-- 侧边栏宽度拖拽手柄：挂在 shell 上（侧栏自身滚动，放里面会被滚走） -->
    <SidebarResizeHandle />

    <div class="main-area">
      <div class="main-scroll">
        <div class="page" :class="{ 'page--reading': isReading }">
          <h1 class="page-title">{{ pageTitle }}</h1>
          <router-view />
        </div>
      </div>
    </div>
  </div>
</template>
