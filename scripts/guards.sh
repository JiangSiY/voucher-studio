#!/usr/bin/env bash
# 仓库守卫（**单一真源**：CI 与 scripts/audit.sh 都调用本脚本）
#
# 为什么做成脚本：这两道 guard 原先在 `.github/workflows/ci.yml` 与 `scripts/audit.sh`
# 里各写了一份，而且已经漂移——audit.sh 的平台门禁漏了 `cfg(any(windows` 与
# `tauri.windows.conf.json` 两项（2026-09-27 审计发现）。定义只能有一份。
#
# 用法：bash scripts/guards.sh <layering|platform|--selftest>
#   layering    命令层（src-tauri/src）不得直连 db / 裸连接 / rusqlite
#   platform    仅 macOS：不得出现 Windows 条件编译、平台配置、.ps1 脚本
#   --selftest  用合成样本证明两条模式确实能命中、且不误报（守卫自身可证）
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"

# ---------- 定义（唯一真源；改动必须同步跑 --selftest） ----------

# 命令层越层。一条 `db::` 覆盖三种历史盲区写法：
#   ① `voucher_core::db::connection::open(...)`（打开裸连接最典型）
#   ② `use …::db::{repos::voucher_repo}`（大括号导入，旧模式 `db::repos` 相邻判据抓不到）
#   ③ `Database::new(conn)`（包装裸连接的类型，另有 `rusqlite` 直接使用）
# 合法路径只有一条：经 `services::query` 门面与领域服务取数/写入。
LAYER_PATTERN='db::|Database|rusqlite|\.with_(conn|tx)\('
LAYER_SCOPE='src-tauri/src'

# 平台范围：只查属性/配置形态，**不查注释里的历史叙述**（那些是有意保留的演进痕迹）。
PLATFORM_PATTERN='#\[cfg\(windows\)\]|cfg\(target_os = "windows"\)|cfg\(any\(windows'
PLATFORM_SCOPES='crates/*/src crates/*/tests src-tauri/src'
PLATFORM_CONFIG='src-tauri/tauri.windows.conf.json'

# ---------- 守卫 ----------

layering() {
  local hits
  hits=$(grep -rnE "${LAYER_PATTERN}" ${LAYER_SCOPE} 2>/dev/null || true)
  if [ -n "${hits}" ]; then
    printf '错误：命令层不得直连 db 层——应经 services::query 门面与领域服务\n' >&2
    printf '%s\n' "${hits}" >&2
    return 1
  fi
  return 0
}

platform() {
  local rc=0 hits
  hits=$(grep -rnE "${PLATFORM_PATTERN}" ${PLATFORM_SCOPES} 2>/dev/null || true)
  if [ -n "${hits}" ]; then
    printf '错误：Windows 平台条件编译不得回归（v0.17.0 起只支持 macOS）\n%s\n' "${hits}" >&2
    rc=1
  fi
  if [ -e "${PLATFORM_CONFIG}" ]; then
    printf '错误：%s 不得回归\n' "${PLATFORM_CONFIG}" >&2
    rc=1
  fi
  if find . -name '*.ps1' -not -path './node_modules/*' -not -path './target/*' | grep -q .; then
    printf '错误：仓库内不得存在 Windows 专用脚本（.ps1）\n' >&2
    rc=1
  fi
  return "${rc}"
}

# ---------- 自测：守卫自身可证 ----------

selftest() {
  local bad=0 line
  # 正样本：下列写法**必须**被越层模式命中
  while IFS= read -r line; do
    if printf '%s\n' "${line}" | grep -qE "${LAYER_PATTERN}"; then
      printf '  [OK]   越层命中：%s\n' "${line}"
    else
      printf '  [FAIL] 越层漏过：%s\n' "${line}"
      bad=$((bad + 1))
    fi
  done <<'SAMPLES'
use voucher_core::db::repos::voucher_repo;
let conn = voucher_core::db::connection::open(&paths.db_file())?;
let db = Database::new(conn);
use voucher_core::db::{repos::attachment_repo, voucher_repo};
use rusqlite::Connection;
ctx.db.with_conn(|c| Ok(()))?;
core.db.with_tx(|t| Ok(()))?;
SAMPLES
  # 负样本：合法写法与注释不得误报
  while IFS= read -r line; do
    if printf '%s\n' "${line}" | grep -qE "${LAYER_PATTERN}"; then
      printf '  [FAIL] 越层误报：%s\n' "${line}"
      bad=$((bad + 1))
    else
      printf '  [OK]   越层放行：%s\n' "${line}"
    fi
  done <<'SAMPLES'
use voucher_core::services::query as qsvc;
let rows = qsvc::list(ctx.core(), &query)?;
// 命令层只经领域服务（enhance_cache / services::query），不得直连仓储
SAMPLES
  # 正样本：Windows 形态必须被平台模式命中
  while IFS= read -r line; do
    if printf '%s\n' "${line}" | grep -qE "${PLATFORM_PATTERN}"; then
      printf '  [OK]   平台命中：%s\n' "${line}"
    else
      printf '  [FAIL] 平台漏过：%s\n' "${line}"
      bad=$((bad + 1))
    fi
  done <<'SAMPLES'
#[cfg(windows)]
#[cfg(target_os = "windows")]
#[cfg(any(windows, target_os = "windows"))]
SAMPLES
  # 负样本：注释里的历史叙述（平台收窄前的实况记载）不得误报
  while IFS= read -r line; do
    if printf '%s\n' "${line}" | grep -qE "${PLATFORM_PATTERN}"; then
      printf '  [FAIL] 平台误报：%s\n' "${line}"
      bad=$((bad + 1))
    else
      printf '  [OK]   平台放行：%s\n' "${line}"
    fi
  done <<'SAMPLES'
// 历史：v0.17.0 前曾支持 Windows（MSVC / WebView2），现已移除
// 平台范围收窄：不再有 windows 专属脚本与配置
SAMPLES
  if [ "${bad}" -gt 0 ]; then
    printf '\n自测失败：%s 项与预期不符——守卫模式可能已失效，先修模式再谈通过\n' "${bad}" >&2
    return 1
  fi
  printf '\n自测通过：越层 7 命中 + 3 放行；平台 3 命中 + 2 放行\n'
  return 0
}

# ---------- 入口 ----------

case "${1:-}" in
  layering)
    layering && printf 'OK：越层门禁通过（命令层无 db:: / Database / rusqlite / 裸 with_conn(tx)）\n'
    ;;
  platform)
    platform && printf 'OK：平台范围门禁通过（无 Windows cfg / 平台配置 / .ps1）\n'
    ;;
  --selftest)
    selftest
    ;;
  *)
    printf '用法：bash scripts/guards.sh <layering|platform|--selftest>\n' >&2
    exit 2
    ;;
esac
