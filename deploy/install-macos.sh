#!/usr/bin/env bash
# 把 center-server 装成常驻服务（launchd），并挂上 Tailscale 私网 HTTPS。
#
# 用法：
#   bash deploy/install-macos.sh                # 构建 release 并安装
#   bash deploy/install-macos.sh --no-build     # 用已构建好的二进制
#   bash deploy/install-macos.sh --uninstall    # 卸载（保留数据）
#
# 说明：
# - 数据目录默认 ~/Library/Application Support/VoucherStudio（与桌面版一致，可迁移）
# - Web 静态文件默认取仓库 dist/；改过前端要重新 npm run build
# - 不碰系统代理、不开公网端口：只监听 127.0.0.1，外部经 Tailscale 访问
set -euo pipefail

LABEL="com.center.server"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DATA_DIR="${CENTER_DATA_DIR:-$HOME/Library/Application Support/VoucherStudio}"
BIN_DIR="$HOME/Library/Application Support/Center/bin"
LOG_DIR="$DATA_DIR/logs"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
WEB_DIR="$REPO/dist"

say() { printf '\033[1;32m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m!!\033[0m %s\n' "$*"; }

uninstall() {
  say "停止并卸载 $LABEL"
  launchctl bootout "gui/$UID/$LABEL" 2>/dev/null || true
  rm -f "$PLIST"
  say "已卸载；数据仍在 $DATA_DIR"
}

if [[ "${1:-}" == "--uninstall" ]]; then uninstall; exit 0; fi

# 1) 构建
if [[ "${1:-}" != "--no-build" ]]; then
  say "构建 release 二进制（首次约 3–6 分钟）"
  . "$HOME/.cargo/env"
  export PKG_CONFIG_PATH="${PKG_CONFIG_PATH:-$HOME/.local/opencv-4.10/lib/pkgconfig}"
  # 本机沙箱无法解包新依赖：仓库内 .cargo-home 是绕行法（见 docs/开发方案.md §5.7.2）
  if [[ -d "$REPO/.cargo-home" ]]; then
    ( cd "$REPO" && CARGO_HOME="$REPO/.cargo-home" cargo build --release -p center-server )
  else
    ( cd "$REPO" && cargo build --release -p center-server )
  fi
fi

SRC="$REPO/target/release/center-server"
[[ -x "$SRC" ]] || { warn "没找到 $SRC，先构建或去掉 --no-build"; exit 1; }
[[ -f "$WEB_DIR/index.html" ]] || { warn "没找到 $WEB_DIR/index.html，先 npm run build"; exit 1; }

# 2) 落位
say "安装二进制与静态文件"
mkdir -p "$BIN_DIR" "$LOG_DIR" "$(dirname "$PLIST")"
cp "$SRC" "$BIN_DIR/center-server"
chmod 755 "$BIN_DIR/center-server"

# 3) 写 plist
say "写入 $PLIST"
sed -e "s#__BIN__#$BIN_DIR/center-server#g" \
    -e "s#__DATA_DIR__#$DATA_DIR#g" \
    -e "s#__WEB_DIR__#$WEB_DIR#g" \
    -e "s#__LOG_DIR__#$LOG_DIR#g" \
    "$REPO/deploy/com.center.server.plist.in" > "$PLIST"

# 4) 装载
say "装载 launchd 服务"
launchctl bootout "gui/$UID/$LABEL" 2>/dev/null || true
# 重复安装时 bootout 与 bootstrap 之间要留一拍：紧接着 bootstrap 偶发
# `Bootstrap failed: 5: Input/output error`（2026-11-18 重装时踩到，隔一秒重试即成功）
sleep 1
if ! launchctl bootstrap "gui/$UID" "$PLIST"; then
  warn "bootstrap 失败，等 1 秒重试一次"
  sleep 1
  launchctl bootstrap "gui/$UID" "$PLIST"
fi
launchctl enable "gui/$UID/$LABEL"
sleep 1
if curl -fsS http://127.0.0.1:8787/api/health >/dev/null; then
  say "服务已就绪：http://127.0.0.1:8787"
else
  warn "健康检查失败，看 $LOG_DIR/center-server.err.log"
  exit 1
fi

# 5) Tailscale 私网 HTTPS（可选，装了就挂）
if command -v tailscale >/dev/null 2>&1; then
  say "挂 Tailscale 私网 HTTPS（443 → 127.0.0.1:8787）"
  tailscale serve --bg --https=443 http://127.0.0.1:8787 || warn "tailscale serve 失败（可能没登录，跳过）"
  tailscale serve status || true
  WEB_URL="https://$(tailscale status --json 2>/dev/null | python3 -c 'import json,sys;d=json.load(sys.stdin);s=d.get("Self",{});print(s.get("DNSName","").rstrip("."))' 2>/dev/null || true)"
  if [[ -z "${WEB_URL#https://}" ]]; then
    warn "取不到 MagicDNS 域名；在 Tailscale 后台看机器名，或 tailscale status"
  fi
else
  warn "没装 tailscale，跳过私网 HTTPS；局域网可临时 CENTER_ADDR=0.0.0.0:8787（务必配防火墙）"
fi

printf '\n完成。\n'
printf '  本机：   http://127.0.0.1:8787\n'
[[ -n "${WEB_URL:-}" ]] && printf '  私网：   %s\n' "$WEB_URL"
printf '  数据：   %s\n' "$DATA_DIR"
printf '  日志：   %s/center-server.{out,err}.log\n' "$LOG_DIR"
printf '  停止：   launchctl bootout gui/%s/%s\n' "$UID" "$LABEL"
printf '  卸载：   bash deploy/install-macos.sh --uninstall\n\n'
printf '第一次打开会要求设置访问密码（PBKDF2-SHA256 存在 auth_users 表）。\n'
printf '合盖睡眠：sudo pmset -a disablesleep 1（拔电前记得关掉）。\n'
