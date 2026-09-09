import { defineStore } from "pinia";
import { ref } from "vue";
import type { AppConfig } from "../api/client";
import { defaultConfig } from "../config-defaults";

/**
 * 设置页共享编辑缓冲（体检 #22：v-model 单向数据流改造）。
 *
 * 旧实现：SettingsPage 持有 `reactive(cfg)` 作为 prop 下发给各 settings Tab，
 * 子组件用 `v-model="cfg.xxx"` 直接改写 props 对象（违反 Vue 单向数据流），
 * 配套 `JSON.parse(JSON.stringify())` 深拷贝 hack。
 *
 * 新实现：把这份「保存前编辑缓冲」提升为独立 store 状态——
 * 页面负责 load（整体替换）与 save（提交缓冲）；各 Tab 读取/写入同一 store
 * 状态（对 store 的写入是显式共享状态变更，不再发生在 props 边界上）。
 * 跨 Tab 共享语义与原共享 buffer 完全一致（如 WebDAV 连接测试会连带其它 Tab
 * 尚未提交的修改一起先保存），同时不再需要为「复制出一份可改对象」而深拷贝。
 */
export const useSettingsDraft = defineStore("settings-draft", () => {
  /** 编辑缓冲：默认值单一真源见 config-defaults.ts；SettingsPage load 后整体替换 */
  const cfg = ref<AppConfig>(defaultConfig());

  /** load：以后端配置整体替换缓冲（补齐旧配置缺失的 voucher 段） */
  function replace(next: AppConfig) {
    cfg.value = {
      ...next,
      voucher: next.voucher ?? { descriptionTemplates: [] },
    };
  }

  return { cfg, replace };
});
