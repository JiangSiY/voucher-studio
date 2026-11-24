// @vitest-environment jsdom
/**
 * 只读同步状态（`wps_sync_status`）的前端契约（S0-07/S0-08 数据源收口）。
 *
 * 能证什么：最后同步时间取只读命令的 `lastSyncAt` 并归一为 `…Z` 秒级；无记录显示
 * 「—（尚无同步记录）」；徽标只按 `pendingCount` 显示（0 不显示）；结果不明（`unknownCount`）
 * 单独成句；队列只渲染 表/幂等键/错误码/状态 四列；挂载只查一次、零 fetch、零同步；
 * 命令失败时降级为「—（状态不可用）」且不影响页面其余部分。
 * 不能证什么：真实 WPS 同步/扫描（本轮 transport=mock，未发任何真实请求）、后端 SQL 口径
 * （由 `crates/voucher-api/tests/wps_status.rs` 覆盖）。
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount, type VueWrapper } from "@vue/test-utils";
import { createPinia, setActivePinia } from "pinia";
import type { DevW2Result, WpsSyncStatus } from "../api/client";
import ReauthBanner from "../components/wps/ReauthBanner.vue";
import DataPage from "../pages/DataPage.vue";

const mocks = vi.hoisted(() => ({
  listVouchers: vi.fn(),
  getVoucher: vi.fn(),
  listImportBatches: vi.fn(),
  setLedgerBooked: vi.fn(),
  devW2MockSync: vi.fn(),
  wpsSyncStatus: vi.fn(),
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

function syncStatus(over: Partial<WpsSyncStatus> = {}): WpsSyncStatus {
  return {
    lastSyncAt: null,
    lastScanComplete: null,
    pendingCount: 0,
    unknownCount: 0,
    pendingItems: [],
    ...over,
  };
}

/** 400006：授权失效、本批停止（用于「状态命令失败但仍要提示重授权」的组合场景）。 */
function authExpired(): DevW2Result {
  return {
    mode: "write:auth_expired",
    dataDir: "/tmp/voucher-w2-sync-status-test",
    mockRequests: { total: 0, list: 0, create: 0, createKeys: [] },
    batch: {
      runId: null,
      stopped: true,
      authRequired: true,
      cooldownUntil: null,
      outcomes: [
        {
          sequence: 1,
          idempotencyKey: "2026-11-24::凭证::记-001",
          status: "unknown",
          remoteRecordId: null,
          linkId: null,
          linkState: null,
          savedLink: false,
          posted: false,
          error: "WPS_E_AUTH: 授权失效（400006），本批已停止",
        },
      ],
    },
  };
}

type BannerProps = Omit<InstanceType<typeof ReauthBanner>["$props"], "authRequired">;

function mountBanner(props: BannerProps): VueWrapper {
  wrapper = mount(ReauthBanner, { props: { authRequired: true, ...props } });
  return wrapper;
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

beforeEach(() => {
  vi.clearAllMocks();
  vi.stubGlobal("fetch", fetchSpy);
  sessionStorage.clear();
  mocks.listVouchers.mockResolvedValue({ items: [], total: 0, thumbs: {}, meta: {} });
  mocks.getVoucher.mockResolvedValue(null);
  mocks.listImportBatches.mockResolvedValue([]);
  mocks.devW2MockSync.mockResolvedValue({
    mode: "pull",
    dataDir: "/tmp/voucher-w2-sync-status-test",
    mockRequests: { total: 0, list: 0, create: 0, createKeys: [] },
  });
  mocks.wpsSyncStatus.mockResolvedValue(syncStatus());
});

afterEach(() => {
  wrapper?.unmount();
  wrapper = null;
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
  sessionStorage.clear();
});

describe("最后同步时间取只读状态（四态）", () => {
  it("有 lastSyncAt：+00:00 带纳秒也归一为 Z 秒级", () => {
    mountBanner({
      lastSyncAt: "2026-11-24T09:30:05.123456789+00:00",
      pendingCount: 0,
      unknownCount: 0,
      pendingItems: [],
    });
    expect(wrapper!.text()).toContain("最后同步时间：2026-11-24T09:30:05Z");
    expect(wrapper!.text()).not.toContain("+00:00");
    expect(wrapper!.text()).not.toContain("123456789");
  });

  it("无记录（null）：显示「—（尚无同步记录）」", () => {
    mountBanner({ lastSyncAt: null, pendingCount: 0, unknownCount: 0, pendingItems: [] });
    expect(wrapper!.find('[data-testid="last-sync-line"]').text()).toBe(
      "最后同步时间：—（尚无同步记录）",
    );
  });

  it("待处理 0 条：队列不渲染、徽标不出现", async () => {
    await mountPage();
    expect(wrapper!.find('[data-testid="w2-pending-badge"]').exists()).toBe(false);
    expect(wrapper!.find('[data-testid="reauth-queue"]').exists()).toBe(false);
  });

  it("待处理 3 条：徽标显示 3（只按 pendingCount，不合并结果不明）", async () => {
    mocks.wpsSyncStatus.mockResolvedValue(
      syncStatus({
        lastSyncAt: "2026-11-24T09:00:00Z",
        pendingCount: 3,
        unknownCount: 2,
        pendingItems: [
          {
            sheetId: 5,
            idempotencyKey: "2026-11-24::凭证::记-001",
            code: "WPS_E_AUTH",
            state: "unknown",
          },
          {
            sheetId: 7,
            idempotencyKey: "2026-11-24::凭证::记-002",
            code: "WPS_E_LINK_BACKFILL",
            state: "pending_backfill",
          },
        ],
      }),
    );
    await mountPage();
    expect(wrapper!.find('[data-testid="w2-pending-badge"]').text()).toBe("3");
  });

  it("队列只渲染四列：表 / 幂等键 / 错误码 / 状态", () => {
    mountBanner({
      lastSyncAt: null,
      pendingCount: 2,
      unknownCount: 1,
      pendingItems: [
        {
          sheetId: 5,
          idempotencyKey: "2026-11-24::凭证::记-001",
          code: "WPS_E_AUTH",
          state: "unknown",
        },
        {
          sheetId: 7,
          idempotencyKey: "2026-11-24::凭证::记-002",
          code: "WPS_E_LINK_BACKFILL",
          state: "pending_backfill",
        },
      ],
    });
    const heads = wrapper!.findAll('[data-testid="reauth-queue"] thead th').map((t) => t.text());
    expect(heads).toEqual(["表", "幂等键", "错误码", "状态"]);
    const rows = wrapper!.findAll('[data-testid="reauth-queue"] tbody tr');
    expect(rows).toHaveLength(2);
    expect(rows[0].text()).toContain("WPS_E_AUTH");
    expect(rows[1].text()).toContain("7");
    expect(rows[1].text()).toContain("WPS_E_LINK_BACKFILL");
    expect(rows[1].text()).toContain("pending_backfill");
  });
});

describe("零网络与降级", () => {
  it("挂载只查一次状态：1 次 wpsSyncStatus、0 次 fetch、0 次同步调用", async () => {
    mocks.wpsSyncStatus.mockResolvedValue(syncStatus({ pendingCount: 3 }));
    await mountPage();
    expect(mocks.wpsSyncStatus).toHaveBeenCalledTimes(1);
    expect(mocks.wpsSyncStatus).toHaveBeenCalledWith();
    expect(mocks.devW2MockSync).not.toHaveBeenCalled();
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("命令失败：降级为「—（状态不可用）」、徽标隐藏，页面其余照常", async () => {
    mocks.wpsSyncStatus.mockRejectedValue(new Error("命令不可用"));
    mocks.devW2MockSync.mockResolvedValue(authExpired());
    await mountPage();

    expect(wrapper!.find('[data-testid="w2-pending-badge"]').exists()).toBe(false);
    // 页面其余部分仍在（列表空态文案）
    expect(wrapper!.text()).toContain("当前视图没有条目。");

    // 失效提示仍要给，但条数改成「不可用」，不编造 0 项
    await wrapper!
      .findAll("button")
      .find((b) => b.text().includes("模拟创建（授权失效 400006）"))!
      .trigger("click");
    await flushPromises();
    const banner = wrapper!.find('[data-testid="reauth-banner"]');
    expect(banner.exists()).toBe(true);
    expect(banner.text()).toContain("需重新授权");
    expect(banner.text()).toContain("待处理项数不可用（状态查询失败）");
    expect(banner.text()).toContain("最后同步时间：—（状态不可用）");
    expect(banner.text()).not.toContain("待处理 0 项");
    expect(mocks.wpsSyncStatus).toHaveBeenCalledTimes(1);
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it("结果不明单独成句，不并入徽标口径", () => {
    mountBanner({
      lastSyncAt: null,
      pendingCount: 1,
      unknownCount: 2,
      pendingItems: [
        { sheetId: 5, idempotencyKey: "k-1", code: "WPS_E_UNKNOWN", state: "unknown" },
      ],
    });
    expect(wrapper!.find('[data-testid="unknown-note"]').text()).toContain("有 2 项结果不明");
    expect(wrapper!.find('[data-testid="unknown-note"]').text()).toContain("不会自动重发");
    expect(wrapper!.text()).toContain("待处理 1 项已保留");
  });
});
