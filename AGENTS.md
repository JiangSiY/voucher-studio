# AGENTS.md — voucher-studio 仓库约定

凭证管理桌面工具（Tauri 2 + Rust + Vue 3）。业务核心在 `crates/voucher-core`（不依赖 Tauri），桌面壳在 `src-tauri`，前端在 `src/`。
规划文档：`docs/开发方案.md`；过程与证据档案：`docs/开发日志.md`。

## 关键命令

```bash
npm run typecheck && npm run lint && npm run test && npm run build   # 前端四门
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast -- --include-ignored            # 与 CI 同口径（含 #[ignore] 真跑）
```

**本机（Windows）跑 cargo 的必备环境**：编译/测试需要 OpenCV + LLVM + MSVC 工具链，先 `source ./activate-dev.ps1` 对应的环境变量（`OpenCV_DIR` / `OPENCV_INCLUDE_PATHS` / `OPENCV_LINK_PATHS` / `OPENCV_LINK_LIBS` / `LIBCLANG_PATH` / `INCLUDE` / `LIB`）。

> **必须显式指定 MSVC 链接器**：在 Git Bash 等环境里 `link.exe` 会被 coreutils 的同名工具抢占（`which link.exe` 指向 `/usr/bin/link.exe`），仅把 MSVC 目录前置到 PATH **不生效**。正确做法：
> `export CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER="C:/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/VC/Tools/MSVC/14.44.35207/bin/Hostx64/x64/link.exe"`
> 否则 `cargo test` 会在链接期失败——不要据此断言"需 CI 复核"。

## 文档纪律（三条）

1. **改动代码后**：`README.md` 与 `docs/开发方案.md` 中与之相关的数字/描述必须同步（尤其用例数、依赖版本、文件路径、功能开关）。规范类文档**以代码实况为准**直接改写；涉及历史决策处保留「（原 X，自 vY 起改为 Z）」形式，不抹演进痕迹。
2. **`docs/voucher-studio-health-check-*`、`-project-analysis-*`、`-fix-status-*` 等报告是时点快照**：**正文永不修改**。需要更新时只在文件头部追加「校正注记」，并在其中指向当前真源。
3. **用例数只有一个口径**：`#[test]` 静态计数（单元 = `crates/voucher-core/src`，集成 = `crates/voucher-core/tests`）。它同时出现在 `README.md` 测试命令段、`docs/验收基线/清单1-非GUI测试映射.md` 修订注记、`docs/开发日志.md` 即时条目——三处必须一致。

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
- 提交前确保：`git status` 无意外文件混入；上表四门 + Rust 三门通过（默认本地即可跑通，见上文链接器说明）。
- **推送前先 `git fetch`**，确认与 `origin/main` 的关系再推（历史上出现过"本地领先/落后各若干"的分叉情形）。
