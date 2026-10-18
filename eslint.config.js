// @ts-check
import js from "@eslint/js";
import pluginVue from "eslint-plugin-vue";
import tseslint from "typescript-eslint";
import globals from "globals";
import prettier from "eslint-config-prettier/flat";

/**
 * ESLint flat config（体检 Q-11 / 第三批第 12 项）：
 * - 基线：eslint:recommended + typescript-eslint:recommended + vue flat/essential
 * - Vue 侧取 essential（防错误类规则）：排版格式类规则（每属性换行等）与存量
 *   风格冲突且无质量收益，不做全量格式化改写；类型问题归 vue-tsc 管
 * - 本项目为 <script setup> 风格
 */
export default tseslint.config(
  { ignores: ["dist/**", "target/**", "src-tauri/**", "spikes/**", "node_modules/**"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  ...pluginVue.configs["flat/essential"],
  {
    files: ["**/*.vue"],
    languageOptions: {
      parserOptions: { parser: tseslint.parser },
    },
  },
  // Prettier 负责排版（挂账 #6）：关掉与格式化冲突的 ESLint 规则，两者不争同一件事
  prettier,
  {
    languageOptions: {
      globals: { ...globals.browser },
    },
    rules: {
      // 入口组件 App.vue 为传统单名（Vue 官方脚手架同款例外）
      "vue/multi-word-component-names": "off",
      // 设置页「编辑缓冲区」模式：子组件直接改共享 cfg 对象属性（v-model 绑定其字段，
      // 拆分前后行为一致的既定设计），对该模式显式豁免
      "vue/no-mutating-props": "off",
      // 类型纪律从严（与体检「前端零 any」结论对齐，防回潮）
      "@typescript-eslint/no-explicit-any": "error",
    },
  },
);
