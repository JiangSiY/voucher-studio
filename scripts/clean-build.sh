#!/usr/bin/env bash
# 构建缓存定期清理（防止 target/ 无限增长）
#
# 背景（2026-10-18）：工作区 `target/` 一度涨到 **39 GB**——682 个依赖 × debug/release/test 三套
# profile × 15 个测试目标（每个都独立链接整套依赖）× 数周的陈旧产物，而 cargo **不会自动 GC**。
# 本脚本把"该怎么清"固化成一条命令，避免每次重新分析。
#
# 用法：
#   bash scripts/clean-build.sh                 # 预览（默认，不删任何东西）
#   bash scripts/clean-build.sh --apply         # 清理 dev 档 + 旧发布 bundle（保留 release 依赖缓存）
#   bash scripts/clean-build.sh --apply --all   # 全量 cargo clean（下次构建全量重编）
#   bash scripts/clean-build.sh --apply --sweep # 叠跑 cargo sweep（需 `cargo install cargo-sweep`）
#   bash scripts/clean-build.sh --apply --sweep --days 30
#
# 安全边界：只在仓库内操作（target/），不碰 src/ docs/ .git/ 与仓库外文件。
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"

APPLY=0
ALL=0
SWEEP=0
DAYS=14
while [ "$#" -gt 0 ]; do
  case "$1" in
    --apply) APPLY=1; shift ;;
    --all) ALL=1; shift ;;
    --sweep) SWEEP=1; shift ;;
    --days) DAYS="${2:?--days 需要一个数字（例：--days 30）}"; shift 2 ;;
    --days=*) DAYS="${1#--days=}"; shift ;;
    -h|--help)
      sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "✗ 未知参数：$1（--help 看用法）" >&2; exit 2 ;;
  esac
done

human() { du -sh "$1" 2>/dev/null | cut -f1 || true; }

echo "== 构建缓存现状（${REPO_ROOT}）=="
if [ -d target ]; then
  echo "  target/            $(human target)"
  [ -d target/debug ] && echo "    ├─ debug/        $(human target/debug)（含测试二进制与增量缓存）"
  [ -d target/debug/incremental ] && echo "    │    └ incremental/ $(human target/debug/incremental)"
  [ -d target/release ] && echo "    └─ release/      $(human target/release)"
  [ -d target/release/bundle ] && echo "         └ bundle/    $(human target/release/bundle)"
  [ -d target/release-artifacts ] && echo "  release-artifacts/ $(human target/release-artifacts)"
else
  echo "  target/ 不存在（已清理）"
fi

# 旧版本发布件：文件名里不含当前版本号的一律视为陈旧（dmg / app 的旧副本）
CUR_VERSION="$(python3 -c 'import json;print(json.load(open("package.json"))["version"])' 2>/dev/null || echo "")"

echo
if [ "${APPLY}" -eq 0 ]; then
  echo "== 预览（未删除任何文件）=="
  echo "  将清理："
  DEV_SIZE="$(human target/debug)"
  echo "    · target/debug/（dev 档产物；约 ${DEV_SIZE:-无}）—— 保留 target/release/"
  [ -n "${CUR_VERSION}" ] && echo "    · target/release/bundle 中版本号不是 ${CUR_VERSION} 的旧 dmg"
  [ "${ALL}" -eq 1 ] && echo "    · （--all）整个 target/"
  [ "${SWEEP}" -eq 1 ] && echo "    · （--sweep）cargo sweep 删除 ${DAYS} 天内未使用的产物"
  echo
  EXTRA=""
  [ "${ALL}" -eq 1 ] && EXTRA="${EXTRA} --all"
  [ "${SWEEP}" -eq 1 ] && EXTRA="${EXTRA} --sweep"
  echo "执行：bash scripts/clean-build.sh --apply${EXTRA}"
  exit 0
fi

echo "== 开始清理 =="
CARGO_BIN="$(command -v cargo || echo "${HOME}/.cargo/bin/cargo")"
if [ "${ALL}" -eq 1 ]; then
  echo "  · cargo clean（全量）"
  "${CARGO_BIN}" clean 2>&1 | tail -1
else
  if [ -d target/debug ]; then
    echo "  · cargo clean --profile dev（只清 dev 档）"
    "${CARGO_BIN}" clean --profile dev 2>&1 | tail -1
  fi
  # 旧版本发布件（保留当前版本）
  if [ -n "${CUR_VERSION}" ] && [ -d target/release/bundle/dmg ]; then
    while IFS= read -r f; do
      case "$(basename "${f}")" in
        *"${CUR_VERSION}"*) : ;;
        *) echo "    - 删除旧发布件：${f}"; rm -f "${f}" ;;
      esac
    done < <(find target/release/bundle/dmg -maxdepth 1 -name "*.dmg" -type f 2>/dev/null)
  fi
  if [ -d target/release-artifacts ]; then
    echo "  · 清理 target/release-artifacts 中的旧件（保留当前版本 + 更新清单/校验和）"
    # latest.json 与 sha256sums.txt 文件名不含版本号，但属于本版发布件，必须保留
    # （否则发版收尾清理会把刚要复核/重传的清单删掉）
    find target/release-artifacts -maxdepth 1 -type f \
      ! -name "*${CUR_VERSION}*" \
      ! -name "sha256sums.txt" ! -name "latest.json" -print -delete 2>/dev/null
  fi
fi

if [ "${SWEEP}" -eq 1 ]; then
  if command -v cargo-sweep >/dev/null 2>&1; then
    echo "  · cargo sweep -t ${DAYS}（删除 ${DAYS} 天内未使用的产物）"
    cargo-sweep sweep -t "${DAYS}" 2>&1 | tail -3
  else
    echo "  ! 未安装 cargo-sweep：cargo install cargo-sweep 后再跑 --sweep"
  fi
fi

echo
echo "== 清理后 =="
if [ -d target ]; then echo "  target/ $(human target)"; else echo "  target/ 不存在（已清理）"; fi
echo "  提示：日常优先用 \`cargo check -p <crate>\` / \`cargo test -p <crate> --test <name>\`，"
echo "        避免每次都 \`--workspace --all-targets\`（15 个测试目标会各链接一套依赖）。"
