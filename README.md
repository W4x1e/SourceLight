# SourceLight

A raw photo editor and organiser that runs on your own computer. No account, no
subscription, no telemetry.

It indexes the folders you already have and never moves or changes your original files.
Ratings, tags, places and edits live in a local SQLite catalogue. The app backs that
catalogue up before upgrades, and Settings → Library can take or restore a backup at any
time.

## Features

- **Library:** filter by date, place, subject, rating, flag and colour label. Collections
  and saved searches. A fast grid, lightbox and compare view for culling, plus duplicate
  and burst grouping.
- **Search:** type what's in the photo ("dog on a beach at sunset") or find similar
  shots. Runs on your machine.
- **Optional AI:** tagging, faces, subject and click-to-select masks, and photo
  descriptions. Models download on first use and can be removed in Settings → AI.
- **Develop:** a non-destructive GPU pipeline with white balance, tone, curves, HSL,
  colour grading, detail and noise reduction, dehaze, masks, healing, magic erase, crop,
  lens corrections, grain and vignette.
- **Places:** GPS photos go on a map. Photos without GPS can be placed from a GPX track or
  grouped by trip.
- **Export:** JPEG, PNG, TIFF or WebP in sRGB, Display P3 or Adobe RGB, with metadata kept
  or stripped. XMP sidecars carry ratings, labels and keywords.
- **Phone app:** browse, cull and edit away from the desk ([docs/MOBILE.md](docs/MOBILE.md)).

Raw files are decoded with [LibRaw](https://www.libraw.org/). ARW, CR3, NEF, RAF, ORF, RW2
and DNG are tested against real files. Other formats LibRaw reads should work but aren't
tested yet. JPEG, PNG, TIFF, WebP and HEIC are supported too.

## Status

In active development. Windows is the main platform. macOS on Apple silicon (14 or later)
works but builds aren't signed yet ([docs/MACOS.md](docs/MACOS.md)).

## Building

You need Node.js 20+, Rust 1.91+ and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).
On Windows you also need LLVM (for `libclang`) and Git for Windows.

```bash
npm ci
npm run tauri dev       # without AI
npm run tauri:ai        # with on-device AI (run ./scripts/fetch-ort.sh first on Windows)
npm run tauri:build:ai  # installer
```

[CONTRIBUTING.md](CONTRIBUTING.md) has the full setup and the checks to run before a pull
request. [docs/README.md](docs/README.md) lists the rest of the docs.

## Contributing

Bug reports and pull requests are welcome. A raw file that decodes wrongly is especially
useful, with a sample if you can share it. You'll be asked to sign the [CLA](CLA.md) on
your first pull request. Report security issues privately ([SECURITY.md](SECURITY.md)).

## Licence

[Mozilla Public License 2.0](LICENSE). Third-party notices are in [NOTICE](NOTICE). The
name and logo are covered by [TRADEMARKS.md](TRADEMARKS.md).
