#!/usr/bin/env bash
#
# Checks what the installer you just built actually carries.
#
#   npm run tauri build          && ./scripts/verify-installer.sh
#   npm run tauri:build:ai       && ./scripts/verify-installer.sh --ai
#
# Reads the NSIS script Tauri generated for that build — every file the
# installer packs is a `File` line in it — and requires exactly the expected
# set: nothing missing, nothing extra.
#
# Both directions have bitten. The AI build is configured by laying
# `tauri.ai.conf.json` over the base config, and had it replaced rather than
# merged the resource list, the installer most people use would have shipped
# without `libraw.dll` and could not develop a raw. It did not — but the
# same check found the installer shipping `export_bindings.exe`, a developer
# tool, because Tauri bundles every binary target of the crate. Those are now
# behind the `dev-tools` feature. `tests/bundle_resources.rs` guards the
# configuration on every test run; this guards the artifact.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
nsi="$here/src-tauri/target/release/nsis/x64/installer.nsi"

ai=0
for arg in "$@"; do
  case "$arg" in
    --ai) ai=1 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

[ -f "$nsi" ] || { echo "no installer script at $nsi — build first" >&2; exit 1; }

expected=(
  COPYRIGHT.LibRaw.txt
  LICENSE.LibRaw.txt
  LICENSE.txt
  NOTICE.txt
  THIRD-PARTY-LICENSES.txt
  libraw.dll
)
if [ "$ai" = 1 ]; then
  expected+=(
    DirectML.dll
    LICENSE.DirectML.txt
    LICENSE.OnnxRuntime.txt
    ThirdPartyNotices.DirectML.txt
    ThirdPartyNotices.OnnxRuntime.txt
    onnxruntime.dll
    models/faces/mesh.onnx
    models/faces/blendshapes.onnx
    models/faces/yunet.onnx
  )
fi

# Every packed file is `File /a "/oname=<name>" "<source>"`. The main
# executable is a separate `File "${MAINBINARYSRCPATH}"` line, and the WebView2
# bootstrapper goes to $TEMP rather than the install directory — neither is a
# resource, so neither is matched here.
# A resource in a subfolder is named with Windows separators; compare with `/`.
actual=$(grep -o 'File /a "/oname=[^"]*"' "$nsi" | sed 's/.*oname=//; s/"$//; s|\\|/|g' | sort)
want=$(printf '%s\n' "${expected[@]}" | sort)

missing=$(comm -23 <(echo "$want") <(echo "$actual"))
extra=$(comm -13 <(echo "$want") <(echo "$actual"))

# The ONNX Runtime and DirectML DLLs packed are the builds
# vendor/onnxruntime/VERSION records, signed by Microsoft. Matching the
# file name alone let any onnxruntime.dll through, to be signed into every
# user's copy by the updater key.
if [ "$ai" = 1 ] && [ -z "$missing" ]; then
  record() { grep -m1 "^$1=" "$here/src-tauri/vendor/onnxruntime/VERSION" | cut -d= -f2-; }
  for pair in "onnxruntime.dll:onnxruntime_dll_sha256" "DirectML.dll:directml_dll_sha256"; do
    name="${pair%%:*}"
    want=$(record "${pair#*:}")
    src=$(grep -o "File /a \"/oname=$name\" \"[^\"]*\"" "$nsi" | sed 's/.*" "//; s/"$//' | head -1)
    [ -n "$src" ] && [ -f "$src" ] || { echo "cannot find the $name the installer packs ($src)"; exit 1; }
    got=$(sha256sum < "$src" | cut -d' ' -f1)
    if [ "$got" != "$want" ]; then
      echo "$name is not the recorded build: sha256 $got, VERSION says $want"
      exit 1
    fi
    if command -v powershell >/dev/null 2>&1; then
      signer=$(powershell -NoProfile -Command "\$s = Get-AuthenticodeSignature -LiteralPath '$src'; if (\$s.Status -eq 'Valid') { \$s.SignerCertificate.Subject }")
      case "$signer" in
        *"O=Microsoft Corporation"*) ;;
        *) echo "$name is not validly signed by Microsoft (${signer:-no valid signature})"; exit 1 ;;
      esac
    fi
  done
  echo "ONNX Runtime and DirectML are the recorded, Microsoft-signed builds"
fi

if [ -z "$missing" ] && [ -z "$extra" ]; then
  echo "installer carries exactly the expected ${#expected[@]} files$([ "$ai" = 1 ] && echo ' (AI build)')"
  exit 0
fi
[ -n "$missing" ] && printf 'MISSING from the installer:\n%s\n' "$(echo "$missing" | sed 's/^/  /')"
[ -n "$extra" ] && printf 'NOT EXPECTED in the installer:\n%s\n' "$(echo "$extra" | sed 's/^/  /')"
if [ "$ai" = 0 ] && echo "$actual" | grep -qx 'onnxruntime.dll'; then
  echo "(this looks like an AI build — did you mean --ai?)"
fi
exit 1
