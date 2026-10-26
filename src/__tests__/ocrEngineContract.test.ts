/**
 * v0.23 OCR 引擎契约：云端优先（DeepSeek 视觉）+ 本地兜底（Apple Vision）。
 *
 * 源码级断言（不挂载组件）：设置页必须能选区 `auto / cloud / local` 三种策略、
 * 云端协议可选 `deepseek / custom`；旧的「本地模型未安装 / PP-OCRv5」文案与模型状态字段
 * 必须清干净；命令层要暴露 `ocr_health` 并在 Tauri 侧注册。
 */
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const read = (p: string) => readFileSync(resolve(process.cwd(), p), "utf8");

describe("OCR 引擎设置契约（云端优先 + 本地 Vision）", () => {
  it("设置页提供 auto / cloud / local 三档引擎策略", () => {
    const src = read("src/components/settings/OcrTab.vue");
    for (const value of ['value="auto"', 'value="cloud"', 'value="local"']) {
      expect(src).toContain(value);
    }
    // 默认推荐的文案要能让人看懂「云端优先、失败用本地」
    expect(src).toContain("云端优先");
    expect(src).toContain("Apple Vision");
  });

  it("云端协议可选 DeepSeek 视觉与自建旧协议", () => {
    const src = read("src/components/settings/OcrTab.vue");
    expect(src).toContain('value="deepseek"');
    expect(src).toContain('value="custom"');
    expect(src).toContain("deepseek-flash");
    // 上传提示：云端识别会把图片发出去，必须让用户知情
    expect(src).toContain("图片会上传");
  });

  it("不再出现 PP-OCRv5 / 本地模型安装相关文案与字段", () => {
    const tab = read("src/components/settings/OcrTab.vue");
    expect(tab).not.toContain("PP-OCRv5");
    expect(tab).not.toContain("本地模型未安装");
    expect(tab).not.toContain("modelsDir");
    const client = read("src/api/client.ts");
    expect(client).not.toContain("modelsDir");
    expect(client).not.toContain("ch_PP-OCRv5");
  });

  it("前端 API 与命令名一一对应（ocr_health / local_ocr_status）", () => {
    const client = read("src/api/client.ts");
    expect(client).toContain('invoke<OcrHealth>("ocr_health")');
    expect(client).toContain('invoke<LocalOcrStatus>("local_ocr_status")');
    expect(client).toContain("lastCloudError");

    const tauriSrc = read("src-tauri/src/main.rs");
    for (const cmd of ["commands::ocr_health", "commands::local_ocr_status"]) {
      expect(tauriSrc).toContain(cmd);
    }
  });

  it("默认配置为云端优先（engine=auto + provider=deepseek）", () => {
    const defaults = read("src/config-defaults.ts");
    expect(defaults).toContain('engine: "auto"');
    expect(defaults).toContain('provider: "deepseek"');
  });
});
