#!/usr/bin/env bash
# 冷备演练（M5）：把「主服务器上的备份」还原到一个空目录，再用第二个实例验证账本一致。
#
# 这一步回答的是「主机挂了，换一台机器要多久能接管」：
#   备份 → 还原到空目录 → 起服务 → 两边凭证条数/金额合计一致 → 通过
#
# 用法：
#   bash deploy/drill-coldstandby.sh [源数据目录]
#   CENTER_BIN=target/release/center-server bash deploy/drill-coldstandby.sh
#
# 默认源数据目录 /tmp/center-drill-src（没有就用 /tmp/center-smoke）。
# 全程只用临时目录，不碰真实账本；结束时自动停掉两个实例。
set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC_DATA="${1:-${CENTER_DATA_DIR:-/tmp/center-drill-src}}"
[[ -d "$SRC_DATA" ]] || SRC_DATA=/tmp/center-smoke
[[ -d "$SRC_DATA" ]] || { echo "源数据目录不存在：$SRC_DATA（先导入几张凭证再演练）"; exit 1; }

BIN="${CENTER_BIN:-$REPO/target/debug/center-server}"
CLI="${CENTER_CLI:-$REPO/target/debug/voucher-cli}"
[[ -x "$BIN" ]] || { echo "缺少 $BIN：先 cargo build -p center-server"; exit 1; }
[[ -x "$CLI" ]] || { echo "缺少 $CLI：先 cargo build -p voucher-cli"; exit 1; }

STAMP="$(date +%Y%m%d-%H%M%S)"
DRILL="/tmp/center-drill-$STAMP"
PORT_A=8795
PORT_B=8796
COOKIE="$DRILL/cookie.txt"
PASS="drill-$STAMP"

say() { printf '\033[1;32m==>\033[0m %s\n' "$*"; }
fail() { printf '\033[1;31m!!\033[0m %s\n' "$*"; exit 1; }

cleanup() {
  [[ -n "${PID_A:-}" ]] && kill "$PID_A" 2>/dev/null || true
  [[ -n "${PID_B:-}" ]] && kill "$PID_B" 2>/dev/null || true
  wait 2>/dev/null || true
}
trap cleanup EXIT

mkdir -p "$DRILL"

# 0) 源数据复制一份再演练：真实账本一个字节都不碰，密码也由本次演练自己设
say "复制源数据 → $DRILL/source"
cp -R "$SRC_DATA" "$DRILL/source"
SRC_DATA="$DRILL/source"
# 副本里清掉登录凭据：演练要自己设密码，否则沿用源库里的旧密码 → 401
if command -v sqlite3 >/dev/null 2>&1 && [[ -f "$SRC_DATA/voucher.db" ]]; then
  sqlite3 "$SRC_DATA/voucher.db" 'DELETE FROM auth_sessions; DELETE FROM auth_users;' 2>/dev/null || true
fi

# 1) 起主实例（源）
say "起主实例：$SRC_DATA → 127.0.0.1:$PORT_A"
CENTER_DATA_DIR="$SRC_DATA" CENTER_ADDR="127.0.0.1:$PORT_A" \
  CENTER_WEB_DIR="$REPO/dist" CENTER_PASSWORD="$PASS" \
  "$BIN" >"$DRILL/source.log" 2>&1 &
PID_A=$!
for _ in $(seq 1 40); do
  curl -fsS "http://127.0.0.1:$PORT_A/api/health" >/dev/null 2>&1 && break
  sleep 0.25
done
curl -fsS "http://127.0.0.1:$PORT_A/api/health" >/dev/null || fail "主实例没起来，看 $DRILL/source.log"

login() {
  curl -fsS -c "$COOKIE" -X POST "http://127.0.0.1:$PORT_A/api/auth/login" \
    -H 'content-type: application/json' -d "{\"password\":\"$PASS\"}" >/dev/null
}
login

cmd() { # cmd <name> <json>
  curl -fsS -b "$COOKIE" -X POST "http://127.0.0.1:$PORT_A/api/cmd/$1" \
    -H 'content-type: application/json' -d "${2:-{\}}"
}

# 2) 备份 → 导出全量 zip
say "在源实例上打一次备份"
SNAP="$(cmd run_backup '{}' | python3 -c 'import json,sys; print(json.load(sys.stdin)["path"])')"
say "快照：$SNAP"
FULL="$(cmd export_full_backup "{\"path\":\"$SNAP\"}" | python3 -c 'import json,sys; print(json.load(sys.stdin))')"
[[ -f "$FULL" ]] || fail "全量备份没生成：$FULL"
say "全量：${FULL}（$(du -h "$FULL" | cut -f1)）"

# 3) 还原到空目录
say "还原到 $DRILL/standby"
"$CLI" restore --zip "$FULL" --target "$DRILL/standby"

# 4) 起第二个实例（模拟 i3 PC 接管）
say "起冷备实例：$DRILL/standby → 127.0.0.1:$PORT_B"
CENTER_DATA_DIR="$DRILL/standby" CENTER_ADDR="127.0.0.1:$PORT_B" \
  CENTER_WEB_DIR="$REPO/dist" CENTER_PASSWORD="$PASS" \
  "$BIN" >"$DRILL/standby.log" 2>&1 &
PID_B=$!
for _ in $(seq 1 40); do
  curl -fsS "http://127.0.0.1:$PORT_B/api/health" >/dev/null 2>&1 && break
  sleep 0.25
done
curl -fsS "http://127.0.0.1:$PORT_B/api/health" >/dev/null || fail "冷备实例没起来，看 $DRILL/standby.log"

# 5) 对比：凭证条数 + 金额合计
counter() { # counter <port>
  curl -fsS -c "$2" -X POST "http://127.0.0.1:$1/api/auth/login" \
    -H 'content-type: application/json' -d "{\"password\":\"$PASS\"}" >/dev/null
  curl -fsS -b "$2" -X POST "http://127.0.0.1:$1/api/cmd/voucher_stats" \
    -H 'content-type: application/json' -d '{}' \
    | python3 -c 'import json,sys; d=json.load(sys.stdin); print(json.dumps({k:d.get(k) for k in ("total","amountCents","thisMonth","draft","reviewed")}, sort_keys=True))'
}
A="$(counter "$PORT_A" "$DRILL/a.txt")"
B="$(counter "$PORT_B" "$DRILL/b.txt")"

echo
say "主实例：$A"
say "冷备：$B"
if [[ "$A" == "$B" ]]; then
  say "通过：备份 → 还原 → 起服务，账本一致（接管演练完成）"
  echo "演练目录：$DRILL"
else
  fail "不一致：主 $A / 冷备 $B"
fi
