#!/usr/bin/env bash
# macOS 打包收尾：把 `npm run tauri build` 产出的 .app 处理成可独立分发的安装包。
#
# 为什么必须有这一步：
#   Rust 侧（opencv crate）链接的是本机安装的 OpenCV dylib，且引用是**绝对路径**
#   （Linux 因库在系统默认搜索路径下而无感）。这样的 .app 只能在装有同一路径
#   OpenCV 的机器上启动，拷给别人即 dyld 报 "Library not loaded"。
#   Tauri 打包器对 .dylib 只做「复制到 Contents/Frameworks」，既不改 install name 也不补
#   rpath（见 tauri-bundler/src/bundle/macos/app.rs，全文无 install_name_tool 调用），
#   所以引用改写必须自己做。
#
# 本脚本四件事：dylib 收进 Contents/Frameworks → 引用统一改 @rpath → 主程序补 rpath →
# 由内到外 ad-hoc 重签名（改了 mach-o 必须重签，否则 arm64 直接拒绝启动）。
#
# 前置：本机已按 README「环境要求」装好 OpenCV 4.10（OPENCV_LIB 可覆盖其 lib 目录）。
# 用法：bash package-macos.sh [path/to/Xxx.app] [输出目录]
#       不带参数时自动取 target/release/bundle/macos/*.app，输出到 target/release-artifacts/。
#       **输出目录不得在 dist/ 下**（那是前端构建目录，任何一次 npm run build 都会清空它，
#       发布件会被静默冲掉——2026-10-09 v0.20.0 发布时实际发生，见开发日志与挂账 #31）。
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OPENCV_LIB="${OPENCV_LIB:-$HOME/.local/opencv-4.10/lib}"
LIBS=(core imgproc imgcodecs calib3d features2d flann)
ABI=410

APP="${1:-}"
OUT_DIR="${2:-$REPO_ROOT/target/release-artifacts}"

# 自检：拒绝输出到前端构建目录（历史事故防回归）
case "$OUT_DIR" in
  "$REPO_ROOT/dist"|"$REPO_ROOT/dist/"*)
    echo "拒绝：输出目录 $OUT_DIR 位于前端构建目录 dist/ 下（会被 npm run build 清空）。请换目录。" >&2
    exit 1
    ;;
esac

if [ -z "$APP" ]; then
  APP="$(ls -d "$REPO_ROOT"/target/release/bundle/macos/*.app 2>/dev/null | head -1 || true)"
  [ -n "$APP" ] || { echo "找不到 app：请先 npm run tauri build，或显式传入 .app 路径" >&2; exit 1; }
fi
APP="$(cd "$(dirname "$APP")" && pwd)/$(basename "$APP")"
[ -d "$APP" ] || { echo "找不到 app：$APP" >&2; exit 1; }

PLIST="$APP/Contents/Info.plist"
EXEC_NAME="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$PLIST")"
SHORT_VER="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$PLIST")"
EXEC="$APP/Contents/MacOS/$EXEC_NAME"
FRAMEWORKS="$APP/Contents/Frameworks"
APP_NAME="$(basename "$APP" .app)"

echo "==> app        : $APP"
echo "==> executable : $EXEC"
echo "==> version    : $SHORT_VER"
echo "==> OpenCV     : $OPENCV_LIB"

[ -f "$EXEC" ] || { echo "找不到可执行文件：$EXEC" >&2; exit 1; }

if ! otool -L "$EXEC" | grep -q "libopencv_core.$ABI.dylib"; then
  echo "==> 主程序未引用 libopencv_core.$ABI.dylib（可能已静态链接），无需处理"
  exit 0
fi

# --- 1) dylib 收进 Contents/Frameworks
mkdir -p "$FRAMEWORKS"
for l in "${LIBS[@]}"; do
  src="$OPENCV_LIB/libopencv_$l.$ABI.dylib"
  [ -f "$src" ] || { echo "缺少 $src（OpenCV 4.10 未装或路径不符，见 README 环境要求）" >&2; exit 1; }
  cp -f "$src" "$FRAMEWORKS/libopencv_$l.$ABI.dylib"
done

# --- 1b) 部署目标自检：包内 dylib 要求的系统版本不得高于主程序，否则 dyld 会拒绝加载
#   （症状：在开发机上一切正常，别人机器一启动即崩，且报错不易联想到"库比系统新"）。
#   踩过一次：OpenCV 未设 CMAKE_OSX_DEPLOYMENT_TARGET 时，CMake 默认取**构建机的系统版本**，
#   于是 dylib 成了 minos 27.0，而主程序是 11.0 —— 该包只能在 macOS 27 上跑。
app_minos="$(vtool -show-build "$EXEC" 2>/dev/null | awk '/minos/ {print $2; exit}')"
if [ -n "$app_minos" ]; then
  bad=""
  for l in "${LIBS[@]}"; do
    lib_minos="$(vtool -show-build "$FRAMEWORKS/libopencv_$l.$ABI.dylib" 2>/dev/null | awk '/minos/ {print $2; exit}')"
    [ -n "$lib_minos" ] || continue
    if [ "$(printf '%s\n%s\n' "$app_minos" "$lib_minos" | sort -V | tail -1)" != "$app_minos" ]; then
      echo "!! libopencv_$l.$ABI.dylib 要求 macOS $lib_minos，高于主程序的 $app_minos" >&2
      bad=1
    fi
  done
  if [ -n "$bad" ]; then
    echo "   该包在 macOS $app_minos ~ $lib_minos 之间的系统上无法启动。" >&2
    echo "   修法：重编 OpenCV 时显式指定部署目标，例如 -DCMAKE_OSX_DEPLOYMENT_TARGET=$app_minos" >&2
    exit 1
  fi
  echo "==> 部署目标检查通过（包内 dylib 与主程序同为 macOS $app_minos 起）"
fi

# --- 2) dylib 自身：改 id，并把互引用也改成 @rpath
for l in "${LIBS[@]}"; do
  tgt="$FRAMEWORKS/libopencv_$l.$ABI.dylib"
  install_name_tool -id "@rpath/libopencv_$l.$ABI.dylib" "$tgt"
  for m in "${LIBS[@]}"; do
    install_name_tool -change "$OPENCV_LIB/libopencv_$m.$ABI.dylib" \
      "@rpath/libopencv_$m.$ABI.dylib" "$tgt" 2>/dev/null || true
  done
done

# --- 3) 主程序：引用改 @rpath，并确保有指向 Frameworks 的 rpath
for m in "${LIBS[@]}"; do
  install_name_tool -change "$OPENCV_LIB/libopencv_$m.$ABI.dylib" \
    "@rpath/libopencv_$m.$ABI.dylib" "$EXEC" 2>/dev/null || true
done
if ! otool -l "$EXEC" | grep -q "@executable_path/../Frameworks"; then
  install_name_tool -add_rpath "@executable_path/../Frameworks" "$EXEC"
fi

# --- 4) 由内到外 ad-hoc 重签名
echo "==> ad-hoc 重签名"
codesign --force --sign - "$FRAMEWORKS"/libopencv_*."$ABI".dylib
codesign --force --sign - "$APP"
codesign --verify --strict --verbose=2 "$APP"
echo "==> 签名校验通过"

# --- 5) 自包含自检：确认没有残留指向本机 OpenCV 的引用
if otool -L "$EXEC" | grep -q "$OPENCV_LIB"; then
  echo "!! 仍有指向 $OPENCV_LIB 的引用，未做到自包含：" >&2
  otool -L "$EXEC" | grep "$OPENCV_LIB" >&2
  exit 1
fi
echo "==> 自包含检查通过"

# --- 6) 打 dmg（app + /Applications 快捷方式），并生成校验和
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"

# GitHub 在上传资产时会把文件名里的空格替换成点（本包实际落库为
# Voucher.Studio_0.15.1_arm64.dmg）。这里就按点分隔生成，使本地文件名、sha256sums.txt
# 里记录的名字、Release 资产名三者一致——否则下载端按校验和文件核对会报"文件不存在"。
DMG_NAME="$(printf '%s_%s_%s.dmg' "$APP_NAME" "$SHORT_VER" "$(uname -m)" | tr ' ' '.')"
DMG="$OUT_DIR/$DMG_NAME"
rm -f "$DMG"
echo "==> 生成 dmg: $DMG"
hdiutil create -volname "$APP_NAME" -srcfolder "$STAGE" -ov -format UDZO -quiet "$DMG"

( cd "$OUT_DIR" && shasum -a 256 "$DMG_NAME" > sha256sums.txt && cat sha256sums.txt )
echo "==> 完成：$OUT_DIR"
