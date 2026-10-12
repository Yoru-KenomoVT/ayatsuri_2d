#!/usr/bin/env bash
# Build and package Ayatsuri2D for macOS (runs on a Mac, e.g. the GitHub
# Actions macOS runner):
#
#   $DIST/ayatsuri2d-<version>-macos-universal.dmg   Ayatsuri2D.app, Apple Silicon + Intel
#
# Usage: packaging/macos/package.sh [--skip-build] [--arch universal|arm64|x86_64]
# Needs: Xcode command line tools (lipo, codesign, iconutil, sips, hdiutil)
# and the rustup targets aarch64-apple-darwin / x86_64-apple-darwin.
# The app is signed ad hoc, not notarized: on first launch users right-click
# the app and choose Open (or allow it under Privacy & Security).
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/macos"

SKIP_BUILD=0
ARCH=universal
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --arch) ARCH="$2"; shift 2 ;;
    -h | --help) sed -n '2,12p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$ARCH" in
  universal) TARGETS="aarch64-apple-darwin x86_64-apple-darwin" ;;
  arm64) TARGETS="aarch64-apple-darwin" ;;
  x86_64) TARGETS="x86_64-apple-darwin" ;;
  *) echo "unknown arch $ARCH" >&2; exit 2 ;;
esac
export MACOSX_DEPLOYMENT_TARGET=11.0

BASENAME="ayatsuri2d-$VERSION-macos-$ARCH"
echo "==> Ayatsuri2D $VERSION for macOS $ARCH"
if [ "$SKIP_BUILD" = 0 ]; then
  for t in $TARGETS; do
    (cd "$ROOT" && cargo build --release --locked --target "$t" -p ayatsuri -p ayatsuri-launcher)
  done
fi

WORK="$CARGO_TARGET_DIR/macos-package"
APP="$WORK/Ayatsuri2D.app"
rm -rf "$WORK"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

# one binary per program, holding every requested architecture
for exe in ayatsuri ayatsuri-launcher; do
  inputs=()
  for t in $TARGETS; do inputs+=("$CARGO_TARGET_DIR/$t/release/$exe"); done
  lipo -create -output "$APP/Contents/MacOS/$exe" "${inputs[@]}"
  strip -x "$APP/Contents/MacOS/$exe"
done

sed -e "s/@APP_ID@/$APP_ID/g" -e "s/@VERSION@/$VERSION/g" "$HERE/Info.plist.in" >"$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"
printf 'APPL????' >"$APP/Contents/PkgInfo"

# the icon set comes from the 1024 px master
ICONSET="$WORK/ayatsuri2d.iconset"
mkdir -p "$ICONSET"
SRC="$ROOT/assets/app-icon/ayatsuri2d-1024.png"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$SRC" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" "$SRC" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns -o "$APP/Contents/Resources/ayatsuri2d.icns" "$ICONSET"
copy_docs "$APP/Contents/Resources"

# ad hoc signature: Apple Silicon refuses to run unsigned code at all
codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"

# disk image with the usual drag-to-Applications layout
DMGROOT="$WORK/dmg"
mkdir -p "$DMGROOT"
cp -R "$APP" "$DMGROOT/"
ln -s /Applications "$DMGROOT/Applications"
cp "$ROOT/LICENSE" "$DMGROOT/LICENSE.txt"
rm -f "$DIST/$BASENAME.dmg"
hdiutil create -volname "Ayatsuri2D $VERSION" -srcfolder "$DMGROOT" -fs HFS+ -format UDZO -ov "$DIST/$BASENAME.dmg"
echo "wrote $DIST/$BASENAME.dmg"
