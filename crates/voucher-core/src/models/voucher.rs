//! 凭证模型（对应 v4.3 的 `models/voucher.py`）

use serde::{Deserialize, Serialize};

use super::enums::VoucherStatus;

/// 凭证（记账原始凭证的电子档案：编号 + 日期 + 摘要 + 金额 + 附件）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Voucher {
    pub id: i64,
    /// 凭证编号，全局唯一，如 20260819-001
    pub voucher_no: String,
    /// 凭证日期 YYYY-MM-DD
    pub voucher_date: String,
    /// 摘要/描述
    pub description: String,
    /// 对方账户（收款方/商户，OCR 可自动抽取）
    pub counterparty: String,
    /// 金额（分；None = 未填写）
    pub amount_cents: Option<i64>,
    pub status: VoucherStatus,
    /// 标签（JSON 数组存储）
    pub tags: Vec<String>,
    /// 备注
    pub note: String,
    pub created_at: String,
    pub updated_at: String,
}

/// 新建凭证入参
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NewVoucher {
    /// 留空则由命名服务自动生成
    pub voucher_no: String,
    /// 留空则取今天
    pub voucher_date: String,
    pub description: String,
    #[serde(default)]
    pub counterparty: String,
    pub amount_cents: Option<i64>,
    pub status: Option<VoucherStatus>,
    pub tags: Vec<String>,
    pub note: String,
}

/// 更新凭证入参（None = 不修改该字段）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VoucherUpdate {
    pub voucher_no: Option<String>,
    pub voucher_date: Option<String>,
    pub description: Option<String>,
    pub counterparty: Option<String>,
    pub amount_cents: Option<i64>,
    /// 显式置空金额的开关（serde 默认 false）
    pub clear_amount: bool,
    pub status: Option<VoucherStatus>,
    pub tags: Option<Vec<String>>,
    pub note: Option<String>,
}

impl Voucher {
    /// 金额的显示文本：12345 → "123.45"
    pub fn amount_label(&self) -> String {
        match self.amount_cents {
            None => String::new(),
            Some(cents) => format_money(cents),
        }
    }
}

/// 分 → 无千分位十进制文本（CSV / 对账导出等**供机器解析**的数值口径："-1234.56"）。
///
/// 负数必须走 `unsigned_abs` 并显式补负号：`cents / 100` 与 `cents % 100` 各自向零截断，
/// 直接拼接会得到 `0.-50`（-50 分）这类畸形金额，用户把 CSV 导入会计软件即得错数
/// （2026-09-23 审计 B5；彼时 `table.rs` 与 `reconcile.rs` 各有一份手写除法）。
pub fn format_money_plain(cents: i64) -> String {
    let abs = cents.unsigned_abs();
    if cents < 0 {
        format!("-{}.{:02}", abs / 100, abs % 100)
    } else {
        format!("{}.{:02}", abs / 100, abs % 100)
    }
}

/// 分 → "1,234.56" 形式
pub fn format_money(cents: i64) -> String {
    let negative = cents < 0;
    let abs = cents.unsigned_abs();
    let yuan = abs / 100;
    let fen = abs % 100;
    let yuan_str = thousands(yuan);
    if negative {
        format!("-{}.{:02}", yuan_str, fen)
    } else {
        format!("{}.{:02}", yuan_str, fen)
    }
}

fn thousands(mut n: u64) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let mut parts = Vec::new();
    while n > 0 {
        parts.push(format!("{:03}", n % 1000));
        n /= 1000;
    }
    parts.reverse();
    // `n == 0` 已在上面早退，故 parts 必非空；用 let-else 消掉 panic 路径（审计：生产 unwrap 清零）
    let Some(first_raw) = parts.first() else {
        return "0".to_string();
    };
    let first = first_raw.trim_start_matches('0');
    let mut s = String::from(if first.is_empty() { "0" } else { first });
    for p in parts.iter().skip(1) {
        s.push(',');
        s.push_str(p);
    }
    s
}
