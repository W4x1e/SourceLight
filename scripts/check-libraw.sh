#!/usr/bin/env bash
#
# The LibRaw tripwire.
#
#   ./scripts/check-libraw.sh          # is the vendored LibRaw the newest release?
#   ./scripts/check-libraw.sh --link   # ...and is LibRaw still a dynamic import?
#
# `cargo deny check advisories` cannot see libraw.dll: it is a vendored native
# binary, not a crate. This compares the version recorded next to the DLL with
# the newest upstream release tag and fails when we have fallen behind, which is
# the moment to read docs/LIBRAW.md and decide whether to roll forward.
set -uo pipefail

cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

RECORD=src-tauri/vendor/libraw/VERSION
CHECK_LINK=0
for arg in "$@"; do
  case "$arg" in
    --link) CHECK_LINK=1 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

field() { sed -n "s/^$1=//p" "$RECORD" | tr -d '\r'; }

vendored=$(field version)
if [ -z "$vendored" ]; then
  echo "no version= line in $RECORD" >&2
  exit 1
fi

# Tags look like 0.22.2, and occasionally a lettered re-release such as 0.21.5b,
# which upstream uses for security fixes. Pre-release and snapshot tags are
# skipped. `sort -V` orders 0.21.5 < 0.21.5b < 0.22.0, which is what we want.
tags=$(curl -fsSL "https://api.github.com/repos/LibRaw/LibRaw/tags?per_page=100" 2>/dev/null \
  | sed -n 's/.*"name": *"\([0-9][0-9.]*[a-z]\{0,1\}\)".*/\1/p')
if [ -z "$tags" ]; then
  echo "could not read LibRaw's tags from GitHub (offline, or rate-limited)" >&2
  exit 3
fi
latest=$(printf '%s\n' $tags | sort -V | tail -1)

status=0
if [ "$latest" = "$vendored" ]; then
  echo "LibRaw $vendored is the newest upstream release."
elif [ "$(printf '%s\n%s\n' "$vendored" "$latest" | sort -V | tail -1)" = "$vendored" ]; then
  echo "LibRaw $vendored is ahead of the newest tag ($latest) — a snapshot build?"
else
  echo "LibRaw $vendored is BEHIND upstream $latest."
  echo "  Release notes: https://github.com/LibRaw/LibRaw/releases"
  echo "  Advisories:    https://github.com/advisories?query=libraw"
  echo "  Procedure:     docs/LIBRAW.md"
  status=1
fi

if [ "$CHECK_LINK" = 1 ]; then
  # The LGPL requires LibRaw to be replaceable, and a replaceable DLL is also
  # what lets a patched LibRaw ship without rebuilding the app. Both depend on
  # libraw.dll being an import rather than code linked into the binary.
  # Globs, not `ls`: an aliased `ls -F` would decorate the names.
  if [ "$(uname)" = Darwin ]; then
    exe=""
    for e in src-tauri/target/debug/deps/sourcelight_lib-*; do
      [ -f "$e" ] && [ -x "$e" ] && { [ -z "$exe" ] || [ "$e" -nt "$exe" ]; } && exe="$e"
    done
    if [ -z "$exe" ]; then
      echo "no test binary yet; run cargo test first" >&2
      status=1
    elif otool -L "$exe" | grep -q '@rpath/libraw\.dylib'; then
      echo "libraw.dylib is a dynamic import of $(basename "$exe")."
    else
      echo "libraw.dylib is NOT dynamically imported by $(basename "$exe") — LGPL compliance is broken." >&2
      echo "Check build.rs links dylib=raw against vendor/libraw/lib/aarch64-apple-darwin." >&2
      status=1
    fi
    exit $status
  fi
  dumpbin=""
  for d in "/c/Program Files"*/"Microsoft Visual Studio"/*/*/VC/Tools/MSVC/*/bin/Hostx64/x64/dumpbin.exe; do
    [ -f "$d" ] && dumpbin="$d" && break
  done
  exe=""
  for e in src-tauri/target/debug/deps/sourcelight_lib-*.exe; do
    [ -f "$e" ] && { [ -z "$exe" ] || [ "$e" -nt "$exe" ]; } && exe="$e"
  done
  # `-dependents`, not `/dependents`: Git Bash rewrites a leading slash into a path.
  if [ -z "$dumpbin" ]; then
    echo "dumpbin.exe not found; install the MSVC build tools" >&2
    status=1
  elif [ -z "$exe" ]; then
    echo "no test binary yet; run cargo test first" >&2
    status=1
  elif ! deps=$("$dumpbin" -nologo -dependents "$exe"); then
    echo "dumpbin failed on $exe" >&2
    status=1
  elif grep -qi 'libraw\.dll' <<<"$deps"; then
    echo "libraw.dll is a dynamic import of $(basename "$exe")."
  else
    echo "libraw.dll is NOT dynamically imported by $(basename "$exe") — LGPL compliance is broken." >&2
    echo "Check build.rs links libraw.lib with dylib=, not libraw_static.lib." >&2
    status=1
  fi
fi

exit $status
