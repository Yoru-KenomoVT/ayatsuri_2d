#!/usr/bin/env bash
# Build and package Ayatsuri2D for 64-bit Windows, from Linux (or anywhere
# with mingw-w64 and NSIS):
#
#   $DIST/ayatsuri2d-<version>-windows-x64-setup.exe   installer (NSIS)
#   $DIST/ayatsuri2d-<version>-windows-x64.zip         portable, unzip and run
#
# Usage: packaging/windows/package.sh [--skip-build]
# Needs: rustup target x86_64-pc-windows-gnu, mingw-w64 (gcc, windres, strip),
# makensis. On Debian/Ubuntu: apt install mingw-w64 nsis.
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/windows"
TARGET=x86_64-pc-windows-gnu

SKIP_BUILD=0
case "${1:-}" in --skip-build) SKIP_BUILD=1 ;; "" ) ;; *) echo "unknown argument: $1" >&2; exit 2 ;; esac
for tool in makensis x86_64-w64-mingw32-gcc x86_64-w64-mingw32-strip; do
  command -v "$tool" >/dev/null || { echo "error: $tool not found" >&2; exit 1; }
done

BASENAME="ayatsuri2d-$VERSION-windows-x64"
echo "==> Ayatsuri2D $VERSION for Windows x64"
if [ "$SKIP_BUILD" = 0 ]; then
  (cd "$ROOT" && cargo build --release --locked --target "$TARGET" -p ayatsuri -p ayatsuri-launcher)
fi
BIN="$CARGO_TARGET_DIR/$TARGET/release"
STAGE="$CARGO_TARGET_DIR/windows-package/$BASENAME"
rm -rf "$CARGO_TARGET_DIR/windows-package"
mkdir -p "$STAGE"
for exe in ayatsuri ayatsuri-launcher; do
  x86_64-w64-mingw32-strip -o "$STAGE/$exe.exe" "$BIN/$exe.exe"
done
cp "$ROOT/assets/app-icon/ayatsuri2d.ico" "$STAGE/"
# Windows wants CRLF and a .txt to show the licence in the installer
sed 's/$/\r/' "$ROOT/LICENSE" >"$STAGE/LICENSE.txt"
cp "$ROOT/LICENSING.md" "$ROOT/README.md" "$ROOT/assets/credits.tsv" "$STAGE/"

(cd "$(dirname "$STAGE")" && rm -f "$DIST/$BASENAME.zip" && zip -qr "$DIST/$BASENAME.zip" "$BASENAME")
echo "wrote $DIST/$BASENAME.zip"

makensis -V2 -INPUTCHARSET UTF8 \
  -DVERSION="$VERSION" -DNUMVER="$NUMVER" -DSRC="$STAGE" \
  -DOUTFILE="$DIST/$BASENAME-setup.exe" "$HERE/ayatsuri2d.nsi"
echo "wrote $DIST/$BASENAME-setup.exe"
