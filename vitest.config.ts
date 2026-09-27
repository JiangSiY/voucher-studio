import { defineConfig, mergeConfig } from "vitest/config";
import viteConfig from "./vite.config";

/**
 * vitest 配置（2026-09-27 批次 7，挂账 #12）。
 *
 * 背景：此前没有配置文件，测试全靠 vitest 默认值——默认 `environment: "node"`，
 * 需要 DOM 的用例（`mount()` 那一批）各自在文件首行写 `// @vitest-environment jsdom`。
 * 本文件**只把当前实际行为显式化，不改行为**：将来要加 setupFiles / 覆盖率 / 超时阈值，
 * 有落脚点，不必"为了加一个配置项先造一个配置文件"。
 *
 * **必须 merge 而不是另起**：vitest 一旦发现 `vitest.config.ts` 就**不再读** `vite.config.ts`，
 * 而 mount 类用例要编译 `.vue`，依赖其中的 `@vitejs/plugin-vue`——不合并会让它们整片失败。
 */
export default mergeConfig(
  viteConfig,
  defineConfig({
    test: {
      // 与默认值一致：逻辑类用例跑 node；需要 DOM 的用例用文件首行注释切 jsdom
      environment: "node",
      // 全部测试都在 src/ 下（按同目录 __tests__ 组织）；显式写出避免误扫 spikes/ 等
      include: ["src/**/*.{test,spec}.?(c|m)[jt]s?(x)"],
      // 契约守卫是"读源码文本做断言"的用例，无需并发隔离；保持默认线程池即可
      reporters: ["default"],
    },
  }),
);
