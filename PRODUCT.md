# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

Rendered as a webview: a Tauri 2 desktop application (React 19 + TypeScript front end,
Rust core) shipped as a Windows NSIS installer, with a macOS (Apple silicon) `.app` and
`.dmg` built but not yet signed for release. There is no browser deployment. The phone app in
`mobile/` is a separate app; design this one for a resizable desktop window, minimum 940×620, default
1500×940, dark theme, no OS-native mobile affordances.

## Users

**Primary: enthusiast raw shooters.** Hobbyist and prosumer photographers with libraries
in the thousands of raw files, who have run out of patience with subscription software
and with folder hierarchies that force one filing decision. They are comfortable with
photographic vocabulary — stops, highlights/shadows, white balance, curves, masks — but
they are not the person who built the app. Design cannot assume author knowledge.

The author is a Sony a6400 shooter and the tool started from that workflow, but the
audience is explicitly wider than one user.

## Product Purpose

Find any photo in a large raw library without having filed it correctly in advance, then
develop it without leaving the app.

Folders force a single hierarchy — date *or* place *or* subject. This product leaves
photos where they are on disk and puts their attributes in an index, so "wildlife shots
from the Karoo trip last March" is a filter rather than a folder someone had to create
ahead of time.

Success is that a photographer can go from a card of raws to a finished, exported image
inside one application, and can find any past photo by the attributes they actually
remember about it.

## Positioning

**Where the product is heading: a raw editor whose library is the substrate.** The
develop pipeline is the committed direction of travel; the organiser is what makes it
usable at library scale. Future work centres the raw pipeline and image quality, and the
library must keep pace with it rather than being treated as finished.

Three things a neighbouring product could not truthfully copy as stated:

- **Filter counts computed against the *other* filters.** Every facet option shows a live
  count derived with the remaining filters applied, so options never collapse to zero the
  moment one is picked. This is a load-bearing interaction property, not a nicety.
- **Schema-order-constrained local AI tagging.** A vision model's output is constrained to
  a JSON schema whose *property order* is deliberate — observations before conclusions, so
  `main_subject` is committed before `genre`. It runs locally by choice, and `genre` is
  drawn from a fixed list so it is dependable as a filter while subjects and lighting stay
  open vocabulary.
- **A full non-destructive GPU develop pipeline inside a local-first organiser**, rather
  than a round-trip to a separate editor.

## Operating Context

- **Windows and Mac desktops**, one resizable window, dark. Keyboard, mouse and trackpad;
  no touch target. On a Mac the app follows Mac conventions (menu bar, ⌘ shortcuts,
  native context menus and alerts) while keeping its own look; see `docs/MACOS.md`.
- **The library lives on the user's own disk.** The app points at a directory of raw files
  and scans recursively; re-scanning picks up new files and skips indexed ones.
- **The index is a SQLite database** in the app's local data directory, alongside a cache
  of generated thumbnails. Deleting it loses tags and places — never photos.
- **Optional on-device AI.** Search, tagging and faces run in-process on ONNX models
  (SigLIP 2, YuNet, AuraFace) downloaded on first use. Detailed descriptions add a vision
  model (Qwen3-VL) on llama.cpp's `llama-server`, run by the app. There are no cloud
  providers. A photo leaves the machine only for a phone the user pairs, or for an AI
  app the user connects to the editing server (`PRIVACY.md`).
- **Places from GPS, trips from dates.** A photo's GPS is read at scan, or taken from a
  GPX track, and shown on a map with its town named. The a6400 records no location, so
  its photos are grouped into date-gap trips, and naming a trip places every photo in it
  without a position of its own.
- **Sessions are long and volume is high.** Scanning and tagging run as background batches
  with progress in the status bar; the user keeps browsing while they run and can stop and
  resume.
- **Judging the develop module requires a release build** — a debug build spends the whole
  frame budget JPEG-encoding each preview (~250ms vs ~50ms optimised).

## Capabilities and Constraints

**Library**

- Recursive scan and index of raw files through LibRaw — seven formats tested end to end
  (Sony ARW, Canon CR3, Nikon NEF, Fujifilm RAF incl. X-Trans, Olympus ORF, Panasonic RW2,
  Adobe DNG) and more accepted — plus JPEG, PNG, TIFF, WebP and HEIC; EXIF read;
  thumbnail cache; import from a memory card.
- Multi-dimensional filtering: date range, place, genre, subject, lighting condition,
  minimum rating, untagged-only, and free-text search over filenames, tags and places —
  all combining freely, all with sibling-aware counts.
- **SigLIP 2 semantic search** over local image embeddings: search the library by a
  natural-language description (in many languages), with names, places and dates in the
  query read as filters, or pick a photo and find visually similar ones. Plus a
  **duplicate/burst detector** for review. All on-device with no upload.
- **On-device AI sets itself up**: the first feature that needs a model downloads it (asking
  first on a metered connection, and always for the multi-gigabyte vision model), and a
  hardware probe picks the vision model the machine's GPU memory suits. Settings → AI shows
  what each feature has done and what is on disk.
- Virtualized grid with adjustable cell size (96–340px), multi-select (Ctrl/Shift-click),
  bulk place assignment, bulk tagging, and ratings 0–5.
- Keyboard-driven lightbox: `←`/`→` navigate, `0`–`5` rate, `Esc` close.
- Trip clustering by gaps in shooting time; naming a cluster applies its place to every
  photo in it.

**AI tagging** (entirely optional — indexing, browsing and manual tags all work without it)

- Providers are **local only**: the built-in SigLIP 2 tagger (in-process, no server) and
  a local vision model on `llama-server` (when Detailed descriptions are on). No account, no key, nothing leaves the device. (A hosted, opt-in
  cloud-tagging option is a possible future addition; it is not built.)
- Output is constrained to a JSON schema. **Property order is load-bearing** and protected
  by a test (`tagger::tests::schema_asks_for_main_subject_before_genre`); `serde_json`'s
  `preserve_order` feature is required, not a preference.
- **Manual tags are never overwritten by the AI.** Re-tagging replaces only AI-sourced tags.

**Develop** (non-destructive; behind the `libraw` Cargo feature, on by default)

- Exposure, warmth/tint relative to as-shot, contrast, highlights, shadows, whites, blacks.
- Point curve, per-band colour mixer (hue/saturation/luminance), colour grading wheels,
  detail, lens correction, look profiles, crop and geometry.
- Five mask types: radial, linear, brush, luminance range, colour range — each with its
  own local adjustments.
- Undo/redo history as whole-stack snapshots; the edit stack is a few hundred bytes of
  JSON in `photos.develop_settings`, versioned, with `#[serde(default)]` on every field so
  older stacks keep loading as the module grows.
- Export to JPEG, PNG, TIFF or WebP with ICC handling; batch export.
- Rendering is a `wgpu` GPU pipeline (DirectX 12 or Metal). Frames are read back and shown
  in the page's canvas; a direct GPU surface is built for Windows but not yet used.

**Constraints**

- **Sony ARW was the first format; multi-vendor raw has since landed** (via LibRaw, incl.
  Canon CR3, Nikon NEF, Fuji X-Trans). macOS runs everything on Apple silicon, Develop and
  on-device AI included, but is not yet signed or shipped (see `docs/MACOS.md`).
  UI language must not hard-code Sony or Windows assumptions (`src/lib/platform.ts` names
  Finder/Explorer and Trash/Recycle Bin).
- LibRaw is vendored for Windows x64 and Apple silicon; an Intel Mac build does not exist
  yet.
- Without the `libraw` feature the app still scans, indexes, tags and filters — it just
  cannot develop.
- Full sensor demosaicing is deliberately not on the hot path. Every ARW embeds a
  full-resolution JPEG preview; extracting it is what makes indexing thousands of raws
  practical. Develop and export decode the sensor through `raw::decoder::RawDecoder`.
- **Dependency licences are permissive-only**, enforced by `src-tauri/deny.toml` in CI.
  No GPL/LGPL/AGPL/CC-BY-SA in the Cargo tree — Rust links statically, so one copyleft
  crate would put the whole binary under its terms (`docs/LICENSING.md`).

**Open decisions**

- Whether and when macOS ships. (Multi-vendor raw has landed.)

## Brand Commitments

- **Name: SourceLight.** The identifier is `app.sourcelight`. Builds before the rename
  used `com.jamestully.photoorganiser`, and the app moves that data folder on first launch.
- Existing assets: `app-icon.svg` (the source), `app-icon.png` and `src-tauri/icons/`.
- Licence: free and open source, under the Mozilla Public License 2.0 (`LICENSE`,
  `docs/LICENSING.md`). Nothing has to be accepted to use it. Outside contributions are
  welcome under the same licence (`CONTRIBUTING.md`).

No aesthetic direction, palette or typography is recorded here — that is design authority,
not product truth, and lives with the incumbent implementation.

## Evidence on Hand

- `README.md` — user-facing feature description, keyboard reference, architecture sketch,
  and the reasoning behind the tag schema order and the preview-over-demosaic decision.
- `CONTRIBUTING.md` — build requirements per platform, the check gate, licence policy.
- `docs/ARCHITECTURE.md` and `docs/CODEBASE-GUIDE.md` — how the system is built.
- The running application itself is the strongest evidence: a mature, heavily
  commented implementation with a coherent existing visual system.
- **No user evidence exists.** There are no users, testimonials, case studies, reviews,
  press, benchmarks, download counts or performance claims beyond the two figures
  measured in `CONTRIBUTING.md` (~250ms debug vs ~50ms release per preview frame) and the
  ~1s-per-photo tagging figure in `README.md`. Future work must not invent any of these.
- There is no launch date and no availability claim yet.

## Product Principles

1. **The photos are the only colour that matters.** Everything the interface does is in
   service of seeing and judging images; chrome recedes.
2. **Never touch the original.** The app only ever reads raw files. Every edit, tag, rating
   and place is index-side state. This is a stated guarantee, not an implementation detail.
3. **Works fully offline, always.** No core function may depend on a network, and no account
   is ever required. AI tagging is optional and runs entirely on-device; if a hosted
   cloud-tagging option is ever added it will be strictly opt-in, never a requirement.
4. **No account, no subscription.** SourceLight is free and open source. Nothing is sold,
   metered or locked, and no feature may come to depend on a payment or a sign-in. This
   constrains what can be built and how it is described.
5. **Attributes, not hierarchies.** Anything a user might want to find photos by becomes a
   filterable attribute with an honest count — never a folder they had to create in
   advance.
6. **Human intent outranks the machine.** Manual tags survive re-tagging. Automation fills
   gaps; it never overwrites a decision a person made.

> Speed at library scale is a strong characteristic of the implementation — embedded-preview
> extraction, a virtualized grid, release-build frame budgets — but it was **not** confirmed
> as a binding promise that outranks completeness. Treat it as a quality bar, not a licence
> to cut features.

## Accessibility & Inclusion

No product-specific accessibility requirement or standard has been established. Two facts
constrain future work regardless:

- The interface is dark-only and low-contrast by intent, because a neutral shell is how a
  photo tool avoids biasing colour judgement. Contrast decisions must be made carefully
  within that constraint rather than by abandoning it.
- The app is keyboard-driven in its densest surfaces (lightbox, grid selection, ratings);
  keyboard parity is an existing expectation, not a new one.
