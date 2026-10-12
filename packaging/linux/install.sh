#!/bin/sh
# Install Ayatsuri2D from this unpacked tarball.
#
#   ./install.sh                     for you only, into ~/.local
#   sudo ./install.sh --prefix /usr/local   for everyone
#   ./install.sh --uninstall [--prefix ...]
#
# Copies bin/ and share/ under the prefix, then refreshes the desktop, icon
# and file-type caches so the menu entry, the icon and .aom files work.
set -eu
HERE="$(cd "$(dirname "$0")" && pwd)"
PREFIX="${HOME}/.local"
UNINSTALL=0
while [ $# -gt 0 ]; do
  case "$1" in
    --prefix) PREFIX="$2"; shift 2 ;;
    --uninstall) UNINSTALL=1; shift ;;
    -h | --help) sed -n '2,10p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
APP_ID=io.github.Yoru_KenomoVT.Ayatsuri2D

files() {
  cd "$HERE" && find bin share -type f | sort
}

if [ "$UNINSTALL" = 1 ]; then
  for f in $(files); do rm -f "$PREFIX/$f"; done
  rmdir "$PREFIX/share/doc/ayatsuri2d" 2>/dev/null || true
  echo "removed Ayatsuri2D from $PREFIX"
else
  for f in $(files); do
    mkdir -p "$(dirname "$PREFIX/$f")"
    case "$f" in bin/*) mode=755 ;; *) mode=644 ;; esac
    install -m "$mode" "$HERE/$f" "$PREFIX/$f"
  done
  echo "installed Ayatsuri2D into $PREFIX"
  case ":$PATH:" in *":$PREFIX/bin:"*) ;; *) echo "note: $PREFIX/bin is not on your PATH" ;; esac
fi

# refresh the caches; each tool is optional
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$PREFIX/share/applications" || true
command -v update-mime-database >/dev/null 2>&1 && update-mime-database "$PREFIX/share/mime" || true
# only a themed folder (one with index.theme, as /usr/share has) can take a
# cache; a per-user hicolor folder without one is read directly
if [ -f "$PREFIX/share/icons/hicolor/index.theme" ] && command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q -t "$PREFIX/share/icons/hicolor" || true
fi
: "$APP_ID"
