// @vitest-environment node
/**
 * 洞察排行的**时间窗口径**守卫（2026-10-16 用户点名「洞察排行面板不跟随时间窗」）。
 *
 * 老毛病：`top_counterparties` / `tag_stats` 只支持「近 N 个月」，前端只好传 0 = 全历史，
 * 于是不管时间窗怎么切，Top 10 排行永远不动。修法是**把窗口的日期区间一路传到底**：
 * `InsightsPage`（窗口真源）→ `TopListsPanel`（props + watch 重查）→ `api/client`（透传）
 * → Tauri 命令 → `services/reports.rs`（`RangeFilter`：区间优先于 months）。
 * 这条链路任何一环掉了，界面看起来都"正常"，所以逐环锁死。
 */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { flat } from "./helpers/designGuard";

const read = (rel: string) => readFileSync(new URL(`../../${rel}`, import.meta.url), "utf8");

const page = read("src/pages/InsightsPage.vue");
const panel = read("src/components/dashboard/TopListsPanel.vue");
const client = read("src/api/client.ts");
const service = read("crates/voucher-core/src/services/reports.rs");
const command = read("src-tauri/src/commands/vouchers.rs");

describe("洞察排行跟随时间窗", () => {
  it("页面把窗口区间传给排行面板", () => {
    expect(page).toMatch(
      /<TopListsPanel :date-from="range\.dateFrom" :date-to="range\.dateTo" \/>/,
    );
  });

  it("面板接收区间、透传给两个接口，并在窗口变化时重查", () => {
    expect(panel).toMatch(
      /defineProps<\{ dateFrom\?: string \| null; dateTo\?: string \| null \}>/,
    );
    expect(panel).toMatch(/api\.topCounterparties\(0, CP_TOP, props\.dateFrom, props\.dateTo\)/);
    expect(panel).toMatch(/api\.tagStats\(0, TAG_TOP, props\.dateFrom, props\.dateTo\)/);
    // 只在 onMounted 拉一次就是老 bug，必须有 watch
    expect(flat(panel)).toMatch(/watch\(\s*\(\) => \[props\.dateFrom, props\.dateTo\]/);
    // 口径标注（规范 §3.1 第 5 条）：窗口内 / 全历史
    expect(panel).toContain("窗口内");
    expect(panel).toContain("全历史");
    expect(panel).toMatch(/按金额 · \{\{ scopeLabel \}\}/);
  });

  it("接口层透传 dateFrom/dateTo", () => {
    expect(flat(client)).toMatch(
      /topCounterparties: \(\s*months\?: number, limit\?: number, dateFrom\?: string \| null, dateTo\?: string \| null/,
    );
    expect(client).toMatch(
      /tagStats: \(months\?: number, limit\?: number, dateFrom\?: string \| null, dateTo\?: string \| null\)/,
    );
    expect(client).toMatch(
      /invoke<CounterpartyRow\[\]>\("top_counterparties", \{ months, limit, dateFrom, dateTo \}\)/,
    );
  });

  it("命令层把区间交给 core", () => {
    expect(command).toMatch(
      /pub fn top_counterparties\([\s\S]{0,400}?date_from: Option<String>,[\s\S]{0,200}?date_to: Option<String>/,
    );
    expect(command).toMatch(
      /pub fn tag_stats\([\s\S]{0,400}?date_from: Option<String>,[\s\S]{0,200}?date_to: Option<String>/,
    );
    expect(command).toMatch(/date_from\.as_deref\(\)/);
  });

  it("core 侧真的按日期过滤，且区间优先于 months", () => {
    // 区间条件与月份前缀都在同一个 helper 里，避免两处口径漂移
    expect(service).toMatch(/struct RangeFilter \{/);
    expect(service).toMatch(/\{date_col\} >= \?/);
    expect(service).toMatch(/\{date_col\} <= \?/);
    expect(service).toMatch(/substr\(\{date_col\}, 1, 7\) >= \?/);
    // months == 0 || has_range → 不再按月份过滤（区间优先）
    expect(service).toMatch(/if months == 0 \|\| has_range \{/);
    // 两个函数都走它
    expect(service.match(/RangeFilter::new\(months, date_from, date_to\)\?/g) ?? []).toHaveLength(
      2,
    );
  });
});
