/**
 * 全局 UI 状态（v0.18）：命令面板开合等**跨组件**的界面状态。
 *
 * 为什么提成 store：工具栏（`WorkbenchLayout`）、全局快捷键（`App.vue`）与命令面板本身
 * 都要控制"面板是否打开"，此前状态只存在于 `App.vue` 的局部 ref，工具栏无从触发。
 * 批次 6 的任务中心（长任务进度浮层）也复用本 store。
 */
import { defineStore } from "pinia";
import { ref } from "vue";

export const useUiStore = defineStore("ui", () => {
  const paletteVisible = ref(false);

  function openPalette() {
    paletteVisible.value = true;
  }

  function closePalette() {
    paletteVisible.value = false;
  }

  function togglePalette() {
    paletteVisible.value = !paletteVisible.value;
  }

  return { paletteVisible, openPalette, closePalette, togglePalette };
});
