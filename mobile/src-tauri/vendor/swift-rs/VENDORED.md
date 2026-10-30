# swift-rs 1.0.8, vendored with one change

Copied from crates.io (`swift-rs` 1.0.8, MIT OR Apache-2.0) and used through
`[patch.crates-io]` in `mobile/src-tauri/Cargo.toml`.

**Why.** Xcode 27's SwiftPM builds static Swift libraries with their `@_cdecl`
exports made local, so the iOS link fails. swift-rs 1.0.8 promotes them again
with `llvm-objcopy` (rustup's `llvm-tools` component), but only in the
package's own object file. Tauri's archive also holds swift-rs's runtime
(`SwiftRs.o`), and its exports `retain_object`, `release_object` and
`string_from_bytes` stayed local: undefined symbols when linking the phone
app for iOS.

**The change** (`src-rs/build.rs`, `globalize_cdecl_symbols`): in Tauri's
archive, the SwiftRs member's exports are promoted too. Only there, because
each package's archive carries its own copy of SwiftRs, and promoting it in
two of them gives Xcode 27's linker duplicate symbols.

**Dropping it.** Once a swift-rs release handles this, delete this folder and
the `[patch.crates-io]` entry.
