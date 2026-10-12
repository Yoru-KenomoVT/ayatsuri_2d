# shellcheck shell=bash
# Shared setup for the packaging scripts. Source it: `. "$(dirname "$0")/../env.sh"`.
#
# Exports:
#   ROOT              workspace root
#   VERSION           [workspace.package] version from Cargo.toml (override: AYATSURI_VERSION)
#   NUMVER            VERSION as four numbers (0.5.0-beta -> 0.5.0.0), for Windows version info
#   DIST              output directory for release artifacts (default: $ROOT/dist/release)
#   APP_ID            the freedesktop / macOS bundle id
#   CARGO_TARGET_DIR  cargo's target dir (default: $ROOT/target)

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export ROOT

# the version lives in one place: [workspace.package] version in Cargo.toml
workspace_version() {
  awk '
    /^\[/ { in_pkg = ($0 == "[workspace.package]"); next }
    in_pkg && $1 == "version" { gsub(/[" ]/, "", $3); print $3; exit }
  ' "$ROOT/Cargo.toml"
}

VERSION="${AYATSURI_VERSION:-$(workspace_version)}"
if [ -z "$VERSION" ]; then
  echo "error: could not read [workspace.package] version from $ROOT/Cargo.toml" >&2
  exit 1
fi
export VERSION
NUMVER="$(printf '%s' "${VERSION%%-*}" | awk -F. '{ printf "%d.%d.%d.0", $1, $2, $3 }')"
export NUMVER

DIST="${DIST:-$ROOT/dist/release}"
mkdir -p "$DIST"
export DIST
export APP_ID=io.github.Yoru_KenomoVT.Ayatsuri2D
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

# licence and credits ride along in every package
copy_docs() {
  install -m644 "$ROOT/LICENSE" "$1/LICENSE"
  install -m644 "$ROOT/LICENSING.md" "$1/LICENSING.md"
  install -m644 "$ROOT/README.md" "$1/README.md"
  install -m644 "$ROOT/assets/credits.tsv" "$1/credits.tsv"
}
