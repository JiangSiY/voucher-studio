// @vitest-environment jsdom
/**
 * S0-07 / S0-08 前端契约（重授权 UI + 查看范围二次确认 + 冷却时间一致）。
 *
 * 能证什么：400006 后出现「需重新授权」提示条、待处理队列与红点；首次失效先弹
 * 「只能看本机缓存」的二次确认，确认态存 sessionStorage 且同一会话不重复弹；
 * 「重新授权」只调用既有本地合成 `auth_recover`（零网络）；冷却截止在提示条与行内
 * 错误列显示为同一个 `…Z` 秒级字符串。
 * 不能证什么：真实 WPS 400006/429（本轮 transport=mock，未发任何真实请求）、
 * 桌面宿主的真实写库与视觉细节。
 */
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import type { DevW2Outcome, DevW2Result } from "../api/client";
import DataPage from "../pages/DataPage.vue";

const mocks = vi.hoisted(() => ({
  listVouchers: vi.fn(),
  getVoucher: vi.fn(),
  listImportBatches: vi.fn(),
  setLedgerBooked: vi.fn(),
  devW2MockSync: vi.fn(),
}));

vi.mock("../api/client", () => ({ api: mocks, isTauri: true }));
vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (p: string) => `asset://localhost/${p}`,
}));
vi.mock("vue-router", () => ({
  useRoute: () => ({ query: {} }),
  useRouter: () => ({ push: vi.fn(), replace: vi.fn() }),
}));

const fetchSpy = vi.fn();
let wrapper: VueWrapper | null = null;

function outcome(over: Partial<DevW2Outcome> = {}): DevW2Outcome {
  return {
    sequence: 1,
    idempotencyKey: "2026-11-24::凭证::记-001",
    status: "succeeded",
    remoteRecordId: null,
    linkId: null,
    linkState: null,
    savedLink: false,
    posted: false,
    error: null,
    ...over,
  };
}

function result(over: Partial<DevW2Result> = {}): DevW2Result {
  return {
    mode: "write:auth_expired",
    dataDir: "/tmp/voucher-w2-scope-test",
    mockRequests: { total: 0, list: 0, create: 0, createKeys: [] },
    ...over,
  };
}

/** 400006：本批停止、待处理项保留（unknown + 已拒绝各一） */
function authExpired(): DevW2Result {
  return result({
    mode: "write:auth_expired",
    batch: {
      runId: null,
      stopped: true,
      authRequired: true,
      cooldownUntil: null,
      outcomes: [
        outcome({
          sequence: 1,
          status: "unknown",
          error: "WPS_E_AUTH: 授权失效（400006），本批已停止",
        }),
        outcome({
          sequence: 2,
          idempotencyKey: "2026-11-24::凭证::记-002",
          status: "rejected",
          error: "WPS_E_AUTH: 授权失效（400006）",
        }),
      ],
    },
  });
}

/** 429：冷却截止带纳秒 +00:00（preflight 写法） */
function rateLimited(): DevW2Result {
  return result({
    mode: "write:rate_limited",
    batch: {
      runId: null,
      stopped: true,
      authRequired: false,
      cooldownUntil: "2099-01-01T00:30:00.123456789+00:00",
      outcomes: [
        outcome({
          sequence: 1,
          status: "unknown",
          error: "WPS_E_RATE: 限流，冷却至 2099-01-01T00:30:00.987654321+00:00，已停止写入",
        }),
      ],
    },
  });
}

function recovered(): DevW2Result {
  return result({ mode: "auth_recover" });
}

async function mountPage(): Promise<VueWrapper> {
  const pinia = createPinia();
  setActivePinia(pinia);
  wrapper = mount(DataPage, {
    global: { plugins: [pinia], stubs: { VoucherEditor: true } },
  });
  await flushPromises();
  return wrapper;
}

async function clickText(text: string) {
  const btn = wrapper!.findAll("button").find((b) => b.text().includes(text));
  expect(btn, `找不到按钮：${text}`).toBeTruthy();
  await btn!.trigger("click");
  await flushPromises();
}

async function clickDialog(text: string) {
  const btn = Array.from(document.querySelectorAll<HTMLButtonElement>(".dialog-ops button")).find(
    (b) => (b.textContent ?? "").includes(text),
  );
  expect(btn, `找不到弹窗按钮：${text}`).toBeTruthy();
  btn!.click();
  await flushPromises();
}

function dialogText(): string {
  return document.querySelector(".dialog")?.textContent ?? "";
}

beforeEach(() => {
  vi.clearAllMocks();
  vi.stubGlobal("fetch", fetchSpy);
  sessionStorage.clear();
  mocks.listVouchers.mockResolvedValue({
    items: [],
    total: 0,
    thumbs: {},
    meta: {},
  });
  mocks.getVoucher.mockResolvedValue(null);
  mocks.listImportBatches.mockResolvedValue([]);
  mocks.devW2MockSync.mockResolvedValue(result());
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
  sessionStorage.clear();
});

describe("重授权 UI 与查看范围二次确认（S0-07）", () => {
  it("400006 后出现提示条 / 待处理队列 / 红点，并先弹只读范围确认", async () => {
    mocks.devW2MockSync.mockResolvedValue(authExpired());
    await mountPage();
    await clickText("模拟创建（授权失效 400006）");

    const banner = wrapper!.find('[data-testid="reauth-banner"]');
    expect(banner.exists()).toBe(true);
    const text = banner.text();
    expect(text).toContain("需重新授权");
    expect(text).toContain("待处理 1 项");
    expect(text).toContain("最后同步时间：—（数据源待补充）");
    expect(text).toContain("未实现（待 A2/真实凭据）");

    const queue = wrapper!.findAll('[data-testid="reauth-queue"] li');
    expect(queue).toHaveLength(1);
    expect(queue[0].text()).toContain("2026-11-24::凭证::记-001");
    expect(queue[0].text()).toContain("WPS_E_AUTH");

    // 行内错误列也在（不只横幅），供逐条核对
    expect(wrapper!.find(".dev-w2-err").text()).toContain("400006");
    expect(wrapper!.find('[data-testid="w2-pending-badge"]').text()).toBe("1");

    const dialog = document.querySelector(".dialog");
    expect(dialog).toBeTruthy();
    expect(dialog!.getAttribute("aria-modal")).toBe("true");
    expect(dialog!.getAttribute("tabindex")).toBe("-1");
    for (const kw of ["可见", "本机", "不可见", "最后同步时间", "不发起任何网络请求"]) {
      expect(dialogText()).toContain(kw);
    }

    // 弹确认本身不发任何请求：只发生了用户点击的那一次本地合成调用
    expect(mocks.devW2MockSync).toHaveBeenCalledTimes(1);
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("确认查看范围后写入 sessionStorage，同一会话再失效不重复弹", async () => {
    mocks.devW2MockSync.mockResolvedValue(authExpired());
    await mountPage();
    await clickText("模拟创建（授权失效 400006）");
    expect(document.querySelector(".dialog")).toBeTruthy();

    await clickDialog("确认，仅查看本机缓存");
    expect(document.querySelector(".dialog")).toBeNull();
    expect(sessionStorage.getItem("voucher.w2.scopeConfirmed")).toBe("1");
    expect(mocks.devW2MockSync).toHaveBeenCalledTimes(1);

    await clickText("模拟创建（授权失效 400006）");
    expect(document.querySelector(".dialog")).toBeNull();
  });

  it("取消不记确认态：下一次失效仍会先弹范围确认", async () => {
    mocks.devW2MockSync.mockResolvedValue(authExpired());
    await mountPage();
    await clickText("模拟创建（授权失效 400006）");

    await clickDialog("返回");
    expect(document.querySelector(".dialog")).toBeNull();
    expect(sessionStorage.getItem("voucher.w2.scopeConfirmed")).toBeNull();

    await clickText("模拟创建（授权失效 400006）");
    expect(document.querySelector(".dialog")).toBeTruthy();
  });

  it("「重新授权」二次确认后只调既有本地合成 auth_recover，待处理项保留", async () => {
    mocks.devW2MockSync.mockResolvedValueOnce(authExpired());
    await mountPage();
    await clickText("模拟创建（授权失效 400006）");
    await clickDialog("确认，仅查看本机缓存");

    mocks.devW2MockSync.mockClear();
    mocks.devW2MockSync.mockResolvedValueOnce(recovered());
    await clickText("重新授权");

    expect(dialogText()).toContain("不发起任何网络请求");
    expect(dialogText()).toContain("真实 OAuth 重授权未实现");

    await clickDialog("执行本地合成恢复");
    expect(mocks.devW2MockSync).toHaveBeenCalledTimes(1);
    expect(mocks.devW2MockSync).toHaveBeenCalledWith("auth_recover", undefined);
    expect(fetchSpy).not.toHaveBeenCalled();

    // 恢复后提示条撤下，但待处理项仍保留（红点不丢）
    expect(wrapper!.find('[data-testid="reauth-banner"]').exists()).toBe(false);
    expect(wrapper!.find('[data-testid="w2-pending-badge"]').text()).toBe("1");
  });
});

describe("冷却截止三处一致（S0-08）", () => {
  it("提示条与行内错误列显示同一个 Z 秒级时间，不残留 +00:00 或纳秒", async () => {
    mocks.devW2MockSync.mockResolvedValue(rateLimited());
    await mountPage();
    await clickText("模拟创建（限流 429）");

    const banner = wrapper!.find('[data-testid="cooldown-banner"]');
    expect(banner.exists()).toBe(true);
    const shown = banner.find("code").text();
    expect(shown).toBe("2099-01-01T00:30:00Z");

    const rowError = wrapper!.find(".dev-w2-err").text();
    expect(rowError).toContain(shown);
    expect(rowError).not.toContain("+00:00");
    expect(rowError).not.toContain("987654321");
    expect(banner.text()).not.toContain("+00:00");

    // 只有冷却、没有授权失效：不弹查看范围确认
    expect(document.querySelector(".dialog")).toBeNull();
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("冷却结束后新批次不再带 cooldown 时，旧提示条撤下", async () => {
    mocks.devW2MockSync.mockResolvedValue(rateLimited());
    await mountPage();
    await clickText("模拟创建（限流 429）");
    expect(wrapper!.find('[data-testid="cooldown-banner"]').exists()).toBe(true);

    mocks.devW2MockSync.mockResolvedValue(
      result({
        mode: "pull",
        batch: {
          runId: 2,
          stopped: false,
          authRequired: false,
          cooldownUntil: null,
          outcomes: [outcome({ sequence: 1, status: "succeeded" })],
        },
      }),
    );
    await clickText("拉取 11 表（只读）");
    expect(wrapper!.find('[data-testid="cooldown-banner"]').exists()).toBe(false);
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("前端不自行推算 30 分钟冷却（单源在 Rust RATE_LIMIT_COOLDOWN_SECS）", () => {
    const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
    for (const rel of [
      "pages/DataPage.vue",
      "components/wps/ReauthBanner.vue",
      "components/wps/ViewScopeDialog.vue",
    ]) {
      const src = readFileSync(resolve(root, rel), "utf8");
      for (const forbidden of ["30 * 60", "1800000", "+30*60*1000"]) {
        expect(src, `${rel} 不得出现 ${forbidden}`).not.toContain(forbidden);
      }
    }
    const scopeSrc = readFileSync(resolve(root, "components/wps/ViewScopeDialog.vue"), "utf8");
    expect(scopeSrc).toContain('from "../ConfirmDialog.vue"');
    expect(scopeSrc).not.toMatch(/\bfetch\(/);
  });
});
