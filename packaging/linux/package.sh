#!/usr/bin/env bash
# Build and package Ayatsuri2D for Linux (<arch> is x86_64 or aarch64):
#
#   $DIST/ayatsuri2d-<version>-linux-<arch>.tar.gz    plain tree (bin/, share/), always
#   $DIST/ayatsuri2d-<version>-linux-<arch>.deb       Debian, Ubuntu, Mint, ...   (needs nfpm)
#   $DIST/ayatsuri2d-<version>-linux-<arch>.rpm       Fedora, openSUSE, ...      (needs nfpm)
#   $DIST/ayatsuri2d-<version>-linux-<arch>.AppImage  any distro                 (needs appimagetool)
#
# Usage: packaging/linux/package.sh [--skip-build] [--formats "tar deb rpm appimage"]
# Formats whose tool is missing are skipped with a note, so a plain dev box still gets the tar.
# Build on an old distro (CI: Ubuntu 22.04, glibc 2.35) so the binaries run on newer ones.
set -euo pipefail
. "$(dirname "${BASH_SOURCE[0]}")/../env.sh"
HERE="$ROOT/packaging/linux"

SKIP_BUILD=0
FORMATS="tar deb rpm appimage"
while [ $# -gt 0 ]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --formats) FORMATS="$2"; shift 2 ;;
    -h | --help) sed -n '2,12p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

ARCH="$(uname -m)"
case "$ARCH" in
  x86_64) NFPM_ARCH=amd64 ;;
  aarch64 | arm64) ARCH=aarch64; NFPM_ARCH=arm64 ;;
  *) echo "unsupported architecture $ARCH" >&2; exit 2 ;;
esac
export NFPM_ARCH
BASENAME="ayatsuri2d-$VERSION-linux-$ARCH"
echo "==> Ayatsuri2D $VERSION for Linux $ARCH ($FORMATS)"

if [ "$SKIP_BUILD" = 0 ]; then
  (cd "$ROOT" && cargo build --release --locked -p ayatsuri -p ayatsuri-launcher)
fi
BIN="$CARGO_TARGET_DIR/release"
WORK="$CARGO_TARGET_DIR/linux-package"
STAGE="$WORK/root"
rm -rf "$WORK"

# ---- stage an FHS tree, shared by every format ----
install -Dm755 "$BIN/ayatsuri" "$STAGE/usr/bin/ayatsuri"
install -Dm755 "$BIN/ayatsuri-launcher" "$STAGE/usr/bin/ayatsuri-launcher"
strip "$STAGE/usr/bin/ayatsuri" "$STAGE/usr/bin/ayatsuri-launcher" 2>/dev/null || true
install -Dm644 "$HERE/$APP_ID.desktop" "$STAGE/usr/share/applications/$APP_ID.desktop"
install -Dm644 "$HERE/$APP_ID.mime.xml" "$STAGE/usr/share/mime/packages/$APP_ID.xml"
mkdir -p "$STAGE/usr/share/metainfo"
sed -e "s/@VERSION@/$VERSION/g" -e "s/@DATE@/$(date -u +%Y-%m-%d)/g" \
  "$HERE/$APP_ID.metainfo.xml.in" >"$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml"
mkdir -p "$STAGE/usr/share/icons"
cp -R "$ROOT/assets/app-icon/hicolor" "$STAGE/usr/share/icons/"
mkdir -p "$STAGE/usr/share/doc/ayatsuri2d"
copy_docs "$STAGE/usr/share/doc/ayatsuri2d"

command -v desktop-file-validate >/dev/null && desktop-file-validate "$STAGE/usr/share/applications/$APP_ID.desktop"
command -v appstreamcli >/dev/null && appstreamcli validate --no-net --explain "$STAGE/usr/share/metainfo/$APP_ID.metainfo.xml" || true

has() { case " $FORMATS " in *" $1 "*) return 0 ;; *) return 1 ;; esac; }

if has tar; then
  mkdir -p "$WORK/tar"
  cp -R "$STAGE/usr" "$WORK/tar/$BASENAME"
  install -m755 "$HERE/install.sh" "$WORK/tar/$BASENAME/install.sh"
  tar -C "$WORK/tar" -czf "$DIST/$BASENAME.tar.gz" "$BASENAME"
  echo "wrote $DIST/$BASENAME.tar.gz"
fi

for fmt in deb rpm; do
  has "$fmt" || continue
  if ! command -v nfpm >/dev/null; then
    echo "note: nfpm not found, skipping .$fmt"
    continue
  fi
  sed "s|\${STAGE}|$STAGE|g" "$HERE/nfpm.yaml" >"$WORK/nfpm.yaml"
  nfpm package -f "$WORK/nfpm.yaml" -p "$fmt" -t "$DIST/$BASENAME.$fmt"
  echo "wrote $DIST/$BASENAME.$fmt"
done

if has appimage; then
  TOOL="$(command -v appimagetool || true)"
  if [ -z "$TOOL" ] && [ -n "${APPIMAGETOOL_URL:-}" ]; then
    TOOL="$CARGO_TARGET_DIR/appimagetool"
    [ -x "$TOOL" ] || { curl -fsSL -o "$TOOL" "$APPIMAGETOOL_URL" && chmod +x "$TOOL"; }
  fi
  if [ -z "$TOOL" ]; then
    echo "note: appimagetool not found (set APPIMAGETOOL_URL to fetch it), skipping .AppImage"
  else
    APPDIR="$WORK/Ayatsuri2D.AppDir"
    mkdir -p "$APPDIR"
    cp -R "$STAGE/usr" "$APPDIR/usr"
    cp "$HERE/$APP_ID.desktop" "$APPDIR/$APP_ID.desktop"
    cp "$ROOT/assets/app-icon/hicolor/256x256/apps/ayatsuri2d.png" "$APPDIR/ayatsuri2d.png"
    cat >"$APPDIR/AppRun" <<'RUN'
#!/bin/sh
# the launcher starts the editor from beside itself
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/ayatsuri-launcher" "$@"
RUN
    chmod +x "$APPDIR/AppRun"
    ARCH="$ARCH" APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --no-appstream "$APPDIR" "$DIST/$BASENAME.AppImage"
    echo "wrote $DIST/$BASENAME.AppImage"
  fi
fi
