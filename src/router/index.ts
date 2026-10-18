import { createRouter, createWebHashHistory } from "vue-router";
import WorkbenchLayout from "../layouts/WorkbenchLayout.vue";

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    {
      path: "/",
      component: WorkbenchLayout,
      children: [
        { path: "", redirect: "/inbox" },
        // v0.18：收件箱成为首页（流程为中心）；旧 /dashboard 链接重定向保留
        {
          path: "inbox",
          name: "inbox",
          component: () => import("../pages/InboxPage.vue"),
          meta: { title: "待处理" },
        },
        { path: "dashboard", redirect: "/inbox" },
        // v0.18：资料库 = 双行列表 + 检视器（取代旧 10 列表格）；旧 /vouchers 仍指到这里
        {
          path: "vouchers",
          name: "vouchers",
          component: () => import("../pages/LibraryPage.vue"),
          meta: { title: "全部凭证" },
        },
        // v0.16 疑似重复核查（静态段须在 vouchers/:id 之前，避免被动态段吃掉）
        {
          path: "duplicates",
          name: "duplicates",
          component: () => import("../pages/DuplicateReviewPage.vue"),
          meta: { title: "重复核查" },
        },
        {
          path: "vouchers/new",
          name: "voucher-new",
          component: () => import("../pages/VoucherFormPage.vue"),
          meta: { title: "新建凭证", reading: true },
        },
        // v0.19（2026-09-28）：编辑页已并入资料库右栏（components/voucher/VoucherEditor.vue）。
        // 旧链接/书签 `/vouchers/:id` 保留为**重定向**：跳到资料库并聚焦该张（不 404）。
        // 注意路径顺序：静态段 `vouchers/new` 必须排在本条之前，否则会被动态段吃掉。
        {
          path: "vouchers/:id",
          redirect: (to) => ({ path: "/vouchers", query: { focus: String(to.params.id) } }),
        },
        {
          path: "search",
          name: "search",
          component: () => import("../pages/SearchPage.vue"),
          meta: { title: "搜索" },
        },
        {
          path: "print",
          name: "print",
          component: () => import("../pages/PrintCenterPage.vue"),
          meta: { title: "打印与导出" },
        },
        {
          path: "insights",
          name: "insights",
          component: () => import("../pages/InsightsPage.vue"),
          meta: { title: "洞察" },
        },
        {
          path: "exports",
          name: "exports",
          component: () => import("../pages/ExportHistoryPage.vue"),
          meta: { title: "导出历史" },
        },
        {
          path: "settings",
          name: "settings",
          component: () => import("../pages/SettingsPage.vue"),
          meta: { title: "设置", reading: true },
        },
        {
          path: "restore",
          name: "restore",
          component: () => import("../pages/RestorePage.vue"),
          meta: { title: "备份与恢复" },
        },
        {
          path: "help",
          name: "help",
          component: () => import("../pages/HelpPage.vue"),
          meta: { title: "帮助", reading: true },
        },
      ],
    },
  ],
});
