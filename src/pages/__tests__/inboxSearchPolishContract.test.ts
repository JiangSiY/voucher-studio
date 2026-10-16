// @vitest-environment node
/**
 * 本轮（2026-10-16 界面细节审查）三条页面级小项的守卫。
 *
 * 1. 收件箱待办行的次要动作：原先写在主 `<button>` 里（`<span @click>`），
 *    Tab 落不到、读屏只当它是主按钮的文字；主按钮 `disabled`（如「识别中…」）时
 *    浏览器还会吞掉内部点击 → 唯一的下钻入口在识别期间完全点不动。
 * 2. 全文搜索固定取 100 条且界面不提示 → 用户把截断结果当成"库里没有"，重复录入。
 * 3. 恢复确认框承诺「就绪并重启」，实际只写入待恢复文件（重启是 staged 卡片上另一颗按钮）。
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const inbox = readFileSync(new URL("../InboxPage.vue", import.meta.url), "utf8");
const search = readFileSync(new URL("../SearchPage.vue", import.meta.url), "utf8");
const restore = readFileSync(new URL("../RestorePage.vue", import.meta.url), "utf8");

describe("收件箱待办行：次要动作是独立可聚焦按钮", () => {
  it("次要动作是 <button class=\"todo-secondary\">，不再是主按钮里的 span", () => {
    expect(inbox).toMatch(/<button v-if="i\.secondary" class="todo-secondary"[^>]*@click="i\.secondary\.run\(\)"/);
    expect(inbox).not.toMatch(/<span v-if="i\.secondary" class="todo-secondary"/);
  });

  it("主按钮里不再嵌可点击元素（行 = 行容器 + 两个并排按钮）", () => {
    const main = inbox.slice(inbox.indexOf('<button class="todo"'), inbox.indexOf("</button>", inbox.indexOf('<button class="todo"')));
    expect(main).not.toContain("@click.stop");
    expect(main).not.toContain("todo-secondary");
    expect(inbox).toContain('<li v-for="i in todoItems" :key="i.key" class="todo-row">');
  });
});

describe("全文搜索：截断必须明示", () => {
  it("多取 1 条只用于判断「还有更多」，界面只展示上限条数", () => {
    expect(search).toMatch(/const SEARCH_LIMIT = 100;/);
    expect(search).toContain("searchVouchers(kw, filters, SEARCH_LIMIT + 1)");
    expect(search).toMatch(/truncated\.value = rows\.length > SEARCH_LIMIT;/);
    expect(search).toContain("rows.slice(0, SEARCH_LIMIT)");
  });

  it("截断时标题带 +、并给出可执行的下一步提示", () => {
    expect(search).toContain("{{ hits.length }}{{ truncated ? \"+\" : \"\" }}");
    expect(search).toContain("只显示前 {{ SEARCH_LIMIT }} 条");
  });
});

describe("恢复确认框：文案与行为一致", () => {
  it("确认按钮说明只写入待恢复文件，不再承诺「就绪并重启」", () => {
    expect(restore).toContain('confirm-text="写入待恢复文件"');
    expect(restore).not.toContain('confirm-text="就绪并重启"');
    expect(restore).toContain("重启前删掉待恢复文件即可取消");
  });
});
