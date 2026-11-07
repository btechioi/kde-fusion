#!/usr/bin/env bash
# Create the source tarball expected by packaging/PKGBUILD.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${1:-$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)}"
NAME="kde-fusion-$VERSION"
OUT="$ROOT/packaging/$NAME.tar.gz"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$TMP/$NAME"
cd "$ROOT"
for entry in Cargo.toml Cargo.lock LICENSE README.md src qml packaging docs systemd scripts; do
  [[ -e "$entry" ]] || continue
  cp -a "$entry" "$TMP/$NAME/"
done
rm -f "$TMP/$NAME/packaging/"*.tar.gz
tar -C "$TMP" -czf "$OUT" "$NAME"
echo "$OUT"
