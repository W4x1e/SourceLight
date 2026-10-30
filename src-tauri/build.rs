// Only the LibRaw configuration below uses these, and that whole module is
// feature-gated — so without `libraw` (how macOS builds today) an unconditional
// import is dead, and clippy runs with `-D warnings`.
#[cfg(feature = "libraw")]
use std::path::{Path, PathBuf};

fn main() {
    tauri_build::build();

    #[cfg(feature = "libraw")]
    libraw::configure();
}

/// LibRaw is linked as a **shared** library, and that is a licensing
/// requirement rather than a preference. LibRaw is LGPL-2.1, which lets an
/// application under other terms (MPL-2.0 here) use it only when the library
/// stays separately replaceable — i.e. dynamically linked. Static linking would
/// make the whole executable a work based on LibRaw, under the LGPL's terms.
///
/// The vendored package ships both `libraw.lib` (a 176 KB import library) and
/// `libraw_static.lib` (a 4 MB static archive). We link the former. The size
/// difference is the quickest way to confirm which is which.
#[cfg(feature = "libraw")]
mod libraw {
    use super::{Path, PathBuf};

    pub fn configure() {
        let vendor = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("vendor/libraw");
        let target = std::env::var("TARGET").unwrap();

        let lib_dir = vendor.join("lib").join(&target);
        if !lib_dir.exists() {
            panic!(
                "no vendored LibRaw for target `{target}` (looked in {}).\n\
                 Vendor LibRaw for this platform (docs/LIBRAW.md), or build \
                 with --no-default-features to omit raw decoding.",
                lib_dir.display()
            );
        }

        println!("cargo:rerun-if-changed=wrapper.h");
        println!("cargo:rerun-if-changed={}", vendor.display());

        // `dylib=`, never `static=` — see the note above.
        println!("cargo:rustc-link-search=native={}", lib_dir.display());
        if target.contains("apple") {
            // The file is `libraw.dylib`, so the library is `raw`. It loads
            // through @rpath: listed in `bundle.macOS.frameworks`
            // (tauri.macos.conf.json), it is copied by tauri-build to
            // `target/Frameworks`, which `@executable_path/../Frameworks` —
            // tauri-build's rpath — reaches from the app in `target/<profile>/`
            // as it does from Contents/MacOS in the bundle. Test binaries sit a
            // level deeper, in `target/<profile>/deps/`, and need one more.
            println!("cargo:rustc-link-lib=dylib=raw");
            println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../../Frameworks");
        } else {
            println!("cargo:rustc-link-lib=dylib=libraw");
        }

        generate_bindings(&vendor);
        stage_runtime_library(&vendor, &target);
    }

    /// Generates `libraw_data_t` and friends from the vendored headers.
    ///
    /// Only the struct layout is generated. Hand-writing it would mean tracking
    /// a large C struct across LibRaw releases, and getting a field offset
    /// wrong yields plausible-looking garbage pixels rather than a crash.
    fn generate_bindings(vendor: &Path) {
        let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("libraw_bindings.rs");

        let bindings = bindgen::Builder::default()
            .header("wrapper.h")
            .clang_arg(format!("-I{}", vendor.join("include").display()))
            // Everything reachable from libraw_data_t, and nothing else.
            .allowlist_type("libraw_.*")
            .allowlist_var("LIBRAW_.*")
            .derive_debug(true)
            .derive_default(false)
            .layout_tests(true)
            .generate()
            .expect("failed to generate LibRaw bindings — is libclang installed?");

        bindings
            .write_to_file(&out)
            .expect("failed to write LibRaw bindings");
    }

    /// Copies `libraw.dll` next to the binaries cargo produces.
    ///
    /// Windows resolves a DLL from the executable's own directory, and cargo
    /// puts test executables in `deps/` rather than alongside the app binary —
    /// so both locations need a copy or `cargo test` fails to start with a
    /// missing-DLL error that says nothing about LibRaw.
    fn stage_runtime_library(vendor: &Path, target: &str) {
        let bin_dir = vendor.join("bin").join(target);
        if !bin_dir.exists() {
            return; // Platforms where the loader finds the library elsewhere.
        }

        // OUT_DIR is <profile>/build/<pkg>-<hash>/out; the profile dir is 3 up.
        let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
        let Some(profile_dir) = out.ancestors().nth(3) else {
            return;
        };

        let Ok(entries) = std::fs::read_dir(&bin_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let from = entry.path();
            let Some(name) = from.file_name() else {
                continue;
            };
            for dest_dir in [profile_dir.to_path_buf(), profile_dir.join("deps")] {
                if dest_dir.exists() {
                    let _ = std::fs::copy(&from, dest_dir.join(name));
                }
            }
        }
    }
}
