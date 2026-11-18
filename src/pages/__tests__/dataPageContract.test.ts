// @vitest-environment node
/**
 * 数据页契约（v0.27 B2）：真实数据 + 行内编辑 + 凭证必填新建。
 *
 * 不变量：
 * - 数据页不写静态行，统一走 `list_vouchers`；
 * - 行展开复用 `VoucherEditor`，不另造编辑面；
 * - 新建条目先填字段、再挂凭证，凭证未选时保存按钮禁用；
 * - 数据页是全站唯一的新建/导入/打印导出入口（按钮各一处）。
 */
import { describe, expect, it } from "vitest";
import { repoFile } from "../../__tests__/helpers/designGuard";

const dataSrc = repoFile("src/pages/DataPage.vue");
const dialogSrc = repoFile("src/components/dialogs/NewRecordDialog.vue");

describe("数据页真实化", () => {
  it("用 list_vouchers 取数，保留视图 chips 与移动卡片", () => {
    expect(dataSrc).toContain("api.listVouchers");
    expect(dataSrc).toContain("VoucherEditor");
    expect(dataSrc).toContain("data-cards");
    expect(dataSrc).not.toContain("const rows: Row[] = [");
  });

  it("新建/导入/打印导出各只有一个入口", () => {
    expect(dataSrc.match(/>\s*新建条目\s*</g)?.length).toBe(1);
    expect(dataSrc.match(/>\s*导入\s*</g)?.length).toBe(1);
    expect(dataSrc.match(/>\s*打印导出\s*</g)?.length).toBe(1);
    expect(dataSrc).toContain("pickAndImport");
    expect(dataSrc).toContain("open('export')");
  });
});

describe("新建条目：数据为主、凭证必填", () => {
  it("字段在前、凭证必填，未选凭证不能保存", () => {
    expect(dialogSrc).toContain("createRecordWithAttachment");
    expect(dialogSrc).toContain("uploadStage");
    expect(dialogSrc).toContain("pickImagePaths");
    expect(dialogSrc).toContain("凭证图片");
    expect(dialogSrc).toContain("不能入账 / 导出");
    expect(dialogSrc).toMatch(/canSave[\s\S]*paths\.value\.length > 0/);
  });
});
