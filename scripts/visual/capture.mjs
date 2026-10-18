/**
 * 视觉回归截图：起一个 Chromium（headless），按页面清单逐张截图。
 *
 * 默认输出到 `docs/验收基线/视觉基线/`（入库的基线），并同步一份到
 * `docs/验收基线/证据/2026-10-18-G3-v0.22.3/`（G3 证据，正常尺寸 PNG）。
 * `compare.mjs` 复用本文件的 `captureAll()`，把新截图写到 `target/visual-diff/current/`。
 *
 * 用法：
 *   node scripts/visual/capture.mjs                 # 基线 + 证据（需要 5173 已启动或自动起）
 *   node scripts/visual/capture.mjs --base-url http://localhost:5173
 *   node scripts/visual/capture.mjs --out <dir> --no-evidence
 */
import { chromium } from "playwright";
import { mkdir, copyFile, stat } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { MOCK, SHOT_NOW, installTauriMock } from "./fixtures.mjs";
import { ensureServer, DEFAULT_BASE_URL } from "./server.mjs";

export const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
export const BASELINE_DIR = path.join(REPO_ROOT, "docs/验收基线/视觉基线");
export const EVIDENCE_DIR = path.join(REPO_ROOT, "docs/验收基线/证据/2026-10-18-G3-v0.22.3");

/** 截图视口与像素密度（验收要求：1440×900、DPR 2、深色、默认字号） */
export const VIEWPORT = { width: 1440, height: 900 };
export const DEVICE_SCALE_FACTOR = 2;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function waitForStableImages(page) {
  await page.evaluate(async () => {
    const pending = Array.from(document.images).filter((img) => !img.complete);
    await Promise.all(
      pending.map(
        (img) =>
          new Promise((resolve) => {
            img.addEventListener("load", resolve, { once: true });
            img.addEventListener("error", resolve, { once: true });
          }),
      ),
    );
    if (document.fonts?.ready) await document.fonts.ready;
  });
}

/** 截图前抹掉两类不确定性：动画/过渡未收敛、输入框光标闪烁。 */
async function prepareForShot(page) {
  await page.addStyleTag({
    content:
      "*,*::before,*::after{transition:none!important;animation:none!important;caret-color:transparent!important}",
  });
  await page.evaluate(() => {
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
    window.scrollTo(0, 0);
  });
  await waitForStableImages(page);
  await sleep(250);
}

/**
 * 页面清单（顺序即截图顺序）。
 * `ready` 只做“元素出现”的等待，`prepare` 里做该页特有的交互。
 */
const SHOTS = [
  {
    name: "vouchers-list",
    hash: "#/vouchers",
    label: "资料库 · 列表 + 右栏编辑器",
    ready: async (page) => {
      await page.waitForSelector(".rows .row[data-row-id]");
      await page.waitForSelector(".inspector .ins-head");
      await page.waitForFunction(() => document.body.innerText.includes("凭证字段"));
    },
  },
  {
    name: "vouchers-editor",
    hash: "#/vouchers",
    label: "资料库 · 选中草稿凭证（编辑器态）",
    prepare: async (page) => {
      await page.waitForSelector(".rows .row[data-row-id]");
      // 105 = 设计外包尾款（草稿状态），比默认第一张更能体现字段区与“未保存”语义
      await page.locator('.rows .row[data-row-id="105"]').click();
      await page.waitForFunction(() =>
        document.querySelector(".ins-head .mono")?.textContent?.includes("2026-10-11-01"),
      );
      await page.waitForFunction(() => document.body.innerText.includes("凭证字段"));
    },
  },
  {
    name: "insights",
    hash: "#/insights",
    label: "洞察 · 时间窗 / 覆盖度 / 存储 / 月度透视",
    ready: async (page) => {
      await page.waitForSelector(".insights");
      await page.waitForSelector(".ledgers .panel");
      await page.waitForFunction(() => document.body.innerText.includes("导出覆盖度"));
      await page.waitForFunction(() => document.querySelectorAll(".bd-month").length > 0);
    },
  },
  {
    name: "print-center",
    hash: "#/print",
    label: "打印中心 · 选中 4 张 + PDF 附录预览",
    prepare: async (page) => {
      await page.waitForSelector(".pick-rows .pick-row");
      const rows = page.locator(".pick-rows .pick-row");
      const n = Math.min(4, await rows.count());
      for (let i = 0; i < n; i++) await rows.nth(i).click();
      await page.waitForFunction(() => document.body.innerText.includes("已选 4 张"));
      await page.waitForSelector(".appendix-grid img");
    },
  },
  {
    name: "settings",
    hash: "#/settings",
    label: "设置 · 通用（默认 tab）",
    ready: async (page) => {
      await page.waitForSelector(".settings-actions");
      await page.waitForFunction(() => document.body.innerText.includes("保存设置"));
    },
  },
  {
    name: "help",
    hash: "#/help",
    label: "帮助 · 三步上手 / 界面在哪 / 快捷键 / 常见问题",
    ready: async (page) => {
      await page.waitForFunction(() => document.body.innerText.includes("三步上手"));
    },
  },
];

/**
 * 截取全部页面。
 * @param {{baseUrl?: string, outDir?: string, evidenceDir?: string|null, quiet?: boolean}} opts
 * @returns {Promise<{shots: {name: string, label: string, file: string, bytes: number, at: string}[]}>}
 */
export async function captureAll(opts = {}) {
  const baseUrl = (opts.baseUrl ?? DEFAULT_BASE_URL).replace(/\/$/, "");
  const outDir = opts.outDir ?? BASELINE_DIR;
  const evidenceDir = opts.evidenceDir === undefined ? EVIDENCE_DIR : opts.evidenceDir;
  const quiet = opts.quiet ?? false;

  await mkdir(outDir, { recursive: true });
  if (evidenceDir) await mkdir(evidenceDir, { recursive: true });

  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({
    viewport: VIEWPORT,
    deviceScaleFactor: DEVICE_SCALE_FACTOR,
    colorScheme: "dark",
    locale: "zh-CN",
    timezoneId: "Asia/Shanghai",
    reducedMotion: "reduce",
  });
  await context.addInitScript(installTauriMock, MOCK);

  const shots = [];
  try {
    const page = await context.newPage();
    // 冻结“现在”到验收日，保证洞察时间窗等文案与基线可复现
    await page.clock.install({ time: new Date(SHOT_NOW) });
    page.on("pageerror", (err) => console.error(`  [pageerror] ${err.message}`));
    page.on("console", (msg) => {
      if (msg.type() === "error") console.error(`  [console] ${msg.text()}`);
    });

    for (const shot of SHOTS) {
      const url = `${baseUrl}/${shot.hash}`;
      if (!quiet) console.log(`→ ${shot.label}  ${url}`);
      await page.goto(url, { waitUntil: "domcontentloaded" });
      // 让 App.vue 的 config/recovery/事件注册与各页 onMounted 的 invoke 全部落地
      await page.waitForFunction(() => document.body.innerText.includes("数据本地存储"));
      if (shot.ready) await shot.ready(page);
      if (shot.prepare) await shot.prepare(page);
      await prepareForShot(page);

      const file = path.join(outDir, `${shot.name}.png`);
      await page.screenshot({ path: file, fullPage: false, animations: "disabled" });
      if (evidenceDir) await copyFile(file, path.join(evidenceDir, `${shot.name}.png`));
      const info = await stat(file);
      if (!quiet)
        console.log(`  ✓ ${path.relative(REPO_ROOT, file)} (${(info.size / 1024).toFixed(0)} KB)`);
      shots.push({
        name: shot.name,
        label: shot.label,
        file,
        bytes: info.size,
        at: new Date().toISOString(),
      });
    }
  } finally {
    await context.close();
    await browser.close();
  }
  return { shots, viewport: VIEWPORT, deviceScaleFactor: DEVICE_SCALE_FACTOR, baseUrl };
}

function parseArgs(argv) {
  const out = { baseUrl: DEFAULT_BASE_URL, outDir: BASELINE_DIR, evidenceDir: EVIDENCE_DIR };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--base-url") out.baseUrl = argv[++i];
    else if (a === "--out") out.outDir = path.resolve(process.cwd(), argv[++i]);
    else if (a === "--no-evidence") out.evidenceDir = null;
    else if (a === "--help" || a === "-h") out.help = true;
  }
  return out;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    console.log(
      "用法: node scripts/visual/capture.mjs [--base-url URL] [--out DIR] [--no-evidence]",
    );
    return;
  }
  const server = await ensureServer({ baseUrl: args.baseUrl });
  try {
    const result = await captureAll({ ...args, baseUrl: server.baseUrl });
    console.log(
      `\n✅ 已生成 ${result.shots.length} 张截图（${VIEWPORT.width}×${VIEWPORT.height} @${DEVICE_SCALE_FACTOR}x）`,
    );
    if (args.evidenceDir)
      console.log(`   证据副本: ${path.relative(REPO_ROOT, args.evidenceDir)}/`);
  } finally {
    await server.stop();
  }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  main().catch((err) => {
    console.error(err);
    process.exitCode = 1;
  });
}
