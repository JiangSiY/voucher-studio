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
    window.addEventListener("load", () => {
      navigator.serviceWorker.register("/sw.js").catch(() => {
        /* 离线壳注册失败不影响主流程 */
      });
    });
  }
}
