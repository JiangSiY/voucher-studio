#!/usr/bin/env bash
# 审计自动检查（对应 docs/审计指南.md §3.1）
#
# 只做"能自动且稳定"的检查；判断类（口径语义等价、a11y 实感、依赖策略、性能）留给人工。
# 输出分节摘要：[OK] / [WARN] / [FAIL] + 计数。默认始终退出 0（便于本地看全貌）；
# 加 `--strict` 时出现 FAIL 则退出码 1（便于后续接 CI）。
#
# 用法：bash scripts/audit.sh [--strict] [--fast]
#   --fast  跳过门禁（前端四门 + Rust 三门），只跑静态检查（本地迭代用）
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"

STRICT=0
FAST=0
for arg in "$@"; do
  case "${arg}" in
    --strict) STRICT=1 ;;
    --fast) FAST=1 ;;
    *) echo "未知参数：${arg}（可用：--strict / --fast）" >&2; exit 2 ;;
  esac
done

FAILS=0
WARNS=0
ok()   { printf '  [OK]   %s\n' "$1"; }
warn() { printf '  [WARN] %s\n' "$1"; WARNS=$((WARNS + 1)); }
fail() { printf '  [FAIL] %s\n' "$1"; FAILS=$((FAILS + 1)); }
section() { printf '\n=== %s ===\n' "$1"; }

# ---------- 1. 门禁复用 ----------
if [ "${FAST}" -eq 0 ]; then
  section "1. 门禁（前端四门 + Rust 三门 + 两道 grep 守卫）"
  if npm run typecheck >/dev/null 2>&1 && npm run lint >/dev/null 2>&1 \
     && npm run test >/dev/null 2>&1 && npm run build >/dev/null 2>&1; then
    ok "前端四门通过"
  else
    fail "前端四门未通过（分别跑 npm run typecheck / lint / test / build 看详情）"
  fi
  # shellcheck disable=SC1090
  [ -f "${HOME}/.cargo/env" ] && . "${HOME}/.cargo/env"
  if cargo fmt --all --check >/dev/null 2>&1; then ok "cargo fmt --check"; else fail "cargo fmt --check 失败"; fi
  if cargo clippy --workspace --all-targets -- -D warnings >/dev/null 2>&1; then
    ok "cargo clippy -D warnings"
  else
    fail "cargo clippy -D warnings 未通过"
  fi
  if cargo test --workspace --no-fail-fast -- --include-ignored >/dev/null 2>&1; then
    ok "cargo test --workspace（含 #[ignore]）"
  else
    fail "cargo test 未通过"
  fi
else
  section "1. 门禁（--fast：已跳过）"
  warn "跳过门禁（--fast）"
fi

section "2. 两道 grep 门禁"
if grep -rnE 'db::repos|\.with_(conn|tx)\(' src-tauri/src >/dev/null 2>&1; then
  fail "越层门禁：命令层出现 db::repos / 裸 with_conn(tx)"
else
  ok "越层门禁（命令层不直连仓储）"
fi
if grep -rnE '#\[cfg\(windows\)\]|cfg\(target_os = "windows"\)' crates/*/src crates/*/tests src-tauri/src >/dev/null 2>&1; then
  fail "平台门禁：出现 Windows 条件编译"
else
  ok "平台范围门禁（仅 macOS）"
fi
if find . -name '*.ps1' -not -path './node_modules/*' -not -path './target/*' | grep -q .; then
  fail "平台门禁：仓库内出现 .ps1"
else
  ok "平台门禁（无 Windows 脚本）"
fi

# ---------- 3. 用例数三处口径 ----------
section "3. 用例数三处口径（README / 清单1 / 开发日志）"
# Rust：单元 = crates/voucher-core/src 里的 #[test]；集成 = crates/voucher-core/tests
RUST_UNIT=$(grep -rhoE '#\[test\]' crates/voucher-core/src | wc -l | tr -d ' ')
RUST_INTEG=$(grep -rhoE '#\[test\]' crates/voucher-core/tests | wc -l | tr -d ' ')
RUST_TAURI=$(grep -rhoE '#\[test\]' src-tauri/src | wc -l | tr -d ' ')
RUST_TOTAL=$((RUST_UNIT + RUST_INTEG + RUST_TAURI))
# 前端：it( / test( 计数（排除 node_modules）
FE_FILES=$(find src -name '*.test.ts' | wc -l | tr -d ' ')
FE_TESTS=$(grep -rhoE '^\s*(it|test)\(' src --include='*.test.ts' | wc -l | tr -d ' ')

printf '  实测：Rust %s（单元 %s + 集成 %s + 壳 %s）；前端 %s 文件 %s 用例\n' \
  "${RUST_TOTAL}" "${RUST_UNIT}" "${RUST_INTEG}" "${RUST_TAURI}" "${FE_FILES}" "${FE_TESTS}"

# 期望：README 的 "cargo test" 注释行、清单1 修订注记、开发日志最新条目三者与实测一致
README_N=$(grep -oE '共 [0-9]+ 用例' README.md | head -1 | grep -oE '[0-9]+' || echo "")
Q1_N=$(grep -oE '测试 \*\*[0-9]+ 个\*\*' docs/验收基线/清单1-非GUI测试映射.md | head -1 | grep -oE '[0-9]+' || echo "")
LOG_N=$(grep -oE 'voucher-core \*\*[0-9]+' docs/开发日志.md | tail -1 | grep -oE '[0-9]+' || echo "")
RUST_CORE=$((RUST_UNIT + RUST_INTEG))
# README 的写法是「voucher-core：… 共 N 用例；加 src-tauri 3 用例共 M」——N 对 voucher-core，M 对总数
README_TOTAL=$(grep -oE '共 [0-9]+ 用例；加 src-tauri 3 用例共 [0-9]+' README.md | head -1 | grep -oE '共 [0-9]+$' | grep -oE '[0-9]+' || echo "")
[ -n "${README_N}" ] && { [ "${README_N}" = "${RUST_CORE}" ] && ok "README 的 voucher-core 数与实测一致（${README_N}）" || warn "README 写 voucher-core ${README_N}，实测 ${RUST_CORE}"; } || warn "README 未找到用例数"
[ -n "${README_TOTAL}" ] && { [ "${README_TOTAL}" = "${RUST_TOTAL}" ] && ok "README 的总数含壳与实测一致（${README_TOTAL}）" || warn "README 总数写 ${README_TOTAL}，实测 ${RUST_TOTAL}"; } || warn "README 未找到含壳总数"
[ -n "${Q1_N}" ] && { [ "${Q1_N}" = "${RUST_CORE}" ] && ok "清单1 与实测一致（${Q1_N}）" || warn "清单1 写 ${Q1_N}，实测 voucher-core ${RUST_CORE}"; } || warn "清单1 未找到用例数"
[ -n "${LOG_N}" ] && { [ "${LOG_N}" = "${RUST_CORE}" ] && ok "开发日志 与实测一致（${LOG_N}）" || warn "开发日志写 ${LOG_N}，实测 voucher-core ${RUST_CORE}"; } || warn "开发日志 未找到用例数"

FE_DOC=$(grep -oE '前端 vitest \*\*[0-9]+ 文件 [0-9]+ 用例\*\*' docs/验收基线/清单1-非GUI测试映射.md | head -1 || echo "")
if [ -n "${FE_DOC}" ]; then
  DOC_F=$(echo "${FE_DOC}" | grep -oE '[0-9]+ 文件' | grep -oE '[0-9]+')
  DOC_T=$(echo "${FE_DOC}" | grep -oE '[0-9]+ 用例' | grep -oE '[0-9]+')
  if [ "${DOC_F}" = "${FE_FILES}" ] && [ "${DOC_T}" = "${FE_TESTS}" ]; then
    ok "清单1 前端用例数与实测一致（${FE_FILES} 文件 ${FE_TESTS} 用例）"
  else
    warn "清单1 前端写 ${DOC_F} 文件 ${DOC_T} 用例，实测 ${FE_FILES} 文件 ${FE_TESTS} 用例"
  fi
else
  warn "清单1 未记录前端用例数"
fi

# ---------- 4. 生产代码危险模式 ----------
section "4. 生产代码危险模式（Rust）"

# 4.1 生产 unwrap/expect：按 #[cfg(test)] 行切分（只统计测试模块之前的代码）
prod_unwrap=0
while IFS= read -r f; do
  cut=$(grep -n '#\[cfg(test)\]' "${f}" | head -1 | cut -d: -f1)
  [ -z "${cut}" ] && cut=$(( $(wc -l < "${f}") + 1 ))
  n=$(head -n $((cut - 1)) "${f}" | grep -cE '\.unwrap\(\)|\.expect\(' || true)
  prod_unwrap=$((prod_unwrap + n))
done < <(find crates/voucher-core/src src-tauri/src -name '*.rs')
if [ "${prod_unwrap}" -lt 12 ]; then
  ok "生产 unwrap/expect 共 ${prod_unwrap} 处（逐处确认是否可消除）"
else
  warn "生产 unwrap/expect 共 ${prod_unwrap} 处（建议逐处复核）"
fi

n=$(grep -rcE '\bpanic!\(' crates/voucher-core/src src-tauri/src 2>/dev/null | awk -F: '{s+=$2} END {print s+0}')
[ "${n}" -le 1 ] && ok "panic! 共 ${n} 处" || warn "panic! 共 ${n} 处（应在测试或不可达分支）"

n=$(grep -rcE '\bunsafe\b' crates/voucher-core/src src-tauri/src 2>/dev/null | awk -F: '{s+=$2} END {print s+0}')
printf '  [OK]   unsafe 共 %s 处（逐处需有注释说明安全性）\n' "${n}"

n=$(grep -rcE 'unwrap_or_default\(' crates/voucher-core/src src-tauri/src 2>/dev/null | awk -F: '{s+=$2} END {print s+0}')
warn "unwrap_or_default 共 ${n} 处（静默容错：确认是「有意」还是「掩盖损坏」）"

n=$(grep -rcE 'format!\("[^"]*(SELECT|INSERT|UPDATE|DELETE)' crates/voucher-core/src 2>/dev/null | awk -F: '{s+=$2} END {print s+0}')
[ "${n}" -le 20 ] && ok "SQL format! 拼接 ${n} 处（需逐处确认只拼结构、不拼用户输入）" || warn "SQL format! 拼接 ${n} 处"

# ---------- 5. 前端健壮性 ----------
section "5. 前端健壮性"
# 计数两个数字：① 完全空的 catch 块；② 有内容但**没有任何日志/重抛**的（"静默吞掉"，须有注释说明为何可吞）
read -r silent empty_catch <<<"$(python3 - <<'PYEOF'
import pathlib, re
silent = 0
empty = 0
for p in pathlib.Path("src").rglob("*"):
    if not p.is_file() or p.suffix not in (".ts", ".vue") or "__tests__" in str(p):
        continue
    s = p.read_text(encoding="utf-8")
    for m in re.finditer(r"catch\s*(?:\([^)]*\))?\s*\{", s):
        end = s.find("}", m.end())
        if end < 0:
            continue
        body = s[m.end():end]
        if body.strip() == "":
            empty += 1
            silent += 1
        elif not re.search(r"log|console|toast|throw|warn\(|error\(", body):
            silent += 1
print(silent, empty)
PYEOF
)"
if [ "${empty_catch}" -eq 0 ] && [ "${silent}" -le 20 ]; then
  ok "catch 块：完全空 ${empty_catch} 处；无日志/重抛（静默）${silent} 处——逐个确认「有注释说明为何可吞」"
else
  warn "catch 块：完全空 ${empty_catch} 处（应为 0）；静默 ${silent} 处——需逐个确认"
fi

n=$(grep -rnE 'as any|@ts-ignore|@ts-expect-error' src --include='*.vue' --include='*.ts' | grep -v '__tests__' | wc -l | tr -d ' ')
[ "${n}" -le 2 ] && ok "as any / @ts-ignore 共 ${n} 处" || warn "as any / @ts-ignore 共 ${n} 处"

n=$(grep -rcE 'v-html' src --include='*.vue' | awk -F: '{s+=$2} END {print s+0}')
[ "${n}" -le 2 ] && ok "v-html 共 ${n} 处（须为受控内容）" || warn "v-html 共 ${n} 处"

n=$(grep -rn 'invoke(' src --include='*.vue' --include='*.ts' | grep -v 'src/api/client.ts' | grep -v '__tests__' | wc -l | tr -d ' ')
[ "${n}" -eq 0 ] && ok "无页面绕过 api 封装直接 invoke" || fail "有 ${n} 处绕过 api 封装直接 invoke"

# 反模式：把后端下发的附件目录**再拼一次路径**（本项目明令禁止；类型字段声明不算）
n=$(grep -rnE 'attachmentsDir' src --include='*.vue' --include='*.ts' \
    | grep -v '__tests__' \
    | grep -vE ':[0-9]+:[[:space:]]*attachmentsDir:' \
    | grep -cE '\$\{attachmentsDir|attachmentsDir\s*\+|\+\s*attachmentsDir' || true)
[ "${n}" -eq 0 ] && ok "前端未自行拼附件路径（必须用后端下发的 fileAbs/enhancedAbs）" || fail "发现 ${n} 处自行拼路径"

# ---------- 6. 契约守卫存在性 ----------
section "6. 契约守卫存在性"
EXPECTED_GUARDS=(
  "src/api/__tests__/invoke-contract.test.ts:前后端 invoke 名一致"
  "src/__tests__/config-defaults.test.ts:配置默认值双端对齐"
  "src/__tests__/tokenContract.test.ts:CSS 令牌引用有定义"
  "src/__tests__/assetScopeContract.test.ts:附件 scope 与不得自拼路径"
  "src/__tests__/drilldownContract.test.ts:下钻参数必须被列表读取"
  "src/__tests__/controlSizeContract.test.ts:控件尺寸统一走令牌"
  "src/__tests__/iaContract.test.ts:信息架构与拖动区"
  "src/__tests__/duplicateReviewContract.test.ts:重复核查三处登记"
)
for item in "${EXPECTED_GUARDS[@]}"; do
  f="${item%%:*}"; desc="${item#*:}"
  [ -f "${f}" ] && ok "守卫在：${desc}" || fail "守卫缺失：${desc}（${f}）"
done

# ---------- 7. 文档纪律抽查 ----------
section "7. 文档纪律抽查"
# 7.1 最新 bump 提交是否有开发日志条目
last_bump=$(git log --oneline -1 --grep='bump [0-9]' 2>/dev/null || echo "")
if [ -n "${last_bump}" ]; then
  ver=$(echo "${last_bump}" | grep -oE 'bump [0-9]+\.[0-9]+\.[0-9]+' | awk '{print $2}')
  if grep -q "v${ver}" docs/开发日志.md; then
    ok "bump ${ver} 有对应开发日志条目"
  else
    fail "bump ${ver} 无开发日志条目（违反 AGENTS.md 开发日志纪律①）"
  fi
fi
# 7.2 注释时效：过期措辞扫描（命中需人工确认是否已实装）
stale=$(grep -rnE '(待补|尚未实装|下一步待做|未实装)' src --include='*.vue' --include='*.ts' | grep -v '__tests__' | wc -l | tr -d ' ')
if [ "${stale}" -eq 0 ]; then ok "无「待补/未实装」类过期措辞"; else warn "有 ${stale} 处「待补/未实装」措辞，需确认是否已过期"; fi
# 7.3 挂账表条目数
n=$(sed -n '/^# 三、当前挂账/,${p}' docs/开发日志.md | grep -cE '^\| [0-9]+ \|')
printf '  [OK]   挂账表条目数：%s（新发现应登记为下一条）\n' "${n}"
# 7.4 快照类文档是否只有头部注记（正文不该改动：只查是否有注记）
for f in docs/voucher-studio-health-check-2026-09.md docs/voucher-studio-project-analysis-2026-09-09.md \
         docs/voucher-studio-fix-status-vs-analysis-2026-09-09.md docs/voucher-studio-status-audit-2026-09-17.md; do
  [ -f "${f}" ] || continue
  if head -20 "${f}" | grep -q '校正注记\|平台范围注记'; then
    ok "快照有头部注记：$(basename "${f}")"
  else
    warn "快照未见头部注记：$(basename "${f}")"
  fi
done

# ---------- 8. 平台与打包抽查 ----------
section "8. 平台与打包"
if [ -f package-macos.sh ]; then
  for key in "自包含检查" "部署目标" "codesign"; do
    grep -q "${key}" package-macos.sh && ok "package-macos.sh 含 ${key} 自检" || warn "package-macos.sh 缺 ${key} 自检"
  done
fi
if command -v pkg-config >/dev/null 2>&1; then
  v=$(PKG_CONFIG_PATH="${PKG_CONFIG_PATH:-${HOME}/.local/opencv-4.10/lib/pkgconfig}" pkg-config --modversion opencv4 2>/dev/null || echo "")
  [ "${v}" = "4.10.0" ] && ok "OpenCV = 4.10.0" || warn "OpenCV 版本为 '${v}'（应为 4.10.0）"
else
  warn "未找到 pkg-config（跳过 OpenCV 版本抽查）"
fi

# ---------- 汇总 ----------
printf '\n=== 汇总 ===\n  FAIL %s ｜ WARN %s\n' "${FAILS}" "${WARNS}"
if [ "${FAILS}" -gt 0 ]; then
  printf '  有 FAIL：按 docs/审计指南.md §2 判定严重度并处置（P0/P1 本轮修，P2/P3 进挂账）\n'
  [ "${STRICT}" -eq 1 ] && exit 1
else
  printf '  自动检查无 FAIL；人工部分见 docs/审计指南.md §3.2 与 §4.3\n'
fi
exit 0
