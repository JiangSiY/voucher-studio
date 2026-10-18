/**
 * 视觉回归对比：重新截一遍页面（进 `target/visual-diff/current/`），与
 * `docs/验收基线/视觉基线/` 的基线逐像素比较。
 *
 * - 输出差异百分比（不同像素 / 总像素）与 diff 图（`target/visual-diff/diff/*.diff.png`）；
 * - 超过阈值（默认 0.5%）以非零码退出，可直接接 CI；
 * - 对比在 Chromium 的 canvas 里做，不引入额外图像依赖。
 *
 * 用法：
 *   node scripts/visual/compare.mjs
 *   node scripts/visual/compare.mjs --threshold 0.1 --base-url http://localhost:5173
 */
import { chromium } from "playwright";
import { mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { captureAll, BASELINE_DIR, REPO_ROOT, VIEWPORT, DEVICE_SCALE_FACTOR } from "./capture.mjs";
import { ensureServer, DEFAULT_BASE_URL } from "./server.mjs";

export const DIFF_ROOT = path.join(REPO_ROOT, "target/visual-diff");

/**
 * 在页面里解码两张 PNG 并逐像素比较，返回差异统计与 diff 图 data URL。
 * 颜色完全一致才算相同（同一浏览器、同一 DPR 下重跑应当是 0 差异）。
 */
async function diffInBrowser(page, aDataUrl, bDataUrl) {
  return page.evaluate(
    async ({ aDataUrl, bDataUrl }) => {
      const load = (src) =>
        new Promise((resolve, reject) => {
          const img = new Image();
          img.onload = () => resolve(img);
          img.onerror = () => reject(new Error(`图片解码失败: ${src.slice(0, 48)}…`));
          img.src = src;
        });
      const [a, b] = await Promise.all([load(aDataUrl), load(bDataUrl)]);
      const width = a.naturalWidth;
      const height = a.naturalHeight;
      if (width !== b.naturalWidth || height !== b.naturalHeight) {
        return {
          sizeMismatch: true,
          baseline: { width, height },
          current: { width: b.naturalWidth, height: b.naturalHeight },
        };
      }

      const canvasA = document.createElement("canvas");
      canvasA.width = width;
      canvasA.height = height;
      const ctxA = canvasA.getContext("2d", { willReadFrequently: true });
      ctxA.drawImage(a, 0, 0);
      const canvasB = document.createElement("canvas");
      canvasB.width = width;
      canvasB.height = height;
      const ctxB = canvasB.getContext("2d", { willReadFrequently: true });
      ctxB.drawImage(b, 0, 0);

      const da = ctxA.getImageData(0, 0, width, height).data;
      const db = ctxB.getImageData(0, 0, width, height).data;
      const out = ctxA.createImageData(width, height);
      const od = out.data;
      let diffPixels = 0;
      for (let i = 0; i < da.length; i += 4) {
        const same = da[i] === db[i] && da[i + 1] === db[i + 1] && da[i + 2] === db[i + 2];
        if (same) {
          // 相同处压成浅灰底图，差异处才醒目（便于一眼定位变化区域）
          const g = Math.round(da[i] * 0.299 + da[i + 1] * 0.587 + da[i + 2] * 0.114);
          const v = Math.round(255 - (255 - g) * 0.3);
          od[i] = v;
          od[i + 1] = v;
          od[i + 2] = v;
          od[i + 3] = 255;
        } else {
          diffPixels += 1;
          od[i] = 255;
          od[i + 1] = 32;
          od[i + 2] = 32;
          od[i + 3] = 255;
        }
      }
      ctxA.putImageData(out, 0, 0);
      return {
        sizeMismatch: false,
        width,
        height,
        totalPixels: width * height,
        diffPixels,
        ratio: diffPixels / (width * height),
        diffDataUrl: canvasA.toDataURL("image/png"),
      };
    },
    { aDataUrl, bDataUrl },
  );
}

function parseArgs(argv) {
  const out = { baseUrl: DEFAULT_BASE_URL, threshold: 0.5 };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--base-url") out.baseUrl = argv[++i];
    else if (a === "--threshold") out.threshold = Number(argv[++i]);
    else if (a === "--help" || a === "-h") out.help = true;
  }
  if (!Number.isFinite(out.threshold) || out.threshold < 0) {
    throw new Error("--threshold 必须是 ≥0 的数字（百分比，例如 0.5）");
  }
  return out;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help) {
    console.log("用法: node scripts/visual/compare.mjs [--threshold 0.5] [--base-url URL]");
    return;
  }

  let files;
  try {
    files = (await readdir(BASELINE_DIR)).filter((f) => f.endsWith(".png")).sort();
  } catch {
    console.error(`找不到基线目录：${BASELINE_DIR}\n先运行 node scripts/visual/capture.mjs`);
    process.exitCode = 1;
    return;
  }
  if (!files.length) {
    console.error(`基线目录为空：${BASELINE_DIR}`);
    process.exitCode = 1;
    return;
  }

  await rm(DIFF_ROOT, { recursive: true, force: true });
  const currentDir = path.join(DIFF_ROOT, "current");
  const diffDir = path.join(DIFF_ROOT, "diff");
  await mkdir(diffDir, { recursive: true });

  const server = await ensureServer({ baseUrl: args.baseUrl });
  let results;
  let captureMeta;
  try {
    console.log(`\n重新截图 → ${path.relative(REPO_ROOT, currentDir)}/`);
    captureMeta = await captureAll({
      baseUrl: server.baseUrl,
      outDir: currentDir,
      evidenceDir: null,
      quiet: true,
    });
    const currentFiles = new Set(captureMeta.shots.map((s) => `${s.name}.png`));
    const baselineSet = new Set(files);

    const browser = await chromium.launch({ headless: true });
    try {
      const page = await browser.newPage();
      results = [];
      for (const file of files) {
        const name = file.replace(/\.png$/, "");
        const baselinePath = path.join(BASELINE_DIR, file);
        const currentPath = path.join(currentDir, file);
        if (!currentFiles.has(file)) {
          results.push({
            name,
            file,
            missingCurrent: true,
            ratio: 1,
            diffPixels: 0,
            totalPixels: 0,
          });
          continue;
        }
        const a = (await readFile(baselinePath)).toString("base64");
        const b = (await readFile(currentPath)).toString("base64");
        const r = await diffInBrowser(
          page,
          `data:image/png;base64,${a}`,
          `data:image/png;base64,${b}`,
        );
        const entry = { name, file, baselinePath, currentPath, ...r };
        if (!r.sizeMismatch) {
          const diffFile = path.join(diffDir, `${name}.diff.png`);
          await writeFile(diffFile, Buffer.from(r.diffDataUrl.split(",")[1], "base64"));
          entry.diffFile = diffFile;
        }
        results.push(entry);
        const pct = r.sizeMismatch ? "尺寸不同" : `${(r.ratio * 100).toFixed(4)}%`;
        console.log(
          `  ${file.padEnd(24)} ${pct.padStart(11)}  ${r.sizeMismatch ? "" : `(${r.diffPixels} / ${r.totalPixels} px)`}`,
        );
      }
      // 基线里没有、但本次截出来的图：提示多余，不阻断
      for (const extra of [...currentFiles].filter((f) => !baselineSet.has(f))) {
        console.log(`  ⚠️  基线缺失（本次多出）：${extra}`);
      }
    } finally {
      await browser.close();
    }
  } finally {
    await server.stop();
  }

  const failed = results.filter(
    (r) => r.missingCurrent || r.sizeMismatch || r.ratio * 100 > args.threshold,
  );
  const maxRatio = results.reduce(
    (m, r) => Math.max(m, r.missingCurrent || r.sizeMismatch ? 1 : r.ratio),
    0,
  );
  const summary = {
    generatedAt: new Date().toISOString(),
    baseUrl: server.baseUrl,
    viewport: VIEWPORT,
    deviceScaleFactor: DEVICE_SCALE_FACTOR,
    thresholdPercent: args.threshold,
    baselineDir: BASELINE_DIR,
    currentDir,
    diffDir,
    total: results.length,
    failed: failed.length,
    maxDiffPercent: Number((maxRatio * 100).toFixed(4)),
    results: results.map((r) => ({
      name: r.name,
      baseline: path.relative(REPO_ROOT, r.baselinePath ?? path.join(BASELINE_DIR, r.file)),
      current: path.relative(REPO_ROOT, r.currentPath ?? path.join(currentDir, r.file)),
      diffImage: r.diffFile ? path.relative(REPO_ROOT, r.diffFile) : null,
      diffPercent: r.sizeMismatch || r.missingCurrent ? null : Number((r.ratio * 100).toFixed(4)),
      diffPixels: r.sizeMismatch || r.missingCurrent ? null : r.diffPixels,
      totalPixels: r.sizeMismatch || r.missingCurrent ? null : r.totalPixels,
      sizeMismatch: !!r.sizeMismatch,
      missingCurrent: !!r.missingCurrent,
    })),
  };
  const reportFile = path.join(DIFF_ROOT, "report.json");
  await writeFile(reportFile, `${JSON.stringify(summary, null, 2)}\n`);

  console.log(
    `\n共 ${results.length} 张 · 最大差异 ${summary.maxDiffPercent.toFixed(4)}% · 阈值 ${args.threshold}% · 报告 ${path.relative(REPO_ROOT, reportFile)}`,
  );
  if (failed.length) {
    console.error(`\n❌ 视觉回归失败：${failed.map((r) => r.file).join(", ")}`);
    console.error(`   diff 图见 ${path.relative(REPO_ROOT, diffDir)}/`);
    process.exitCode = 1;
  } else {
    console.log(`\n✅ 视觉回归通过（全部 ≤ ${args.threshold}%）`);
  }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  main().catch((err) => {
    console.error(err);
    process.exitCode = 1;
  });
}
