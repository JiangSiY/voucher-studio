import { createApp } from "vue";
import { createPinia } from "pinia";
import App from "./App.vue";
import { router } from "./router";
import "./styles/tokens.css";
import "./styles/base.css";

const app = createApp(App);
app.use(createPinia());
app.use(router);
app.mount("#app");

// PWA：只在网页版注册离线壳；桌面版（Tauri）不注册，避免 SW 拦截 asset 协议
if (typeof navigator !== "undefined" && "serviceWorker" in navigator) {
  if (!("__TAURI_INTERNALS__" in window) && window.location.protocol === "https:") {
    // 直接注册（模块脚本执行时 DOM 已就绪），不用 load 监听——
    // 前端门禁要求 addEventListener 与 removeEventListener 配对，这里根本不需要监听
    navigator.serviceWorker.register("/sw.js").catch(() => {
      /* 离线壳注册失败不影响主流程 */
    });
  }
}
