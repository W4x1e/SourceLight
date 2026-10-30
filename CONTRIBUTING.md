# Contributing

SourceLight is open source, under the [Mozilla Public License 2.0](LICENSE),
and contributions are welcome: bug reports, sample raw files, fixes, features
and documentation. Everyone taking part agrees to the
[code of conduct](CODE_OF_CONDUCT.md). Security problems are the exception to
"open an issue": report them privately, as [SECURITY.md](SECURITY.md) describes.

Useful, and needing no code at all:

- **Bug reports** — what you did, what happened, what you expected.
- **Raw files that decode wrongly**, or come from a camera not yet covered
  (below).
- **Reproducible performance findings** — a library size, a machine, and a slow path.

## Pull requests

Pull requests are welcome.

- **Open an issue first for anything big** — a new feature, a new dependency,
  a different approach — so the design is agreed before the work is done.
  A small fix can go straight to a pull request.
- **Run the gate before you open it**: `./scripts/check.sh`, described below,
  with `--ai` if the change touches on-device AI, `--raw` if it touches
  decoding and `--mobile` if it touches the phone app
  ([docs/MOBILE.md](docs/MOBILE.md) has the phone app's setup). CI then runs
  on the pull request itself.
- **Keep the documents true.** A change that makes a document or a code comment
  wrong fixes it in the same pull request. A user-visible change gets a line in
  [CHANGELOG.md](CHANGELOG.md) under **Unreleased**, and interface text goes
  through `src/i18n` ([docs/I18N.md](docs/I18N.md)).
- **Sign the contributor licence agreement** once, on your first pull
  request, below.

### The contributor licence agreement

Contributions come in under the [SourceLight Contributor Licence Agreement](CLA.md).
You keep the copyright in your work. You grant the maintainer a licence to
use it, including under licences other than the MPL-2.0, and that licence
passes to whoever takes the project over. Releases already published under
the MPL-2.0 stay under it.

You sign once. On your first pull request a bot asks you to, and you reply
with the sentence it gives. That covers every contribution you make after it.
[docs/LICENSING.md](docs/LICENSING.md) says what the licences mean in practice.

## Reporting a raw file that decodes wrongly

Raw support is only as good as the files it has been tested against, so a raw
that opens badly is one of the most useful reports there is. Open a bug report
(the template asks for all of this) with:

- **The camera model**, and the lens and firmware if they might matter.
- **What you see**: a screenshot of SourceLight beside the camera's own JPEG, or
  another raw developer's rendering of the same file.
- **The app version** (Settings → About), the operating system and the GPU.
- **Whether you can share the file.** If you can, attach it zipped (GitHub
  refuses a bare raw) or link to it, and say whether it may become a test
  fixture or is for diagnosis only. The decode tests use samples from
  [raw.pixls.us](https://raw.pixls.us), listed in
  `src-tauri/tests/fixtures/raw-samples.tsv`; contributing a sample there,
  under a free licence, lets every build test against it.

## Building from source

Requirements:

- Node 20+ and Rust 1.91 or newer
- Windows (x64), or a Mac with Apple silicon (below)
- **On Windows:**
  - The Visual Studio C++ build tools (the "Desktop development with C++"
    workload) and WebView2, which Windows 11 already has. Tauri's
    [prerequisites page](https://v2.tauri.app/start/prerequisites/) covers both.
  - LLVM. `bindgen` generates the LibRaw struct definitions at build time and
    needs `libclang`. Install LLVM with `winget install LLVM.LLVM`. The build
    finds it in `C:\Program Files\LLVM` on its own. If LLVM is somewhere else,
    set `LIBCLANG_PATH` to the folder that holds `libclang.dll`. In PowerShell:
    `$env:LIBCLANG_PATH = 'D:\Tools\LLVM\bin'`.
  - Bash, from [Git for Windows](https://gitforwindows.org/). The scripts in
    `scripts/`, `check.sh` among them, run in Git Bash.

[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and
[docs/CODEBASE-GUIDE.md](docs/CODEBASE-GUIDE.md) explain how the code fits
together; [docs/README.md](docs/README.md) indexes the rest.

```bash
npm ci
npm run tauri dev
```

`npm ci` installs exactly what `package-lock.json` records. Use `npm install`
only when you are changing dependencies.

To build without raw decoding — no LLVM and no LibRaw needed, and the library,
tagging and filtering all still work:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --no-default-features
```

**On-device AI on Windows.** `npm run tauri:ai` and `cargo test --features ai`
need ONNX Runtime and DirectML, which are not in the repository. Run
`./scripts/fetch-ort.sh` once. It downloads about 220 MB, keeps about 35 MB in
`src-tauri/vendor/onnxruntime/`, and checks each DLL against the hash recorded
there. Without it the AI tests skip, and `npm run tauri:build:ai` stops on the
missing files.

### On a Mac (Apple silicon)

Everything runs, Develop included. The Xcode Command Line Tools are the only
requirement beyond Node and Rust: `bindgen` finds their libclang unaided, and
LibRaw is vendored for Apple silicon (`scripts/vendor-libraw-macos.sh` builds
it). For the AI build, fetch ONNX Runtime 1.24.4 into
`src-tauri/vendor/onnxruntime/` first (see its README).

```bash
npm run tauri dev
npm run tauri:ai              # with on-device AI
npm run tauri:build:ai:mac    # SourceLight.app and a .dmg
```

`tauri.macos.conf.json` is laid over `tauri.conf.json` on a Mac automatically.
The `.dmg` step scripts Finder to lay out its window, which fails without
Automation permission; `CI=true` skips the layout. The bundle is ad-hoc
signed, so Gatekeeper warns on another Mac until Developer ID signing and
notarisation are set up ([docs/MACOS.md](docs/MACOS.md)).

**Keep development away from your real library.** A debug build (`tauri dev`,
`tauri:ai`) opens the catalogue in `SOURCELIGHT_DATA_DIR` when it is set, so
a development catalogue can live anywhere. A release build — the `.app`, or an
installed copy — deliberately ignores that variable and always opens the real
one in the app's data folder (`~/Library/Application Support/app.sourcelight`
on a Mac, `%LOCALAPPDATA%\app.sourcelight` on Windows).

To try a finished build — first launch, a folder dropped on the Dock, an
installer — build the test variant instead:

```bash
npm run tauri:build:test:mac   # SourceLight Test.app (no .dmg)
npm run tauri:build:test       # the Windows installer
```

`tauri.test.conf.json` gives it its own identifier
(`app.sourcelight.test`), so its catalogue, models, window state
and webview storage are separate from the real app's, both can run at once, and
it has no updater. It starts empty and downloads its own models; on a tight disk,
copy or link `models/` across from the real app's folder first.

**Run in release when judging how the Develop module feels.** A debug build
JPEG-encodes each preview frame in ~250ms versus ~50ms optimised, and that is
the whole frame budget — the GPU work is a third of a millisecond either way.

Before opening a pull request, run the gate: types, formatting, clippy, the
frontend and Rust tests, the bindings check and the dependency licences. CI
runs most of these checks on every pull request and every push to `main`, on
Windows and macOS. `.github/workflows/ci.yml` is the exact list. The gate is
much quicker on a warm build, and it runs the GPU tests, which skip on CI's
Windows runners because those have no GPU.

```bash
./scripts/check.sh            # the checks that gate a commit
./scripts/check.sh --ai       # ...and the build that ships, with on-device AI
./scripts/check.sh --raw      # ...and every test against real raw files
```

`check.sh` insists that the GPU tests run. It fails on a machine with no
hardware GPU, such as a virtual machine. It also fails while the app is open,
because the app holds the GPU, so close the app first. On a machine with no
GPU, run `SOURCELIGHT_REQUIRE= ./scripts/check.sh`: the GPU tests skip, and the
list at the end says so.

`--raw` fetches the sample raws with `scripts/fetch-raw-fixtures.sh` into
`src-tauri/tests/fixtures/raws` the first time. The list of tests that did not
run, at the end, is worth reading: a test that needs a model or a real
catalogue skips without one.

## Looking at the UI without Tauri

`npm run dev` on its own serves the frontend to a plain browser, with no
backend behind it. In that case `src/dev/mockBackend.ts` stands in: a fixed,
fake 48-photo library with generated placeholder images, so Browse, Cull, the
Lightbox, Develop's panels, Places, People and Settings all render with
realistic content. It is for looking at layout and type, not behaviour.
Nothing renders through the GPU, and nothing is saved. It is never part of a
production build, and `tauri dev` uses the real backend.

- An unknown command logs `[mock backend] no fixture for "…"`. Add a fixture
  to `HANDLERS` when a new surface needs one.
- To look at a different state, override a fixture from the console, e.g.
  `localStorage["mock:list_library_folders"] = JSON.stringify({ folders: [], unfiled: 0 })`
  for the first-run welcome screen, then reload.
  `localStorage.removeItem("mock:list_library_folders")` puts it back.
- To see what a slow command shows while it runs, delay its answer:
  `localStorage["mock:delay:open_develop"] = "3000"` holds Develop's first
  photo opening for three seconds.
- The fixtures include deliberately long strings (a place name, a subject, a
  teleconverter lens, a long filename), because dense rows have to survive
  real-world text.
- `__clipAudit()` in the browser console reports text cut off by a box that
  hides overflow, and controls pushed below the window with no scroller to
  reach them. Deliberate `…` truncation is counted separately. Run it at the
  minimum window size (940×620) as well as the default (1500×940).

## Dependency licences

`src-tauri/deny.toml` enforces a permissive-only licence policy, checked by the
gate and in CI. **Do not add a dependency under GPL, LGPL, AGPL or CC-BY-SA**,
and do not weaken the allowlist to make a build pass. Rust links statically, so
a single copyleft crate anywhere in the tree would put the whole binary under
its terms, or oblige everyone who distributes a build to make it relinkable.
SourceLight is MPL-2.0 so that it can be combined with code under other
licences; one GPL crate would quietly end that. [docs/LICENSING.md](docs/LICENSING.md)
has the whole policy, AI models included.

The gate checks licences with cargo-deny. Install it once with
`cargo install cargo-deny`. Without it the gate skips the check and leaves it
to CI. `check.sh --ai` also regenerates the third-party notices, which needs
`cargo install cargo-about --features cli`.

If you need functionality that only a copyleft library provides, raise it in an
issue first — the answer is usually to link it as a separate shared library
rather than to pull it into the Cargo tree (this is exactly how LibRaw, which is
LGPL, is used — see [NOTICE](NOTICE)).

## Updating LibRaw

`libraw.dll` is vendored as a native binary, so `cargo deny` cannot report its
security advisories. [docs/LIBRAW.md](docs/LIBRAW.md) covers how we notice a
LibRaw release and the roll-forward procedure. In short:

- `src-tauri/vendor/libraw/VERSION` records the version and the binaries' hashes.
  Tests fail if the DLL, import library, headers or `NOTICE` disagree with it.
- `./scripts/check-libraw.sh` (or `./scripts/check.sh --upstream`) fails when
  upstream has a newer release.
- The maintainer's GitHub account watches
  [LibRaw/LibRaw](https://github.com/LibRaw/LibRaw) for releases (*Watch → Custom
  → Releases*). That is the subscription, so a fix is noticed without anyone
  having to run the script.

## Commit messages

Subjects read as prose, in the imperative, saying what changed for the user
rather than which files moved — `Stop content search from answering a fresh
install with silence`, not `fix(store): add ensureAi gate`. The body explains
*why*, and especially why the obvious alternative was wrong; that reasoning is
the part nobody can reconstruct from the diff six months later.

Every commit that touches tracked work ends with a trailer linking it, so the
issue timeline and the history agree:

```
Stop content search from answering a fresh install with silence

<why, and what the alternative would have cost>

Closes #3
```

- **`Closes #N`** — the issue is done. GitHub closes it on merge to `main`.
- **`Refs #N`** — progress on it, or context for it, but it stays open.
- Several are fine, one per line.

Use `Refs` when in doubt. A prematurely closed issue is harder to notice than an
open one.
