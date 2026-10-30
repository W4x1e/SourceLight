#!/usr/bin/env bash
#
# Builds src-tauri/icons/icon.icns — the macOS app icon — from app-icon.png,
# the 1024 px rendering of app-icon.svg.
#
#   ./scripts/make-macos-icon.sh
#
# A Mac icon is not the artwork edge to edge: it is Apple's rounded-rect shape,
# 824 px inset on a 1024 canvas, with transparent corners and a soft shadow.
# The artwork as a full-bleed square is what macOS puts on a grey plate in the
# Dock, the look of an app that was not made for it. This draws the artwork
# into that shape (make-macos-icon.swift) and packs every size macOS asks for.
set -euo pipefail
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
swiftc -O scripts/make-macos-icon.swift -o "$work/make-icon"
"$work/make-icon" app-icon.png "$work/icon-1024.png"

set_dir="$work/icon.iconset"
mkdir "$set_dir"
for size in 16 32 128 256 512; do
  sips -z $size $size "$work/icon-1024.png" --out "$set_dir/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z $double $double "$work/icon-1024.png" --out "$set_dir/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$set_dir" -o src-tauri/icons/icon.icns
echo "wrote src-tauri/icons/icon.icns"
