# Updates (direct-download build)

A build from the Microsoft Store, if there is ever one, is updated by the Store
and needs none of this. The direct-download installer updates itself through
`tauri-plugin-updater`: a quiet check fifteen seconds after launch, a
slim notice when a newer version exists, and **Settings → About → Check for
updates / Install and restart**.

## It is off until you switch it on

`tauri-plugin-updater` will not start without a public key, and the app will
not register it until `tauri.conf.json` has both a key and an endpoint
(`src-tauri/src/updates.rs`). Until then the About tab says updates are not set
up, the launch check makes no network request, and nothing else changes.

Two things have to exist first, and both are decisions only the maintainer can
make:

1. **A signing key.** Every installed copy trusts its public half for good. Lose
   the private half — or its password — and **no installed copy can ever be
   updated again**; their owners would have to reinstall by hand.
2. **A public place to serve updates from.** The repository's GitHub releases
   will do, and so will a website or any static host.

## One-time setup

**1. Generate the key**, outside the repository:

```bash
npx tauri signer generate -w "$HOME/.tauri/sourcelight-updater.key"
```

Choose a password. Put the key file *and* the password in your password
manager now — this is the only copy.

**2. Configure the app.** Add to `src-tauri/tauri.conf.json`, with the contents
of `sourcelight-updater.key.pub` as `pubkey`:

```json
"plugins": {
  "updater": {
    "pubkey": "<contents of sourcelight-updater.key.pub>",
    "endpoints": ["https://<your public host>/latest.json"],
    "windows": { "installMode": "passive" }
  }
}
```

`passive` shows a small progress window and needs no clicks. The endpoint is
fetched on every check, so it must always serve the *newest* `latest.json`.

**3. `"requireSignedVersion": true` — required, from the first signed
release.** It stops anyone who can change `latest.json` on the host
from advertising "9.9.9" pointing at an old, genuinely signed installer and
rolling every copy back. It only works if the signer records the version
inside the signature, and releases signed before that are rejected. After
your first signed build, check:

```bash
base64 -d < release/v<X.Y.Z>/*-ai-setup.exe.sig | grep -a 'trusted comment'
```

If that line includes the version, turn it on. If it does not, leave it off —
it would refuse every update.

## Releasing with updates

```bash
TAURI_SIGNING_PRIVATE_KEY="$(cat "$HOME/.tauri/sourcelight-updater.key")" \
TAURI_SIGNING_PRIVATE_KEY_PASSWORD='…' \
SOURCELIGHT_RELEASE_URL='https://<your public host>/v0.1.1' \
  ./scripts/release.sh 0.1.1
```

Given to the one command, not `export`ed into the shell. `release.sh`
takes the key out of its environment before anything else runs, so no build
script, test or npm package of the checks and builds can read it, and signs
the finished AI installer with `tauri signer sign` in a last step that alone
sees it. A leaked updater key can never be revoked.

`release.sh` signs the AI installer — the build most people install, so the one
updates move people onto — and writes `release/v0.1.1/latest.json` pointing at
`$SOURCELIGHT_RELEASE_URL/<installer>`. Setting only one of the two variables is
refused: an unsigned update is rejected by every installed copy, and a manifest
without a download address offers an update that cannot download.

Then upload the AI installer to `$SOURCELIGHT_RELEASE_URL`, and replace the
endpoint's `latest.json` with the new one.

## Verifying it end to end

Install version N. Release N+1 as above. Launch N: within about fifteen seconds
a notice offers N+1; **See what's new** opens About; **Install and restart**
downloads it, verifies the signature against the key built into N, and runs the
installer. The library and every edit are untouched — they live in the app's
data folder, which an update never touches.

## On a Mac: not wired up yet

`release.sh` builds and signs the Windows installers only. The Mac updater needs
three things first:

- **Developer ID signing and notarisation.** An update that replaces a signed,
  notarised app must be signed the same way, or Gatekeeper refuses it
  ([MACOS.md](MACOS.md), Tier 3).
- **The updater artefact.** `tauri build` makes a signed `.app.tar.gz` for the
  updater when `createUpdaterArtifacts` is on and the signing key is set, the
  same key as Windows.
- **A `darwin-aarch64` entry** in `latest.json` beside the Windows one, pointing
  at that archive, which `release.sh` would have to build on a Mac and add.
