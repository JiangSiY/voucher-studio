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
    BizFields, DOC_TYPE_CASH_IN, DOC_TYPE_CASH_OUT, DOC_TYPE_PAYABLE, DOC_TYPE_RECEIVABLE,
    DOC_TYPE_WAREHOUSE, TICKET_SPECIAL_VAT,
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
    "doc_type, category, biz_domain, flow_dir, ticket_type, invoice_no, invoice_date, \
     project_key, project_name, counterparty_key, material_name, material_qty, material_unit, \
     archive_name, source_doc_ref, gl_voucher_no, gl_batch, gl_voucher_date, gl_booked_at";

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
                    ticket_type: r.get(4)?,
                    invoice_no: r.get(5)?,
                    invoice_date: r.get(6)?,
                    project_key: r.get(7)?,
                    project_name: r.get(8)?,
                    counterparty_key: r.get(9)?,
                    material_name: r.get(10)?,
                    material_qty: r.get(11)?,
                    material_unit: r.get(12)?,
                    archive_name: r.get(13)?,
                    source_doc_ref: r.get(14)?,
                    gl_voucher_no: r.get(15)?,
                    gl_batch: r.get(16)?,
                    gl_voucher_date: r.get(17)?,
                    gl_booked_at: r.get(18)?,
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
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)
             ON CONFLICT(voucher_id) DO UPDATE SET
               doc_type=excluded.doc_type, category=excluded.category,
               biz_domain=excluded.biz_domain, flow_dir=excluded.flow_dir,
               ticket_type=excluded.ticket_type, invoice_no=excluded.invoice_no,
               invoice_date=excluded.invoice_date, project_key=excluded.project_key,
               project_name=excluded.project_name, counterparty_key=excluded.counterparty_key,
               material_name=excluded.material_name, material_qty=excluded.material_qty,
               material_unit=excluded.material_unit, archive_name=excluded.archive_name,
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
            b.ticket_type,
            b.invoice_no,
            b.invoice_date,
            b.project_key,
            b.project_name,
            b.counterparty_key,
            b.material_name,
            b.material_qty,
            b.material_unit,
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
        "项目" => Ok(archive::project(
            &v.counterparty,
            &b.category,
            &v.voucher_date,
        )),
        "电费" => Err(CoreError::NotImplemented(
            "电费附件名要在 M3 与电费台账一起实现（表号/抄表月）",
        )),
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
}
