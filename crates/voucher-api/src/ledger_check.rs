//! 台账对账命令（M4）：台账 CSV ↔ 凭证库比对与差异导出

use std::path::PathBuf;

use serde_json::Value;

use voucher_core::services::ledger_check::{self, CheckReport};

use crate::{get, to_value, ApiResult, CoreContext};

pub fn ledger_check(ctx: &CoreContext, args: Value) -> ApiResult<Value> {
    let csv_path: String = get(&args, "csvPath")?;
    let tolerance: Option<i64> = get(&args, "toleranceCents").unwrap_or(None);
    let report = ledger_check::check(ctx, &PathBuf::from(&csv_path), tolerance.unwrap_or(0))?;
    to_value(report)
}

pub fn ledger_check_export(ctx: &CoreContext, args: Value) -> ApiResult<Value> {
    let csv_path: String = get(&args, "csvPath")?;
    let tolerance: Option<i64> = get(&args, "toleranceCents").unwrap_or(None);
    let report: CheckReport =
        ledger_check::check(ctx, &PathBuf::from(&csv_path), tolerance.unwrap_or(0))?;
    let path = ledger_check::export_issues(ctx, &report)?;
    Ok(Value::String(path))
}
