//! 汇总册命令（v0.26 M3）：图片册 / 发票册 / PDF 合并。

use voucher_core::services::pack::{self, PackKind, PackResult};

use crate::{core, ApiResult, CoreContext};

pub fn pack_build(ctx: &CoreContext, voucher_ids: Vec<i64>, kind: String) -> ApiResult<PackResult> {
    let k = PackKind::parse(&kind).map_err(crate::ApiError::from_core)?;
    core(pack::build(ctx, &voucher_ids, k))
}
