import { createRouter, createWebHashHistory } from "vue-router";
import WorkbenchLayout from "../layouts/WorkbenchLayout.vue";

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    {
      path: "/",
      component: WorkbenchLayout,
      children: [
        { path: "", redirect: "/dashboard" },
        { path: "dashboard", name: "dashboard", component: () => import("../pages/DashboardPage.vue"), meta: { title: "仪表盘" } },
        { path: "vouchers", name: "vouchers", component: () => import("../pages/VoucherListPage.vue"), meta: { title: "凭证列表" } },
        { path: "vouchers/new", name: "voucher-new", component: () => import("../pages/VoucherFormPage.vue"), meta: { title: "新建凭证" } },
        { path: "vouchers/:id", name: "voucher-edit", component: () => import("../pages/VoucherFormPage.vue"), meta: { title: "编辑凭证" } },
        { path: "search", name: "search", component: () => import("../pages/SearchPage.vue"), meta: { title: "搜索" } },
        { path: "print", name: "print", component: () => import("../pages/PrintCenterPage.vue"), meta: { title: "打印中心" } },
        { path: "exports", name: "exports", component: () => import("../pages/ExportHistoryPage.vue"), meta: { title: "导出历史" } },
        { path: "settings", name: "settings", component: () => import("../pages/SettingsPage.vue"), meta: { title: "设置" } },
        { path: "restore", name: "restore", component: () => import("../pages/RestorePage.vue"), meta: { title: "备份与恢复" } },
        { path: "help", name: "help", component: () => import("../pages/HelpPage.vue"), meta: { title: "帮助" } },
      ],
    },
  ],
});
