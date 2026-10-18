# AGENTS.md — voucher-studio 仓库约定

凭证管理桌面工具（Tauri 2 + Rust + Vue 3）。业务核心在 `crates/voucher-core`（不依赖 Tauri），桌面壳在 `src-tauri`，前端在 `src/`。
规划文档：`docs/开发方案.md`；过程与证据档案：`docs/开发日志.md`。

## 关键命令

**预览验证一律用开发模式，不要打包**（2026-09-27 用户要求）：改完代码后让用户
`npm run tauri dev` 看效果（前端改动热更新生效；**Rust / `tauri.conf.json` / capabilities 改动需重启 dev**）。
只在发布点打包：版本号 bump、发 Release，或用户明确说"打包"——打包只走 `bash package-macos.sh`（见 README「构建」）。

```bash
npm run typecheck && npm run lint && npm run test && npm run build   # 前端四门
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast -- --include-ignored            # 与 CI 同口径（含 #[ignore] 真跑）
```

**本机（macOS）跑 cargo 的必备环境**：编译/测试需要 OpenCV 4.10 与 libclang，两者都不在系统默认搜索路径上，故每次新开 shell 先备好环境：

- `. "$HOME/.cargo/env"`（工具链 PATH；harness 的 shell 快照会覆盖 PATH，不 source 会 `cargo: command not found`）
- `export PKG_CONFIG_PATH="$HOME/.local/opencv-4.10/lib/pkgconfig"`（OpenCV 4.10 为源码编译，见 README；**不要**装 brew 的 opencv/opencv@4，版本不兼容会让 Rust 侧编译失败）
- libclang：`$HOME/lib/libclang.dylib` → Command Line Tools 的同名库（符号链接）。`DYLD_*` 会被系统剥离，`LIBCLANG_PATH` 无效，只能靠这个固定路径
- 需要判断"是不是环境问题"时：`pkg-config --modversion opencv4` 必须打印 `4.10.0`

## 文档纪律（三条）

1. **改动代码后**：`README.md` 与 `docs/开发方案.md` 中与之相关的数字/描述必须同步（尤其用例数、依赖版本、文件路径、功能开关）。规范类文档**以代码实况为准**直接改写；涉及历史决策处保留「（原 X，自 vY 起改为 Z）」形式，不抹演进痕迹。
2. **`docs/voucher-studio-*` 报告是时点快照**（`-health-check-*`、`-project-analysis-*`、`-fix-status-*`、`-status-audit-*`、`-audit-*` 五族）：**正文永不修改**。需要更新时只在文件头部追加「校正注记」，并在其中指向当前真源。审计报告统一命名 `docs/voucher-studio-audit-YYYY-MM-DD.md`（日期粒度，模板见 `docs/审计指南.md`）；自动核对：`bash scripts/audit.sh` 第 7.4 节按 glob 检查全部快照是否带头部注记。
3. **用例数只有一个口径**：Rust 用 `#[test]` 静态计数（单元 = `crates/voucher-core/src`，集成 = `crates/voucher-core/tests`）。它同时出现在 `README.md` 测试命令段、`docs/验收基线/清单1-非GUI测试映射.md` 修订注记、`docs/开发日志.md` 即时条目——三处必须一致。**前端 vitest 一律写「实测」**（`npm run test` 报的「文件数 + 用例数」；2026-10-18 起不再写 `it(` 静态计数：有动态生成用例，静态数会长期差 1）。自动核对：`bash scripts/audit.sh`（见 `docs/审计指南.md`）。

## 开发日志纪律（强制）

出现以下任一时机，**必须在 `docs/开发日志.md` 末尾（即时区）追加一条**，并同步更新文末「当前挂账」表：

- 版本号 bump；过门（G1–G4）；完成一次审计/体检；CI 修复或 CI 规则变更；任何"改动源码"的批次收口。

条目固定 7 字段：**日期 / 版本·门 / 触发 / 关键提交（sha + 标题）/ 门禁证据（命令 + 结果 + run 号或日志路径）/ 产出物 / 遗留与挂账**。
历史条目为重建内容时必须标注「重建」与依据来源（tag / 提交 / 报告），不得伪装成当时的即时记录。

## 验收证据归档

真机走查（G3/G4）的截图与脚本**必须入库**，不要放在仓库外的临时目录：

- 目录：`docs/验收基线/证据/<日期>-G3-vX.Y.Z/`
- 走查结论写入独立记录表并引用这些路径；**不要**使用 `file:///` 绝对路径链接到仓库外（仓库外目录随时会被清理——`.archive-g3-step/` 等 7 个历史走查目录已全部丢失，导致 G1–G3 的证据链断裂）。

## 提交规范

- 中文 conventional commits：`feat(scope):` / `fix(scope):` / `docs:` / `test:` / `refactor:` / `chore:` / `ci:`；scope 常用 `core` / `frontend` / `tauri` / `test` / `ci`。
- 一个逻辑变更一笔提交；文档与代码**分开提交**。
- 提交前确保：`git status` 无意外文件混入；上表四门 + Rust 三门 + 两道 grep 门禁通过（本地环境见上文 macOS 段）。门禁的**唯一真源**是 `scripts/guards.sh`（`bash scripts/guards.sh layering` / `platform`，`--selftest` 可自证模式有效；CI 与 `scripts/audit.sh` 都调它，不得再各自内联一份——2026-09-27 审计发现两处定义已漂移）。
- **推送前先 `git fetch`**，确认与 `origin/main` 的关系再推（历史上出现过"本地领先/落后各若干"的分叉情形）。
