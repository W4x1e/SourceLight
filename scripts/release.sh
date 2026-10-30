#!/usr/bin/env bash
#
# Builds both installers and stages them as a *draft* GitHub release.
#
#   ./scripts/release.sh 0.1.0                # check, build, verify, draft
#   ./scripts/release.sh 0.1.0 --skip-checks  # when check.sh has just passed
#
# With TAURI_SIGNING_PRIVATE_KEY and SOURCELIGHT_RELEASE_URL set, the AI
# installer is also signed for the updater and a `latest.json` manifest is
# written for it — see docs/UPDATES.md.
#
# Local, not a GitHub Actions workflow, on purpose. Windows runners bill at
# twice the rate and a cold build of this tree is about seventy minutes — a
# tag-triggered workflow building both configurations would cost a few hundred
# minutes per release on an account that has already nearly run out once. This
# machine builds the same installers incrementally in a quarter of an hour.
# When the planned self-hosted runner exists, this script is what that
# workflow should call.
#
# Nothing here becomes public. The release is created as a draft, and a draft
# does not even create its tag until it is published — so review the notes and
# the assets on GitHub, then press Publish.
set -euo pipefail

cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }
say() { printf '\n\033[1m== %s\033[0m\n' "$*"; }

version="${1:-}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "usage: $0 X.Y.Z [--skip-checks]"
shift
skip_checks=0
for arg in "$@"; do
  case "$arg" in
    --skip-checks) skip_checks=1 ;;
    *) die "unknown option: $arg" ;;
  esac
done
tag="v$version"

say "preflight"

# One version, three files. The installer takes it from tauri.conf.json, the
# About box from Cargo.toml, and the frontend from package.json; a release
# where they disagree reports different numbers in different places.
conf_version=$(grep -m1 '"version"' src-tauri/tauri.conf.json | sed 's/.*"version": *"\([^"]*\)".*/\1/')
cargo_version=$(grep -m1 '^version' src-tauri/Cargo.toml | sed 's/version *= *"\([^"]*\)"/\1/')
npm_version=$(grep -m1 '"version"' package.json | sed 's/.*"version": *"\([^"]*\)".*/\1/')
for pair in "tauri.conf.json:$conf_version" "Cargo.toml:$cargo_version" "package.json:$npm_version"; do
  [ "${pair#*:}" = "$version" ] || die "${pair%%:*} says ${pair#*:}, not $version — bump all three together"
done

# Released from exactly what is on origin/main, so the tag can be recreated
# by anyone and the build is not of a working tree only this machine has.
# `.claude/` is local tool configuration, never part of the product.
[ "$(git rev-parse --abbrev-ref HEAD)" = "main" ] || die "not on main"
git fetch --quiet origin main
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || die "main is not the same commit as origin/main — pull or push first"
# Untracked files too: Tauri lays an untracked
# `src-tauri/tauri.windows.conf.json` over the signed build by itself, and
# `--untracked-files=no` could not see one.
[ -z "$(git status --porcelain -- . ':!.claude' ':!release')" ] || die "uncommitted or untracked files in the tree"
# And what is ignored but would still change the build: a config overlay,
# a cargo config, an environment file.
overlays=$(git ls-files --others --ignored --exclude-standard -- \
  'src-tauri/tauri.*.conf.json' 'src-tauri/Tauri*.toml' '.cargo/config*' 'src-tauri/.cargo/config*' '.env*')
[ -z "$overlays" ] || die "ignored files that would change the build: $overlays"

git rev-parse -q --verify "refs/tags/$tag" >/dev/null && die "tag $tag already exists"
gh release view "$tag" >/dev/null 2>&1 && die "a release $tag already exists on GitHub"

# Release notes come from the CHANGELOG, which is the thing beta users read to
# learn what changed. A release without its section is a release nobody can
# explain, so it is refused rather than drafted empty.
notes=$(mktemp)
trap 'rm -f "$notes"' EXIT
# Plain string matching, not a regex: the heading is all brackets and dots, and
# every layer of quoting between here and awk wants to escape them differently.
awk -v heading="## [$version]" '
  index($0, heading) == 1 { on = 1; next }
  on && index($0, "## [") == 1 { exit }
  on { print }
' CHANGELOG.md > "$notes"
[ -s "$notes" ] && grep -q '[^[:space:]]' "$notes" \
  || die "CHANGELOG.md has no '## [$version]' section — move the Unreleased entries under one"

# Updater signing, only when both halves exist: the key that signs the
# installer, and the public address it will be served from, which the manifest
# has to name. One without the other is refused rather than half-done — a
# manifest pointing nowhere would offer every installed copy an update that
# cannot download.
#
# The key is taken out of the environment here, before anything else runs:
# exported, it reached every build script, test and npm package of
# the checks and both builds, any one of which could have sent it away, and
# a leaked updater key can never be revoked. It is used once, at the end, to
# sign the finished installer.
signing_key="${TAURI_SIGNING_PRIVATE_KEY:-}"
signing_password="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
unset TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD
if [ -n "$signing_key" ]; then
  [ -n "${SOURCELIGHT_RELEASE_URL:-}" ] \
    || die "TAURI_SIGNING_PRIVATE_KEY is set but SOURCELIGHT_RELEASE_URL is not — latest.json must name where the installer will be downloaded from"
elif [ -n "${SOURCELIGHT_RELEASE_URL:-}" ]; then
  die "SOURCELIGHT_RELEASE_URL is set but TAURI_SIGNING_PRIVATE_KEY is not — an unsigned update is refused by every installed copy"
fi

# The webview's packages exactly as package-lock.json pins them: the
# signed installer was built from whatever node_modules this machine had, and
# nothing audited it.
say "npm packages from the lockfile"
npm ci
git diff --quiet -- package-lock.json || die "npm ci changed package-lock.json — commit the lockfile first"
npm audit --omit=dev --audit-level=high || die "npm audit found high-severity advisories in the shipped packages"

if [ "$skip_checks" = 0 ]; then
  say "checks (the shipping build, against real raws)"
  ./scripts/check.sh --ai --raw || die "checks failed — not releasing"
fi

out="release/$tag"
rm -rf "$out"
mkdir -p "$out"
stem=$(grep -m1 '"productName"' src-tauri/tauri.conf.json | sed 's/.*"productName": *"\([^"]*\)".*/\1/' | tr ' ' '-')
built="src-tauri/target/release/bundle/nsis"

# Both configurations write the same bundle path, so each installer is
# verified and copied out before the next build overwrites it.
say "base installer"
npm run tauri build
./scripts/verify-installer.sh
cp "$built"/*_"$version"_x64-setup.exe "$out/${stem}_${version}_x64-setup.exe"

say "AI installer (the one most people install)"
npm run tauri:build:ai
./scripts/verify-installer.sh --ai
ai_installer="${stem}_${version}_x64-ai-setup.exe"
cp "$built"/*_"$version"_x64-setup.exe "$out/$ai_installer"

# The manifest installed copies poll. It points only at the AI installer: that
# is the build most people install, and so the one updates move people onto. Signed
# here, finished, in the one step that sees the key.
if [ -n "$signing_key" ]; then
  say "sign the AI installer for the updater"
  TAURI_SIGNING_PRIVATE_KEY="$signing_key" TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$signing_password" \
    npx tauri signer sign "$out/$ai_installer"
  unset signing_key signing_password
  [ -f "$out/$ai_installer.sig" ] || die "signing wrote no $ai_installer.sig"
  NOTES="$notes" SIG="$out/$ai_installer.sig" OUT="$out/latest.json" \
    VERSION="$version" URL="${SOURCELIGHT_RELEASE_URL%/}/$ai_installer" node -e '
    const fs = require("fs");
    fs.writeFileSync(process.env.OUT, JSON.stringify({
      version: process.env.VERSION,
      notes: fs.readFileSync(process.env.NOTES, "utf8").trim(),
      pub_date: new Date().toISOString(),
      platforms: {
        "windows-x86_64": {
          signature: fs.readFileSync(process.env.SIG, "utf8").trim(),
          url: process.env.URL,
        },
      },
    }, null, 2) + "\n");
  '
  echo "wrote $out/latest.json — upload it beside the installer at the configured endpoint"
fi

(cd "$out" && sha256sum -- *.exe > SHA256SUMS.txt)
extra=()
[ -f "$out/latest.json" ] && extra=("$out/latest.json" "$out/$ai_installer.sig")

say "draft release $tag"
gh release create "$tag" \
  --draft \
  --target "$(git rev-parse HEAD)" \
  --title "$tag" \
  --notes-file "$notes" \
  "$out"/*.exe "$out/SHA256SUMS.txt" "${extra[@]}"

printf '\n\033[32mdrafted %s\033[0m — review it on GitHub and publish when ready.\n' "$tag"
printf 'Installers and checksums are also in %s/\n' "$out"
