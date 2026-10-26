//! 命令层只读查询门面（体检 #11）
//!
//! # 背景
//!
//! `src-tauri` 命令层是 GUI → 服务层的边界，不应直连 `db::repos` 仓储类型或裸
//! `with_conn`/`with_tx`（越层直连使 SQL 口径、事务边界与错误包装散落 GUI 壳）。
//! 本模块为命令层提供统一的只读查询入口：取数收敛在 core，命令层只做参数整理
//! 与 JSON 透传。
//!
//! # 约定
//!
//! - **只读**：此处不开启写事务；写路径继续走对应领域服务的 `with_tx`（UoW）。
//! - **DTO 稳定**：re-export 的仓储类型 / 命令 DTO 的 `serde` 形状即前端契约，
//!   改动需前后端同步评审（本批仅改变类型所在路径，不改任何 JSON 字段）。
//! - **命名**：与历史命令动词一致（thumbs / duplicates / ocr_preview / next_no…），
//!   便于对照检索。
//! - 命令层禁止出现 `db::repos` 路径与 `.with_conn(`，由 CI grep 门禁固化。

use std::collections::HashMap;

use serde::Serialize;

use crate::db::repos::{attachment_repo, ocr_layout_repo, voucher_repo};
use crate::models::enums::VoucherStatus;
use crate::models::{Attachment, Voucher};
use crate::services::{dashboard, dup_review, naming, ocr_fields, ocr_layout};
use crate::{CoreContext, Result};

// ---- 稳定 DTO re-export：命令层不再引用 db::repos 路径，契约形状不变 ----

/// 凭证列表查询条件（原 `db::repos::voucher_repo::VoucherQuery`）
pub use crate::db::repos::voucher_repo::{
    SearchFilters, VoucherNeighbor, VoucherQuery, VoucherStats,
};

/// 会计期间锁（原 `db::repos::period_repo::PeriodLock`）
pub use crate::db::repos::period_repo::PeriodLock;

/// 增强版命中来源（原 `db::repos::enhance_hit_repo::HitSource`）
pub use crate::db::repos::enhance_hit_repo::HitSource;

/// v0.16 处置：核查队列 / 证据 / 置信度 / 统计（命令层不再引用 services::dup_review 路径）
pub use crate::services::dup_review::{EvidenceLevel, PairEvidence, PendingSummary, ReviewGroup};

/// 处置记录（审计日志；`removedPayload` 为被删凭证留痕快照）
pub use crate::models::dup_decision::DupDecision;

/// v0.18 工作台（仪表盘）只读聚合：窗口概览 / 待办与健康度 / 轻量重复计数
pub use crate::services::dashboard::{
    BreakdownRow, DataHealth, ExportCoverage, FormatCount, Overview as DashboardOverview,
    StorageUsage, Todos as DashboardTodos,
};
pub use crate::services::dup_review::PendingCount as DupPendingCount;

// ---- 命令 DTO（原定义在命令层，现收敛到 core，JSON 契约不变） ----

/// D-1 列表缩略图信息：相对路径 + 下发给界面用的**绝对路径**（前端 convertFileSrc 直接用绝对路径）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherThumb {
    pub file_path: String,
    pub enhanced_path: Option<String>,
    pub enhanced_source: String,
    /// 原图绝对路径（非法 rel 为 None）；不判存在性，缺失由前端回退阶梯兜
    pub file_abs: Option<String>,
    /// 增强版绝对路径（不可显示时为 None）
    pub enhanced_abs: Option<String>,
}

/// v0.16 保存后提示项：疑似重复的**对端凭证 + 证据**
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateSuspect {
    pub voucher: Voucher,
    pub evidence: PairEvidence,
}

/// 当前页所有凭证的首附件缩略图（key = voucher.id）。
/// 每张凭证只取第一张附件；没有附件的凭证不出现在结果里（前端按 missing 显示占位）。
/// 空 id 列表跳过 SQL，避免无谓查询。
pub fn voucher_thumbs(ctx: &CoreContext, ids: &[i64]) -> Result<HashMap<i64, VoucherThumb>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = ctx
        .db
        .with_conn(|conn| attachment_repo::first_thumb_for_vouchers(conn, ids))?;
    Ok(rows
        .into_iter()
        .map(|(vid, file_path, enhanced_path, enhanced_source)| {
            let file_abs = ctx.paths.abs_string_checked(&file_path);
            // 增强版是"可重建缓存"、不入备份：恢复/清理后可能已不在磁盘上。
            // 与 PDF 导出、OCR 同口径——不可显示时下发 None，前端回退原图。
            let enhanced_path =
                super::attachment_view::visible_enhanced(ctx, enhanced_path.as_deref());
            let enhanced_abs = enhanced_path
                .as_deref()
                .and_then(|rel| ctx.paths.abs_string_checked(rel));
            (
                vid,
                VoucherThumb {
                    file_path,
                    enhanced_path,
                    enhanced_source,
                    file_abs,
                    enhanced_abs,
                },
            )
        })
        .collect())
}

/// 单凭证疑似重复（保存后前端提示）：同日期同金额的**其他凭证 + 证据**。
///
/// v0.16 起**已判定（不是重复 / 已确认并删除）的对不再返回**——旧版每次都重算全量，
/// 导致"每次保存都重弹同一个提示"且无法消除。
pub fn suspected_duplicates(ctx: &CoreContext, id: i64) -> Result<Vec<DuplicateSuspect>> {
    let rows = dup_review::undecided_suspects(ctx, id)?;
    Ok(rows
        .into_iter()
        .map(|(voucher, evidence)| DuplicateSuspect { voucher, evidence })
        .collect())
}

/// v0.16：待核查队列（只含还有未判定对的组；组内对按置信度排序）
pub fn duplicate_review_queue(ctx: &CoreContext) -> Result<Vec<ReviewGroup>> {
    dup_review::review_queue(ctx)
}

/// v0.16：待处理统计（侧栏徽标 / 仪表盘面板）
pub fn duplicate_pending_summary(ctx: &CoreContext) -> Result<PendingSummary> {
    dup_review::pending_summary(ctx)
}

/// v0.16：处置历史（新 → 旧）
pub fn duplicate_decisions(ctx: &CoreContext, limit: u32) -> Result<Vec<DupDecision>> {
    dup_review::history(ctx, limit)
}

// ---------- v0.18 工作台（仪表盘）只读聚合 ----------

/// 工作台概览：**统一窗口口径**（`date_from`/`date_to` 为 `YYYY-MM-DD`，含端点）+ 全库固定项。
///
/// 两端都给时附上一等长窗口的金额/张数（供前端算环比；不足两个窗口时不编造）。
pub fn dashboard_overview(
    ctx: &CoreContext,
    date_from: Option<String>,
    date_to: Option<String>,
    status: Option<VoucherStatus>,
) -> Result<DashboardOverview> {
    dashboard::overview(ctx, date_from, date_to, status)
}

/// 待办与健康度：把已落库但此前没有出口的数据变成可行动的计数
/// （待识别 / 未填金额 / 陈旧草稿 / 未封账往月 / 备份新鲜度 / 疑似重复）。
pub fn dashboard_todos(ctx: &CoreContext) -> Result<DashboardTodos> {
    dashboard::todos(ctx)
}

/// 数据健康（洞察页「数据健康」卡）：全库口径的无图 / 待识别 / 未被佐证三项计数
pub fn data_health(ctx: &CoreContext) -> Result<DataHealth> {
    dashboard::data_health(ctx)
}

/// 疑似重复**轻量**计数（徽标/待办卡用）：不构造队列、不查证据；
/// 口径与核查队列一致（有用例锁定）。
pub fn duplicate_pending_count(ctx: &CoreContext) -> Result<DupPendingCount> {
    dup_review::pending_count(ctx)
}

/// v0.18 批次 5：月度透视（月份 × 对方 / 月份 × 标签，每月前 N 个键）
pub fn monthly_breakdown(
    ctx: &CoreContext,
    date_from: Option<String>,
    date_to: Option<String>,
    dim: String,
    per_month: Option<u32>,
) -> Result<Vec<BreakdownRow>> {
    dashboard::monthly_breakdown(ctx, date_from, date_to, &dim, per_month.unwrap_or(6))
}

/// v0.18 批次 5：导出覆盖度（窗口内"已导出 / 从未导出"台账 + 格式分布）
pub fn export_coverage(
    ctx: &CoreContext,
    date_from: Option<String>,
    date_to: Option<String>,
) -> Result<ExportCoverage> {
    dashboard::export_coverage(ctx, date_from, date_to)
}

/// v0.18 批次 5：存储占用（原图体积走 SQL；增强版体积需显式开启逐文件 stat）
pub fn storage_usage(ctx: &CoreContext, with_enhanced_bytes: bool) -> Result<StorageUsage> {
    dashboard::storage_usage(ctx, with_enhanced_bytes)
}

/// 按当前筛选**只取 id**（列表页跨页全选）；顺序与列表一致。
pub fn voucher_ids(ctx: &CoreContext, query: &VoucherQuery) -> Result<Vec<i64>> {
    ctx.db.with_conn(|conn| voucher_repo::list_ids(conn, query))
}

/// 附件实体查询（附件不存在返回 `Ok(None)`，由调用方决定文案）。
pub fn attachment(ctx: &CoreContext, id: i64) -> Result<Option<Attachment>> {
    ctx.db.with_conn(|conn| attachment_repo::get(conn, id))
}

/// OCR 字段提取预览（只读）：从附件已识别文本抽取结构化字段，不写库。
/// 优先级与填单一致：云端结构化要素 > 本地版面几何配对 > 纯文本抽取；附件不存在返回 NotFound。
pub fn ocr_preview(ctx: &CoreContext, attachment_id: i64) -> Result<ocr_fields::ExtractedFields> {
    ctx.db.with_conn(|conn| {
        let att = attachment_repo::get(conn, attachment_id)?
            .ok_or_else(|| crate::CoreError::NotFound(format!("附件 {attachment_id} 不存在")))?;
        let text = att.ocr_text.as_deref().unwrap_or("").to_string();

        // 引擎来源：结果卡要显示「这次是谁认出来的」，故三层里谁命中就留谁的名字
        let mut engine: Option<String> = None;

        // 1. 云端要素（重新识别不再付费；要素比文本正则准）
        let cloud = crate::db::repos::ocr_field_cache_repo::get(conn, &att.file_hash)?;
        let structured = cloud.and_then(|(cache_engine, _model, json)| {
            if crate::services::ocr::is_cloud_engine(&cache_engine) {
                serde_json::from_str::<serde_json::Value>(&json)
                    .ok()
                    .map(|v| (cache_engine, v))
            } else {
                None
            }
        });

        let mut fields = match structured {
            Some((cache_engine, v)) => {
                engine = Some(cache_engine);
                ocr_fields::from_structured(&v, &text)
            }
            None => {
                // 2. 本地版面坐标（几何配对） 3. 纯文本
                let layout = ocr_layout_repo::get_layout(conn, attachment_id)?;
                match layout {
                    Some((layout_engine, lines)) if !lines.is_empty() => {
                        engine = Some(layout_engine);
                        ocr_layout::extract_with_layout(&lines)
                    }
                    _ => ocr_fields::extract_validated(&text),
                }
            }
        };

        // 三层都没给出引擎名时（老附件），退回识别缓存里记录的那个
        if engine.is_none() {
            engine = crate::db::repos::ocr_cache_repo::get(conn, &att.file_hash)?.map(|(e, _)| e);
        }
        fields.engine = engine;

        // 流水/明细列表页：别把第一行的金额/对方当成本凭证的要素
        if ocr_fields::looks_like_statement_list(&text) {
            ocr_fields::suppress_for_statement_list(&mut fields);
        }
        Ok(fields)
    })
}

/// 凭证编号预览（设置页）：用当前命名模板计算指定日期（空 = 今天）的下一个编号。
pub fn next_voucher_no(ctx: &CoreContext, date: &str) -> Result<String> {
    let engine = naming::NamingEngine::new(&ctx.config().naming.template);
    let d = if date.is_empty() {
        crate::utils::paths::today()
    } else {
        date.to_string()
    };
    ctx.db.with_conn(|conn| engine.next_no(conn, &d))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CoreError;

    /// 建临时数据目录 + 完整 CoreContext（schema 就绪、配置落默认值）。
    fn temp_ctx() -> (tempfile::TempDir, CoreContext) {
        let dir = tempfile::tempdir().unwrap();
        let ctx = CoreContext::init(Some(dir.path().to_path_buf())).unwrap();
        (dir, ctx)
    }

    #[test]
    fn voucher_thumbs_empty_ids_returns_empty_map() {
        let (_dir, ctx) = temp_ctx();
        let thumbs = voucher_thumbs(&ctx, &[]).unwrap();
        assert!(thumbs.is_empty(), "空 id 列表应跳过 SQL 直接返回空 map");
    }

    #[test]
    fn attachment_missing_returns_ok_none() {
        let (_dir, ctx) = temp_ctx();
        assert!(
            matches!(attachment(&ctx, 42_101), Ok(None)),
            "不存在的附件应返回 Ok(None)，由调用方决定文案"
        );
    }

    #[test]
    fn ocr_preview_missing_attachment_returns_not_found() {
        let (_dir, ctx) = temp_ctx();
        let err = ocr_preview(&ctx, 42_102).expect_err("不存在的附件应返回错误");
        let is_expected = match err {
            CoreError::NotFound(ref msg) => msg.contains("42102"),
            _ => false,
        };
        assert!(
            is_expected,
            "不存在的附件应返回 NotFound 且带附件 id，实际 {err:?}"
        );
    }

    #[test]
    fn no_image_filter_keeps_only_vouchers_without_attachment() {
        let (_dir, ctx) = temp_ctx();
        ctx.db
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO vouchers (voucher_no, voucher_date, created_at, updated_at)
                     VALUES ('N-1', '2026-10-01', '2026-10-01T00:00:00', '2026-10-01T00:00:00')",
                    [],
                )?;
                conn.execute(
                    "INSERT INTO vouchers (voucher_no, voucher_date, created_at, updated_at)
                     VALUES ('N-2', '2026-10-02', '2026-10-02T00:00:00', '2026-10-02T00:00:00')",
                    [],
                )?;
                conn.execute(
                    "INSERT INTO attachments (voucher_id, file_name, file_path, file_hash, created_at)
                     VALUES (1, 'only.jpg', 'p/only.jpg', 'h-only', '2026-10-01T00:00:00')",
                    [],
                )?;
                Ok(())
            })
            .unwrap();

        let with_image = voucher_ids(
            &ctx,
            &VoucherQuery {
                no_image: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(with_image, vec![2], "只应返回没有附件的那张（N-2）");

        let all = voucher_ids(&ctx, &VoucherQuery::default()).unwrap();
        assert_eq!(all.len(), 2, "默认不过滤时两张都在");
    }

    #[test]
    fn suspected_duplicates_missing_voucher_returns_empty() {
        let (_dir, ctx) = temp_ctx();
        let out = suspected_duplicates(&ctx, 42_103).unwrap();
        assert!(out.is_empty(), "凭证不存在时疑似重复应为空");
    }
}
