# Security

## Reporting a vulnerability

Please report a security problem privately, not in a public issue, through
GitHub's **private vulnerability reporting**: the repository's **Security**
tab, then **Report a vulnerability**
([direct link](https://github.com/W4x1e/sourcelight/security/advisories/new)).

Say which version and operating system are affected, how to reproduce it, and
what an attacker would gain. SourceLight is maintained by one person, so allow
a few days for a reply. A confirmed problem is fixed in a release, and you are
credited in its notes unless you would rather not be.

## Scope

SourceLight runs on the user's own machine, with no accounts and no servers of
its own. What matters most is where it talks to the network, and what it does
with files it did not make:

- **The phone link** ([docs/MOBILE.md](docs/MOBILE.md)): pairing (the QR code
  and its one-time token), the listener phones connect to, and what a paired
  or unpaired device can get from the desktop. Built only with the
  `phone-link` feature, and off until a phone is paired.
- **The self-updater** ([docs/UPDATES.md](docs/UPDATES.md)): anything that
  would let an unsigned or substituted installer through the signature check.
- **Model and runtime downloads**: the AI models (`src-tauri/src/ai_models.rs`)
  and llama.cpp's `llama-server` (`src-tauri/src/llama_pack.rs`), each pinned
  to a revision and a SHA-256. Anything that installs or runs a file that has
  not passed that check.
- **The local vision-model server**: `llama-server` listens on 127.0.0.1 with
  a key made for each run (`src-tauri/src/sidecar.rs`). Reaching it from
  another process or machine without that key.
- **Files from elsewhere**: raw files and JPEG, PNG, TIFF, WebP or HEIC images,
  XMP sidecars, GPX tracks, LUTs and catalogue backups that crash the app, or
  worse, when opened. A crash inside LibRaw is worth reporting to
  [LibRaw](https://github.com/LibRaw/LibRaw) too.
- **The custom URL schemes** the interface loads images through (`thumb://`,
  `develop://`, `full://`).
- **The editing server for AI apps** ([docs/AI-EDIT.md](docs/AI-EDIT.md)):
  off until switched on in Settings, then on 127.0.0.1 only, refusing
  requests from web pages and requests without this install's token. Reaching
  it from a browser, another machine or another account without the token, or
  using it to do more than edit the photo open in Develop. That includes
  `SourceLight --mcp`, which passes an AI app's messages from stdin to
  the running app's server.

Out of scope: anything that needs an attacker to control the user's account or
machine already; the security of the sites models are downloaded from (Hugging
Face, GitHub), beyond how the app checks what it downloads; and the developer
tools (the browser mock backend, the `dev-tools` binaries).

## Supported versions

Fixes go into the next release; there are no long-term support branches. Check
a problem against the latest release, or against `main`.
