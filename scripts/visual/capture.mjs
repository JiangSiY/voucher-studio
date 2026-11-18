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
import { MOCK, installTauriMock } from "./fixtures.mjs";
import { ensureServer, DEFAULT_BASE_URL } from "./server.mjs";

export const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
export const BASELINE_DIR = path.join(REPO_ROOT, "docs/验收基线/视觉基线");
export const EVIDENCE_DIR = path.join(REPO_ROOT, "docs/验收基线/证据/2026-11-18-B5-v0.27");

/** 截图视口与像素密度（验收要求：1440×900、DPR 2、深色、默认字号） */
export const VIEWPORT = { width: 1440, height: 900 };
export const DEVICE_SCALE_FACTOR = 2;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function waitForStableImages(page) {
  await Promise.race([
    page.evaluate(async () => {
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
    }),
    new Promise((resolve) => setTimeout(resolve, 3000)),
  ]);
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
    name: "workbench",
    hash: "#/workbench",
    label: "工作台 · KPI / 趋势 / 待办",
    ready: async (page) => {
      await page.waitForSelector(".wb-kpis .wb-kpi");
      await page.waitForSelector(".wb-grid .wb-card");
    },
  },
  {
    name: "data",
    hash: "#/data",
    label: "数据 · 视图 chips + 表格",
    ready: async (page) => {
      await page.waitForSelector(".data-table .data-row");
      await page.waitForSelector(".data-chips .chip");
    },
  },
  {
    name: "data-expanded",
    hash: "#/data",
    label: "数据 · 行内展开编辑",
    prepare: async (page) => {
      await page.locator(".data-table .data-row").first().click({ force: true });
      await page.waitForSelector(".data-expand");
      await page.waitForTimeout(1500);
    },
  },
  {
    name: "reconcile",
    hash: "#/reconcile",
    label: "对账 · 台账比对",
    ready: async (page) => {
      await page.waitForSelector(".page-head");
      await page.waitForSelector(".page-head");
    },
  },
  {
    name: "accounting",
    hash: "#/accounting",
    label: "会计投影 · 期间/分录/映射",
    ready: async (page) => {
      await page.waitForSelector(".page-head");
    },
  },
  {
    name: "modal-settings",
    hash: "#/data?modal=settings",
    label: "弹层 · 设置",
    ready: async (page) => {
      await page.waitForSelector(".modal-card .settings-actions, .modal-card .settings-body");
    },
  },
  {
    name: "modal-export",
    hash: "#/data?modal=export",
    label: "弹层 · 打印与导出",
    ready: async (page) => {
      await page.waitForSelector(".modal-card .pick-row");
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
    page.on("pageerror", (err) => console.error(`  [pageerror] ${err.message}`));
    page.on("console", (msg) => {
      if (msg.type() === "error") console.error(`  [console] ${msg.text()}`);
    });

    for (const shot of SHOTS) {
      const url = `${baseUrl}/${shot.hash}`;
      if (!quiet) console.log(`→ ${shot.label}  ${url}`);
      await page.goto(url, { waitUntil: "domcontentloaded" });
      // 让 App.vue 的 config/recovery/事件注册与各页 onMounted 的 invoke 全部落地
      await page.waitForSelector(".topbar");
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
