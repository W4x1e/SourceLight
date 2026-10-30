# Licensing

SourceLight's own licence, what it means for the people who use, change and
pass it on, and the rules for the third-party code, data and models inside it.

## SourceLight is MPL-2.0

SourceLight is free and open-source software, licensed under the **Mozilla
Public License, version 2.0**. The full text is [LICENSE](../LICENSE).

> This Source Code Form is subject to the terms of the Mozilla Public License,
> v. 2.0. If a copy of the MPL was not distributed with this file, You can
> obtain one at https://mozilla.org/MPL/2.0/.

That notice is kept at the top level, in [README.md](../README.md),
[NOTICE](../NOTICE) and LICENSE, rather than at the head of every file, as the
licence's Exhibit A allows. It covers every file of SourceLight's own. The
third-party files in the tree (LibRaw, the fonts, the geographic data, vendored
crates) keep their own licences, listed in NOTICE.

What it means in practice:

- **Using it.** Anyone may run SourceLight, for any purpose, on any number of
  machines. The licence grants rights to people who copy, change or
  distribute the software and asks nothing of someone who only runs it, so
  there is nothing to accept: no click-through in the app, no licence page in
  the installer. Settings → About shows the licence and the third-party
  notices. It comes with no warranty (sections 6 and 7).
- **Changing it.** The copyleft is per file. Distribute a build and the
  source of its MPL-covered files, your changes to them included, must be
  available under the MPL-2.0. Files you write yourself can be under any
  licence you like, and SourceLight can be combined with code under other
  terms, proprietary included, into a larger work (section 3.3). Only the
  MPL-covered files carry the obligation.
- **Contributing.** Contributors sign the
  [contributor licence agreement](../CLA.md) once. They keep the copyright in
  their work and grant the maintainer a licence to it that includes other
  terms than the MPL-2.0, and that passes to whoever takes the project over.
  That keeps the project's licensing in one place: it can be offered under
  other terms alongside the MPL-2.0, or handed on, without asking every past
  contributor. Versions already released under the MPL-2.0 stay under it.
  See [CONTRIBUTING.md](../CONTRIBUTING.md).
- **Redistributing it.** Anyone may distribute SourceLight, changed or not,
  under the MPL-2.0. With a build, say where its source is (section 3.2(a)):
  a link to the repository at the matching tag does it. Keep the notices, and
  meet the third-party obligations below.
- **The name and icon.** The licence grants no rights to them (section 2.3).
  [TRADEMARKS.md](../TRADEMARKS.md) says what is fine without asking. In
  short: a changed build or a fork needs its own name and icon.

## Third-party code

**Dependencies are permissive, by allow-list.** `src-tauri/deny.toml` lists
the licences a Rust dependency may carry (MIT, Apache-2.0, BSD, ISC, Zlib,
MPL-2.0 and a few more), and `cargo deny` fails the gate (`scripts/check.sh`)
and CI on anything else. The phone app's crates are held to the same file
(`check.sh --mobile`). GPL, AGPL, LGPL and CC-BY-SA are absent on purpose:
Rust links statically, so one such crate would put the whole binary under its
terms, or oblige everyone who distributes a build to make it relinkable, and
SourceLight could no longer be combined freely as the MPL intends. Do not
weaken the list to make a build pass. If only a copyleft library will do,
raise it first: the answer is usually a separately replaceable shared library,
which is how LibRaw is used.

**The notices that ship are generated, and checked.**

- `src-tauri/THIRD-PARTY-LICENSES.txt` lists every crate compiled into the
  Windows AI build with its licence text, plus the npm packages, fonts and
  bundled face models written into `about.hbs`. cargo-about generates it from
  the dependency graph (`src-tauri/about.toml`, whose accepted list mirrors
  deny.toml), and `./scripts/check.sh --ai` regenerates it and fails when the
  committed copy is stale. After changing dependencies:

  ```bash
  cargo install cargo-about --features cli
  cd src-tauri && cargo about generate --manifest-path Cargo.toml --features ai ../about.hbs -o THIRD-PARTY-LICENSES.txt
  ```

- [NOTICE](../NOTICE), written by hand, covers what the graph cannot see: the
  shared libraries loaded at runtime, the AI models and the geographic data.
- The installer carries both, with LICENSE, beside the program
  (`bundle.resources` in `src-tauri/tauri.conf.json`, held there by
  `tests/bundle_resources.rs` and checked in a built installer by
  `scripts/verify-installer.sh`). Settings → About shows LICENSE (compiled
  into the app) and THIRD-PARTY-LICENSES.txt.

**AI models: MIT or Apache-2.0, pinned.** The app downloads models on the
user's behalf, so a model whose licence is research-only, non-commercial or
conditional on registering with its publisher is never offered, however good.
It would put conditions on the user that SourceLight's own licence does not.
`ai_models::PERMISSIVE` is the list, `is_permissive` checks a catalogue entry
against it, and `every_model_is_permissively_licensed` in `ai_models.rs` fails
on any model outside it; `vlm.rs` holds the vision models to the same list.
Every file is pinned to a fixed revision and the SHA-256 it had when its
licence was read, so a publisher replacing or relicensing weights makes the
download fail rather than change what is installed.

## Obligations when you distribute a build

Besides the MPL-2.0 for SourceLight itself:

- **LibRaw (LGPL-2.1) must stay dynamically linked and replaceable**:
  `libraw.dll` beside the program on Windows, `libraw.dylib` in
  `Contents/Frameworks` on macOS, never linked statically. `build.rs` links the
  import library, and the gate and CI assert the import is dynamic. Ship
  `LICENSE.LibRaw.txt`, `COPYRIGHT.LibRaw.txt` and the NOTICE section that says
  where LibRaw's source is: that is the source offer. Changing LibRaw itself
  means publishing those changes. On macOS, replacing the library means
  re-signing the bundle, which NOTICE explains; an App Store build cannot be
  re-signed by its user and would need its own answer ([MACOS.md](MACOS.md)).
- **DirectML.dll is not open source.** It is a Microsoft redistributable under
  the Microsoft Software License Terms, which allow shipping it inside an
  application, unmodified, for Windows. Only the Windows AI build carries it,
  with `LICENSE.DirectML.txt` and its third-party notices; builds without
  `--features ai`, and every macOS build, contain no copy.
- **ONNX Runtime (MIT)** ships with its licence and third-party notices beside
  it, as the AI build already does.
- **GeoNames (CC BY 4.0)** names the towns, from data compiled into the app,
  and needs attribution. It is given in Settings → About, on the Places map
  and in NOTICE. Keep it, including in anything derived from that data.
- **Natural Earth** (the map's land outline) is public domain: nothing is
  owed, and it is credited anyway.
- **IBM Plex (SIL OFL 1.1)**: the fonts' licence travels with them
  (THIRD-PARTY-LICENSES.txt, and `src/fonts/`). They may not be sold on their
  own, and a modified version may not use the name "Plex".
- **The bundled face models** (YuNet, MIT; MediaPipe Face Mesh and Blendshape,
  Apache-2.0) have their notices in THIRD-PARTY-LICENSES.txt.
