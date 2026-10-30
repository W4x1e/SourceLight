# macOS

Where the Mac port stands, and the two plans that finish it: **Develop on macOS**,
and **making it feel like a Mac app** rather than a web page in a window.

Everything below was run or measured on an Apple M4 Mac mini (16 GB, macOS 27)
against a library of real Sony A6400 raws, unless it says otherwise.

## What works today

| Area | State |
|---|---|
| Library: scan, embedded previews, EXIF, thumbnails over `thumb://` | Works (73 ARWs) |
| Filters, date grouping, ratings, flags, Cull, selection panel | Works |
| Folder picker, Show in Finder, Move to Trash | Works (native `NSOpenPanel`, `open -R`, Trash) |
| On-device AI: SigLIP 2 search and tags, faces | Works. Faces on **CoreML** (same 16 people, half the CPU time); SigLIP 2 on the CPU, 22 s to index 73 photos — CoreML cannot compile it, and was no faster where it could |
| Model download on first import | Starts without asking on an unmetered connection (Network.framework) |
| Detailed descriptions (local vision model) | Works. The app downloads llama.cpp's own build (12 MB, hash-checked) and Qwen's GGUF, and runs the model on `llama-server` itself: Qwen3-VL 4B, picked for 16 GB, tags a photo in 2.3–2.5 s and describes one in 6.3–6.8 s. Before that, the runtime and model came through Ollama; see *The local vision model* below |
| HEIC and HEIF (iPhone, Fujifilm HIF) | Read through ImageIO, upright, with their own colour profile (an iPhone's Display P3 is kept for Develop). Nothing to install, unlike Windows |
| Quit after AI has run | Clean (ONNX Runtime 1.24.4 + `release_ort` on exit; 1.22.0 aborted on every quit). CoreML's compiled models are kept in `~/Library/Caches/<identifier>/coreml` and reused; before, every quit left them in `$TMPDIR` (4.4 GB by 2026-09-28). See `develop::coreml_cache` |
| `.app` and `.dmg` | Build with `npm run tauri:build:ai:mac` (ad-hoc signed). `npm run tauri:build:test:mac` makes *SourceLight Test.app*, with its own identifier and data folder, for trying a bundle without touching the real catalogue |
| Develop, export, full-resolution loupe | Works — LibRaw built for macOS 14, all six fixture formats decode, pipeline on Metal. In the app: opened an ARW, dragged Exposure live, ⌘Z, exported a Display P3 JPEG with its EXIF through the native save panel |
| Checks | `./scripts/check.sh --ai --raw` passes on the M4 (2026-09-28): 1,457 tests with the GPU and the six-camera raw set required; only tests needing downloaded models or a real catalogue skip |

Minimum macOS is **14.0**, set by ONNX Runtime 1.24.4. Apple silicon only.

## The local vision model

Measured on 2026-09-27 on the same M4 (16 GB), through the app's own tagger
code (the same prompts, image sizes, context and JSON schema), on the 73 A6400
previews, with the pinned Ollama v0.32.14:

| Model | Tag, per photo | Describe, per photo | Resident | Lowest free memory |
|---|---|---|---|---|
| Qwen3-VL 2B | 5.1 s | 5.0 s | 1.7 GB | 60% |
| **Qwen3-VL 4B** (picked for 16 GB) | **8.2 s** | **7.9 s** | **3.1 GB** | 51% |
| Qwen3-VL 8B | 14.9 s | 13.2 s | 5.2 GB | 32% |
| qwen2.5vl 7B, one at a time | 13.2 s | — | 6.6 GB | 28% |
| qwen2.5vl 7B, two at a time | 11.8 s | 13.7 s | 6.6 GB | 28% |

All 73 tag replies were valid JSON for every model. The same models run 6–12×
faster on an RTX-class GPU (`docs/AI-MODEL-SPIKES.md`): the M4's 10-core GPU,
not its memory, is what limits them. Loading a model takes 2–3 s; pulling the
4B and loading it took 42 s on a fast connection.

**The hardware probe gets the memory right.** A Mac's GPU memory is counted
as 70% of its RAM: 11.2 GB here, where Metal's `recommendedMaxWorkingSetSize`
is 11.8 GB and Ollama reports 11.3 GB available. Every model's "will run"
verdict held, with no new swap during any run. The probe used to grade the M4
as "integrated graphics, a compact build is a better fit", because wgpu
reports Apple silicon as an integrated GPU; it now grades Apple silicon by its
memory, like a discrete GPU.

**What the runs found, and what changed:**

- **Ollama runs the Qwen3-VL family one request at a time**, on every
  platform: "model architecture does not currently support parallel
  requests", and its `llama-server` starts with `-np 1` whatever
  `OLLAMA_NUM_PARALLEL` says. Only qwen2.5vl gets two slots, and on the M4 they
  buy 11%. The client still sends two, which only queues the second.
- **`llama-server` kept a prompt cache in system memory** — 8 GB by default,
  ~90 MB per photo. A tagging run never repeats a prompt, so it only ever
  missed; on 16 GB it reached 6.2 GB and pushed the Mac 5.5 GB into swap.
  The managed server now starts with `LLAMA_ARG_CACHE_RAM=0`.
- **Quitting left the model loaded.** The app stopped its Ollama with SIGKILL,
  and the `llama-server` under it — the process holding the weights — outlived
  the app indefinitely. It now sends SIGTERM first, and Ollama takes its runner
  down in under a second (`sidecar::stop`).
- **The app mistook one model for another.** With Qwen3-VL 2B pulled, it
  believed the 4B was too (the check compared only the family name), skipped
  the pull, and every description failed with "model not found" — while the
  Describe button showed nothing at all. The check now needs the exact tag,
  and a run that stops early says why. Not Mac-specific.
- Ollama starts Qwen3-VL with `--image-min-tokens 1024`, so the 896 px tag
  image is scaled back up and a tag costs about what a description does.
- **Tags and a description can come from one request.** Most of a request
  is reading the image, so asking for both at once costs 10.3 s a photo
  through Ollama where the two requests cost 16.1 s (`Tagger::tag_and_caption`,
  with `description` last in the schema, so the genre is decided as before).
  Raising Ollama's batch size (`num_batch` 512 → 2048) made no difference.

### Running the model directly: tags three times quicker

Ollama's API fixes the two things that decide a photo's time. It starts
Qwen3-VL with `--image-min-tokens 1024`, so any image — even the small one
tags are asked of — costs ~1,000 tokens, and reading the image is most of a
request. And it runs Qwen3-VL one request at a time. The runtime Ollama
downloads also contains the `llama-server` Ollama runs these models on, so on
a Mac the app now runs that itself, on the weights Ollama pulled (read from its
manifest), and sizes the image per request (`tagger::llama`,
`sidecar::llama_server_args`). Ollama still downloads and stores the models.

Measured with Qwen3-VL 4B, per photo:

| Request | Through Ollama | Direct, 1 at a time | Direct, 2 at a time |
|---|---|---|---|
| Tags, 640 px image (~260 tokens) | 8.2 s | 2.9 s | **2.5 s** (2.3 s in the app, 31 photos) |
| Description, 1280 px (~1,080 tokens) | 7.9 s | 7.2 s | **6.3–6.8 s** |
| Tags and description, 1280 px | 10.3 s | 9.4 s | **8.3 s** |

- **Tags need little of the image.** At ~256 image tokens they agreed with
  full-size tags on 13 of 16 genres, and the three that differed were photos
  two genres fit equally (a crowded beach as street or event).
- **Text in the frame needs all of it.** At 384 and 256 tokens the model
  didn't miss a sign's text — it *invented* some ("CAMPANIA" and "CANTINA" for
  "GARDEN BAR"). At 512 it read the large sign and missed the small text; only
  full size read "Actv 30" on a Venice waterbus. So descriptions keep 1280 px,
  and tagging no longer writes descriptions on a Mac: tags arrive three times
  sooner, and Describe follows at full size.
- **Two at once, not four.** Writing the reply is bound by memory bandwidth,
  so a second reply rides along (2.5 s against 2.9 s for tags); four was slower
  than two (4.3 s against 3.9 s for 256-token tags and descriptions).
- **The instructions are read once per slot.** They come before the image in
  each request, so a slot reuses the ~460 tokens it read for the previous
  photo; Ollama put the image first and reused 3.
- The 2B model tags in 1.4 s and describes in 5.5 s the same way, for a Mac
  with less memory.
- The server is started when a pass begins, stopped two minutes after the last
  one ends (freeing the model's memory), and stopped on quit.

Every platform now runs the model this way, and Ollama is gone: the
app downloads llama.cpp's official build and Qwen's own GGUFs itself. On
Windows that is the Vulkan build, as fast as Ollama's CUDA on an RTX and 33 MB
instead of 1.46 GB. The measurements are below and in
[AI-MODEL-SPIKES.md](AI-MODEL-SPIKES.md).

### Without Ollama: llama.cpp, MLX and Apple's model

Measured 2026-09-29 on the same M4 with `scripts/model-eval/vlm_serving.py`,
`vlm_mlx.py` and `afm/`: the app's own prompts and image sizes, 22 previews
after two warm-ups. Qwen3-VL 4B throughout, except for Apple's model.

| Per photo | Tags, 640 px | Description, 1280 px | Both, 1280 px |
|---|---|---|---|
| Ollama's `llama-server` (today), 1 at a time | 3.04 s | 7.41 s | 9.45 s |
| Official llama.cpp b11234 + Qwen's GGUF, 1 at a time | 3.01 s | 7.47 s | 9.01 s |
| Ollama's `llama-server`, 2 at once | 2.57 s | 6.92 s | 8.18 s |
| Official llama.cpp + Qwen's GGUF, 2 at once | 2.55 s | 6.74 s | 8.06 s |
| MLX (mlx-vlm 0.7.4, 4-bit), 1 at a time | 3.15 s | 7.64 s | 9.00 s |
| Apple's on-device model | 1.49 s | 1.74 s | |

- **llama.cpp's official build: go.** The official build with Qwen's own files matches Ollama's
  build within ~2%, with identical token counts and 4.1–4.6 GB peak memory
  either way. The one-line grammar: tags 2.33 s against 2.30. First load
  7 s (Metal compiles), then 1 s. The 4B did not loop (longest description
  77 words).
- **MLX: no-go.** It decodes faster (35–38 tokens/s against ~31) but
  reads the image no faster, so a photo takes the same time. Without a
  grammar the tag prompt gives no JSON at all (0 of 22: it answers in
  `key: value` lines); mlx-vlm's schema processor fixes that (22/22). Not
  worth a second engine and model format.
- **Apple's model: not a replacement.** Fast, and no failures or
  refusals in 66 requests, but it reads a 1280 px image as only ~296 tokens.
  Its genre agrees with Qwen on 16 of 22 (13 at 1280 px), its main subjects
  are generic, it invents details (a hiker became a "penguin"; "BBC" on an
  antenna), and its text reading is unreliable. At most a zero-download
  first pass on macOS 27 with Apple Intelligence on. The background rate
  limit was not verified. (`@Generable` needs Xcode's macro plugin, so the
  CLI uses `DynamicGenerationSchema`.)

---

## Plan 1 — Develop on macOS

**Status: steps 1, 2, 4, 5 and 6 are done; step 3 is decided.**

The full suite runs against the six-camera fixture set with
`SOURCELIGHT_REQUIRE=gpu,raw`, so a skipped GPU or raw test fails: **687
pass**, and the only failures are the two `db` path tests. That covers CR3, NEF,
ORF, RW2, ARW and X-Trans RAF decoding, the WGSL develop pipeline on Metal, the
golden-image, colour-match and middle-grey tests, batch export and the
full-resolution preview.

### Step 1 — Vendor LibRaw as a dylib

**Done.** LibRaw's own `LibRaw-0.22.2-macOS.zip` ships **static archives only**,
arm64 only despite the download page's "ARM/Intel", and — the deciding find —
its objects are built for **macOS 26**, whatever the page says. A static link
would make the app a derived work of an LGPL library (see `build.rs`), and
wrapping those objects in a dylib would not run on the macOS 14–25 the app
supports. So `scripts/vendor-libraw-macos.sh` compiles the official source
release, `LibRaw-0.22.2.tar.gz`, with upstream's own `Makefile.dist` and flags
(`-O3`, zlib, the reentrant `_r` objects), pinned to arm64 and the app's minimum
macOS, into `lib/aarch64-apple-darwin/libraw.dylib` (1.2 MB; links only the
system's libz, libc++ and libSystem). The build is byte-reproducible with one
Xcode. `VERSION` records the tarball's hash, which the script enforces, and the
dylib's, which `tests/vendored_libraw.rs` checks. The headers were already
right: they are byte-identical to the source release's.

Intel Macs are not covered: that needs the same build for x86_64 (and an x86_64
ONNX Runtime).

### Step 2 — Link and load it (done)

- `build.rs`: on Apple targets link `dylib=raw` (the file is `libraw.dylib`, not
  `liblibraw.dylib`), and skip `stage_runtime_library`, which is Windows' DLL
  search order problem.
- `tauri.macos.conf.json`: add the dylib to `bundle.macOS.frameworks`.
  `tauri-build` merges the platform config even for plain `cargo` builds, copies
  every framework to `target/Frameworks`, and adds the
  `@executable_path/../Frameworks` rpath — so dev builds and the `.app` load it
  from the same relative place.
- Test binaries live one level deeper (`target/debug/deps`), so `build.rs` adds
  a second rpath, `@executable_path/../../Frameworks`, for them.
- `bindgen` finds the Command Line Tools' libclang unaided. In CI,
  `LIBCLANG_PATH` is now set on Windows only: `clang-sys` searches *only* that
  variable when it is set, so the old job-wide Windows path would break a Mac.
- `--no-default-features` is gone from the npm scripts and the CI macOS job,
  which now fetches the raw fixtures and requires the raw tests. `check.sh`'s
  fallback switches itself off now that the dylib exists.
- `check-libraw.sh --link`, and a CI step, assert with `otool -L` that the
  binary imports `@rpath/libraw.dylib`.

### Step 3 — LGPL and code signing (decided: document re-signing)

LGPL §6 requires that a user can replace LibRaw. On a signed, notarised app,
replacing a file in `Contents/Frameworks` breaks the bundle signature, and the
hardened runtime's library validation refuses a dylib signed by anyone else.
Two defensible positions:

- **Document relinking**: replace the dylib, then `codesign --force --deep -s -`
  the app (ad-hoc re-sign) and run it locally. Keeps library validation on.
- **Entitlement** `com.apple.security.cs.disable-library-validation`: a
  replacement loads without re-signing, but so could an injected library.

Chosen: the first. `NOTICE` now says how (with the `codesign` command). It still
wants a look from whoever reviews licensing — and note that the Mac App Store,
if it is ever used to distribute the app, is a separate question: an App Store
user cannot re-sign, so LGPL there needs its own answer.

### Step 4 — Make the develop preview stop blocking the UI (done)

With LibRaw in, Develop works through the **readback presenter**: render, read
the pixels back, send a BMP over `develop://`. It is fast enough on Metal. In a
release build with a 1400×933 frame, a slider step costs **about 5 ms**: render
0.5 ms, readback 4.5 ms (1.3 ms waiting for the GPU, 1.7 ms copying), BMP
encode 0.4 ms. Opening a photo adds 36–44 ms of GPU work once, in `prepare`
(upload and demosaic of the full raw).

The real problem is where that work runs. `develop://` is registered with
`register_uri_scheme_protocol`, the synchronous form. WKWebView calls a scheme
handler **on the main thread**, so every frame of a slider drag held up the
whole AppKit run loop — menus, window drags, typing — for the length of a
render. (WebView2 does the same on Windows; it hurt `thumb://` there, see
`docs/BENCHMARKS.md`.) It is now asynchronous and renders on the blocking pool,
as `thumb://` and `full://` do — on both platforms. Frames still go one at a
time: `useLatestFrame` keeps one in flight and rendering holds the develop
lock. Not yet exercised on Windows.

### Step 5 — Direct rendering on Metal (done)

Develop's photo is drawn straight into a `CAMetalLayer` beneath the webview,
instead of being read back, wrapped as a BMP and decoded by WebKit every frame.
On by default on a Mac; **Settings › About › Diagnostics › Draw the Develop
preview on the GPU** turns it off, and the images take over again.

- `present/appkit.rs`: an `NSView` hosting a `CAMetalLayer` we create, added to
  the webview's superview **beneath** the `WKWebView`, which is made
  non-opaque (`drawsBackground = NO`, the switch Tauri's own transparent
  windows use). The page cuts a hole: `html.direct-canvas` clears the
  backgrounds down to the canvas and hides only the Library's own layout —
  panels, dialogs and menus over Develop still show.
- `place()` converts the page's physical pixels to points (`backingScaleFactor`)
  and converts through the webview's coordinates to the superview's, so the
  flip is AppKit's job; frame changes are wrapped in a `CATransaction` with
  actions off, or the layer would animate every move for a quarter second.
  Moving and teardown run on the main thread and never wait for it (the
  presenter is called under the develop lock).
- **Colour:** the layer is tagged sRGB. Measured in the app: the same pixels
  through the surface and through the readback image are identical, value for
  value. A P3 (or EDR) layer, with the shader writing P3, is the way to let the
  preview show the gamut the Display P3 export already holds.
- The readback presenter stays alongside it rather than being replaced
  (`DevelopState::direct`), so every `develop://` image keeps working, and the
  URL handler on a webview thread never touches a swapchain.
- **The split view is one frame:** the edit drawn across the canvas, then the
  before over the left half without a clear (`render_to_surface`'s `left_of`),
  so it follows a drag like the rest. The page draws only the line and the
  labels.
- **Sliders are live on the surface.** Sliders commit to history on release, and
  nothing used to be drawn until then, on either path. The surface is sent the
  in-progress edit (`DirectFrame::draft`, only while it differs from the
  committed one) and draws it at once. **The histogram follows too**, on every
  platform: `develop_histogram` takes the same draft, one request at a time.
- **Measured in a release build** (the test bundle, M4, 2026-09-28, with
  `SOURCELIGHT_TRACE_FRAMES=1`, which prints each frame's Rust-side time):
  a fast Exposure drag drew **60 frames a second** — the display's rate — at
  1.7 ms a frame, with a 1.6 ms histogram beside each; a pan at 1:1, 60 a
  second at 0.3 ms; a split-view drag 1.9 ms a frame. Slower drags draw fewer
  frames only because the slider's value changes less often: the pointer moves
  in whole points.

Checked by hand in the app on the M4 (2026-09-28): fit and 1:1, panning,
dragging a slider, crop (the overlay's handles sit on the photo's corners, and
the whole frame shows while dragging), hold-to-peek, split (the before half
still while the after follows the drag), back to the Library, and a window
resize. `develop_render::the_direct_surface_fits_and_centres_the_photo_in_the_canvas`
checks the layout against a real raw, and
`the_split_view_draws_the_before_left_of_the_line_and_the_edit_right_of_it`
the split, column by column.

One trap found on the way: React mounting Develop twice in development turns
the surface on, off and on again, and the first surface's queued teardown gave
the webview its background back after the second had taken it away — a blank
canvas. Live surfaces are counted (`appkit::LIVE`).

### Step 6 — Test hygiene (done, bar the fixture-specific tests)

- `develop_render::the_readback_presenter_produces_a_usable_frame` fails its
  33 ms budget in a release build on the Mac (47 ms), and only because it timed
  the first frame after `prepare`, which still owed the photo's 36–44 ms upload
  and demosaic. It now waits for the device after `prepare`, so it measures what
  the budget means — slider to pixels.
- Two tests assume the six-camera fixture set rather than any folder of raws,
  and fail against the A6400 folder:
  `full_preview::every_fixture_offers_more_than_the_cached_preview` (an A6400
  ARW embeds only a 1616×1080 preview) and the halation test (its statistic
  depends on which frame sorts first). Neither is a macOS problem.
- `scripts/check.sh`, `check-libraw.sh` and `fetch-raw-fixtures.sh` are now
  executable in git.

### Effort left

An Intel build, if Intel Macs matter. The direct surface could move to a P3
layer, to show the gamut a Display P3 export holds.

Not simply by tagging the layer P3, though. ColorSync would then clip anything
beyond the screen's gamut channel by channel, which is the hue shift the
develop shader's own gamut map exists to avoid. And "is this screen P3" is not
a reliable switch: on the Mac mini (2026-09-28) both an AOC AG346UCD, a
QD-OLED panel with a wider gamut than P3, and a Dell S2722DGM report
`canRepresent(.p3) == false`, because their profiles come from EDID. The
accurate version renders into the screen's own primaries (`NSScreen.colorSpace`)
through the shader's gamut map, and tags the layer with that space so ColorSync
has nothing left to do; it follows the window between screens.

---

## Plan 2 — Feel like a Mac app

**Principle: native chrome and behaviour; keep the instrument.** `DESIGN.md` is a
deliberate identity — IBM Plex, flat neutral greys, squared corners — and pro
photo tools on the Mac keep their own look too. What
gives a webview app away is not its colours. It is a browser-style title bar
over its own toolbar, a generic menu bar, menus that are not menus, scrollbars
that never hide, and the missing small conventions. Fix those; do not
reskin the app in system widgets. Ordered by impact for the effort:

### Tier 1 — the window and the menu bar (done)

All four are in (`feat/macos-native`) and were checked in the app on an M4:
one bar in Browse, Cull, Develop and Compare with the traffic lights in the
rail's corner, dragging from the bars, full screen reclaiming the inset; menus
built from the keymap, with rating and labels from the Photo menu, ⌘Z, ⌘, ⌘1–4,
⌘W, ⌘F and ⌘A (now "Select all photos"); the window reopening where it was
left. Found on the way and fixed: WebKit's autocorrect on the search box, the
unprefixed `user-select` WebKit ignored, and day headings drawn over the rail
after a search emptied the grid. Bare keys (ratings, flags) are deliberately
not menu equivalents — they would fire while typed into a field.

1. **One title bar, not two.** In `tauri.macos.conf.json` only:
   `"titleBarStyle": "Overlay"`, `"hiddenTitle": true`, and a
   `trafficLightPosition` that centres the traffic lights in the top bar row.
   The top bar (`TopBar.tsx`) and the rail's top cell get
   `data-tauri-drag-region` (needs the `core:window:allow-start-dragging`
   permission). The rail's logo cell becomes the traffic lights' home: the
   logo moves down or goes on the Mac. In full screen the lights disappear, so
   the inset must too (`onResized` + `isFullscreen()`). Today the window shows
   a separate title bar above the app's own bar, which is the
   single most web-app thing about it.
2. **A real menu bar.** Today it is Tauri's generic default. Build it from the
   keymap (`lib/keymap.ts`), so the menus show the user's own rebound
   shortcuts and one action has one definition:
   - *SourceLight*: About, Settings… ⌘, (opens the Settings workspace), Hide,
     Quit.
   - *File*: Add Folder… ⌘O, Import from Card…, Export…, Show in Finder.
   - *Edit*: Undo/Redo routed to the catalogue undo when focus is not in a
     text field. Cut, Copy, Paste and Select All must remain the predefined
     items: they are what make ⌘C/⌘V work in a WKWebView text field at all.
   - *View*: Browse, Cull, Places, People, Develop as ⌘1–⌘5, Show Filters,
     thumbnail size, Enter Full Screen.
   - *Photo*: Rating, Flag and Colour Label submenus, Open in Develop, Find
     Similar, Move to Trash ⌘⌫. Items enable and disable with the selection.
   - *Window*, and *Help* (Keyboard Shortcuts; macOS adds menu search).
3. **Remember the window.** Size, position and full screen across launches
   (`tauri-plugin-window-state`). The first launch here came up at 1281×1320,
   not the configured 1500×940.
4. **Mac shortcuts.** The keymap already folds Ctrl and ⌘ into `mod` and shows
   ⌘. Add the conventions people type without thinking: ⌘, Settings, ⌘F search,
   ⌘⌫ Move to Trash, ⌘W closes the Lightbox or overlay, ⌘M, ⌃⌘F.

### Tier 2 — behaviour (done)

Checked in the app on an M4: the right-click menu is an NSMenu on a Mac, the
Mac keeps its own overlay scrollbars, the viewer pinches, ⌘-scrolls and pans
like Preview, and background work shows on the Dock tile, whose icon bounces once when
a run of more than 20 seconds ends while the app is in the background. Confirmations are
NSAlerts; ⌘Q or the close button during an export or a card import asks first
(the menu's Quit is the app's own item, since AppKit's terminates unasked);
folders dropped on the window join the library. The folder and export pickers
were already native. Elastic scrolling needed no change.

Photos drag out of the grid to Finder, Mail or an editor as their original
files, always as copies (fixed in Rust: a same-disk drop would otherwise move
the original). A folder dropped on the Dock icon joins the library too:
`Info.plist` declares folders as a document type the app can open, at rank
*Alternate* so Finder stays the default for opening them, and
`RunEvent::Opened` hands them to the same path as a drop on the window.

5. **Native context menus.** The photo and folder menus are drawn in HTML.
   Tauri's `Menu.popup()` gives a real `NSMenu`: keyboard navigation, type-ahead,
   the system look, VoiceOver. The star row becomes a *Rating* submenu with a
   check mark. Keep HTML popovers only where the content is not a list of
   commands.
6. **Scrolling.** The custom `::-webkit-scrollbar` styles turn off macOS overlay
   scrollbars, so bars never hide and ignore *System Settings → Appearance →
   Show scroll bars*. Drop them on the Mac. Keep `overscroll-behavior: none`
   on the page, but allow the elastic bounce inside the grid and rails.
7. **Trackpad.** Pinch to zoom in the Lightbox and Develop, two-finger swipe for
   next and previous, and smart zoom (double tap). `ZoomableImage` handles the
   wheel; check what WebKit sends for a pinch (`gesturechange`, or a wheel with
   `ctrlKey`) and handle both.
8. **Dialogs and panels.** The folder picker is already native. Use the native
   save panel for export destinations, and a native sheet for the questions
   that end or discard work (quitting mid-export, moving to the Trash). Rich
   in-app dialogs (Export, Import) stay as they are.
9. **Drag and drop.** Drop a folder on the window or the Dock icon to add it
   to the library; drag photos out to Finder, Mail or an editor.
10. **The Dock.** Progress on the Dock tile during analysis and export, and a
    notification when a long export finishes while the app is in the
    background.

### Tier 3 — identity and polish (icon and contrast done; signing needs a decision)

Done: the Mac icon is drawn into Apple's grid (`scripts/make-macos-icon.sh`),
and Increase Contrast is followed. Reduce Motion already was. Signing (15) is
still open: it needs an Apple Developer account. The icon and the mark at the top of the mode rail are the
same: a 3×3 grid lit from the top-left, drawn in `app-icon.svg`.

To change the icon: edit `app-icon.svg`, run `npx tauri icon app-icon.svg` for
every platform's set, write a 1024 px `app-icon.png` from it, then run
`scripts/make-macos-icon.sh` on a Mac for the Mac icon.

11. **App icon.** The icon is a full-bleed square. Recent macOS masks
    non-conforming icons into a squircle on a grey plate. Make a proper macOS
    icon (Icon Composer `.icon`, or at least Apple's rounded-rect grid with its
    margin).
12. **One name.** Done: the menu bar, Dock and window title say SourceLight.
    The identifier is `app.sourcelight`; the catalogue's location is
    derived from it, so it must not change again. A library from before the
    rename is moved across on first launch (`identity.rs`).
13. **System settings.** Honour Reduce Motion (`prefers-reduced-motion`) and
    Increase Contrast (`prefers-contrast`).
14. **No vibrancy.** Translucent sidebars are the obvious "Mac" move, but they
    tint the UI with whatever is behind the window, and `DESIGN.md`'s neutral
    greys exist so nothing colours the judgement of a photo. Leave it out.
15. **Signing.** Developer ID signing and notarisation, a styled `.dmg`, and the
    updater's macOS artefact. A Gatekeeper warning on first launch undoes all of
    the above.

### Open decisions

- LGPL in the Mac App Store, if that channel is pursued.
- Intel Macs: worth an x86_64 build?
- Native context menus trade the inline star row for a Rating submenu.
- macOS 14 as the minimum (from ONNX Runtime 1.24.4).
