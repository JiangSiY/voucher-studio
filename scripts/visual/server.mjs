/**
 * 视觉脚本的 Vite 服务守卫：优先复用已经在跑的 5173（例如 `npm run tauri dev` 或
 * 手动 `npm run dev`），没有才自己起一个，并在结束时只杀自己起的那个。
 *
 * 这样 `capture.mjs` / `compare.mjs` 都能“一键”运行，又不会碰别人的进程。
 */
import { spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
export const DEFAULT_BASE_URL = process.env.VISUAL_BASE_URL || "http://localhost:5173";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function reachable(url) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 1500);
  try {
    const res = await fetch(url, { signal: controller.signal });
    return res.status < 500;
  } catch {
    return false;
  } finally {
    clearTimeout(timer);
  }
}

/**
 * @param {{baseUrl?: string, timeoutMs?: number}} opts
 * @returns {Promise<{baseUrl: string, owned: boolean, stop: () => Promise<void>}>}
 */
export async function ensureServer(opts = {}) {
  const baseUrl = (opts.baseUrl ?? DEFAULT_BASE_URL).replace(/\/$/, "");
  const timeoutMs = opts.timeoutMs ?? 90_000;

  if (await reachable(baseUrl)) {
    console.log(`ℹ️  ${baseUrl} 已在运行，直接复用（不会停止该进程）`);
    return { baseUrl, owned: false, stop: async () => {} };
  }
  if (!/^http:\/\/(127\.0\.0\.1|localhost):5173$/.test(baseUrl)) {
    throw new Error(`无法连接 ${baseUrl}，且只会在端口 5173 上自动启动 Vite。请先启动 npm run dev。`);
  }

  console.log(`ℹ️  ${baseUrl} 未运行，自动启动 vite（npm run dev -- --port 5173 --strictPort）…`);
  const child = spawn(
    process.platform === "win32" ? "npm.cmd" : "npm",
    ["run", "dev", "--", "--port", "5173", "--strictPort"],
    { cwd: REPO_ROOT, detached: true, stdio: ["ignore", "pipe", "pipe"] },
  );
  const logs = [];
  child.stdout.on("data", (b) => logs.push(b.toString()));
  child.stderr.on("data", (b) => logs.push(b.toString()));

  const stop = async () => {
    if (!child.pid || child.killed) return;
    try {
      process.kill(-child.pid, "SIGTERM");
    } catch {
      try {
        child.kill("SIGTERM");
      } catch {
        /* already gone */
      }
    }
    // 等端口释放，最多 5s
    for (let i = 0; i < 25; i++) {
      if (!(await reachable(baseUrl))) return;
      await sleep(200);
    }
  };

  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await reachable(baseUrl)) {
      return { baseUrl, owned: true, stop };
    }
    if (child.exitCode != null) {
      await stop();
      throw new Error(`vite 启动失败（exit ${child.exitCode}）：\n${logs.join("")}`);
    }
    await sleep(300);
  }
  await stop();
  throw new Error(`等待 vite 就绪超时（${timeoutMs}ms）：\n${logs.join("")}`);
}
