#!/usr/bin/env bash
# Regenerate the app icon set in assets/app-icon from assets/icon-dark.svg:
# the glyph on a dark rounded tile, so it reads on light and dark desktops.
#
#   hicolor/<N>x<N>/apps/ayatsuri2d.png   Linux (freedesktop icon theme)
#   ayatsuri2d.ico                        Windows (exe resource, installer)
#   ayatsuri2d-1024.png                   macOS (package.sh turns it into .icns)
#   ayatsuri2d-256.png                    the window icon the app sets at runtime
#
# Needs rsvg-convert and ImageMagick (magick). Run after changing the logo.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/assets/app-icon"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$OUT"

rsvg-convert -w 720 -h 720 "$ROOT/assets/icon-dark.svg" -o "$TMP/glyph.png"
magick -size 1024x1024 xc:none -fill '#17161b' \
  -draw "roundrectangle 64,64 959,959 200,200" \
  "$TMP/glyph.png" -gravity center -composite "$OUT/ayatsuri2d-1024.png"

for n in 16 24 32 48 64 128 256 512; do
  mkdir -p "$OUT/hicolor/${n}x${n}/apps"
  magick "$OUT/ayatsuri2d-1024.png" -filter Lanczos -resize "${n}x${n}" \
    "$OUT/hicolor/${n}x${n}/apps/ayatsuri2d.png"
done
cp "$OUT/hicolor/256x256/apps/ayatsuri2d.png" "$OUT/ayatsuri2d-256.png"
magick "$OUT/ayatsuri2d-1024.png" -filter Lanczos -define icon:auto-resize=256,128,64,48,32,24,16 \
  "$OUT/ayatsuri2d.ico"
echo "icons written to $OUT"
