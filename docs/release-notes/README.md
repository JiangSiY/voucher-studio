# Release 说明（`docs/release-notes/`）

每次发版前在这里放一份 `<版本号>.md`，作为 GitHub Release 的正文；`scripts/release.sh` 会**强制检查**它存在
（缺了直接拒绝发版），保证 Release 说明不会被忘掉。

约定：
- 文件名 = 纯版本号，如 `0.22.3.md`（不带 `v`）；
- 内容面向用户：本版做了什么、修了什么、安装与校验方法（参考历史 Release 正文）；
- **必须含「安装」一节**：未公证说明 + 首次打开方法（右键→打开 / `xattr -dr com.apple.quarantine`）+ 自包含自检命令
  （`otool -L` 不应出现本机绝对路径）——见 README「安装（分发给别人时）」；
- 发布后**不要删**，它们是各版本的正式说明存档。

## 自动更新资产（2026-10-18 起）

`scripts/release.sh` 除 dmg 与 `sha256sums.txt` 外，还会上传：

- `Voucher.Studio_<版本>_arm64.app.tar.gz` + `.sig`：更新包与 minisign 签名（在 `package-macos.sh`
  修复 dylib **之后**重打，不能用 bundler 预生成的）；
- `latest.json`：更新源索引（`version` / `notes` / `pub_date` / `platforms.darwin-aarch64.{signature,url}`）。

这些资产同样**发布后不删**——客户端会按版本号回看历史 `latest.json` 的 URL 布局；
删掉某个版本的 `app.tar.gz` 会让还停在旧版的用户更新 404。
