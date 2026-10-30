# How SourceLight is built

The map of the system: what runs, where data lives, and how a click becomes
work. Start here, then read [CODEBASE-GUIDE.md](CODEBASE-GUIDE.md) for the
module-by-module tour (with Rust explained along the way).

The diagrams are [Mermaid](https://mermaid.js.org/), which GitHub renders in
place. In an editor, a Mermaid preview extension shows them.

---

## 1. The system at a glance

SourceLight is one desktop app built with **Tauri 2**: a native **Rust core**
that does all the work, and a **React + TypeScript** interface that runs in the
operating system's own webview (WebView2 on Windows, WKWebView on a Mac). The
only other process it ever runs is optional: llama.cpp's **`llama-server`**,
which runs the vision model behind *Detailed descriptions*, started and
stopped by the app itself.

```mermaid
flowchart TB
    user(["Photographer"])

    subgraph app["SourceLight — one process"]
        direction LR
        ui["React interface<br/>src/"]
        core["Rust core<br/>src-tauri/src/"]
        ui -- "commands (typed IPC)" --> core
        core -- "events, and images over<br/>thumb:// full:// develop://" --> ui
    end

    subgraph native["Native libraries, in the same process"]
        direction LR
        libraw["LibRaw<br/>raw decoding"]
        gpu["wgpu<br/>Metal / DirectX 12"]
        ort["ONNX Runtime<br/>CoreML / DirectML / CPU"]
        osapi["OS services<br/>dialogs, menus, Trash,<br/>Dock, HEIC codecs"]
    end

    subgraph disk["On disk"]
        direction LR
        photos[("Photo folders<br/>read only")]
        db[("library.db<br/>the catalogue")]
        cache[("thumbs/<br/>previews")]
        models[("models/<br/>AI models")]
    end

    subgraph sidecar["Optional: detailed descriptions"]
        direction LR
        runner["llama-server · 127.0.0.1, a free port<br/>runs the vision model"]
    end

    net{{"Internet — model and runtime downloads, pinned and hash-checked,<br/>and the update check once one is set up"}}

    user --> ui
    core --> native
    core --> disk
    core -- "HTTP on loopback;<br/>starts and stops it" --> sidecar
    core -. "with consent" .-> net
```

Three rules explain most of the design:

- **The originals are never written.** Everything you decide — ratings, tags,
  places, people, edits — lives in `library.db`. Delete it and you lose the
  notes, never a photo. (Writing an `.xmp` sidecar beside a photo, or moving a
  reject to the Trash, happens only when you ask.)
- **The core owns data and work; the interface only shows and asks.** React
  never touches a file or the database. It calls a command and waits, or
  listens for progress events.
- **The photos stay on the machine.** No account, no telemetry. The network
  is used to download models and the llama.cpp runtime, each pinned to a
  version and checked against a SHA-256 in the source, and to check for
  updates once an update server is set up. A photo leaves only for a phone
  the user pairs, or for an AI app the user connects to the editing server.
  [PRIVACY.md](../PRIVACY.md) lists every request.

---

## 2. The interface ↔ core bridge

Every feature is a variation of the same loop.

```mermaid
sequenceDiagram
    autonumber
    participant C as React component
    participant S as store/ slice (Zustand)
    participant A as lib/api.ts
    participant B as bindings.ts (generated)
    participant R as a commands/ file
    participant M as module (db, scan, gpu…)

    C->>S: action, e.g. setFilter()
    S->>A: api.getPhotos(filter)
    A->>B: commands.getPhotos(filter)
    B->>R: invoke("get_photos", …)
    R->>M: db::photos(…)
    M-->>R: Vec<Photo>
    R-->>B: Result<Vec<Photo>, String>
    B-->>A: { status: "ok", data }
    A-->>S: Photo[] (or throws)
    S-->>C: state changes, component re-renders
```

- **Commands** are Rust functions marked `#[tauri::command]` and
  `#[specta::specta]`, one file per area in `commands/`, and listed in
  `specta_builder()` in `lib.rs` (234 of them). `export_bindings` generates
  `src/bindings.ts` from them, so the two languages always agree on types. `./scripts/check.sh` fails if the file is
  stale.
- **Where a command runs.** A plain `#[tauri::command]` on a sync function
  runs on the **main thread**: while it works, the window, its menus and every
  other command wait. That is right for quick setters that must apply in
  order (a rating, a slider). Anything that reads a lot, touches the disk or
  runs a model is `#[tauri::command(async)]`, which runs the same body on a
  worker thread; its answers can then arrive out of order, so the interface
  keeps only the latest (`reloadSeq` in `store/library.ts`). One deliberate
  exception: `render_direct`, which draws Develop's photo into the GPU layer
  beneath the page, stays on the main thread — it moves that layer, which is a
  window operation, and done there the move lands with the frame drawn into
  it. It costs about 2 ms a frame. Develop's histogram is async.
- **Events** go the other way: long jobs `emit` progress (`scan-progress`,
  `caption-progress`, `ai-download-progress`…) and the interface `listen`s.
- **Custom URL schemes** carry images, which are too big for JSON:
  `thumb://` serves cached previews, `full://` the full-resolution embedded
  JPEG for a 1:1 loupe, `develop://` each rendered Develop frame. All three
  are asynchronous, so a slow file never blocks the window.

---

## 3. What happens when…

### …you add a folder

```mermaid
sequenceDiagram
    autonumber
    actor U as You
    participant UI as Interface
    participant L as commands/library.rs
    participant SC as scan.rs
    participant RW as raw/ and raster.rs
    participant DB as library.db
    participant AI as Automatic analysis

    U->>UI: Add folder… (or drop it on the window or Dock)
    UI->>L: add_library_folder, then scan_library
    L->>SC: walk the folder (off the async threads)
    loop every photo file
        SC->>RW: embedded preview + EXIF (raw)<br/>or the image itself (JPEG, HEIC…)
        RW-->>SC: 1600 px preview, metadata, focus score
        SC->>DB: insert or update the row
        SC-->>UI: scan-progress
    end
    L->>DB: name places from GPS, refresh statistics
    L-->>UI: ScanResult (added, missing, failed + first reason)
    UI->>AI: runAnalysis(): locations, focus, search index,<br/>tags, faces, then descriptions in batches
```

**Why previews, not full decodes.** Every raw embeds a camera-rendered JPEG.
Extracting it costs milliseconds; demosaicing the sensor costs seconds. So the
library runs entirely on previews, and the sensor is decoded only when a photo
is opened in Develop or exported.

### …you drag a slider in Develop

```mermaid
sequenceDiagram
    autonumber
    participant UI as Develop.tsx
    participant L as commands/ (develop.rs, render.rs)
    participant D as develop/ (edit stack)
    participant G as gpu/ (wgpu pipeline)
    participant P as present/ (readback)

    Note over UI,G: Opening a photo: LibRaw decodes the sensor once,<br/>and the mosaic pass demosaics it on the GPU (cached).
    UI->>L: set_develop_stack(stack)
    L->>D: validate and store (undo history = whole snapshots)
    UI->>L: GET develop://frame?…
    L->>G: develop pass: every adjustment, in one shader
    G->>P: read the frame back from the GPU
    P-->>UI: an image, drawn in the canvas (about 5 ms a frame on an M4)
```

The edit stack is a few hundred bytes of versioned JSON in the photo's
`develop_settings` column. Export runs the *same* shader at full resolution,
so the preview and the file cannot disagree.

### …you turn on Detailed descriptions

```mermaid
sequenceDiagram
    autonumber
    actor U as You
    participant UI as aiReady.ts
    participant L as ensure_ai_ready (commands/ai.rs)
    participant H as hardware.rs + vlm.rs
    participant P as llama_pack.rs + ai_models.rs
    participant R as llama-server (sidecar.rs)

    U->>UI: Detailed descriptions: On
    UI->>L: ensure_ai_ready("captions", consented = false)
    L->>H: how much GPU memory? → pick a model
    L-->>UI: needs: runtime 32 MB + Qwen3-VL 4B 3.0 GB
    UI->>U: one dialog naming both, and the total
    U->>UI: Download and set up
    UI->>L: ensure_ai_ready("captions", consented = true)
    L->>P: download the pinned llama.cpp build, check its SHA-256, unpack
    L->>P: download Qwen's GGUF + projector, each pinned and hash-checked
    L->>R: start on the model's files, on the right GPU, wait until loaded
    L-->>UI: ready
    Note over L,R: Tags: a 640 px image, up to four photos at once (0.23 s each on an RTX 5070 Ti, 2.3 s on an M4).<br/>Descriptions: 1280 px, so text stays legible.<br/>Idle for two minutes, or quit: the server is stopped.
```

It replaced a private Ollama, whose API made every photo slower: see
[AI-MODEL-SPIKES.md](AI-MODEL-SPIKES.md). On Windows the runtime is the Vulkan
build, which runs NVIDIA, AMD and Intel GPUs alike and falls back to the CPU;
the server runs on the GPU the other models use, or on the CPU when
there is only integrated graphics. There it also runs in a job object the app
holds, so it ends with the app even when the app crashes or is ended from Task
Manager (`sidecar::die_with_app`).

The same ladder serves every AI feature. For the ONNX models (search, tags,
faces, masks) it downloads into `models/` instead, and nothing needs a server.

---

## 4. The Rust core, by responsibility

```mermaid
flowchart TB
    lib["lib.rs — AppState, the command list, startup, the run loop"]
    commands["commands/ — every command, one file per area"]

    subgraph data["Catalogue"]
        db["db/ — SQLite schema, 52 migrations, queries by area"]
        models_rs["models.rs — types shared with the UI"]
        backup["backup.rs — snapshots before risky changes"]
        fts["fts.rs — safe full-text queries"]
    end

    subgraph ingest_g["Getting photos in"]
        scan["scan.rs — walk folders, index files"]
        ingest["ingest.rs — import from a memory card"]
        raw["raw/ — LibRaw, embedded previews*"]
        raster["raster.rs — JPEG, PNG, TIFF, WebP, HEIC*"]
        exif["exif_read.rs*"]
        quality["quality.rs — focus and exposure scores"]
        dedup["dedup.rs — byte-identical copies"]
    end

    subgraph places_g["Places and time"]
        geo["geo.rs — GPS to town, offline"]
        gpx["gpx.rs — place from a phone's track"]
        trips["trips.rs — group by date gaps"]
    end

    subgraph develop_g["Develop and export"]
        develop["develop/ — the edit stack, masks, looks, auto*"]
        gpu_m["gpu/ — wgpu pipeline, WGSL shaders*"]
        present["present/ — getting frames on screen"]
        export["export/ — files, ICC, EXIF, batches"]
        full["full_preview.rs — 1:1 loupe pixels"]
    end

    subgraph ai_g["On-device AI"]
        ai_setup["ai_setup.rs — the readiness ladder"]
        ai_models["ai_models.rs — ONNX model downloads"]
        tagger["tagger/ — built-in (SigLIP 2), and the vision model on llama-server"]
        search["search.rs — embeddings and ranking"]
        faces["faces.rs + people.rs — faces into people"]
        vlm["vlm.rs — vision-model catalogue and fit"]
        hardware["hardware.rs — GPU, memory, disk, metered"]
        sidecar["sidecar.rs + llama_pack.rs — llama-server, and its runtime download"]
    end

    subgraph interop["Interop and the app itself"]
        xmp["xmp.rs — sidecars to and from Lightroom"]
        shell["shell.rs — Finder/Explorer, Trash"]
        jobs["jobs.rs — cancellable long jobs"]
        updates["updates.rs — updater (off until configured)"]
        crash["crash.rs — crash and error logs"]
        phone["phone_link.rs — the phone link (sl-link, sl-protocol)"]
        mcp["mcp.rs — the AI editing server, off by default"]
    end

    lib --> commands
    commands --> data & ingest_g & places_g & develop_g & ai_g & interop
```

\* In the develop core, `src-tauri/crates/sl-develop`, which the phone app
shares. `lib.rs` re-exports it under the same names, so the rest of the app
reads `crate::gpu` or `crate::raster` as before. The app's own `develop/` and
`raw/` add what only the desktop does: LibRaw (`raw/libraw.rs`), the AI masks,
noise reduction's and erase's models, and Develop's live state
(`develop/session.rs`).

Platform-specific code is kept in a few named places, each with a Windows and
a Mac branch: `present/` (drawing frames), `hardware.rs` (memory, metered
network), `raster/wic.rs` and `raster/imageio.rs` in sl-develop (HEIC),
`shell.rs` and `sidecar.rs` (stopping the
vision model's server, and on Windows the job object that ends it with the
app). Everything else — SQLite, wgpu, LibRaw, ONNX Runtime — is portable.

---

## 5. The interface, by responsibility

```mermaid
flowchart TB
    main["main.tsx — boot: menu, Dock progress,<br/>quit guard, folder drops"]
    app["App.tsx — the shell: mode rail, workspace,<br/>one overlay at a time, dialogs"]
    store["store.ts + store/ — shared UI state and the actions<br/>that call the core, one slice per area"]
    api["lib/api.ts → bindings.ts"]

    subgraph workspaces["Workspaces (the mode rail)"]
        browse["BrowseWorkspace — grid + FilterRail"]
        cull["CullWorkspace — loupe, survey, suggestions"]
        places["PlacesWorkspace — map, trips, GPX"]
        people["PeopleWorkspace — naming and merging"]
    end

    subgraph overlays["Overlays"]
        lightbox["Lightbox"]
        compare["CompareView — duplicates, bursts"]
        develop["Develop + dev/ panels and overlays"]
        settings["SettingsPanel — AI, Library, Export, About"]
    end

    logic["lib/*.ts — pure logic with tests beside it:<br/>keymap, queryReading, survey, stacks, suggest…"]

    main --> app --> workspaces & overlays
    workspaces & overlays --> store --> api
    workspaces & overlays --> logic
```

`npm run dev` on its own serves this interface to a browser with
`src/dev/mockBackend.ts` standing in for the core: a fake 48-photo library for
looking at layout without Tauri.

---

## 6. Where the data lives

```
<app data>/                      app.sourcelight in the OS's
│                                app-data folder, or SOURCELIGHT_DATA_DIR
├── library.db                   the catalogue (SQLite, WAL mode)
├── backups/                     catalogue snapshots: daily (a week kept), before
│                                upgrades and re-tags (3 kept), and manual ones
├── thumbs/                      <hash>_p.jpg previews, <hash>_t.jpg thumbnails
├── luts/                        imported .cube looks
├── logs/                        crash and interface error logs
├── models/                      ONNX models: siglip2/, faces/, birefnet(_fp16).onnx, sam2/
│   └── vision/                  the vision models' GGUFs, if downloaded
└── llama-runtime/               llama.cpp's llama-server, if downloaded
```

The catalogue's main tables:

```mermaid
erDiagram
    library_folders ||--o{ photos : "contains"
    photos ||--o{ photo_tags : ""
    tags ||--o{ photo_tags : ""
    photos ||--o{ collection_photos : ""
    collections ||--o{ collection_photos : ""
    photos ||--o{ photo_places : ""
    places ||--o{ photo_places : ""
    photos ||--o{ faces : "has"
    people ||--o{ faces : "is"
    shoots ||--o{ shoot_files : "imported"
    photos ||--o{ exports : "exported"
    photos ||--o| photo_embeddings : "searched by"
    faces ||--o| face_embeddings : "grouped by"
    photos {
        int id
        text path "the original, never written"
        text preview_path
        text develop_settings "edit stack, JSON"
        text caption "from the vision model"
        int rating
        int flag "pick or reject"
        real sharpness
    }
    faces {
        int person_id
        real sharpness
        int eye_open
    }
    photo_embeddings {
        blob embedding "SigLIP 2 vector, for search"
    }
    face_embeddings {
        blob embedding "AuraFace vector"
    }
```

Also: `trips`, `smart_collections`, `presets`, `face_links` and
`people_apart` (faces you said are, or are not, the same person), `settings`
(key–value), and `captions_fts` (full-text index over captions,
kept in step by triggers). The schema only ever grows by appended migrations
in `db/migrations.rs`, and the catalogue is backed up before any migration runs.

---

## 7. Builds and platforms

| | Windows | macOS |
|---|---|---|
| Webview | WebView2 | WKWebView |
| GPU | DirectX 12 | Metal |
| AI acceleration | DirectML; magic erase on the CPU | CoreML (faces, noise reduction); SigLIP 2, AI masks and erase on the CPU |
| LibRaw | `libraw.dll`, vendored | `libraw.dylib`, built by `scripts/vendor-libraw-macos.sh` |
| HEIC | WIC + Microsoft's free Store codecs | ImageIO, built in |
| Package | NSIS installer | `.app` and `.dmg` (Apple silicon, macOS 14+) |
| Status | the main platform | runs from source and bundles; unsigned (see `docs/MACOS.md`) |

Cargo features: `libraw` (on by default: raw decoding and Develop), `ai`
(ONNX Runtime: search, tags, faces, AI masks; `npm run tauri:ai`),
`dev-tools` (the binding generator and other developer binaries, never
shipped) and `phone-link` (the phone link; off in releases until the phone
app ships; `npm run tauri:phone`).
