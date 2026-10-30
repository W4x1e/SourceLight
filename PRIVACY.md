# Privacy

SourceLight runs on your computer. It has no accounts and no server of its
own, and it has no telemetry: no analytics, no usage statistics, and no crash
reports sent anywhere. This page lists every network request the app can
make, what starts it, and what it sends.

## What stays on your computer

- Your photos, their file names and their folders.
- The catalogue and its backups: ratings, flags, labels, keywords, captions,
  places, people, faces and edits.
- What the AI works out: search data, tags, descriptions, faces, and the face
  templates People groups them by. Every model runs on your computer.
- Logs and crash logs. They are written to the `logs` folder in the app's data
  folder and nowhere else. Settings → About shows a diagnostics summary you can
  copy into a bug report yourself.

The data folder is `app.sourcelight` in your system's app-data folder:
`%LOCALAPPDATA%\app.sourcelight` on Windows and
`~/Library/Application Support/app.sourcelight` on a Mac.

## Network requests from the desktop app

Downloads and the update check carry no photo, no file name, nothing from the
catalogue and no identifier for you. The server sees your IP address, as it
would for any download. The phone link does carry your library, but only
between your computer and your paired phones, encrypted end to end.

| What | Host | When | What is sent | Your choice |
|---|---|---|---|---|
| AI models for search, tags, faces, People, AI masks, erase and noise reduction | `huggingface.co`, and release downloads on `github.com`, with their download servers | The first time a feature needs its model. Automatic analysis also fetches the search, tag and face models after the first photos are added. | A request for a fixed file, pinned to a version and checked against its SHA-256. The user agent is `SourceLight`. | On a metered or unknown connection the app asks first. On an unmetered one the automatic download starts with a notice and can be stopped in Activity. Turning AI off in Settings → AI stops all of them. |
| The runtime and model for Detailed descriptions | llama.cpp's releases on `github.com`, and Qwen's models on `huggingface.co` | Turning on Detailed descriptions, or choosing another vision model | The same as above | Off by default. The app asks first and names the sizes. After a yes, a missing runtime or model downloads again without asking on an unmetered connection. |
| The update check | None in this source. A build only checks if its maker sets up an update server (`plugins.updater` in `src-tauri/tauri.conf.json`). | 15 seconds after launch, when you press Check for updates, and when you choose to install | A request for the update manifest, then the installer | Settings → About → "Check for updates when the app starts", on by default. |
| The phone link | Your phone, directly | Phone connections on, and a phone paired | The library as the phone shows it (photo details, thumbnails and previews), edits and verdicts both ways, photos added from the phone, and shoot tracks. End-to-end encrypted. | Off until you turn on phone connections in Settings → Phone. |
| The phone link's relay | The public relays run by n0, the makers of iroh, for now | Phone connections on, with Away from home on | The relay sees this computer's IP address, its link key's public half, and when and how much the devices exchange. It cannot read the traffic. | Away from home is its own switch in Settings → Phone. It is on by default once phone connections are on. |

The phone link is built only with the `phone-link` feature, and releases do not
have it yet. Until our own relays run, Away from home uses n0's public ones.

## Network requests from the phone app

The phone app is not released yet. As built:

| What | Host | When | What is sent | Your choice |
|---|---|---|---|---|
| The link | Your computer, directly | Syncing, adding photos to the library, sending a shoot track | End-to-end encrypted, as above | You pair the phone yourself. |
| The relay | n0's public relays | Whenever the link runs, even when the computer is set to home only | The phone's IP address and its link key's public half. It cannot read the traffic. | None yet. |
| Location | The phone's own location service (Android's location manager, which can ask Google's network location; Core Location on iOS) | Recording a shoot track | Whatever the system's location service sends | You start each track, after the system asks for permission. The track goes only to your computer. |

## Servers that stay on your computer

- **The vision model's server.** `llama-server` listens on 127.0.0.1, on a free
  port, with a key made for each run. It is started with `--offline`, so it
  fetches nothing. Photos go to it and never leave the computer.
- **The editing server for AI apps** ([docs/AI-EDIT.md](docs/AI-EDIT.md)). Off
  by default. "Let AI apps edit photos" in Settings → AI turns it on, on
  127.0.0.1:47811. It refuses requests from web pages, and every request needs
  this install's token, kept in `edit-server-token` in the data folder.
  SourceLight sends nothing out through it. The AI app you connect gets the
  open photo as a 1024 px preview, its edit settings, and the names of your
  presets and LUTs, and does with them what it does with any attachment. An app
  that uses a cloud model, such as Claude Desktop, Codex or Gemini CLI, sends
  them to that model's provider. LM Studio with a local model keeps them on
  your computer.

## Not the app's own

- The system webview (WebView2 on Windows, WKWebView on a Mac) and the
  operating system's own checks (SmartScreen, Gatekeeper) make connections of
  their own, under your system's settings.
- Building from source downloads dependencies: npm and crates.io packages,
  ONNX Runtime, and LibRaw on a Mac.

A problem with any of this is a security report: see [SECURITY.md](SECURITY.md).
