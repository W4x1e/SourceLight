#!/usr/bin/env bash
#
# Fetches the real raw files the decode tests need.
#
#   ./scripts/fetch-raw-fixtures.sh [dest]
#
# Then point the tests at them:
#
#   SOURCELIGHT_TEST_RAW=<dest> cargo test --manifest-path src-tauri/Cargo.toml
#
# Idempotent: a file already present with the right digest is left alone, so
# this is cheap to run on every build and safe to point at a warm cache.
#
# Every download is verified against the digest in the manifest before it is
# accepted, for the same reason the app verifies the AI models it downloads —
# an unverified fetch over the network is a supply chain, whether or not anyone
# calls it one. A file that fails is deleted rather than left to be picked up
# by the next run.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
manifest="$here/src-tauri/tests/fixtures/raw-samples.tsv"
dest="${1:-$here/src-tauri/tests/fixtures/raws}"

[ -f "$manifest" ] || { echo "no manifest at $manifest" >&2; exit 1; }

# Trailing whitespace is stripped from every line before parsing, because git's
# autocrlf hands this file back with CRLF endings on a Windows checkout and the
# carriage return then lands inside the URL — where curl rejects it as
# "malformed input to a URL function", a message that says nothing whatsoever
# about line endings. .gitattributes pins the manifest to LF as well; this is
# the belt to that pair of braces, and it is the one that survives someone
# cloning with different settings.
mkdir -p "$dest"

# `shasum -a 256` on macOS, `sha256sum` elsewhere; both ship on GitHub runners.
digest() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

have=0 got=0 failed=0
while IFS=$'\t' read -r name size sha url; do
  # Skip comments and blank lines.
  case "${name:-}" in ''|\#*) continue ;; esac
  out="$dest/$name"

  if [ -f "$out" ] && [ "$(digest "$out")" = "$sha" ]; then
    have=$((have + 1))
    continue
  fi

  printf 'fetching %s (%s MB)... ' "$name" "$((size / 1048576))"
  if ! curl -fsSL --retry 3 --retry-delay 2 -o "$out.part" "$url"; then
    echo "download failed"
    rm -f "$out.part"
    failed=$((failed + 1))
    continue
  fi

  actual="$(digest "$out.part")"
  if [ "$actual" != "$sha" ]; then
    echo "DIGEST MISMATCH"
    echo "  expected $sha" >&2
    echo "  actual   $actual" >&2
    echo "  The upstream file changed, or the download was tampered with." >&2
    echo "  Verify the source before updating the manifest." >&2
    rm -f "$out.part"
    failed=$((failed + 1))
    continue
  fi

  # Only named once it is known-good, so an interrupted run cannot leave a
  # partial file that looks complete to the next one.
  mv "$out.part" "$out"
  echo "ok"
  got=$((got + 1))
done < <(sed "s/[[:space:]]*$//" "$manifest")

echo "raw fixtures: $have cached, $got fetched, $failed failed -> $dest"
[ "$failed" -eq 0 ]
