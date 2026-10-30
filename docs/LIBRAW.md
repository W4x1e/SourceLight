# LibRaw: tracking and rolling forward

`libraw.dll` is a C/C++ raw parser that every file in a scanned folder reaches.
That makes it the app's largest attack surface. It is
also invisible to `cargo deny check advisories`, which only walks the Cargo tree.
This page covers how we notice a LibRaw security fix and how we ship it.

## What is vendored

[`src-tauri/vendor/libraw/VERSION`](../src-tauri/vendor/libraw/VERSION) is the
record, in plain `key=value` lines:

| key | meaning |
| --- | --- |
| `version` | the LibRaw release, e.g. `0.22.2` |
| `package` | the upstream archive it came from |
| `source` | where that archive is published |
| `dll_sha256` | SHA-256 of `bin/x86_64-pc-windows-msvc/libraw.dll` |
| `lib_sha256` | SHA-256 of `lib/x86_64-pc-windows-msvc/libraw.lib` (the import library) |
| `macos_package` | the source release the macOS dylib is built from, e.g. `LibRaw-0.22.2.tar.gz` |
| `macos_source_sha256` | SHA-256 of that tarball; the build script refuses any other |
| `dylib_sha256` | SHA-256 of `lib/aarch64-apple-darwin/libraw.dylib` |

The file is checked, not just written down:

- **`raw::libraw::tests::links_against_the_vendored_libraw`** (needs the `libraw`
  feature) asserts that the DLL actually loaded reports the recorded `version`.
  A stray `libraw.dll` earlier on the load path fails it.
- **`vendored_libraw::the_binaries_match_the_record`** (no feature needed) hashes
  the Windows DLL and import library and the macOS dylib against the record. If
  any is replaced without updating it, this fails, on either platform.
- **`vendored_libraw::the_header_matches_the_record`** reads
  `include/libraw/libraw_version.h`. The bindings are generated from that header,
  so a DLL rolled forward without its headers fails here.

## Noticing a release

LibRaw publishes no CVE feed of its own. Security fixes arrive as ordinary
releases, sometimes as a lettered re-release such as `0.21.5b`. Three things,
in increasing order of effort:

1. **Watch the repository for releases.** On
   <https://github.com/LibRaw/LibRaw>, choose *Watch → Custom → Releases*. This
   is the subscription. It has to be done from the maintainer's own GitHub account.
2. **Run the tripwire.** `./scripts/check-libraw.sh` compares `VERSION` with the
   newest upstream tag and exits non-zero when we are behind. `./scripts/check.sh --upstream`
   runs it as part of the local gates. It needs the network, so it is opt-in.
3. **Check the advisories** when a release lands: GitHub's advisory database
   (<https://github.com/advisories?query=libraw>) and NVD
   (<https://nvd.nist.gov/vuln/search/results?query=libraw>). A release
   that fixes a CVE is a roll-forward now. A release that only adds cameras can
   wait for the next planned update.

## Rolling forward

The DLL is dynamically linked (enforced; see below), so a patched LibRaw needs
no change to our code unless its ABI moved.

1. **Fetch** the official Windows package from <https://www.libraw.org/download>
   (`LibRaw-<version>-Win64.zip`). Use the project's own build. We ship it
   unmodified (`NOTICE`).
2. **Replace** these from the archive:
   - `bin/libraw.dll` → `src-tauri/vendor/libraw/bin/x86_64-pc-windows-msvc/libraw.dll`
   - `lib/libraw.lib` → `src-tauri/vendor/libraw/lib/x86_64-pc-windows-msvc/libraw.lib`
     (the import library, **not** `libraw_static.lib`)
   - `libraw/*.h` → `src-tauri/vendor/libraw/include/libraw/`
   - `COPYING.LGPL`/`LICENSE.LGPL` and `COPYRIGHT`, if they changed
3. **Rebuild the macOS dylib.** LibRaw's own macOS package is static-only (and
   built for a newer macOS than the app supports), so the Mac library is built
   from the source release: set `version` in `VERSION`, delete
   `macos_source_sha256` (the new tarball has a new hash), and run
   `./scripts/vendor-libraw-macos.sh` on a Mac. It checks the new headers match,
   builds with upstream's own makefile and flags for the app's minimum macOS,
   and prints the three `macos_*`/`dylib_sha256` lines to record. The same Xcode
   gives the same bytes; a different one may not, which is why the dylib's own
   hash is recorded too.
4. **Update the record.** Set `version`, `package`, and the hashes in
   `VERSION`, using `sha256sum` (or `shasum -a 256`) on the binaries. Update the
   LibRaw section of `NOTICE`.
5. **Regenerate the bindings.** `bindgen` runs from `build.rs` against the new
   headers on the next build: `cargo clean -p sourcelight && cargo build`. If a
   struct we read changed layout, it fails to compile here rather than misreading
   memory at runtime.
6. **Run the format fixtures.** `./scripts/check.sh --raw`, with
   `SOURCELIGHT_TEST_RAW` pointing at the fixture folder, decodes a real file
   of every supported format through the new DLL.
7. **Re-run the dynamic-link assertion.** `./scripts/check-libraw.sh --link`
   runs `dumpbin /dependents` (Windows) or `otool -L` (macOS) on the test binary
   and fails unless LibRaw is an import. This is the LGPL requirement, and it is also what makes step 2
   enough on its own.
8. **Ship.** Add a changelog entry that names the LibRaw version and any CVEs it
   fixes.

## Not done: process isolation

Decoding still runs in the app process, so a LibRaw memory-safety bug is a
potential code-execution path there. Isolating it in a separate process is the
real fix. It is deliberately deferred as disproportionate at this stage,
and this page is the tripwire that makes deferring it reasonable.
