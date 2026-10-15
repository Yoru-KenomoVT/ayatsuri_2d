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

# MP4, WebM and MOV export need ffmpeg. With none on PATH, offer to fetch
# one for Ayatsuri2D only (into its settings folder, checked against the
# builder's published checksum): no root, nothing added to PATH. A
# system-wide install leaves it to each user; the editor offers it at start.
get_ffmpeg() {
  command -v ffmpeg >/dev/null 2>&1 && [ -z "${AYATSURI_PRETEND_NO_FFMPEG:-}" ] && return 0
  conf="${XDG_CONFIG_HOME:-$HOME/.config}/ayatsuri2d/ffmpeg/ffmpeg"
  [ -x "$conf" ] && return 0
  if [ "$(id -u)" = 0 ]; then
    echo "note: no ffmpeg found; each user can install it from Ayatsuri2D, or install your distro's ffmpeg package"
    return 0
  fi
  answer=y
  if [ -t 0 ]; then
    printf "MP4, WebM and MOV export need ffmpeg, which is not installed.\nDownload it for Ayatsuri2D now (about 30 MB, no root, nothing added to PATH)? [Y/n] "
    read -r answer || answer=n
  fi
  case "$answer" in
    "" | y | Y | yes | Yes)
      if "$PREFIX/bin/ayatsuri-launcher" --get-ffmpeg; then
        :
      else
        echo "note: ffmpeg could not be downloaded; Ayatsuri2D will offer it again when it starts"
      fi
      ;;
    *) echo "skipped; Ayatsuri2D will offer ffmpeg when it starts" ;;
  esac
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
  get_ffmpeg
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
