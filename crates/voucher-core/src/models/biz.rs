//! 台账对齐字段（WPS「项目管理台账 V5」对齐，v0.26）
//!
//! 为什么单独一个模型而不是塞进 [`Voucher`](crate::models::Voucher)：
//! `Voucher` 是「档案」口径（编号/日期/摘要/金额），本模型是「台账单据」口径
//! （类型/分类/业务域/票据/项目/对方/归档名）。二者生命周期不同——台账字段会随
//! 复核逐步补齐，档案字段从导入那刻就定了。分开存放（`voucher_ext` 1:1 表），
//! 列表查询不必为了一个分类字段去 JOIN 二十列。
//!
//! 字段值域与 V5 对齐（见 `docs/开发方案.md` 的「台账对齐」一节）：
//! - `doc_type`：`应收` / `应付` / `仓库` / `资金收` / `资金付` / `电费` / `项目`
//! - `flow_dir`：`收` / `付`（仅资金类单据有值）
//! - `ticket_type`：`增值税专票` / `增值税普通发票` / `普票` / `收据` / `无票`
//! - `biz_domain`：`项目收入` / `往来款` / `项目成本` / `运维` / `固定资产` / `工资社保` / `公司费用`

use serde::{Deserialize, Serialize};

/// 单据类型：应收（对客户的债权）
pub const DOC_TYPE_RECEIVABLE: &str = "应收";
/// 单据类型：应付（对供应商/施工队的债务）
pub const DOC_TYPE_PAYABLE: &str = "应付";
/// 单据类型：仓库（材料出入库）
pub const DOC_TYPE_WAREHOUSE: &str = "仓库";
/// 单据类型：资金收（银行/微信/支付宝/现金收款）
pub const DOC_TYPE_CASH_IN: &str = "资金收";
/// 单据类型：资金付
pub const DOC_TYPE_CASH_OUT: &str = "资金付";

/// 票据类型：增值税专用发票（唯一强制要求发票号的类型）
pub const TICKET_SPECIAL_VAT: &str = "增值税专票";

/// 台账对齐字段（全部可空字符串，未填写 = `""`，便于前端直接绑定输入框）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BizFields {
    /// 单据类型（应收/应付/仓库/资金收/资金付/电费/项目）
    pub doc_type: String,
    /// 分类（V5「3 单据·分类」21 个值之一，如 工商业安装款/电费/材料/施工外包）
    pub category: String,
    /// 业务域（V5「3 单据·业务域」：项目收入/往来款/项目成本/运维/固定资产/工资社保/公司费用）
    pub biz_domain: String,
    /// 收支方向（`收` / `付`；仅资金类）
    pub flow_dir: String,
    /// 结算方式（V5 的「账户」字段：银行转账/微信/支付宝/承兑/现金/其他）
    pub account: String,
    /// 票据类型（增值税专票/增值税普通发票/普票/收据/无票）
    pub ticket_type: String,
    /// 发票号（专票必填）
    pub invoice_no: String,
    /// 开票日期 YYYY-MM-DD
    pub invoice_date: Option<String>,
    /// 项目主数据键（V5「2 项目」的行标识；台账导入时写入）
    pub project_key: String,
    /// 项目名称（冗余，供归档名与列表展示；台账里形如 `张帅-585kW-自投电站`）
    pub project_name: String,
    /// 对方主数据键（V5「1 往来对象」的行标识）
    pub counterparty_key: String,
    /// 物料名称（仓库单必填）
    pub material_name: String,
    /// 数量（仓库单）
    pub material_qty: Option<f64>,
    /// 单位（仓库单）
    pub material_unit: String,
    /// 归档名（复刻 V5「附件名称」公式，重命名磁盘文件与列表展示同用）
    pub archive_name: String,
    /// 来源单据编号（V5 单据号/流水序号，回写台账时定位行）
    pub source_doc_ref: String,
    /// 金蝶凭证字（如 `记-0012`；回写后填）
    pub gl_voucher_no: String,
    /// 金蝶凭证批次（导出时的批次号）
    pub gl_batch: String,
    /// 金蝶凭证日期 YYYY-MM-DD
    pub gl_voucher_date: Option<String>,
    /// 人工确认已入账时间（ISO8601）
    pub gl_booked_at: Option<String>,
}

impl BizFields {
    /// 是否全是默认值（用于「从未补过台账字段」的判断，避免为老凭证写空扩展行）
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// 单据类型是否属于资金类（收/付）
    pub fn is_cash(&self) -> bool {
        matches!(self.doc_type.as_str(), DOC_TYPE_CASH_IN | DOC_TYPE_CASH_OUT)
    }
}
