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

use crate::db::repos::{attachment_repo, ocr_layout_repo};
use crate::models::{Attachment, Voucher};
use crate::services::{dedup, naming, ocr_fields, ocr_layout};
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

/// 疑似重复组（原 `services::dedup::DuplicateGroup`）
pub use crate::services::dedup::DuplicateGroup;

// ---- 命令 DTO（原定义在命令层，现收敛到 core，JSON 契约不变） ----

/// D-1 列表缩略图信息：相对附件根目录的路径，前端用 convertFileSrc 转换
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoucherThumb {
    pub file_path: String,
    pub enhanced_path: Option<String>,
    pub enhanced_source: String,
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
            (
                vid,
                VoucherThumb {
                    file_path,
                    enhanced_path,
                    enhanced_source,
                },
            )
        })
        .collect())
}

/// 单凭证疑似重复（保存后前端提示）：同日期同金额的其他凭证。
pub fn suspected_duplicates(ctx: &CoreContext, id: i64) -> Result<Vec<Voucher>> {
    ctx.db.with_conn(|conn| dedup::suspected_by_id(conn, id))
}

/// 全库疑似重复扫描（仪表盘「疑似重复」面板）：同日期同金额、组内 ≥2 张。
pub fn duplicate_groups(ctx: &CoreContext) -> Result<Vec<DuplicateGroup>> {
    ctx.db.with_conn(dedup::scan_groups)
}

/// 附件实体查询（附件不存在返回 `Ok(None)`，由调用方决定文案）。
pub fn attachment(ctx: &CoreContext, id: i64) -> Result<Option<Attachment>> {
    ctx.db.with_conn(|conn| attachment_repo::get(conn, id))
}

/// OCR 字段提取预览（只读）：从附件已识别文本抽取结构化字段，不写库。
/// 有版面数据走几何配对（含置信度），否则回退纯文本抽取；附件不存在返回 NotFound。
pub fn ocr_preview(ctx: &CoreContext, attachment_id: i64) -> Result<ocr_fields::ExtractedFields> {
    ctx.db.with_conn(|conn| {
        let text = attachment_repo::get(conn, attachment_id)?
            .map(|a| a.ocr_text)
            .ok_or_else(|| crate::CoreError::NotFound(format!("附件 {attachment_id} 不存在")))?;
        let layout = ocr_layout_repo::get_layout(conn, attachment_id)?;
        Ok(match layout {
            Some(lines) if !lines.is_empty() => ocr_layout::extract_with_layout(&lines),
            _ => ocr_fields::extract_validated(text.as_deref().unwrap_or("")),
        })
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
