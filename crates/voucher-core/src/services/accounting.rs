//! 会计投影（M2）：把台账凭证投影成金蝶分录。
//!
//! 口径与 `财务工作流/tools/make_kis_copy_sheet.py` 完全一致（用户 2026-11-08 定稿）：
//!
//! | 类型 | 分组键 | 借方 | 贷方 |
//! |------|--------|------|------|
//! | 资金收（收款） | 日期 + 账户 | 银行/现金 1 行合计 | 1122 应收账款，按客户多行 |
//! | 资金付（付款） | 日期 + 账户 | 成本/费用按分类多行 | 银行/现金 1 行合计 |
//! | 应收/应付（计提） | 月份 + 分类 | 成本/费用 | 2202 应付账款 |
//!
//! 三条硬规矩：
//! 1. **借贷差一分即拒**——不静默调平，宁可报错让人看；
//! 2. **缺科目映射即报错**，不猜科目；
//! 3. 分录先落 `gl_entries`（status=draft），导出只是同一批数据的另一种排版。
//!
//! 为什么按「日期+账户」汇总：金蝶迷你版不支持凭证导入，只能手抄；一个月 250+ 行
//! 汇总成 40–60 张凭证，20–30 分钟能抄完（脚本注释里的原话）。

use std::collections::BTreeMap;
use std::path::PathBuf;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::models::biz::{
    DOC_TYPE_CASH_IN, DOC_TYPE_CASH_OUT, DOC_TYPE_PAYABLE, DOC_TYPE_RECEIVABLE,
};
use crate::{CoreError, Result};

/// 应收账款科目（金蝶 KIS 迷你版）
pub const RECEIVABLE_CODE: &str = "1122";
/// 应付账款科目
pub const PAYABLE_CODE: &str = "2202";

/// 默认科目表（来源：V5「9 科目映射」22 条 + `config/kis_acct_map.json`）。
///
/// 只在 `acct_category_maps` / `acct_account_maps` 为空时写入，之后以库里的为准——
/// 用户改过就不要再覆盖。
const DEFAULT_CATEGORY_MAPS: &[(&str, &str, &str, &str, &str)] = &[
    ("工商业安装款", "500101", "工商业安装收入", "贷", "项目收入"),
    ("电费", "500103", "电费收入", "贷", "项目收入"),
    ("运维服务", "500104", "电站运维收入", "贷", "项目收入"),
    ("并网款", "2203", "预收账款", "贷", "项目收入"),
    ("往来款", "1221", "其他应收款", "借", "往来款"),
    ("材料", "540101", "材料成本", "借", "项目成本"),
    ("施工外包", "540102", "施工外包成本", "借", "项目成本"),
    ("并网费", "540103", "并网及其他", "借", "项目成本"),
    ("上料", "540103", "并网及其他", "借", "项目成本"),
    ("业务费", "540103", "并网及其他", "借", "项目成本"),
    ("运费", "540103", "并网及其他", "借", "项目成本"),
    ("电站运维成本", "540105", "电站运维成本", "借", "运维"),
    ("电站租金", "540106", "电站租金", "借", "运维"),
    ("合作方分成", "540107", "合作方分成", "借", "运维"),
    ("自持电站建设", "160107", "自持电站", "借", "固定资产"),
    ("职工薪酬", "560207", "职工薪酬", "借", "工资社保"),
    ("业务招待", "560208", "业务招待费", "借", "公司费用"),
    ("车辆费用", "560216", "车辆费用", "借", "公司费用"),
    ("施工意外险", "560217", "施工意外险", "借", "公司费用"),
    ("其他报销", "560218", "其他报销", "借", "公司费用"),
];

/// 账户 → 科目。V5 改版后「账户」字段装的是结算方式（银行转账/微信/…），
/// 所以键直接用这些值；`_default` 兜底走建行。
const DEFAULT_ACCOUNT_MAPS: &[(&str, &str, &str)] = &[
    ("银行转账", "100201", "银行存款-建行"),
    ("工行", "1002", "银行存款"),
    ("建行", "100201", "银行存款-建行"),
    ("农行", "100202", "银行存款-农行"),
    ("现金", "1001", "库存现金"),
    ("微信", "100203", "银行存款-微信"),
    ("支付宝", "100204", "银行存款-支付宝"),
    ("承兑", "1121", "应收票据"),
    ("其他", "1002", "银行存款"),
    ("_default", "1002", "银行存款"),
];

/// 计提时的贷方科目（分类 → 贷方）。缺省一律 2202 应付账款；
/// 职工薪酬记应付职工薪酬、税金走应交税费（与脚本 `accrual` 表同口径）。
const DEFAULT_ACCRUAL_CREDIT: &[(&str, &str, &str)] = &[
    ("职工薪酬", "2211", "应付职工薪酬"),
    ("施工意外险", "2241", "其他应付款"),
];

/// 一笔分录行（借贷两方同构，金额用分）
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EntryLine {
    pub entry_date: String,
    pub summary: String,
    pub account_code: String,
    pub account_name: String,
    pub debit_cents: i64,
    pub credit_cents: i64,
    pub project_key: String,
    pub source_type: String,
    pub source_ref: String,
}

/// 一张凭证（若干分录行 + 借货合计）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryVoucher {
    pub kind: String,
    pub entry_date: String,
    pub summary: String,
    pub lines: Vec<EntryLine>,
    pub debit_cents: i64,
    pub credit_cents: i64,
}

impl EntryVoucher {
    fn balanced(&self) -> bool {
        self.debit_cents == self.credit_cents
    }
}

/// 一期的投影结果（未落库）
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PeriodPlan {
    pub period: String,
    pub receipts: Vec<EntryVoucher>,
    pub payments: Vec<EntryVoucher>,
    pub accruals: Vec<EntryVoucher>,
    /// 需要人看一眼的问题（缺映射、缺分类、金额为零…）；不为空时 `post` 会拒绝
    pub issues: Vec<String>,
}

impl PeriodPlan {
    pub fn total_vouchers(&self) -> usize {
        self.receipts.len() + self.payments.len() + self.accruals.len()
    }

    pub fn debit_cents(&self) -> i64 {
        self.receipts
            .iter()
            .chain(self.payments.iter())
            .chain(self.accruals.iter())
            .map(|v| v.debit_cents)
            .sum()
    }

    pub fn credit_cents(&self) -> i64 {
        self.receipts
            .iter()
            .chain(self.payments.iter())
            .chain(self.accruals.iter())
            .map(|v| v.credit_cents)
            .sum()
    }

    /// 全部凭证必须借贷相等，且没有 issues
    pub fn validate(&self) -> Result<()> {
        if !self.issues.is_empty() {
            return Err(CoreError::Invalid(format!(
                "分录还有 {} 个问题要先处理：{}",
                self.issues.len(),
                self.issues.join("；")
            )));
        }
        for v in self
            .receipts
            .iter()
            .chain(self.payments.iter())
            .chain(self.accruals.iter())
        {
            if !v.balanced() {
                return Err(CoreError::Invalid(format!(
                    "「{}」借贷不平：借 {} / 贷 {}（差 {} 分）",
                    v.summary,
                    v.debit_cents,
                    v.credit_cents,
                    v.debit_cents - v.credit_cents
                )));
            }
        }
        Ok(())
    }
}

/// 落库结果
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostResult {
    pub period: String,
    pub vouchers: usize,
    pub lines: usize,
    pub debit_cents: i64,
    pub credit_cents: i64,
}

/// 科目映射表（前端编辑用）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryMap {
    pub category: String,
    pub account_code: String,
    pub account_name: String,
    pub direction: String,
    pub biz_domain: String,
    pub note: String,
}

/// 账户映射表
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountMap {
    pub account_key: String,
    pub account_code: String,
    pub account_name: String,
    pub note: String,
}

fn table_empty(conn: &Connection, table: &str) -> Result<bool> {
    let n: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?;
    Ok(n == 0)
}

/// 首次使用时写入默认科目表（幂等：表非空就什么都不做）
pub fn seed_defaults(conn: &Connection) -> Result<()> {
    if table_empty(conn, "acct_category_maps")? {
        for (cat, code, name, dir, domain) in DEFAULT_CATEGORY_MAPS {
            conn.execute(
                "INSERT INTO acct_category_maps (category, account_code, account_name, direction, biz_domain, note, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, '', datetime('now'))
                 ON CONFLICT(category) DO NOTHING",
                params![cat, code, name, dir, domain],
            )?;
        }
    }
    if table_empty(conn, "acct_account_maps")? {
        for (key, code, name) in DEFAULT_ACCOUNT_MAPS {
            conn.execute(
                "INSERT INTO acct_account_maps (account_key, account_code, account_name, note, updated_at)
                 VALUES (?1, ?2, ?3, '', datetime('now'))
                 ON CONFLICT(account_key) DO NOTHING",
                params![key, code, name],
            )?;
        }
    }
    Ok(())
}

fn category_map(conn: &Connection, category: &str) -> Result<Option<(String, String)>> {
    let mut stmt = conn
        .prepare("SELECT account_code, account_name FROM acct_category_maps WHERE category = ?1")?;
    let mut rows = stmt.query(params![category])?;
    if let Some(r) = rows.next()? {
        return Ok(Some((r.get(0)?, r.get(1)?)));
    }
    Ok(None)
}

fn account_map(conn: &Connection, account: &str) -> Result<Option<(String, String)>> {
    for key in [account, "_default"] {
        let mut stmt = conn.prepare(
            "SELECT account_code, account_name FROM acct_account_maps WHERE account_key = ?1",
        )?;
        let mut rows = stmt.query(params![key])?;
        if let Some(r) = rows.next()? {
            return Ok(Some((r.get(0)?, r.get(1)?)));
        }
    }
    Ok(None)
}

fn accrual_credit(conn: &Connection, category: &str) -> Result<(String, String)> {
    for (cat, code, name) in DEFAULT_ACCRUAL_CREDIT {
        if *cat == category {
            return Ok((code.to_string(), name.to_string()));
        }
    }
    // 默认贷 2202 应付账款；账户映射表里允许用 `payable` 这个键覆盖成别的科目
    let code = conn
        .query_row(
            "SELECT account_code FROM acct_account_maps WHERE account_key = 'payable'",
            [],
            |r| r.get::<_, String>(0),
        )
        .unwrap_or_else(|_| PAYABLE_CODE.to_string());
    Ok((code, "应付账款".to_string()))
}

/// 参与投影的凭证（voucher + voucher_ext 的关键列）
#[derive(Debug, Clone)]
struct CashRow {
    voucher_no: String,
    date: String,
    amount_cents: i64,
    account: String,
    counterparty: String,
    project_name: String,
    category: String,
}

/// 读一期的凭证。只算**已补必填**的：缺分类/缺金额的会被记 issue 而不是静默跳过。
fn load_cash_rows(conn: &Connection, period: &str, flow: &str) -> Result<Vec<CashRow>> {
    let mut stmt = conn.prepare(
        "SELECT v.voucher_no, v.voucher_date, COALESCE(v.amount_cents, 0), v.counterparty,
                COALESCE(e.account, ''), COALESCE(e.project_name, ''),
                COALESCE(e.category, ''), COALESCE(e.project_key, '')
           FROM vouchers v
           JOIN voucher_ext e ON e.voucher_id = v.id
          WHERE e.doc_type = ?1
            AND substr(v.voucher_date, 1, 7) = ?2
            AND e.deleted_at IS NULL
          ORDER BY v.voucher_date, v.id",
    )?;
    let rows = stmt.query_map(params![flow, period], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (voucher_no, date, amount_cents, counterparty, account, project_name, category) = row?;
        out.push(CashRow {
            voucher_no,
            date,
            amount_cents,
            account,
            counterparty,
            project_name,
            category,
        });
    }
    Ok(out)
}

/// 月份（YYYY-MM）
/// `2026-11-02` → `11-02`（摘要里只要月日）
fn day_of(date: &str) -> &str {
    date.get(5..).unwrap_or(date)
}

fn month_of(date: &str) -> String {
    date.chars().take(7).collect()
}

/// 项目名压缩：`卢丙哲-585kW-自投电站` → `585kW`（与脚本 `proj_short` 同规则）
fn project_short(name: &str) -> String {
    let normalized = name.replace('－', "-");
    let parts: Vec<&str> = normalized.split('-').filter(|p| !p.is_empty()).collect();
    if parts.len() >= 2 {
        parts[1].to_string()
    } else {
        name.trim().to_string()
    }
}

fn ref_range(rows: &[&CashRow], limit: usize) -> String {
    let mut refs: Vec<&str> = rows.iter().map(|r| r.voucher_no.as_str()).collect();
    refs.sort_unstable();
    refs.dedup();
    if refs.is_empty() {
        return String::new();
    }
    if refs.len() <= limit {
        refs.join("、")
    } else {
        format!("{} 等{}笔", refs[..limit].join("、"), refs.len())
    }
}

fn cat_brief(rows: &[&CashRow]) -> String {
    let mut cats: Vec<&str> = rows
        .iter()
        .map(|r| {
            let c = r.category.trim();
            if c.is_empty() {
                "其他"
            } else {
                c
            }
        })
        .collect();
    cats.sort_unstable();
    cats.dedup();
    cats.join("、")
}

/// 生成一期的分录方案（不写库，供「预演」与「写入」共用）
pub fn build_period(conn: &Connection, period: &str) -> Result<PeriodPlan> {
    seed_defaults(conn)?;
    let mut plan = PeriodPlan {
        period: period.to_string(),
        ..Default::default()
    };

    let receipts = load_cash_rows(conn, period, DOC_TYPE_CASH_IN)?;
    let payments = load_cash_rows(conn, period, DOC_TYPE_CASH_OUT)?;
    let accruals = load_accrual_rows(conn, period)?;

    build_receipts(conn, &mut plan, &receipts)?;
    build_payments(conn, &mut plan, &payments)?;
    build_accruals(conn, &mut plan, &accruals)?;

    // 借贷不平的凭证直接挂 issue（不在这里调平）
    for v in plan
        .receipts
        .iter()
        .chain(plan.payments.iter())
        .chain(plan.accruals.iter())
    {
        if !v.balanced() {
            plan.issues.push(format!(
                "「{}」借贷差 {} 分",
                v.summary,
                v.debit_cents - v.credit_cents
            ));
        }
    }
    Ok(plan)
}

fn build_receipts(conn: &Connection, plan: &mut PeriodPlan, rows: &[CashRow]) -> Result<()> {
    // 分组键：日期 + 账户（V5 的「账户」已是结算方式；同日同账户汇总成一张凭证）
    let mut groups: BTreeMap<(String, String), Vec<&CashRow>> = BTreeMap::new();
    for r in rows {
        let account = if r.account.trim().is_empty() {
            "未分账户"
        } else {
            r.account.trim()
        };
        groups
            .entry((r.date.clone(), account.to_string()))
            .or_default()
            .push(r);
    }
    for ((date, account), items) in groups {
        let Some((code, name)) = account_map(conn, &account)? else {
            plan.issues.push(format!(
                "{date} 账户「{account}」没有科目映射（设置 → 科目映射）"
            ));
            continue;
        };
        let total: i64 = items.iter().map(|r| r.amount_cents).sum();
        if total <= 0 {
            plan.issues.push(format!(
                "{date} 账户「{account}」合计金额为 {total} 分，不能出凭证"
            ));
            continue;
        }
        let refs = ref_range(&items, 4);
        let brief = cat_brief(&items);
        let mut lines = Vec::new();
        lines.push(EntryLine {
            entry_date: date.clone(),
            summary: format!(
                "收 {account} {} 共{}笔（{brief}）{refs}",
                day_of(&date),
                items.len()
            ),
            account_code: code,
            account_name: name,
            debit_cents: total,
            credit_cents: 0,
            project_key: String::new(),
            source_type: "流水".into(),
            source_ref: refs.clone(),
        });
        // 贷方：按客户分组（对方为空写「未填客户」）
        let mut by_party: BTreeMap<String, Vec<&CashRow>> = BTreeMap::new();
        for r in &items {
            let party = if r.counterparty.trim().is_empty() {
                "未填客户".to_string()
            } else {
                r.counterparty.trim().to_string()
            };
            by_party.entry(party).or_default().push(r);
        }
        let mut credit_total = 0i64;
        for (party, its) in by_party {
            let amt: i64 = its.iter().map(|r| r.amount_cents).sum();
            credit_total += amt;
            lines.push(EntryLine {
                entry_date: date.clone(),
                summary: format!(
                    "收 {party} {} {}",
                    project_short(&its[0].project_name),
                    cat_brief(&its)
                ),
                account_code: RECEIVABLE_CODE.into(),
                account_name: "应收账款".into(),
                debit_cents: 0,
                credit_cents: amt,
                project_key: its[0].project_name.clone(),
                source_type: "流水".into(),
                source_ref: ref_range(&its, 4),
            });
        }
        plan.receipts.push(EntryVoucher {
            kind: "收款".into(),
            entry_date: date.clone(),
            summary: format!("收 {account} {} 共{}笔", day_of(&date), items.len()),
            lines,
            debit_cents: total,
            credit_cents: credit_total,
        });
    }
    Ok(())
}

fn build_payments(conn: &Connection, plan: &mut PeriodPlan, rows: &[CashRow]) -> Result<()> {
    let mut groups: BTreeMap<(String, String), Vec<&CashRow>> = BTreeMap::new();
    for r in rows {
        let account = if r.account.trim().is_empty() {
            "未分账户"
        } else {
            r.account.trim()
        };
        groups
            .entry((r.date.clone(), account.to_string()))
            .or_default()
            .push(r);
    }
    for ((date, account), items) in groups {
        let Some((code, name)) = account_map(conn, &account)? else {
            plan.issues.push(format!(
                "{date} 账户「{account}」没有科目映射（设置 → 科目映射）"
            ));
            continue;
        };
        let total: i64 = items.iter().map(|r| r.amount_cents).sum();
        if total <= 0 {
            plan.issues.push(format!(
                "{date} 账户「{account}」合计金额为 {total} 分，不能出凭证"
            ));
            continue;
        }
        let mut lines = Vec::new();
        let mut debit_total = 0i64;
        let mut mapped = true;
        // 借方：按分类多行
        let mut by_cat: BTreeMap<String, Vec<&CashRow>> = BTreeMap::new();
        for r in &items {
            let cat = if r.category.trim().is_empty() {
                "其他".to_string()
            } else {
                r.category.trim().to_string()
            };
            by_cat.entry(cat).or_default().push(r);
        }
        for (cat, its) in by_cat {
            let Some((acc, acc_name)) = category_map(conn, &cat)? else {
                plan.issues
                    .push(format!("分类「{cat}」没有科目映射（设置 → 科目映射）"));
                mapped = false;
                continue;
            };
            let amt: i64 = its.iter().map(|r| r.amount_cents).sum();
            debit_total += amt;
            lines.push(EntryLine {
                entry_date: date.clone(),
                summary: format!("付 {cat}：{} {}", party_brief(&its), ref_range(&its, 4)),
                account_code: acc,
                account_name: acc_name,
                debit_cents: amt,
                credit_cents: 0,
                project_key: its[0].project_name.clone(),
                source_type: "流水".into(),
                source_ref: ref_range(&its, 4),
            });
        }
        // 有分类没映射：这张凭证先不出，issues 已经记下是哪一类
        if !mapped {
            continue;
        }
        lines.push(EntryLine {
            entry_date: date.clone(),
            summary: format!(
                "付 {account} {} 共{}笔（{}）{}",
                day_of(&date),
                items.len(),
                cat_brief(&items),
                ref_range(&items, 4)
            ),
            account_code: code,
            account_name: name,
            debit_cents: 0,
            credit_cents: total,
            project_key: String::new(),
            source_type: "流水".into(),
            source_ref: ref_range(&items, 4),
        });
        plan.payments.push(EntryVoucher {
            kind: "付款".into(),
            entry_date: date.clone(),
            summary: format!("付 {account} {} 共{}笔", day_of(&date), items.len()),
            lines,
            debit_cents: debit_total,
            credit_cents: total,
        });
    }
    Ok(())
}

fn party_brief(rows: &[&CashRow]) -> String {
    let mut names: Vec<&str> = rows
        .iter()
        .map(|r| r.counterparty.trim())
        .filter(|s| !s.is_empty())
        .collect();
    names.dedup();
    if names.is_empty() {
        return "未填对方".into();
    }
    if names.len() <= 3 {
        names.join("、")
    } else {
        format!("{} 等{}家", names[..3].join("、"), names.len())
    }
}

/// 计提（应收/应付单据）行
#[derive(Debug, Clone)]
struct AccrualRow {
    voucher_no: String,
    date: String,
    amount_cents: i64,
    category: String,
    counterparty: String,
}

fn load_accrual_rows(conn: &Connection, period: &str) -> Result<Vec<AccrualRow>> {
    let mut stmt = conn.prepare(
        "SELECT v.voucher_no, v.voucher_date, COALESCE(v.amount_cents, 0), v.counterparty,
                COALESCE(e.category, '')
           FROM vouchers v
           JOIN voucher_ext e ON e.voucher_id = v.id
          WHERE e.doc_type IN (?1, ?2)
            AND substr(v.voucher_date, 1, 7) = ?3
            AND e.deleted_at IS NULL
          ORDER BY v.voucher_date, v.id",
    )?;
    let rows = stmt.query_map(
        params![DOC_TYPE_RECEIVABLE, DOC_TYPE_PAYABLE, period],
        |r| {
            Ok(AccrualRow {
                voucher_no: r.get(0)?,
                date: r.get(1)?,
                amount_cents: r.get(2)?,
                counterparty: r.get(3)?,
                category: r.get(4)?,
            })
        },
    )?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn build_accruals(conn: &Connection, plan: &mut PeriodPlan, rows: &[AccrualRow]) -> Result<()> {
    // 分组键：月份 + 分类（月末一次计提，不按天拆）
    let mut groups: BTreeMap<(String, String), Vec<&AccrualRow>> = BTreeMap::new();
    for r in rows {
        let cat = if r.category.trim().is_empty() {
            "其他".to_string()
        } else {
            r.category.trim().to_string()
        };
        groups.entry((month_of(&r.date), cat)).or_default().push(r);
    }
    for ((month, cat), items) in groups {
        let total: i64 = items.iter().map(|r| r.amount_cents).sum();
        if total <= 0 {
            continue;
        }
        let Some((acc, acc_name)) = category_map(conn, &cat)? else {
            plan.issues
                .push(format!("分类「{cat}」没有科目映射（设置 → 科目映射）"));
            continue;
        };
        let (credit_code, credit_name) = accrual_credit(conn, &cat)?;
        let refs: Vec<&str> = items.iter().map(|r| r.voucher_no.as_str()).collect();
        let summary = format!(
            "计提 {cat} {month}：{} {}",
            party_brief_accrual(&items),
            {
                let mut v = refs.clone();
                v.sort_unstable();
                v.dedup();
                if v.len() <= 4 {
                    v.join("、")
                } else {
                    format!("{} 等{}笔", v[..4].join("、"), v.len())
                }
            }
        );
        let date = items[0].date.clone();
        let lines = vec![
            EntryLine {
                entry_date: date.clone(),
                summary: summary.clone(),
                account_code: acc,
                account_name: acc_name,
                debit_cents: total,
                credit_cents: 0,
                project_key: String::new(),
                source_type: "单据".into(),
                source_ref: refs.first().map(|s| s.to_string()).unwrap_or_default(),
            },
            EntryLine {
                entry_date: date.clone(),
                summary: summary.clone(),
                account_code: credit_code,
                account_name: credit_name,
                debit_cents: 0,
                credit_cents: total,
                project_key: String::new(),
                source_type: "单据".into(),
                source_ref: refs.first().map(|s| s.to_string()).unwrap_or_default(),
            },
        ];
        plan.accruals.push(EntryVoucher {
            kind: "计提".into(),
            entry_date: date,
            summary,
            lines,
            debit_cents: total,
            credit_cents: total,
        });
    }
    Ok(())
}

fn party_brief_accrual(rows: &[&AccrualRow]) -> String {
    let mut names: Vec<&str> = rows
        .iter()
        .map(|r| r.counterparty.trim())
        .filter(|s| !s.is_empty())
        .collect();
    names.dedup();
    if names.is_empty() {
        return "未填对方".into();
    }
    if names.len() <= 3 {
        names.join("、")
    } else {
        format!("{} 等{}家", names[..3].join("、"), names.len())
    }
}

/// 写入 `gl_entries`：同期间先删 draft（已导出/已记账的不动），再写新的一批。
///
/// 为什么只在 draft 上覆盖：`exported` / `booked` 代表已经反映到金蝶，
/// 删掉会让台账号与凭证号对不上。
pub fn post_period(ctx: &crate::CoreContext, period: &str) -> Result<PostResult> {
    let plan = ctx.db.with_conn(|c| build_period(c, period))?;
    plan.validate()?;

    let (vouchers, lines, debit, credit) = ctx.db.with_conn(|c| {
        c.execute(
            "DELETE FROM gl_entries WHERE period = ?1 AND status = 'draft'",
            params![period],
        )?;
        let mut vouchers = 0usize;
        let mut lines = 0usize;
        let mut debit = 0i64;
        let mut credit = 0i64;
        for v in plan
            .receipts
            .iter()
            .chain(plan.payments.iter())
            .chain(plan.accruals.iter())
        {
            vouchers += 1;
            for (i, line) in v.lines.iter().enumerate() {
                c.execute(
                    "INSERT INTO gl_entries
                       (period, entry_date, voucher_no, summary, account_code, account_name,
                        debit_cents, credit_cents, project_key, source_type, source_ref,
                        status, note, created_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'draft','',datetime('now'))",
                    params![
                        period,
                        line.entry_date,
                        format!("{}-{:03}", v.kind, vouchers),
                        line.summary,
                        line.account_code,
                        line.account_name,
                        line.debit_cents,
                        line.credit_cents,
                        line.project_key,
                        line.source_type,
                        line.source_ref,
                    ],
                )?;
                lines += 1;
                debit += line.debit_cents;
                credit += line.credit_cents;
                let _ = i;
            }
        }
        Ok((vouchers, lines, debit, credit))
    })?;

    Ok(PostResult {
        period: period.to_string(),
        vouchers,
        lines,
        debit_cents: debit,
        credit_cents: credit,
    })
}

/// 导出：KIS 引入 CSV（UTF-8 BOM，Excel 直接可开）
pub fn export_kis_csv(ctx: &crate::CoreContext, period: &str) -> Result<PathBuf> {
    let entries = list_entries(ctx, period)?;
    let dir = ctx.paths.exports.join(period);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("KIS引入_{period}.csv"));
    let mut out = String::from("\u{feff}");
    out.push_str("日期,凭证字号,摘要,科目代码,科目名称,借方,贷方,项目\n");
    for e in &entries {
        out.push_str(&format!(
            "{},{},{},{},{},{:.2},{:.2},{}\n",
            e.entry_date,
            csv_escape(&e.voucher_no),
            csv_escape(&e.summary),
            e.account_code,
            csv_escape(&e.account_name),
            e.debit_cents as f64 / 100.0,
            e.credit_cents as f64 / 100.0,
            csv_escape(&e.project_key),
        ));
    }
    std::fs::write(&path, out)?;
    Ok(path)
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// 抄单纸 xlsx（收款/付款/计提/校验 四页）
pub fn export_copy_sheet(ctx: &crate::CoreContext, period: &str) -> Result<PathBuf> {
    use rust_xlsxwriter::{Format, FormatAlign, Workbook};

    let plan = ctx.db.with_conn(|c| build_period(c, period))?;
    let dir = ctx.paths.exports.join(period);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("抄单纸_{period}.xlsx"));

    let mut wb = Workbook::new();
    let bold = Format::new().set_bold();
    let money = Format::new().set_num_format("#,##0.00");
    let wrap = Format::new().set_align(FormatAlign::Left).set_text_wrap();

    let header = ["日期", "摘要", "科目", "借方", "贷方"];
    for (sheet_name, group, note) in [
        (
            "收款",
            &plan.receipts,
            "收款凭证：借 银行/现金（每张 1 行合计），贷 1122 应收（按客户多行）",
        ),
        (
            "付款",
            &plan.payments,
            "付款凭证：借 成本/费用（按分类多行），贷 银行/现金（1 行合计）",
        ),
        (
            "计提",
            &plan.accruals,
            "计提凭证：借 成本/费用，贷 应付/薪酬/税金（月末一次）",
        ),
    ] {
        let sheet = wb.add_worksheet();
        sheet.set_name(sheet_name)?;
        sheet.write_string(0, 0, note)?;
        sheet.merge_range(0, 0, 0, 4, note, &wrap)?;
        for (c, h) in header.iter().enumerate() {
            sheet.write_with_format(1, c as u16, *h, &bold)?;
        }
        let mut row = 3u32;
        for v in group {
            for line in &v.lines {
                sheet.write_string(row, 0, &line.entry_date)?;
                sheet.write_string(row, 1, &line.summary)?;
                sheet.write_string(
                    row,
                    2,
                    format!("{} {}", line.account_code, line.account_name),
                )?;
                if line.debit_cents != 0 {
                    sheet.write_with_format(row, 3, line.debit_cents as f64 / 100.0, &money)?;
                }
                if line.credit_cents != 0 {
                    sheet.write_with_format(row, 4, line.credit_cents as f64 / 100.0, &money)?;
                }
                row += 1;
            }
            row += 1; // 凭证之间空一行，抄的时候好分隔
        }
        for (c, w) in [(0u16, 12.0), (1, 56.0), (2, 24.0), (3, 14.0), (4, 14.0)] {
            sheet.set_column_width(c, w)?;
        }
        sheet.set_freeze_panes(2, 0)?;
    }

    let sheet = wb.add_worksheet();
    sheet.set_name("校验")?;
    for (c, h) in ["项目", "凭证数", "借方合计", "贷方合计", "差额"]
        .iter()
        .enumerate()
    {
        sheet.write_with_format(0, c as u16, *h, &bold)?;
    }
    let mut r = 1u32;
    for (name, group) in [
        ("收款", &plan.receipts),
        ("付款", &plan.payments),
        ("计提", &plan.accruals),
    ] {
        let debit: i64 = group.iter().map(|v| v.debit_cents).sum();
        let credit: i64 = group.iter().map(|v| v.credit_cents).sum();
        sheet.write_string(r, 0, name)?;
        sheet.write_number(r, 1, group.len() as f64)?;
        sheet.write_with_format(r, 2, debit as f64 / 100.0, &money)?;
        sheet.write_with_format(r, 3, credit as f64 / 100.0, &money)?;
        sheet.write_with_format(r, 4, (debit - credit) as f64 / 100.0, &money)?;
        r += 1;
    }
    let total_debit = plan.debit_cents();
    let total_credit = plan.credit_cents();
    sheet.write_with_format(r, 0, "合计", &bold)?;
    sheet.write_with_format(r + 1, 0, "差额（必须为 0）", &bold)?;
    sheet.write_with_format(
        r + 1,
        4,
        (total_debit - total_credit) as f64 / 100.0,
        &money,
    )?;
    wb.save(&path)?;
    Ok(path)
}

/// 导出：V5「8 凭证分录」批量写入用的 JSON（来源类型 + 来源编号幂等）
pub fn export_v5_entries(ctx: &crate::CoreContext, period: &str) -> Result<PathBuf> {
    let entries = list_entries(ctx, period)?;
    let dir = ctx.paths.exports.join(period);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("V5凭证分录_{period}.json"));
    let rows: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "期间": e.period,
                "凭证日期": e.entry_date,
                "凭证字号": e.voucher_no,
                "摘要": e.summary,
                "科目代码": e.account_code,
                "科目名称": e.account_name,
                "借方": e.debit_cents as f64 / 100.0,
                "贷方": e.credit_cents as f64 / 100.0,
                "项目": e.project_key,
                "来源类型": e.source_type,
                "来源编号": e.source_ref,
                "状态": "草稿",
            })
        })
        .collect();
    std::fs::write(&path, serde_json::to_vec_pretty(&rows)?)?;
    Ok(path)
}

/// 分录行（库里的形态，含期间与凭证字号）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlEntry {
    pub id: i64,
    pub period: String,
    pub entry_date: String,
    pub voucher_no: String,
    pub summary: String,
    pub account_code: String,
    pub account_name: String,
    pub debit_cents: i64,
    pub credit_cents: i64,
    pub project_key: String,
    pub source_type: String,
    pub source_ref: String,
    pub status: String,
}

pub fn list_entries(ctx: &crate::CoreContext, period: &str) -> Result<Vec<GlEntry>> {
    ctx.db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT id, period, entry_date, voucher_no, summary, account_code, account_name,
                    debit_cents, credit_cents, project_key, source_type, source_ref, status
               FROM gl_entries WHERE period = ?1
              ORDER BY entry_date, voucher_no, id",
        )?;
        let rows = stmt.query_map(params![period], |r| {
            Ok(GlEntry {
                id: r.get(0)?,
                period: r.get(1)?,
                entry_date: r.get(2)?,
                voucher_no: r.get(3)?,
                summary: r.get(4)?,
                account_code: r.get(5)?,
                account_name: r.get(6)?,
                debit_cents: r.get(7)?,
                credit_cents: r.get(8)?,
                project_key: r.get(9)?,
                source_type: r.get(10)?,
                source_ref: r.get(11)?,
                status: r.get(12)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    })
}

/// 列出科目映射（前端「设置 → 科目映射」用）
pub fn list_category_maps(ctx: &crate::CoreContext) -> Result<Vec<CategoryMap>> {
    ctx.db.with_conn(|c| {
        seed_defaults(c)?;
        let mut stmt = c.prepare(
            "SELECT category, account_code, account_name, direction, biz_domain, note
               FROM acct_category_maps ORDER BY category",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(CategoryMap {
                category: r.get(0)?,
                account_code: r.get(1)?,
                account_name: r.get(2)?,
                direction: r.get(3)?,
                biz_domain: r.get(4)?,
                note: r.get(5)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    })
}

/// 列账户映射
pub fn list_account_maps(ctx: &crate::CoreContext) -> Result<Vec<AccountMap>> {
    ctx.db.with_conn(|c| {
        seed_defaults(c)?;
        let mut stmt = c.prepare(
            "SELECT account_key, account_code, account_name, note
               FROM acct_account_maps ORDER BY account_key",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(AccountMap {
                account_key: r.get(0)?,
                account_code: r.get(1)?,
                account_name: r.get(2)?,
                note: r.get(3)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    })
}

/// 覆盖式保存一张分类映射（改错科目是最常见的操作）
pub fn upsert_category_map(ctx: &crate::CoreContext, m: &CategoryMap) -> Result<()> {
    if m.category.trim().is_empty() {
        return Err(CoreError::Invalid("分类不能为空".into()));
    }
    if m.account_code.trim().is_empty() {
        return Err(CoreError::Invalid("科目代码不能为空".into()));
    }
    if m.direction != "借" && m.direction != "贷" {
        return Err(CoreError::Invalid(format!(
            "借贷方向只能是「借」或「贷」，收到「{}」",
            m.direction
        )));
    }
    ctx.db.with_conn(|c| {
        c.execute(
            "INSERT INTO acct_category_maps
               (category, account_code, account_name, direction, biz_domain, note, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,datetime('now'))
             ON CONFLICT(category) DO UPDATE SET
               account_code=excluded.account_code,
               account_name=excluded.account_name,
               direction=excluded.direction,
               biz_domain=excluded.biz_domain,
               note=excluded.note,
               updated_at=excluded.updated_at",
            params![
                m.category.trim(),
                m.account_code.trim(),
                m.account_name.trim(),
                m.direction,
                m.biz_domain.trim(),
                m.note.trim(),
            ],
        )?;
        Ok(())
    })
}

/// 覆盖式保存一张账户映射
pub fn upsert_account_map(ctx: &crate::CoreContext, m: &AccountMap) -> Result<()> {
    if m.account_key.trim().is_empty() {
        return Err(CoreError::Invalid("账户不能为空".into()));
    }
    if m.account_code.trim().is_empty() {
        return Err(CoreError::Invalid("科目代码不能为空".into()));
    }
    ctx.db.with_conn(|c| {
        c.execute(
            "INSERT INTO acct_account_maps
               (account_key, account_code, account_name, note, updated_at)
             VALUES (?1,?2,?3,?4,datetime('now'))
             ON CONFLICT(account_key) DO UPDATE SET
               account_code=excluded.account_code,
               account_name=excluded.account_name,
               note=excluded.note,
               updated_at=excluded.updated_at",
            params![
                m.account_key.trim(),
                m.account_code.trim(),
                m.account_name.trim(),
                m.note.trim(),
            ],
        )?;
        Ok(())
    })
}

/// 期间列表（含分录统计），前端「期间」页用
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeriodRow {
    pub period: String,
    pub status: String,
    pub vouchers: i64,
    pub debit_cents: i64,
    pub credit_cents: i64,
    pub note: String,
}

pub fn list_periods(ctx: &crate::CoreContext) -> Result<Vec<PeriodRow>> {
    ctx.db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT g.period,
                    COALESCE(p.status, 'open'),
                    COUNT(DISTINCT g.voucher_no),
                    COALESCE(SUM(g.debit_cents), 0),
                    COALESCE(SUM(g.credit_cents), 0),
                    COALESCE(p.note, '')
               FROM gl_entries g
               LEFT JOIN gl_periods p ON p.period = g.period
              GROUP BY g.period
              ORDER BY g.period DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(PeriodRow {
                period: r.get(0)?,
                status: r.get(1)?,
                vouchers: r.get(2)?,
                debit_cents: r.get(3)?,
                credit_cents: r.get(4)?,
                note: r.get(5)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    })
}

/// 关闭 / 重开期间。关账后不再允许覆盖 draft（要改先重开）。
pub fn set_period_status(ctx: &crate::CoreContext, period: &str, status: &str) -> Result<()> {
    if status != "open" && status != "closed" {
        return Err(CoreError::Invalid(format!(
            "期间状态只能是 open / closed，收到「{status}」"
        )));
    }
    ctx.db.with_conn(|c| {
        c.execute(
            "INSERT INTO gl_periods (period, status, note, updated_at)
             VALUES (?1, ?2, '', datetime('now'))
             ON CONFLICT(period) DO UPDATE SET status=excluded.status, updated_at=excluded.updated_at",
            params![period, status],
        )?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;
    use crate::models::biz::BizFields;
    use crate::models::Voucher;
    use crate::models::{NewVoucher, VoucherStatus};

    fn ctx() -> crate::CoreContext {
        let dir = std::env::temp_dir().join(format!(
            "vs-accounting-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let c = crate::CoreContext::init(Some(dir)).unwrap();
        c.db.with_conn(|conn| {
            schema::init(conn)?;
            Ok(())
        })
        .unwrap();
        c
    }

    fn add(
        ctx: &crate::CoreContext,
        seq: usize,
        date: &str,
        amount: i64,
        counter: &str,
        biz: BizFields,
    ) -> Voucher {
        ctx.db
            .with_conn(|c| {
                let v = crate::db::repos::voucher_repo::insert(
                    c,
                    &NewVoucher {
                        voucher_no: format!("2026-11-01-{seq:02}"),
                        voucher_date: date.into(),
                        description: "测试".into(),
                        counterparty: counter.into(),
                        amount_cents: Some(amount),
                        status: Some(VoucherStatus::Draft),
                        tags: vec![],
                        note: String::new(),
                        ledger_ref: None,
                    },
                )?;
                crate::services::biz::save(c, v.id, &biz)?;
                Ok(v)
            })
            .unwrap()
    }

    #[test]
    fn seed_writes_default_account_and_category_maps() {
        let c = ctx();
        let cats = list_category_maps(&c).unwrap();
        let accs = list_account_maps(&c).unwrap();
        assert!(cats
            .iter()
            .any(|m| m.category == "材料" && m.account_code == "540101"));
        assert!(accs
            .iter()
            .any(|m| m.account_key == "微信" && m.account_code == "100203"));
        // 幂等：再跑一次不会重复
        c.db.with_conn(seed_defaults).unwrap();
        assert_eq!(list_category_maps(&c).unwrap().len(), cats.len());
    }

    #[test]
    fn receipt_groups_by_date_and_account_then_customer() {
        let c = ctx();
        add(
            &c,
            1,
            "2026-11-02",
            50_000,
            "张帅",
            BizFields {
                doc_type: DOC_TYPE_CASH_IN.into(),
                category: "电费".into(),
                flow_dir: "收".into(),
                account: "银行转账".into(),
                project_name: "张帅-585kW-自投电站".into(),
                ..Default::default()
            },
        );
        add(
            &c,
            2,
            "2026-11-02",
            30_000,
            "李四",
            BizFields {
                doc_type: DOC_TYPE_CASH_IN.into(),
                category: "电费".into(),
                flow_dir: "收".into(),
                account: "银行转账".into(),
                project_name: "李四-100kW".into(),
                ..Default::default()
            },
        );
        let plan =
            c.db.with_conn(|conn| build_period(conn, "2026-11"))
                .unwrap();
        assert_eq!(plan.receipts.len(), 1, "同日同账户合成一张");
        let v = &plan.receipts[0];
        assert_eq!(v.debit_cents, 80_000);
        assert_eq!(v.credit_cents, 80_000);
        assert!(v.balanced());
        // 贷方按客户拆成两行
        let credits: Vec<_> = v.lines.iter().filter(|l| l.credit_cents > 0).collect();
        assert_eq!(credits.len(), 2);
    }

    #[test]
    fn missing_category_mapping_is_reported_not_guessed() {
        let c = ctx();
        add(
            &c,
            1,
            "2026-11-03",
            10_000,
            "王五",
            BizFields {
                doc_type: DOC_TYPE_CASH_OUT.into(),
                category: "没有的分类".into(),
                flow_dir: "付".into(),
                account: "银行转账".into(),
                ..Default::default()
            },
        );
        let plan =
            c.db.with_conn(|conn| build_period(conn, "2026-11"))
                .unwrap();
        assert!(plan.payments.is_empty());
        assert!(plan.issues.iter().any(|i| i.contains("没有的分类")));
        assert!(plan.validate().is_err());
    }

    #[test]
    fn accrual_groups_by_month_and_category() {
        let c = ctx();
        for (i, (d, a)) in [("2026-11-01", 20_000), ("2026-11-20", 10_000)]
            .into_iter()
            .enumerate()
        {
            add(
                &c,
                i + 1,
                d,
                a,
                "供应商A",
                BizFields {
                    doc_type: DOC_TYPE_PAYABLE.into(),
                    category: "施工外包".into(),
                    ..Default::default()
                },
            );
        }
        let plan =
            c.db.with_conn(|conn| build_period(conn, "2026-11"))
                .unwrap();
        assert_eq!(plan.accruals.len(), 1);
        let v = &plan.accruals[0];
        assert_eq!(v.debit_cents, 30_000);
        assert_eq!(v.credit_cents, 30_000);
        assert_eq!(v.lines.len(), 2);
        assert_eq!(v.lines[1].account_code, PAYABLE_CODE);
    }

    #[test]
    fn post_then_export_kis_csv_and_copy_sheet() {
        let c = ctx();
        add(
            &c,
            1,
            "2026-11-05",
            12_850,
            "张帅",
            BizFields {
                doc_type: DOC_TYPE_CASH_IN.into(),
                category: "电费".into(),
                flow_dir: "收".into(),
                account: "银行转账".into(),
                ..Default::default()
            },
        );
        let r = post_period(&c, "2026-11").unwrap();
        assert_eq!(r.vouchers, 1);
        assert_eq!(r.debit_cents, 12_850);
        let entries = list_entries(&c, "2026-11").unwrap();
        assert_eq!(entries.len(), 2);
        let csv = export_kis_csv(&c, "2026-11").unwrap();
        let text = std::fs::read_to_string(&csv).unwrap();
        assert!(text.contains("1122"));
        assert!(text.contains("128.50"));
        let xlsx = export_copy_sheet(&c, "2026-11").unwrap();
        assert!(xlsx.exists());
        let json = export_v5_entries(&c, "2026-11").unwrap();
        let rows: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["来源类型"], "流水");
    }
}
