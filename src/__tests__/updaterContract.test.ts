// @vitest-environment node
/**
 * 自动更新契约（挂账 #39，2026-10-18 用户定「自动更新要做」，托盘/开机自启不做）。
 *
 * 更新链路的四个必要环节缺一个就悄悄失效：
 * ① `tauri.conf.json` 里必须有 `plugins.updater.endpoints` 与 `pubkey`（否则 `check()` 直接报错）；
 * ② 权限集里必须有 `updater:default`（否则前端调用被 IPC 拒绝）；
 * ③ 设置页要有「检查更新」入口（否则功能等于不可见）；
 * ④ 发版脚本必须重新打 `.app.tar.gz`、签名并写 `latest.json`——**不能**用 bundler 在
 *    `package-macos.sh` 之前生成的更新包（那时 OpenCV dylib 还没进 Frameworks，更新后必崩）。
 */
import { describe, expect, it } from "vitest";
import { repoFile } from "./helpers/designGuard";

const conf = JSON.parse(repoFile("src-tauri/tauri.conf.json"));
const caps = JSON.parse(repoFile("src-tauri/capabilities/default.json"));
const releaseSh = repoFile("scripts/release.sh");
const generalTab = repoFile("src/components/settings/GeneralTab.vue");
const pkg = JSON.parse(repoFile("package.json"));

describe("自动更新", () => {
  it("tauri.conf.json 配了更新源与公钥", () => {
    const updater = conf.plugins?.updater;
    expect(updater, "缺少 plugins.updater").toBeTruthy();
    expect(updater.endpoints[0]).toContain("releases/latest/download/latest.json");
    // 公钥是 minisign 公钥（base64），长度足够即可
    expect(updater.pubkey.length).toBeGreaterThan(100);
  });

  it("开启自动更新的应用不能自己再让 bundler 出更新包（必须由 release.sh 在打包后重做）", () => {
    expect(conf.bundle.createUpdaterArtifacts).toBe(false);
  });

  it("权限集给了 updater:default", () => {
    expect(caps.permissions).toContain("updater:default");
  });

  it("设置 → 通用 有检查更新入口，并接入 @tauri-apps/plugin-updater", () => {
    expect(generalTab).toContain("检查更新");
    expect(generalTab).toContain("@tauri-apps/plugin-updater");
    expect(generalTab).toContain("downloadAndInstall");
    expect(pkg.dependencies["@tauri-apps/plugin-updater"]).toBeTruthy();
  });

  it("发版脚本在 package-macos.sh 之后重打更新包 + 签名 + 写 latest.json", () => {
    const iPack = releaseSh.indexOf("bash package-macos.sh");
    const iTar = releaseSh.indexOf('-czf "${UPD_TGZ}"'); // 真正的打包动作（注释里也提到 .app.tar.gz，不能用它定位）
    expect(iPack).toBeGreaterThan(-1);
    expect(iTar).toBeGreaterThan(iPack); // 顺序即正确性：打包修复之后才做更新包
    expect(releaseSh).toContain("tauri signer sign");
    expect(releaseSh).toContain("latest.json");
    expect(releaseSh).toContain("darwin-aarch64");
    // 上传清单里必须带更新资产，漏传等于更新源 404
    expect(releaseSh).toContain("app.tar.gz.sig");
    // 更新包不能带 AppleDouble：macOS 的 bsdtar 默认把扩展属性写成 `._Xxx` 条目，
    // 而 tauri updater 解包是 `path().skip(1)`，第一条 `._App.app` 会变成空路径 → EISDIR，
    // 用户侧表现为「检查更新失败：failed to unpack `._Voucher Studio.app`」（2026-10-27 真机踩到）
    expect(releaseSh).toContain("COPYFILE_DISABLE=1");
    expect(releaseSh).toContain("--no-mac-metadata");
    expect(releaseSh).toContain("--no-xattrs");
    // 并且发版脚本自己也要守住：包内成员形态不合法时直接判红
    expect(releaseSh).toContain("AppleDouble");
  });
});
