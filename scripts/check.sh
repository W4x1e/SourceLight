#!/usr/bin/env bash
#
# Everything CI checks, on the machine that is already going to compile it
# anyway. Run it before opening a pull request.
#
#   ./scripts/check.sh            # the checks that gate a commit
#   ./scripts/check.sh --ai       # ...and the build that actually ships
#   ./scripts/check.sh --raw      # ...and every test against real raw files
#   ./scripts/check.sh --upstream # ...and is the vendored LibRaw still current?
#   ./scripts/check.sh --mobile   # ...and the phone app's Rust core (mobile/)
#
# CI (.github/workflows/ci.yml) runs most of these on every pull request and
# push to main, but a cold build there is about seventy minutes. Here the
# incremental build is already warm, and there is a hardware GPU, which hosted
# Windows runners lack, so the GPU tests actually run.
#
# The one thing this cannot replace is **macOS**, which only CI checks from a
# Windows machine. Keep that in mind before assuming a green run here means a
# green port.
set -uo pipefail

cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

WITH_AI=0
WITH_RAW=0
WITH_UPSTREAM=0
WITH_MOBILE=0
for arg in "$@"; do
  case "$arg" in
    --ai) WITH_AI=1 ;;
    --raw) WITH_RAW=1 ;;
    --upstream) WITH_UPSTREAM=1 ;;
    --mobile) WITH_MOBILE=1 ;;
    --all) WITH_AI=1; WITH_RAW=1; WITH_UPSTREAM=1; WITH_MOBILE=1 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

# LibRaw is vendored per target (Windows x64 and Apple silicon), and build.rs
# panics on a host that has none, such as an Intel Mac or Linux. There the
# checks run without it instead, and say so: raw decoding and Develop go
# unchecked in that run.
host=$(rustc -vV | sed -n 's/^host: //p')
libraw_flag=""
if [ ! -d "src-tauri/vendor/libraw/lib/$host" ]; then
  libraw_flag="--no-default-features"
  printf '\033[33mnote:\033[0m no vendored LibRaw for %s, so cargo runs with\n' "$host"
  printf '      --no-default-features: raw decoding is not built or tested here.\n'
fi
export libraw_flag

failed=()
step() {
  local name="$1"; shift
  printf '\n\033[1m== %s\033[0m\n' "$name"
  if "$@"; then
    printf '\033[32mok\033[0m  %s\n' "$name"
  else
    printf '\033[31mFAILED\033[0m  %s\n' "$name"
    failed+=("$name")
  fi
}

# The app holds the GPU adapter while it runs, which does not merely slow the
# GPU tests — it makes them look hung. Worth saying up front rather than after
# twenty minutes of wondering.
if tasklist 2>/dev/null | grep -qi "sourcelight.exe"; then
  printf '\033[33mwarning:\033[0m the app is running. Close it, or the GPU tests\n'
  printf '         will crawl on a software adapter and look like a hang.\n'
fi

# Tests that need something a checkout does not contain — real raws, a copy of
# a real catalogue, downloaded models, a hardware GPU — skip without it, and a
# skip used to report green, indistinguishable from a pass. Now every skip
# is logged and counted at the end, and a skip of anything this run *declares*
# it has is a failure: the GPU always (this is the machine with one; a GPU skip
# here means the app was left open and the tests fell back to a software
# rasteriser), and real raws with --raw. Set SOURCELIGHT_REQUIRE yourself to
# override — e.g. SOURCELIGHT_REQUIRE= on a machine with no GPU.
skip_log="$PWD/src-tauri/target/skipped-tests.tsv"
test_log="$PWD/src-tauri/target/test-output.log"
mkdir -p src-tauri/target
rm -f "$skip_log" "$test_log"
export SOURCELIGHT_SKIP_LOG="$skip_log"
declared="gpu"
[ "$WITH_RAW" = 1 ] && declared="gpu,raw"
export SOURCELIGHT_REQUIRE="${SOURCELIGHT_REQUIRE-$declared}"

# Real sensor data, fetched before any test runs so that *every* suite sees it —
# including the `ai` build below, which is the one that ships. Opt-in because it
# adds a few minutes of rendering, not because any of it is optional.
if [ "$WITH_RAW" = 1 ]; then
  step "fetch raw fixtures" ./scripts/fetch-raw-fixtures.sh
  export SOURCELIGHT_TEST_RAW="$PWD/src-tauri/tests/fixtures/raws"
fi

# Runs the tests with their output kept, so the report at the end can say how
# many of the tests that reported a pass actually ran.
run_tests() {
  cargo test --manifest-path src-tauri/Cargo.toml $libraw_flag "$@" 2>&1 | tee -a "$test_log"
  return "${PIPESTATUS[0]}"
}

step "typecheck"      npx tsc --noEmit
step "frontend tests" npm test
# `--all` takes in the phone link's crates in src-tauri/crates (docs/MOBILE.md).
step "rustfmt"        cargo fmt --manifest-path src-tauri/Cargo.toml --all --check
# `dev-tools` so the developer binaries (bindings generator, seeder) are linted
# too — without it `--all-targets` silently skips them.
# `phone-link` too, so the desktop's half of the link and its stand-in phone
# (`phone_sim`) are linted though no release builds them yet.
step "clippy"         cargo clippy --manifest-path src-tauri/Cargo.toml $libraw_flag --all-targets --features dev-tools,phone-link -- -D warnings
step "rust tests"     run_tests
# The phone link's crates are workspace members, not part of the app's
# package, so the two commands above do not reach them.
step "clippy (link)"  cargo clippy --manifest-path src-tauri/Cargo.toml -p sl-protocol -p sl-link --all-targets -- -D warnings
step "link tests"     run_tests -p sl-protocol -p sl-link
# The develop core is a workspace member too, with the GPU's golden renders.
step "clippy (develop)" cargo clippy --manifest-path src-tauri/Cargo.toml -p sl-develop --all-targets -- -D warnings
step "develop tests"  run_tests -p sl-develop

# `ai` is not a default feature, but it is the build in the installer — so it is
# the one whose breakage reaches a user first.
if [ "$WITH_AI" = 1 ]; then
  step "clippy (ai)" cargo clippy --manifest-path src-tauri/Cargo.toml $libraw_flag --all-targets --features ai -- -D warnings
  step "tests (ai)"  run_tests --features ai
fi

# The phone app's Rust core (docs/MOBILE.md) is its own Cargo project with its
# own target/, so a first run builds Tauri again for it. Opt-in for that
# reason. Its interface is already in the typecheck above (tsconfig includes
# mobile/src). Android and iOS builds are not checked here: see MOBILE.md.
if [ "$WITH_MOBILE" = 1 ]; then
  step "rustfmt (phone)" bash -c 'cargo fmt --manifest-path mobile/src-tauri/Cargo.toml --check &&
    cargo fmt --manifest-path mobile/plugins/shoot-track/Cargo.toml --check &&
    cargo fmt --manifest-path mobile/plugins/qr-scan/Cargo.toml --check &&
    cargo fmt --manifest-path mobile/plugins/gallery/Cargo.toml --check &&
    cargo fmt --manifest-path mobile/plugins/photos/Cargo.toml --check &&
    cargo fmt --manifest-path mobile/gpu-test/Cargo.toml --check'
  # The plugins are path dependencies, not workspace members: named with -p
  # so they are linted and tested too, in the app's own target/. sl-develop
  # is linted and tested with the desktop's crates above.
  phone_pkgs="-p sourcelight-mobile -p sl-gpu-test -p tauri-plugin-gallery -p tauri-plugin-photos -p tauri-plugin-qr-scan -p tauri-plugin-shoot-track"
  step "clippy (phone)"  cargo clippy --manifest-path mobile/src-tauri/Cargo.toml --all-targets $phone_pkgs -- -D warnings
  step "tests (phone)"   cargo test --manifest-path mobile/src-tauri/Cargo.toml $phone_pkgs
  step "dependency licences (phone)" bash -c '
    command -v cargo-deny >/dev/null 2>&1 || { echo "skipped: cargo install cargo-deny"; exit 0; }
    cargo deny --manifest-path mobile/src-tauri/Cargo.toml --config src-tauri/deny.toml check licenses bans sources advisories'
  # The notices the phone app shows under Open-source licences, regenerated
  # and compared, as the desktop's are.
  step "third-party notices are current (phone)" bash -c '
    command -v cargo-about >/dev/null 2>&1 || { echo "skipped: cargo install cargo-about --features cli"; exit 0; }
    tmp=$(mktemp)
    (cd mobile/src-tauri && cargo about generate --manifest-path Cargo.toml -c about.toml ../about.hbs -o "$tmp") || exit 1
    diff -q --strip-trailing-cr "$tmp" mobile/THIRD-PARTY-LICENSES.txt >/dev/null || {
      echo "mobile/THIRD-PARTY-LICENSES.txt is stale: regenerate it as mobile/src-tauri/about.toml says"; rm -f "$tmp"; exit 1; }
    rm -f "$tmp"'
fi

# LGPL compliance: LibRaw is usable from an application under other terms
# (MPL-2.0 here) only while it stays a separately replaceable shared library,
# and nothing else in the build would complain if that stopped being true.
if command -v powershell >/dev/null 2>&1; then
  step "LibRaw is dynamically linked" powershell -NoProfile -Command '
    $dumpbin = Get-ChildItem "C:\Program Files*\Microsoft Visual Studio\*\*\VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $dumpbin) { Write-Host "skipped: no dumpbin"; exit 0 }
    $exe = Get-ChildItem src-tauri/target/debug/deps/sourcelight_lib-*.exe | Sort-Object LastWriteTime | Select-Object -Last 1
    if (-not $exe) { Write-Host "skipped: no test binary built yet"; exit 0 }
    if ((& $dumpbin.FullName /dependents $exe.FullName) -match "libraw\.dll") { exit 0 }
    Write-Error "libraw.dll is NOT dynamically imported - LGPL compliance is broken"
    exit 1'
fi

# The LibRaw tripwire. cargo-deny's advisories cannot see a native DLL,
# so this compares the version recorded beside it with upstream's newest tag.
# Opt-in because it needs the network; the record itself is kept honest by
# tests/vendored_libraw.rs on every run.
if [ "$WITH_UPSTREAM" = 1 ]; then
  step "LibRaw is current" ./scripts/check-libraw.sh
fi

step "bindings are current" bash -c '
  cargo run --manifest-path src-tauri/Cargo.toml $libraw_flag --features dev-tools --bin export_bindings --quiet &&
  git diff --exit-code -- src/bindings.ts'

# Skipped when cargo-deny is missing, except for the shipping build: a
# release on a machine without it reported a green licence check.
step "dependency licences" bash -c '
  command -v cargo-deny >/dev/null 2>&1 || {
    [ "'"$WITH_AI"'" = 1 ] && { echo "cargo-deny is required for --ai: cargo install cargo-deny"; exit 1; }
    echo "skipped: cargo install cargo-deny"; exit 0; }
  cargo deny --manifest-path src-tauri/Cargo.toml check licenses bans sources advisories'

# The notices the installer ships, regenerated and compared: they had
# gone a release stale, still listing a model that no longer shipped.
if [ "$WITH_AI" = 1 ]; then
  step "third-party notices are current" bash -c '
    command -v cargo-about >/dev/null 2>&1 || {
      echo "cargo-about is required for --ai: cargo install cargo-about --features cli"; exit 1; }
    tmp=$(mktemp)
    (cd src-tauri && cargo about generate --manifest-path Cargo.toml --features ai ../about.hbs -o "$tmp") || exit 1
    diff -q --strip-trailing-cr "$tmp" src-tauri/THIRD-PARTY-LICENSES.txt >/dev/null || {
      echo "src-tauri/THIRD-PARTY-LICENSES.txt is stale: regenerate it as NOTICE says"; rm -f "$tmp"; exit 1; }
    rm -f "$tmp"'
fi

# What a green run did not cover. Informational: a skip of anything declared
# above has already failed its step.
passed=$(grep -ho '[0-9]* passed' "$test_log" 2>/dev/null | awk '{s+=$1} END {print s+0}')
if [ -s "$skip_log" ]; then
  skipped=$(wc -l < "$skip_log" | tr -d ' ')
  printf '\n\033[1m== tests that did not run\033[0m\n'
  printf '%s reported passing, of which \033[33m%s did not run\033[0m:\n' "$passed" "$skipped"
  cut -f1 "$skip_log" | sort | uniq -c | awk '{printf "  %4d  needing %s\n", $1, $2}'
  sort -u "$skip_log" | awk -F'\t' '{printf "        %-5s %s — %s\n", $1, $2, $3}'
else
  printf '\n%s tests reported passing, and every one of them ran.\n' "$passed"
fi

printf '\n'
if [ ${#failed[@]} -eq 0 ]; then
  printf '\033[32mall checks passed\033[0m\n'
  exit 0
fi
printf '\033[31m%d failed:\033[0m %s\n' "${#failed[@]}" "${failed[*]}"
exit 1
