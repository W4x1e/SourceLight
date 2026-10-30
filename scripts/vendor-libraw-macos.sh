#!/usr/bin/env bash
#
# Builds the macOS LibRaw — src-tauri/vendor/libraw/lib/aarch64-apple-darwin/libraw.dylib —
# from LibRaw's own source release, and prints the lines vendor/libraw/VERSION
# records for it. See docs/LIBRAW.md.
#
#   ./scripts/vendor-libraw-macos.sh            # the version VERSION names
#
# Why a build and not LibRaw's macOS download, as Windows uses theirs: that
# package carries static archives only, and a static link would make the app a
# derived work of an LGPL library (see build.rs). Its objects are also built for
# macOS 26, whatever the download page says, while the app supports 14. So this
# compiles the same source release with upstream's own makefile and flags
# (Makefile.dist: -O3, zlib for deflated DNGs, the thread-safe `_r` objects),
# only pinned to the app's architecture and minimum macOS, and links the result
# as a dylib that loads through @rpath from Contents/Frameworks.
#
# The build is reproducible with one toolchain: the same Xcode gives the same
# bytes. A different Xcode may not, which is why VERSION records the dylib's
# hash as well as the source's, and the tests check the one that is committed.
set -euo pipefail

cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

record() { grep "^$1=" src-tauri/vendor/libraw/VERSION | cut -d= -f2; }
version=$(record version)
tarball="LibRaw-${version}.tar.gz"
minos=$(sed -n 's/.*"minimumSystemVersion": "\([0-9.]*\)".*/\1/p' src-tauri/tauri.macos.conf.json)
[ -n "$minos" ] || { echo "no minimumSystemVersion in tauri.macos.conf.json" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

echo "fetching $tarball..."
curl -fsSL --retry 3 -o "$work/$tarball" "https://www.libraw.org/data/$tarball"
got=$(shasum -a 256 "$work/$tarball" | cut -d' ' -f1)
want=$(record macos_source_sha256 || true)
if [ -n "$want" ] && [ "$got" != "$want" ]; then
  echo "$tarball is not the one VERSION records ($got, want $want)" >&2
  exit 1
fi

mkdir "$work/src"
tar -xzf "$work/$tarball" -C "$work/src" --strip-components=1
src="$work/src"

# The bindings are generated from the vendored headers, so they must be the
# ones this dylib is built from.
if ! diff -rq "$src/libraw" src-tauri/vendor/libraw/include/libraw >/dev/null; then
  echo "the vendored headers differ from $tarball's — roll them forward too" >&2
  exit 1
fi

echo "building for arm64, macOS $minos..."
make -s -C "$src" -j"$(sysctl -n hw.ncpu)" -f Makefile.dist lib/libraw_r.a \
  CC=clang CXX=clang++ \
  CFLAGS="-O3 -I. -w -DUSE_ZLIB -arch arm64 -mmacosx-version-min=$minos" >/dev/null

shlib=$(awk '/#define LIBRAW_SHLIB_CURRENT/ {print $3}' "$src/libraw/libraw_version.h")
out=src-tauri/vendor/libraw/lib/aarch64-apple-darwin
mkdir -p "$out"
clang++ -dynamiclib -arch arm64 -mmacosx-version-min="$minos" \
  -Wl,-all_load "$src/lib/libraw_r.a" \
  -install_name @rpath/libraw.dylib \
  -current_version "$shlib.0.0" -compatibility_version "$shlib.0.0" \
  -lz -o "$out/libraw.dylib"

echo
echo "wrote $out/libraw.dylib. Record in vendor/libraw/VERSION:"
echo "macos_package=$tarball"
echo "macos_source_sha256=$got"
echo "dylib_sha256=$(shasum -a 256 "$out/libraw.dylib" | cut -d' ' -f1)"
