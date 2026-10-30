# Vendored ONNX Runtime (DirectML on Windows x64, CoreML on macOS arm64)

The AI build (`--features ai`) loads ONNX Runtime as a shared library at runtime
(`ort`'s `load-dynamic`). Unlike a static link, nothing in the build emits
`onnxruntime.dll` — it must be vendored here and copied beside the executable by
the bundler, exactly as `../libraw/` is for LibRaw.

## What goes in this directory (git-ignored, fetched per machine)

| File | Source | Notes |
|------|--------|-------|
| `onnxruntime.dll`                  | ORT DirectML nupkg | DirectML EP is inside it; ~16 MB |
| `DirectML.dll`                     | DirectML redist    | the EP's dependency; ~18 MB |
| `LICENSE.OnnxRuntime.txt`          | ORT nupkg root     | MIT |
| `ThirdPartyNotices.OnnxRuntime.txt`| ORT nupkg root     | |
| `LICENSE.DirectML.txt`             | DirectML nupkg root| Microsoft redistributable EULA (not MIT) |
| `ThirdPartyNotices.DirectML.txt`   | DirectML nupkg root| |

Only `README.md` and `VERSION` are committed; everything else is `.gitignore`d
(binary + large) and fetched per machine, like the LibRaw and dev ONNX packages.
`VERSION` records which packages and the SHA-256 of each binary: a copy
fetched on another machine, or replaced on this one, fails
`tests/vendored_onnxruntime.rs` and `scripts/verify-installer.sh --ai` instead
of being signed into a release.

## Version — must match what `ort` targets

`ort = "=2.0.0-rc.10"` is built against **ONNX Runtime 1.22.0**
(`ort-sys`'s `ONNXRUNTIME_VERSION`). The dll must be **>= 1.22.0** (ORT's C API
is forward-compatible; older would be missing symbols `ort` calls). Use exactly
1.22.0 for parity.

## Where to fetch it

On Windows, `./scripts/fetch-ort.sh` does all of this: it downloads both
packages, takes the files in the table above, and checks the DLLs against
`VERSION`.

DirectML is **not** on `ort`'s CDN (its Windows builds are CPU/CUDA only) and
**not** on the microsoft/onnxruntime GitHub releases (CPU + CUDA zips only). It
ships via NuGet — a `.nupkg` is a zip:

- **`onnxruntime.dll` + notices** — `Microsoft.ML.OnnxRuntime.DirectML` **1.22.0**
  (<https://www.nuget.org/packages/Microsoft.ML.OnnxRuntime.DirectML/1.22.0>).
  DLL at `runtimes/win-x64/native/onnxruntime.dll`; `LICENSE` +
  `ThirdPartyNotices.txt` at the root. (This package tops out at 1.24.4 — there
  is no 1.28 DirectML build.)
- **`DirectML.dll` + its licence** — `Microsoft.AI.DirectML` **1.15.4** (the
  version `Microsoft.ML.OnnxRuntime.DirectML` 1.22.0 depends on). DLL at
  `bin/x64-win/DirectML.dll`; `LICENSE.txt` + `ThirdPartyNotices.txt` at root.

Do **not** use the CUDA (`-gpu`) build — it drags a 267 MB provider DLL and needs
an NVIDIA GPU + CUDA toolkit; see the note on the `ort` features in
`../../Cargo.toml`. `onnxruntime_providers_shared.dll` is deliberately not
vendored: it is only for out-of-process providers (CUDA/TensorRT); the DirectML
EP is compiled into `onnxruntime.dll`.

## Bundling — AI builds only

`bundle.resources` in the main `tauri.conf.json` is applied to **every** bundle,
so the ORT DLLs must not go there (a plain `tauri build` would then fail on the
missing files, or bloat by ~35 MB). They live in the overlay config
`../tauri.ai.conf.json` instead, merged in only by the AI build:

    npm run tauri:build:ai   # tauri build --features ai --config tauri.ai.conf.json

The overlay maps each file to a bare filename so it lands **beside the .exe**,
where the Windows loader and `ort`'s `LoadLibrary("onnxruntime.dll")` fallback
find it with no `ORT_DYLIB_PATH` needed. On a fresh checkout the AI build errors
until the files above are fetched into this directory.

## macOS (Apple silicon)

| File | Source | Notes |
|------|--------|-------|
| `libonnxruntime.dylib`             | `onnxruntime-osx-arm64-1.24.4.tgz`, `lib/` | CoreML EP is inside it (used for the face models; SigLIP 2 runs on the CPU, see `tagger::builtin`); ~35 MB. Copy the real file, not the symlink (`cp -L`) |
| `LICENSE.OnnxRuntime.txt`          | same archive, `LICENSE`             | MIT |
| `ThirdPartyNotices.OnnxRuntime.txt`| same archive, `ThirdPartyNotices.txt` | |

From <https://github.com/microsoft/onnxruntime/releases/tag/v1.24.4>. Drop the
`.dSYM` beside the dylib; it is 70 MB of debug symbols nobody ships.

**Why 1.24.4 and not the Windows 1.22.0.** Measured on an M4 with the same
models and photos:

- *Quitting.* With 1.22.0 a process that has run a model aborts on the way out
  (`mutex lock failed: Invalid argument`, from ONNX Runtime's own C++ static
  destructors) unless the environment is released first — a crash report for
  every quit. 1.24.4 exited cleanly even without that. The app releases it on
  exit anyway (`develop::ai::release_ort`), which covers both.
- *Speed.* SigLIP 2 indexed 73 photos on the CPU in 21.8 s on 1.24.4 against
  28.4 s on 1.22.0.
- *CoreML* compiles the face models on both and cannot compile SigLIP 2's
  export on 1.24.4 — which costs nothing, because on 1.22.0, where it did
  compile, it was no faster than the CPU. The tagger skips it on a Mac.

1.24.4 is the newest release `ort` 2.0.0-rc.10 accepts (1.30 fails its API check
at load). It needs macOS 14, hence `minimumSystemVersion` in
`../../tauri.macos.conf.json`. It is also the last DirectML release, so Windows
can move to it too.

Build and bundle with the Mac overlay after the AI one, so the Windows DLLs are
dropped and the dylib goes into `Contents/Frameworks`, where
`develop::ai::ensure_ort_dylib` looks for it:

    npm run tauri:build:ai:mac

From source, `ensure_ort_dylib` finds the dylib here too, so `npm run
tauri:ai` needs no `ORT_DYLIB_PATH`.
