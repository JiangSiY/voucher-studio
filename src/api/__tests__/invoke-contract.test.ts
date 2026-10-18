// @vitest-environment node
/**
 * invoke 名一致性快照（体检报告第一批 #7）
 *
 * 目的：消灭最高频回归——`src/api/client.ts` 里 invoke 的后端命令名，
 * 与 `src-tauri/src/main.rs` `generate_handler![...]` 注册的 handler 不同步。
 * 任何一侧改名/漏注册/漏封装都会让本测试失败并打印两侧差集。
 *
 * 只读对比，不依赖 Tauri 运行时：
 * - client.ts：抽取所有 `invoke<T>("命令名", ...)` 字面量；
 * - main.rs：抽取 `generate_handler![commands::xxx, ...]` 内的标识符；
 * 断言两侧集合完全相等。
 */
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");

function extractClientInvokeNames(): string[] {
  const clientSrc = readFileSync(path.join(projectRoot, "src", "api", "client.ts"), "utf8");
  // 只匹配调用点 `invoke<...>("name", ...)` 或 `invoke("name", ...)`，第一个参数必须是字符串字面量。
  // 有意排除本地声明 `function invoke<T>(cmd: string, ...)`（generic 后跟 cmd，不是引号）。
  const names: string[] = [];
  const re = /invoke(?:<[^>]*>)?\s*\(\s*["']([A-Za-z0-9_]+)["']/g;
  for (const m of clientSrc.matchAll(re)) {
    names.push(m[1]);
  }
  return [...new Set(names)].sort();
}

function extractRustHandlerNames(): string[] {
  const mainSrc = readFileSync(path.join(projectRoot, "src-tauri", "src", "main.rs"), "utf8");
  const start = mainSrc.indexOf("generate_handler![");
  if (start < 0) {
    throw new Error("main.rs 中找不到 generate_handler![ ... ]");
  }
  const block = mainSrc.slice(start, mainSrc.indexOf("])", start));
  const names: string[] = [];
  const re = /commands::([A-Za-z0-9_]+)/g;
  for (const m of block.matchAll(re)) {
    names.push(m[1]);
  }
  return [...new Set(names)].sort();
}

describe("前后端 invoke 契约", () => {
  it("client.ts 的 invoke 名 = main.rs 注册的全部 handler（两侧差集可读）", () => {
    const client = extractClientInvokeNames();
    const rust = extractRustHandlerNames();

    const onlyClient = client.filter((n) => !rust.includes(n));
    const onlyRust = rust.filter((n) => !client.includes(n));

    const lines: string[] = [];
    lines.push(`client.ts invoke 数：${client.length}`);
    lines.push(`main.rs handler 数：${rust.length}`);
    if (onlyClient.length) {
      lines.push(
        `仅在 client.ts（Rust 未注册，调用会报 command not found）：${onlyClient.join(", ")}`,
      );
    }
    if (onlyRust.length) {
      lines.push(`仅在 main.rs（前端未封装，功能不可达）：${onlyRust.join(", ")}`);
    }
    expect(onlyClient, lines.join("\n")).toEqual([]);
    expect(onlyRust, lines.join("\n")).toEqual([]);
  });
});
