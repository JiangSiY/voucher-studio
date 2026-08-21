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
    let first = parts.first().unwrap();
    let first = first.trim_start_matches('0');
    let mut s = String::from(if first.is_empty() { "0" } else { first });
    for p in parts.iter().skip(1) {
        s.push(',');
        s.push_str(p);
    }
    s
}
