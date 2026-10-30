#!/usr/bin/env bash
#
# Fetches the ONNX Runtime that the Windows AI build loads (`--features ai`)
# into src-tauri/vendor/onnxruntime/. README.md there says what each file is.
#
#   ./scripts/fetch-ort.sh [dest]
#
# It downloads two NuGet packages (about 220 MB), keeps the two DLLs and their
# notices (about 35 MB), and deletes the packages. Each DLL must match the
# SHA-256 in vendor/onnxruntime/VERSION, which also names the package versions.
#
# Idempotent: a package whose DLL is present with the right digest, and whose
# notices are present, is not downloaded again.
#
# Windows x64 only. The Mac's runtime comes from a release archive instead
# (README.md, "macOS").
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
record="$here/src-tauri/vendor/onnxruntime/VERSION"
dest="${1:-$here/src-tauri/vendor/onnxruntime}"

[ -f "$record" ] || { echo "no record at $record" >&2; exit 1; }
command -v unzip >/dev/null 2>&1 || { echo "needs unzip (Git for Windows has it)" >&2; exit 1; }

# A value from VERSION. The tr drops the carriage return a CRLF checkout adds.
value() {
  local v
  v=$(sed -n "s/^$1=//p" "$record" | tr -d '\r')
  [ -n "$v" ] || { echo "no $1= line in $record" >&2; exit 1; }
  printf '%s' "$v"
}

# `shasum -a 256` on macOS, `sha256sum` elsewhere.
digest() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# fetch <"Id Version ..." from VERSION> <sha256> <DLL in package>=<name> <notice in package>=<name>...
fetch() {
  local id version pkg_url pkg actual pair want="$2" dll_pair="$3"
  read -r id version _ <<<"$1"
  shift 3
  local dll="${dll_pair#*=}"

  local complete=1
  if [ -f "$dest/$dll" ] && [ "$(digest "$dest/$dll")" != "$want" ]; then
    echo "replacing $dll: it is not the build VERSION records"
    complete=0
  fi
  [ -f "$dest/$dll" ] || complete=0
  for pair in "$@"; do
    [ -f "$dest/${pair#*=}" ] || complete=0
  done
  if [ "$complete" = 1 ]; then
    echo "$id $version: already present"
    return
  fi

  # The flat-container URL, which nuget.org serves to a plain HTTP client.
  local lower
  lower=$(printf '%s' "$id" | tr '[:upper:]' '[:lower:]')
  pkg_url="https://api.nuget.org/v3-flatcontainer/$lower/$version/$lower.$version.nupkg"
  pkg="$tmp/$lower.$version.nupkg"
  printf 'fetching %s %s... ' "$id" "$version"
  curl -fsSL --retry 3 --retry-delay 2 -o "$pkg" "$pkg_url"
  echo "$(wc -c < "$pkg" | tr -d ' ') bytes"

  # A .nupkg is a zip. Extract only the named files, each into a fresh folder.
  local out="$tmp/$lower"
  local inside=()
  for pair in "$@"; do inside+=("${pair%%=*}"); done
  unzip -q "$pkg" "${dll_pair%%=*}" "${inside[@]}" -d "$out"
  rm -f "$pkg"

  actual=$(digest "$out/${dll_pair%%=*}")
  if [ "$actual" != "$want" ]; then
    echo "DIGEST MISMATCH for $dll from $id $version" >&2
    echo "  expected $want" >&2
    echo "  actual   $actual" >&2
    echo "  The package changed upstream, or the download was tampered with." >&2
    echo "  Verify the source before updating VERSION." >&2
    exit 1
  fi

  mkdir -p "$dest"
  for pair in "$@"; do
    cp "$out/${pair%%=*}" "$dest/${pair#*=}"
  done
  # Named only once it is in place, so an interrupted copy is never taken for
  # a complete one.
  cp "$out/${dll_pair%%=*}" "$dest/$dll.part"
  mv -f "$dest/$dll.part" "$dest/$dll"
  rm -rf "$out"
  echo "$dll: matches VERSION"
}

# Read first: a failed lookup inside a command's arguments would not stop the script.
ort_package=$(value package)
ort_sha=$(value onnxruntime_dll_sha256)
dml_package=$(value directml_package)
dml_sha=$(value directml_dll_sha256)
version=$(value version)

fetch "$ort_package" "$ort_sha" \
  runtimes/win-x64/native/onnxruntime.dll=onnxruntime.dll \
  LICENSE=LICENSE.OnnxRuntime.txt \
  ThirdPartyNotices.txt=ThirdPartyNotices.OnnxRuntime.txt
fetch "$dml_package" "$dml_sha" \
  bin/x64-win/DirectML.dll=DirectML.dll \
  LICENSE.txt=LICENSE.DirectML.txt \
  ThirdPartyNotices.txt=ThirdPartyNotices.DirectML.txt

echo "ONNX Runtime $version -> $dest"
