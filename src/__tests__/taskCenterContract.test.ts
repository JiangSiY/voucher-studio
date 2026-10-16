// @vitest-environment node
/**
 * 任务中心契约（用户 2026-10-16：「再然后做任务中心」；规范 §1.5 要求「任务中心（全局浮层，可最小化）」）。
 *
 * 锁三件容易悄悄退化的东西：
 *   1. 浮层挂在 `App.vue`（任何页面都可见），且**可最小化**（规范明文要求）；
 *   2. 长任务真的接了线——导入 / 关闭前备份（`App.vue`）、生成导出（`PrintCenterPage`）、
 *      批量识别（`OcrTab` + `ocr:progress`）、重建增强（`EnhanceTab` + `enhance:progress`）；
 *   3. 不跟 toast 抢角落：右下角归 toast，任务中心在内容区左下。
 */
import { existsSync, readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const read = (rel: string) => readFileSync(new URL(`../../${rel}`, import.meta.url), "utf8");

const app = read("src/App.vue");
const center = read("src/components/TaskCenter.vue");
const store = read("src/stores/tasks.ts");
const print = read("src/pages/PrintCenterPage.vue");
const ocr = read("src/components/settings/OcrTab.vue");
const enhance = read("src/components/settings/EnhanceTab.vue");
const baseCss = read("src/styles/base.css");

describe("任务中心", () => {
  it("浮层挂在 App.vue，且任何页面都可见（不是某个页面内部的进度条）", () => {
    expect(app).toContain('import TaskCenter from "./components/TaskCenter.vue"');
    expect(app).toMatch(/<TaskCenter \/>/);
    expect(app).toContain("useTaskStore");
  });

  it("可最小化：头部就是开关，带 aria-expanded 且收起后仍在", () => {
    expect(center).toContain("store.toggleCollapsed()");
    expect(center).toMatch(/:aria-expanded="!store\.collapsed"/);
    expect(center).toContain('aria-controls="task-center-list"');
    expect(center).toMatch(/v-show="!store\.collapsed"/);
    expect(store).toContain("function toggleCollapsed()");
    expect(store).toMatch(/collapsed\.value = false; \/\/ 有新任务就展开|collapsed\.value = false;/);
  });

  it("进度条：有总量才画确定态，不知道进度走不确定态（不假装卡在 0%）", () => {
    expect(center).toMatch(/t\.total <= 0/);
    expect(center).toContain('role="progressbar"');
    expect(center).toContain("is-indeterminate");
  });

  it("不跟 toast 抢角落：右下角归 toast，任务中心在内容区左下并跟随侧栏宽度", () => {
    expect(baseCss).toMatch(/\.toast-host \{[\s\S]{0,120}bottom: var\(--s-5\);[\s\S]{0,80}right: var\(--s-5\)/);
    const css = center.slice(center.indexOf("<style scoped>"));
    expect(css).toMatch(/\.task-center \{[\s\S]{0,160}left: calc\(var\(--sidebar-w\) \+ var\(--s-5\)\)/);
    // 收起态/窄窗是 56px 图标栏，靠 --sidebar-w 跟不动，必须单独纠
    expect(css).toContain(':root[data-sidebar-collapsed="true"] .task-center');
    expect(css).toMatch(/@media \(max-width: 760px\)/);
    // z-index 必须低于 toast（1000），结果提示永远压在上面
    expect(css).toMatch(/z-index: 900/);
  });

  it("导入与关闭前备份接进任务中心（Rust 事件驱动）", () => {
    expect(app).toMatch(/import:started"[\s\S]{0,400}taskStore\.start\("import"/);
    expect(app).toMatch(/import:done"[\s\S]{0,2000}taskStore\.finish\(importTaskId/);
    expect(app).toMatch(/import:error"[\s\S]{0,400}taskStore\.fail\(importTaskId/);
    expect(app).toMatch(/backup:closing"[\s\S]{0,300}taskStore\.start\("backup"/);
    expect(app).toMatch(/backup:done"[\s\S]{0,400}taskStore\.finish\(closingBackupTaskId/);
    expect(app).toMatch(/backup:failed"[\s\S]{0,400}taskStore\.fail\(closingBackupTaskId/);
  });

  it("生成导出 / 批量识别 / 重建增强都上报进度与终态", () => {
    expect(print).toMatch(/taskStore\.start\("export"/);
    expect(print).toMatch(/taskStore\.finish\(taskId/);
    expect(print).toMatch(/taskStore\.fail\(taskId/);

    expect(ocr).toMatch(/taskStore\.start\("ocr"/);
    expect(ocr).toMatch(/ocr:progress"[\s\S]{0,300}taskStore\.progress\(ocrTaskId/);
    expect(ocr).toMatch(/taskStore\.finish\(taskId/);

    expect(enhance).toMatch(/taskStore\.start\("enhance"/);
    expect(enhance).toMatch(/enhance:progress"[\s\S]{0,600}taskStore\.progress\(enhanceTaskId/);
    expect(enhance).toMatch(/taskStore\.finish\(enhanceTaskId/);
    expect(enhance).toMatch(/taskStore\.fail\(enhanceTaskId/);
  });

  it("store 自身具备运行时/已完成/汇总三个观察面（组件不自己算）", () => {
    for (const k of ["running", "finished", "runningCount", "summary", "trim"]) {
      expect(store, `store 缺少 ${k}`).toContain(k);
    }
    expect(existsSync(new URL("../../src/stores/tasks.ts", import.meta.url))).toBe(true);
  });
});
