/**
 * 台账字段的常量与默认值。
 *
 * 为什么单独成文件（而不是放 `api/client.ts`）：组件单测会 `vi.mock("api/client")`，
 * 只给 `api` 对象——从 client 取常量的组件在测试里会拿到 undefined，
 * 表现为「setup function 抛错」。常量放这里，测试与运行时都稳定。
 */

/** 单据类型（与后端 `crates/voucher-core/src/services/biz.rs` 的 DOC_TYPES 同源） */
export const DOC_TYPES = ["应收", "应付", "仓库", "资金收", "资金付", "电费", "项目"] as const;
export type DocType = (typeof DOC_TYPES)[number];

/** 结算方式（V5「4 资金流水」的账户字段值域） */
export const ACCOUNT_KEYS = ["银行转账", "微信", "支付宝", "承兑", "现金", "其他"] as const;

/** 台账对齐字段（`voucher_ext`）：一张凭证的「入账要素」 */
export interface BizFields {
  docType: string;
  category: string;
  bizDomain: string;
  flowDir: string;
  /** 结算方式（V5 账户） */
  account: string;
  ticketType: string;
  invoiceNo: string;
  invoiceDate: string | null;
  projectKey: string;
  projectName: string;
  counterpartyKey: string;
  materialName: string;
  materialQty: number | null;
  materialUnit: string;
  /** 归档名（按 V5 公式算出，落盘用） */
  archiveName: string;
  /** 来源单据号 / 流水序号（台账 AutoNumber 的镜像） */
  sourceDocRef: string;
  glVoucherNo: string;
  glBatch: string;
  glVoucherDate: string | null;
  glBookedAt: string | null;
}

export const EMPTY_BIZ_FIELDS: BizFields = {
  docType: "",
  category: "",
  bizDomain: "",
  flowDir: "",
  account: "",
  ticketType: "",
  invoiceNo: "",
  invoiceDate: null,
  projectKey: "",
  projectName: "",
  counterpartyKey: "",
  materialName: "",
  materialQty: null,
  materialUnit: "",
  archiveName: "",
  sourceDocRef: "",
  glVoucherNo: "",
  glBatch: "",
  glVoucherDate: null,
  glBookedAt: null,
};
