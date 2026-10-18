#!/usr/bin/env bash
# 一键发版（含**过期产物清理**与**产物自包含校验**）
#
# 背景（2026-10-18）：v0.22.0–v0.22.2 三次发版手工执行时**漏掉了 `package-macos.sh`**，
# 发布出去的 .app 主二进制仍引用 `/Users/<本机>/.local/opencv-4.10/lib/*.dylib`、也没有
# `Contents/Frameworks` → 换台机器就是 dyld "Library not loaded"。本脚本把发版全流程固化，
# 并在第 6 步**硬校验**产物自包含（绝对路径引用必须为 0、Frameworks 必须齐全、引用必须是 @rpath）。
#
# 用法：
#   bash scripts/release.sh 0.22.3                 # 预览：只打印将执行的步骤（不改任何文件）
#   bash scripts/release.sh 0.22.3 --apply         # 真正发版
#   bash scripts/release.sh 0.22.3 --apply --keep-build   # 发版后不清理构建缓存
#   bash scripts/release.sh 0.22.3 --apply --skip-gates   # 跳过门禁（不推荐）
#
# 前置（脚本会检查）：工作区干净、在 main 上、已推送、锁文件与 manifest 一致（`cargo metadata --locked`
#                     + `npm ci --dry-run`，防依赖漂移）、`docs/开发日志.md` 已写本版条目、
#                     `docs/release-notes/<version>.md` 存在（Release 说明）、
#                     自动更新签名私钥存在（`~/.tauri/voucher-studio-updater.key`，可用
#                     `TAURI_SIGNING_PRIVATE_KEY_PATH` 覆盖）。
#
# 自动更新（挂账 #39）：第 5 步 package-macos.sh 之后**重新**打 `.app.tar.gz` 并签名——不能直接用
# tauri bundler 在打包时生成的更新包，因为那时 OpenCV dylib 还没被收进 Frameworks（正是 2026-10-18
# 发布缺陷的同一根因）。第 6 步会解包校验更新包，第 7 步把 dmg / update tar.gz / .sig / latest.json 一起上传。
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"
export PATH="${HOME}/.cargo/bin:${PATH}"
export PKG_CONFIG_PATH="${PKG_CONFIG_PATH:-${HOME}/.local/opencv-4.10/lib/pkgconfig}"

UPDATER_KEY="${TAURI_SIGNING_PRIVATE_KEY_PATH:-${HOME}/.tauri/voucher-studio-updater.key}"
REPO_SLUG="$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null || true)"

VERSION=""
APPLY=0
KEEP_BUILD=0
SKIP_GATES=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --apply) APPLY=1; shift ;;
    --keep-build) KEEP_BUILD=1; shift ;;
    --skip-gates) SKIP_GATES=1; shift ;;
    -h|--help) sed -n '2,24p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
    -*) echo "✗ 未知参数：$1（--help 看用法）" >&2; exit 2 ;;
    *) VERSION="$1"; shift ;;
  esac
done

fail() { echo "✗ $*" >&2; exit 1; }
step() { echo; echo "== $* =="; }
run() {
  if [ "${APPLY}" -eq 1 ]; then
    echo "+ $*"
    "$@" || fail "命令失败：$*"
  else
    echo "  [预览] $*"
  fi
}

[ -n "${VERSION}" ] || fail "用法：bash scripts/release.sh <版本号> [--apply]（例：0.22.3）"
case "${VERSION}" in v*) fail "版本号不要带 v 前缀（tag 会自动加）：${VERSION}" ;; esac
printf '%s' "${VERSION}" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' || fail "版本号必须是 x.y.z：${VERSION}"

LABEL="预览"; [ "${APPLY}" -eq 1 ] && LABEL="执行"
echo "== 发版 ${VERSION}（${LABEL}）=="
[ "${APPLY}" -eq 0 ] && echo "（预览模式：不修改任何文件；加 --apply 执行）"

step "1/7 预检"
[ -z "$(git status --porcelain)" ] || fail "工作区不干净，先提交或 stash"
[ "$(git branch --show-current)" = "main" ] || fail "当前不在 main 分支"
git fetch -q origin main || fail "git fetch 失败"
[ "$(git rev-list --count origin/main..HEAD)" -eq 0 ] || fail "本地有未推送提交，先 push"
grep -q "${VERSION}" docs/开发日志.md || fail "docs/开发日志.md 里还没有 ${VERSION} 的条目（先写开发日志）"
[ -f "docs/release-notes/${VERSION}.md" ] || fail "缺少 Release 说明：docs/release-notes/${VERSION}.md"
[ -f "${UPDATER_KEY}" ] || fail "缺少自动更新签名私钥：${UPDATER_KEY}（先用 npx tauri signer generate 生成）"

# 锁文件一致性：没有 CI 做第二环境验证时，靠这里拦住"本地能编、别的机器拉不到同一套依赖"
cargo metadata --locked --format-version 1 >/dev/null 2>&1 \
  || fail "Cargo.lock 与 Cargo.toml 不一致（跑 cargo update 并提交 Cargo.lock）"
npm ci --dry-run >/dev/null 2>&1 \
  || fail "package-lock.json 与 package.json 不一致（跑 npm install 并提交 package-lock.json）"

echo "  ✓ 工作区干净 / main / 已推送 / 锁文件一致 / 开发日志与 Release 说明就绪 / 更新签名私钥就绪"

step "2/7 门禁"
if [ "${SKIP_GATES}" -eq 1 ]; then
  echo "  (--skip-gates：跳过)"
else
  run npm run typecheck
  run npm run lint
  run npm run test
  run npm run build
  run cargo fmt --all --check
  run cargo clippy --workspace --all-targets -- -D warnings
  run cargo test --workspace --no-fail-fast -- --include-ignored
  run bash scripts/guards.sh layering
  run bash scripts/guards.sh platform
  run bash scripts/audit.sh --fast
fi

step "3/7 过期产物清理（本次新增：发版前清 dev 档 + 旧版本 dmg / release-artifacts）"
run bash scripts/clean-build.sh --apply

step "4/7 版本号 bump（package.json / src-tauri/tauri.conf.json / Cargo.toml）"
if [ "${APPLY}" -eq 1 ]; then
  python3 - "${VERSION}" <<'PY'
import json, pathlib, re, sys
ver = sys.argv[1]
p = pathlib.Path("package.json"); d = json.loads(p.read_text()); d["version"] = ver
p.write_text(json.dumps(d, ensure_ascii=False, indent=2) + "\n")
p = pathlib.Path("src-tauri/tauri.conf.json"); s = p.read_text()
s2 = re.sub(r'("version":\s*")[0-9]+\.[0-9]+\.[0-9]+(")', rf'\g<1>{ver}\g<2>', s, count=1)
assert s2 != s, "tauri.conf.json 未找到 version 字段"
p.write_text(s2)
p = pathlib.Path("Cargo.toml"); s = p.read_text()
s2 = re.sub(r'(?m)^version = "[0-9]+\.[0-9]+\.[0-9]+"$', f'version = "{ver}"', s, count=1)
assert s2 != s, "Cargo.toml 未找到 workspace version"
p.write_text(s2)
print(f"  ✓ 三处版本号 → {ver}")
PY
else
  echo "  [预览] 把 package.json / src-tauri/tauri.conf.json / Cargo.toml 的版本号改为 ${VERSION}"
fi

step "5/7 构建 + 打包（**必须** package-macos.sh：收 OpenCV dylib + 改 @rpath + 重签 + 出 dmg）"
run npm run tauri build
run bash package-macos.sh

if [ "${APPLY}" -eq 1 ]; then
  UPD_TGZ="target/release-artifacts/Voucher.Studio_${VERSION}_arm64.app.tar.gz"
  tar -czf "${UPD_TGZ}" -C "target/release/bundle/macos" "Voucher Studio.app" || fail "打更新包失败"
  npx tauri signer sign -f "${UPDATER_KEY}" -p "" "${UPD_TGZ}" >/dev/null || fail "更新包签名失败"
  python3 - "${VERSION}" "${UPD_TGZ}.sig" "target/release-artifacts/latest.json" "docs/release-notes/${VERSION}.md" "${REPO_SLUG}" <<'PYUPD'
import datetime, json, pathlib, sys
version, sig_path, out, notes_path, slug = sys.argv[1:6]
sig = pathlib.Path(sig_path).read_text().strip()
notes = ""
try:
    raw = pathlib.Path(notes_path).read_text()
    body = [ln.strip() for ln in raw.splitlines() if ln.strip() and not ln.lstrip().startswith("#")]
    notes = " ".join(body)[:600]
except OSError:
    pass
url = f"https://github.com/{slug}/releases/download/v{version}/Voucher.Studio_{version}_arm64.app.tar.gz"
data = {
    "version": version,
    "notes": notes,
    "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "platforms": {"darwin-aarch64": {"signature": sig, "url": url}},
}
pathlib.Path(out).write_text(json.dumps(data, ensure_ascii=False, indent=2) + "\n")
print(f"  ✓ 更新包 + latest.json 就绪（{out}）")
PYUPD
else
  echo "  [预览] tar 打包 target/release/bundle/macos/Voucher Studio.app → *.app.tar.gz"
  echo "  [预览] npx tauri signer sign -f ${UPDATER_KEY} <tar.gz>（生成 .sig）"
  echo "  [预览] 写 target/release-artifacts/latest.json（version / notes / pub_date / darwin-aarch64.url+signature）"
fi

step "6/7 产物自包含校验（堵住 2026-10-18 的发布缺陷）"
APP="target/release/bundle/macos/Voucher Studio.app"
if [ "${APPLY}" -eq 0 ]; then
  echo "  [预览] 校验：主二进制无 /Users 绝对路径引用、Contents/Frameworks 含 6 个 OpenCV dylib、"
  echo "         引用为 @rpath、CFBundleShortVersionString == ${VERSION}、dmg + sha256sums.txt 存在"
else
  [ -d "${APP}" ] || fail "找不到 .app：${APP}"
  EXEC_NAME="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "${APP}/Contents/Info.plist")"
  EXEC="${APP}/Contents/MacOS/${EXEC_NAME}"
  SHORT_VER="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "${APP}/Contents/Info.plist")"
  [ "${SHORT_VER}" = "${VERSION}" ] || fail "包内版本 ${SHORT_VER} ≠ ${VERSION}"

  # ① 绝对路径引用必须为 0（本机 OpenCV 路径不得残留）
  abs_refs="$(otool -L "${EXEC}" | grep -E '^[[:space:]]*/(Users|opt/homebrew|usr/local)' || true)"
  [ -z "${abs_refs}" ] || { echo "${abs_refs}" >&2; fail "主二进制仍引用本机绝对路径（没走 package-macos.sh？）"; }
  # ② Frameworks 必须齐全
  for l in core imgproc imgcodecs calib3d features2d flann; do
    ls "${APP}/Contents/Frameworks/libopencv_${l}.410.dylib" >/dev/null 2>&1 \
      || fail "Contents/Frameworks 缺少 libopencv_${l}.410.dylib"
  done
  # ③ 引用必须走 @rpath
  otool -L "${EXEC}" | grep -q "@rpath/libopencv_core.410.dylib" || fail "OpenCV 引用不是 @rpath"
  # ④ dmg 与校验和
  DMG="$(ls "target/release-artifacts/Voucher.Studio_${VERSION}_arm64.dmg" 2>/dev/null || true)"
  [ -n "${DMG}" ] || fail "找不到 dmg：target/release-artifacts/Voucher.Studio_${VERSION}_arm64.dmg"
  [ -f "target/release-artifacts/sha256sums.txt" ] || fail "缺少 target/release-artifacts/sha256sums.txt"
  ( cd target/release-artifacts && shasum -a 256 -c sha256sums.txt ) || fail "sha256 校验失败"
  echo "  ✓ 自包含校验通过：Frameworks 6 个 dylib / 引用 @rpath / 无本机绝对路径"
  echo "  ✓ $(cd target/release-artifacts && cat sha256sums.txt)"

  # ⑤ 自动更新包：解包后必须同样自包含（否则用户更新后打不开）
  UPD_TGZ="target/release-artifacts/Voucher.Studio_${VERSION}_arm64.app.tar.gz"
  [ -f "${UPD_TGZ}" ] || fail "缺少更新包：${UPD_TGZ}"
  [ -f "${UPD_TGZ}.sig" ] || fail "缺少更新包签名：${UPD_TGZ}.sig"
  [ -f "target/release-artifacts/latest.json" ] || fail "缺少 latest.json"
  UPD_TMP="$(mktemp -d)"
  tar -xzf "${UPD_TGZ}" -C "${UPD_TMP}" || fail "更新包解压失败"
  UPD_EXEC="${UPD_TMP}/Voucher Studio.app/Contents/MacOS/${EXEC_NAME}"
  [ -x "${UPD_EXEC}" ] || fail "更新包里找不到主二进制"
  [ -n "$(ls "${UPD_TMP}/Voucher Studio.app/Contents/Frameworks"/libopencv_core.410.dylib 2>/dev/null)" ] \
    || fail "更新包缺 Frameworks（更新后会 dyld 崩）"
  [ -z "$(otool -L "${UPD_EXEC}" | grep -E '^[[:space:]]*/(Users|opt/homebrew|usr/local)' || true)" ] \
    || fail "更新包主二进制仍引用本机绝对路径"
  python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); assert d["version"]==sys.argv[2]; sig=d["platforms"]["darwin-aarch64"]["signature"]; assert len(sig)>40' "target/release-artifacts/latest.json" "${VERSION}" \
    || fail "latest.json 内容不合法"
  rm -rf "${UPD_TMP}"
  echo "  ✓ 自动更新包自包含校验通过（Frameworks / 无绝对路径 / latest.json）"
fi

step "7/7 提交 / tag / 发布 / 核验"
if [ "${APPLY}" -eq 0 ]; then
  echo "  [预览] git add 版本三处 + Cargo.lock → commit \"chore(release): bump ${VERSION}\""
  echo "  [预览] git push origin main；git tag -a v${VERSION} -m \"v${VERSION}\"；git push origin v${VERSION}"
  echo "  [预览] gh release create v${VERSION} --title ... --notes-file docs/release-notes/${VERSION}.md"
  echo "         target/release-artifacts/Voucher.Studio_${VERSION}_arm64.dmg target/release-artifacts/sha256sums.txt"
  echo "         target/release-artifacts/Voucher.Studio_${VERSION}_arm64.app.tar.gz{,.sig} target/release-artifacts/latest.json"
  echo "  [预览] 下载回来 shasum -c + cmp 核验"
  [ "${KEEP_BUILD}" -eq 1 ] || echo "  [预览] 收尾：bash scripts/clean-build.sh --apply（清 dev 档 + 旧 dmg）"
else
  git add package.json src-tauri/tauri.conf.json Cargo.toml Cargo.lock
  git commit -q -m "chore(release): bump ${VERSION}" || fail "commit 失败"
  git push origin main || fail "push main 失败"
  git tag -a "v${VERSION}" -m "v${VERSION}"
  git push origin "v${VERSION}" || fail "push tag 失败"
  gh release create "v${VERSION}" \
    --title "v${VERSION}" \
    --notes-file "docs/release-notes/${VERSION}.md" \
    "target/release-artifacts/Voucher.Studio_${VERSION}_arm64.dmg" \
    "target/release-artifacts/sha256sums.txt" \
    "target/release-artifacts/Voucher.Studio_${VERSION}_arm64.app.tar.gz" \
    "target/release-artifacts/Voucher.Studio_${VERSION}_arm64.app.tar.gz.sig" \
    "target/release-artifacts/latest.json" || fail "gh release create 失败"
  echo "  ✓ Release 已发布：https://github.com/$(gh repo view --json nameWithOwner -q .nameWithOwner)/releases/tag/v${VERSION}"
  # 发布后核验：下载回来与服务端 digest 对比 + 逐字节 cmp
  VTMP="$(mktemp -d)"
  gh release download "v${VERSION}" --dir "${VTMP}" || fail "下载核验失败"
  ( cd "${VTMP}" && shasum -a 256 -c sha256sums.txt ) || fail "下载件 sha256 校验失败"
  cmp "${VTMP}/Voucher.Studio_${VERSION}_arm64.dmg" \
      "target/release-artifacts/Voucher.Studio_${VERSION}_arm64.dmg" || fail "下载件与本地件不一致"
  echo "  ✓ 发布后核验通过（sha256 -c OK + cmp 逐字节一致）"
  rm -rf "${VTMP}"
  if [ "${KEEP_BUILD}" -eq 0 ]; then
    step "收尾：清理构建缓存（dev 档 + 旧版本发布件）"
    bash scripts/clean-build.sh --apply
  fi
fi

echo
echo "== 完成 =="
