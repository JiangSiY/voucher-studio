//! 台账对齐字段：读写、必填校验、归档名计算（Center v0.26）
//!
//! 三件事刻意放在一处，因为它们共用同一份字段语义：
//! 1. **读写** `voucher_ext`（1:1 扩展表，见 `db/schema.rs` 的 `SCHEMA_V14`）；
//! 2. **必填校验**——复核（reviewed）之前必须补齐，否则凭证库就是「一堆无法入账的图片」；
//! 3. **归档名**——复刻 V5 的「附件名称」公式，重命名磁盘文件与列表展示同用。
//!
//! 必填清单的取舍：宁可少而要，不可多而虚。V5 设计原则写的是「录入面 ≤ 6 个必填」，
//! 这里同样只卡 6 项（日期/金额/对方/单据类型/分类 + 条件项），其余一律选填。

use rusqlite::{params, Connection, OptionalExtension};

use crate::models::biz::{
    BizFields, DOC_TYPE_CASH_IN, DOC_TYPE_CASH_OUT, DOC_TYPE_METER, DOC_TYPE_PAYABLE,
    DOC_TYPE_PROJECT, DOC_TYPE_RECEIVABLE, DOC_TYPE_WAREHOUSE, TICKET_SPECIAL_VAT,
};
use crate::models::Voucher;
use crate::services::naming::archive;
use crate::{CoreError, Result};

/// 已知单据类型（写入前校验，避免拼错后归档名/分录全部失灵）
pub const DOC_TYPES: &[&str] = &[
    DOC_TYPE_RECEIVABLE,
    DOC_TYPE_PAYABLE,
    DOC_TYPE_WAREHOUSE,
    DOC_TYPE_CASH_IN,
    DOC_TYPE_CASH_OUT,
    "电费",
    "项目",
];

const COLS: &str =
    "doc_type, category, biz_domain, flow_dir, account, ticket_type, invoice_no, invoice_date, \
     project_key, project_name, counterparty_key, material_name, material_qty, material_unit, \
     meter_no, meter_month, archive_name, source_doc_ref, gl_voucher_no, gl_batch, \
     gl_voucher_date, gl_booked_at";

/// 读取（无扩展行时返回全默认值——老凭证无需迁移，读出来就是「未填写」）
pub fn load(conn: &Connection, voucher_id: i64) -> Result<BizFields> {
    let row = conn
        .query_row(
            &format!("SELECT {COLS} FROM voucher_ext WHERE voucher_id = ?1"),
            params![voucher_id],
            |r| {
                Ok(BizFields {
                    doc_type: r.get(0)?,
                    category: r.get(1)?,
                    biz_domain: r.get(2)?,
                    flow_dir: r.get(3)?,
                    account: r.get(4)?,
                    ticket_type: r.get(5)?,
                    invoice_no: r.get(6)?,
                    invoice_date: r.get(7)?,
                    project_key: r.get(8)?,
                    project_name: r.get(9)?,
                    counterparty_key: r.get(10)?,
                    material_name: r.get(11)?,
                    material_qty: r.get(12)?,
                    material_unit: r.get(13)?,
                    meter_no: r.get(14)?,
                    meter_month: r.get(15)?,
                    archive_name: r.get(16)?,
                    source_doc_ref: r.get(17)?,
                    gl_voucher_no: r.get(18)?,
                    gl_batch: r.get(19)?,
                    gl_voucher_date: r.get(20)?,
                    gl_booked_at: r.get(21)?,
                })
            },
        )
        .optional()?;
    Ok(row.unwrap_or_default())
}

/// 写入（UPSERT；`updated_at` 由调用方给，便于测试固定时间）
pub fn save(conn: &Connection, voucher_id: i64, b: &BizFields) -> Result<()> {
    if !b.doc_type.is_empty() && !DOC_TYPES.contains(&b.doc_type.as_str()) {
        return Err(CoreError::Invalid(format!(
            "未知的单据类型「{}」（可用：{}）",
            b.doc_type,
            DOC_TYPES.join(" / ")
        )));
    }
    conn.execute(
        &format!(
            "INSERT INTO voucher_ext (voucher_id, {COLS}, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)
             ON CONFLICT(voucher_id) DO UPDATE SET
               doc_type=excluded.doc_type, category=excluded.category,
               biz_domain=excluded.biz_domain, flow_dir=excluded.flow_dir,
               account=excluded.account,
               ticket_type=excluded.ticket_type, invoice_no=excluded.invoice_no,
               invoice_date=excluded.invoice_date, project_key=excluded.project_key,
               project_name=excluded.project_name, counterparty_key=excluded.counterparty_key,
               material_name=excluded.material_name, material_qty=excluded.material_qty,
               material_unit=excluded.material_unit, meter_no=excluded.meter_no,
               meter_month=excluded.meter_month, archive_name=excluded.archive_name,
               source_doc_ref=excluded.source_doc_ref, gl_voucher_no=excluded.gl_voucher_no,
               gl_batch=excluded.gl_batch, gl_voucher_date=excluded.gl_voucher_date,
               gl_booked_at=excluded.gl_booked_at, updated_at=excluded.updated_at"
        ),
        params![
            voucher_id,
            b.doc_type,
            b.category,
            b.biz_domain,
            b.flow_dir,
            b.account,
            b.ticket_type,
            b.invoice_no,
            b.invoice_date,
            b.project_key,
            b.project_name,
            b.counterparty_key,
            b.material_name,
            b.material_qty,
            b.material_unit,
            b.meter_no,
            b.meter_month,
            b.archive_name,
            b.source_doc_ref,
            b.gl_voucher_no,
            b.gl_batch,
            b.gl_voucher_date,
            b.gl_booked_at,
            crate::utils::paths::now_iso(),
        ],
    )?;
    Ok(())
}

/// 必填项清单（空数组 = 可以复核）。
///
/// 顺序即前端展示顺序：先档案层（日期/金额/对方），再台账层（类型/分类），最后条件项。
pub fn missing_required(v: &Voucher, b: &BizFields) -> Vec<String> {
    let mut out = Vec::new();
    if v.voucher_date.trim().is_empty() {
        out.push("凭证日期".to_string());
    }
    if v.amount_cents.is_none() {
        out.push("金额".to_string());
    }
    if v.counterparty.trim().is_empty() && b.counterparty_key.trim().is_empty() {
        out.push("对方".to_string());
    }
    if b.doc_type.trim().is_empty() {
        out.push("单据类型".to_string());
    }
    if b.category.trim().is_empty() {
        out.push("分类".to_string());
    }
    // 条件必填：专票必须能对上票号（税务口径），仓库单必须说清是什么物料
    if b.ticket_type.trim() == TICKET_SPECIAL_VAT && b.invoice_no.trim().is_empty() {
        out.push("发票号（增值税专票必填）".to_string());
    }
    if b.doc_type.trim() == DOC_TYPE_WAREHOUSE && b.material_name.trim().is_empty() {
        out.push("物料名称（仓库单必填）".to_string());
    }
    // 电费单的归档名是 `{日期}-{电表}-抄{月度}-{金额}`，没有月度就产不出名字
    if b.doc_type.trim() == DOC_TYPE_METER && b.meter_month.trim().is_empty() {
        out.push("电费月度（电费单必填）".to_string());
    }
    out
}

/// 复核前拦截：缺项一次性报全（不是遇到第一个就返回，否则用户要来回试三遍）
pub fn ensure_required(v: &Voucher, b: &BizFields) -> Result<()> {
    let missing = missing_required(v, b);
    if missing.is_empty() {
        Ok(())
    } else {
        Err(CoreError::RequiredMissing(missing))
    }
}

/// 归档名：按单据类型选公式。
///
/// 需要的字段缺了就报错而不是「算个残缺名」——残缺名一旦落盘，回写台账时对不上，
/// 人工还得再猜一次。电费单的公式（表号/抄表月/负数前缀）留到 M3 与电费台账一起做。
pub fn archive_name_for(v: &Voucher, b: &BizFields) -> Result<String> {
    let doc_no = || -> Result<i64> {
        b.source_doc_ref
            .trim()
            .parse::<i64>()
            .map_err(|_| CoreError::Invalid("归档名需要来源单据号（V5 单据号/流水序号）".into()))
    };
    match b.doc_type.as_str() {
        DOC_TYPE_RECEIVABLE | DOC_TYPE_PAYABLE | DOC_TYPE_WAREHOUSE => Ok(archive::doc(
            &v.voucher_date,
            &b.project_name,
            &b.doc_type,
            doc_no()?,
            &v.counterparty,
            &b.material_name,
        )),
        DOC_TYPE_CASH_IN | DOC_TYPE_CASH_OUT => {
            let dir = if b.flow_dir.trim().is_empty() {
                if b.doc_type == DOC_TYPE_CASH_IN {
                    "收"
                } else {
                    "付"
                }
            } else {
                b.flow_dir.trim()
            };
            Ok(archive::cash(
                &v.voucher_date,
                &b.project_name,
                dir,
                doc_no()?,
                &v.counterparty,
            ))
        }
        DOC_TYPE_PROJECT => Ok(archive::project(
            &v.counterparty,
            &b.category,
            &v.voucher_date,
        )),
        DOC_TYPE_METER => {
            if b.meter_month.trim().is_empty() {
                return Err(CoreError::RequiredMissing(vec![
                    "电费月度（电费单必填）".to_string()
                ]));
            }
            let amount = v
                .amount_cents
                .ok_or_else(|| CoreError::RequiredMissing(vec!["金额".to_string()]))?;
            Ok(archive::meter(
                &v.voucher_date,
                &b.meter_no,
                &b.meter_month,
                amount,
            ))
        }
        other => Err(CoreError::Invalid(format!(
            "单据类型「{other}」没有对应的归档名公式"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;
    use crate::models::{NewVoucher, VoucherStatus};

    fn mem() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        schema::init(&conn).unwrap();
        conn
    }

    fn voucher(id: i64) -> Voucher {
        Voucher {
            id,
            voucher_no: format!("2026-07-16-{id:02}"),
            voucher_date: "2026-07-16".into(),
            description: "工程款".into(),
            counterparty: "张帅".into(),
            amount_cents: Some(1_234_500),
            status: VoucherStatus::Draft,
            tags: vec![],
            note: String::new(),
            ledger_ref: None,
            created_at: "2026-07-16T00:00:00Z".into(),
            updated_at: "2026-07-16T00:00:00Z".into(),
        }
    }

    #[test]
    fn load_returns_defaults_without_row() {
        let conn = mem();
        let b = load(&conn, 999).unwrap();
        assert_eq!(b, BizFields::default());
    }

    #[test]
    fn save_roundtrip_and_validate() {
        let conn = mem();
        let mut v = voucher(1);
        v.ledger_ref = Some("doc:123".into());
        // 先让 vouchers 行存在（外键）
        crate::db::repos::voucher_repo::insert(&conn, &NewVoucher::default()).ok();
        let b = BizFields {
            doc_type: DOC_TYPE_PAYABLE.into(),
            category: "施工外包".into(),
            source_doc_ref: "12".into(),
            ..Default::default()
        };
        save(&conn, 1, &b).unwrap();
        assert_eq!(load(&conn, 1).unwrap(), b);
        // 改一个字段再存（UPSERT 路径）
        let b2 = BizFields {
            invoice_no: "INV-1".into(),
            ..b
        };
        save(&conn, 1, &b2).unwrap();
        assert_eq!(load(&conn, 1).unwrap().invoice_no, "INV-1");
        // 未知类型被拒
        let bad = BizFields {
            doc_type: "乱写".into(),
            ..Default::default()
        };
        assert!(save(&conn, 1, &bad).is_err());
    }

    #[test]
    fn missing_required_covers_conditional_rules() {
        let v = voucher(1);
        let empty = BizFields::default();
        let miss = missing_required(&v, &empty);
        assert_eq!(miss, vec!["单据类型", "分类"]);

        let mut b = BizFields {
            doc_type: DOC_TYPE_WAREHOUSE.into(),
            category: "材料".into(),
            ..Default::default()
        };
        assert!(missing_required(&v, &b).contains(&"物料名称（仓库单必填）".to_string()));
        b.material_name = "电缆".into();
        b.ticket_type = TICKET_SPECIAL_VAT.into();
        assert!(missing_required(&v, &b).contains(&"发票号（增值税专票必填）".to_string()));
        b.invoice_no = "12345678".into();
        assert!(missing_required(&v, &b).is_empty());

        let mut no_amount = voucher(2);
        no_amount.amount_cents = None;
        assert_eq!(missing_required(&no_amount, &b), vec!["金额"]);
        assert!(ensure_required(&no_amount, &b).is_err());
    }

    #[test]
    fn archive_name_switches_by_doc_type() {
        let v = voucher(1);
        let payable = BizFields {
            doc_type: DOC_TYPE_PAYABLE.into(),
            category: "施工外包".into(),
            source_doc_ref: "1".into(),
            project_name: "张帅-585kW-自投电站".into(),
            ..Default::default()
        };
        assert_eq!(
            archive_name_for(&v, &payable).unwrap(),
            "260716-张帅-应付0001-张帅"
        );
        let cash = BizFields {
            doc_type: DOC_TYPE_CASH_OUT.into(),
            flow_dir: "付".into(),
            source_doc_ref: "43".into(),
            ..Default::default()
        };
        assert_eq!(
            archive_name_for(&v, &cash).unwrap(),
            "260716-公司费用-付0043-张帅"
        );
        // 缺来源单据号 → 明确报错，不产出残缺名
        let no_ref = BizFields {
            doc_type: DOC_TYPE_RECEIVABLE.into(),
            ..Default::default()
        };
        assert!(archive_name_for(&v, &no_ref).is_err());
    }

    #[test]
    fn meter_archive_name_uses_month_and_amount() {
        let mut v = voucher(1);
        v.amount_cents = Some(123_456);
        let meter = BizFields {
            doc_type: DOC_TYPE_METER.into(),
            category: "电费".into(),
            meter_no: "表A-01".into(),
            meter_month: "2026-10".into(),
            ..Default::default()
        };
        assert_eq!(
            archive_name_for(&v, &meter).unwrap(),
            "260716-表A-01-抄2610-1234.56"
        );
        // 月度缺失：不进归档名，而是明确告诉用户缺什么
        let no_month = BizFields {
            doc_type: DOC_TYPE_METER.into(),
            category: "电费".into(),
            ..Default::default()
        };
        assert!(matches!(
            archive_name_for(&v, &no_month),
            Err(CoreError::RequiredMissing(_))
        ));
        assert!(missing_required(&v, &no_month).contains(&"电费月度（电费单必填）".to_string()));
    }
}

/// 一张凭证的台账字段全貌（前端编辑器一次拿全）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BizDetail {
    pub voucher_id: i64,
    pub fields: BizFields,
    /// 还缺哪些必填项（空数组 = 可复核）
    pub missing: Vec<String>,
    /// 按当前字段算出的归档名（算不出时为空串）
    pub archive_name: String,
    /// 算不出归档名的原因（成功时为空串）——直接给用户看，别让他猜
    pub archive_error: String,
}

/// 读凭证 + 扩展字段 + 必填缺项 + 归档名预览
pub fn detail(ctx: &crate::CoreContext, voucher_id: i64) -> Result<BizDetail> {
    ctx.db.with_conn(|conn| {
        let v = crate::db::repos::voucher_repo::get(conn, voucher_id)?
            .ok_or_else(|| CoreError::NotFound(format!("凭证 {voucher_id} 不存在")))?;
        let fields = load(conn, voucher_id)?;
        let missing = missing_required(&v, &fields);
        let (archive_name, archive_error) = match archive_name_for(&v, &fields) {
            Ok(name) => (name, String::new()),
            Err(CoreError::NotImplemented(msg)) => (String::new(), msg.to_string()),
            Err(e) => (String::new(), e.to_string()),
        };
        Ok(BizDetail {
            voucher_id,
            fields,
            missing,
            archive_name,
            archive_error,
        })
    })
}

/// 归档改名结果（一条附件一行）
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveApplied {
    pub voucher_id: i64,
    pub archive_name: String,
    pub files: Vec<RenamedFile>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenamedFile {
    pub attachment_id: i64,
    pub old_name: String,
    pub new_name: String,
}

/// 按台账口径给附件改名，并写回 `archive_name`。
///
/// 顺序刻意是「先算 → 再落盘 → 最后写库」：
/// - 算不出来（缺单据号）就整笔拒绝，不留半成品；
/// - 落盘失败（磁盘满/无权限）时库还没改，重试是幂等的；
/// - 同名已存在时自动加 `-2`、`-3`，绝不覆盖别人的凭证。
pub fn apply_archive_name(ctx: &crate::CoreContext, voucher_id: i64) -> Result<ArchiveApplied> {
    let (voucher, fields) = ctx.db.with_conn(|conn| {
        let v = crate::db::repos::voucher_repo::get(conn, voucher_id)?
            .ok_or_else(|| CoreError::NotFound(format!("凭证 {voucher_id} 不存在")))?;
        let b = load(conn, voucher_id)?;
        Ok((v, b))
    })?;
    let name = archive_name_for(&voucher, &fields)?;
    let files = rename_attachments(ctx, &voucher, &name)?;
    ctx.db.with_conn(|conn| {
        save(
            conn,
            voucher_id,
            &BizFields {
                archive_name: name.clone(),
                ..fields.clone()
            },
        )?;
        for f in &files {
            conn.execute(
                "INSERT INTO attachment_ext (attachment_id, original_file_name, archive_name, role)
                 VALUES (?1, ?2, ?3, 'primary')
                 ON CONFLICT(attachment_id) DO UPDATE SET archive_name=excluded.archive_name",
                params![f.attachment_id, f.old_name, f.new_name],
            )?;
            let updated = conn.execute(
                "UPDATE attachments SET file_name = ?2 WHERE id = ?1",
                params![f.attachment_id, f.new_name],
            )?;
            if updated == 0 {
                return Err(CoreError::NotFound(format!(
                    "附件 {} 不存在",
                    f.attachment_id
                )));
            }
        }
        Ok(())
    })?;
    Ok(ArchiveApplied {
        voucher_id,
        archive_name: name,
        files,
    })
}

fn rename_attachments(
    ctx: &crate::CoreContext,
    voucher: &Voucher,
    archive_name: &str,
) -> Result<Vec<RenamedFile>> {
    let attachments = ctx
        .db
        .with_conn(|conn| crate::db::repos::attachment_repo::list_by_voucher(conn, voucher.id))?;
    if attachments.is_empty() {
        return Err(CoreError::Invalid(
            "这张凭证还没有附件，先补附件再改名".into(),
        ));
    }
    let month_dir = &voucher.voucher_date[..7.min(voucher.voucher_date.len())];
    let dir = ctx.paths.attachments.join(month_dir.replace('-', "/"));
    std::fs::create_dir_all(&dir)?;
    let mut out = Vec::new();
    for a in attachments {
        let src = ctx.paths.attachment_abs(&a.file_path);
        if !src.is_file() {
            return Err(CoreError::NotFound(format!(
                "附件文件不在磁盘上：{}",
                src.display()
            )));
        }
        let ext = src
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
            .unwrap_or_default();
        let mut dest = dir.join(format!("{archive_name}{ext}"));
        let mut n = 2;
        while dest.exists() && dest != src {
            dest = dir.join(format!("{archive_name}-{n}{ext}"));
            n += 1;
        }
        if dest != src {
            std::fs::rename(&src, &dest)?;
        }
        let rel = dest
            .strip_prefix(&ctx.paths.base)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .map_err(|_| CoreError::Invalid("改名后的路径不在数据目录内".into()))?;
        ctx.db.with_conn(|conn| {
            conn.execute(
                "UPDATE attachments SET file_path = ?2 WHERE id = ?1",
                params![a.id, rel],
            )?;
            Ok(())
        })?;
        out.push(RenamedFile {
            attachment_id: a.id,
            old_name: a.file_name.clone(),
            new_name: dest
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
        });
    }
    Ok(out)
}
