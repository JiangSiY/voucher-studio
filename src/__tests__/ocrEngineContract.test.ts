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

  it("附件面板在「已识别」旁显示引擎来源标签", () => {
    const panel = read("src/components/voucher/AttachmentPanel.vue");
    // 引擎标签紧挨 OCR 状态标签，且只在识别完成且有引擎时出现
    expect(panel).toContain("a.ocrStatus === 'done' && a.ocrEngine");
    expect(panel).toContain("OCR_ENGINE_SHORT[a.ocrEngine]");
    expect(panel).toContain("ocrEngineLabel(a.ocrEngine)");
    // 标签表收敛在一处（附件面板与结果卡共用）
    const format = read("src/composables/format.ts");
    expect(format).toContain("OCR_ENGINE_LABELS");
    expect(format).toContain("OCR_ENGINE_SHORT");
    expect(format).toContain("export function ocrEngineLabel");
    // 后端详情接口要把引擎带到附件上
    expect(read("src/api/client.ts")).toContain("ocrEngine?: string | null;");
  });

  it("识别结果卡显示本次识别引擎与列表页提示", () => {
    const card = read("src/components/OcrResultCard.vue");
    // 引擎标签：中文名收敛在 format.ts，卡片只负责渲染
    const format = read("src/composables/format.ts");
    expect(format).toContain("云端 DeepSeek 视觉");
    expect(format).toContain("本地 Apple Vision");
    expect(card).toContain("ocrEngineLabel(props.extracted?.engine)");
    expect(card).toContain("engineLabel");
    // 列表页提示：金额与对方留空要说清原因
    expect(card).toContain("extracted.statementList");
    expect(card).toContain("疑似流水/明细列表页");
    const client = read("src/api/client.ts");
    expect(client).toContain("statementList?: boolean");
    expect(client).toContain("engine?: string | null");
  });

  it("默认配置为云端优先（engine=auto + provider=deepseek）", () => {
    const defaults = read("src/config-defaults.ts");
    expect(defaults).toContain('engine: "auto"');
    expect(defaults).toContain('provider: "deepseek"');
  });
});
