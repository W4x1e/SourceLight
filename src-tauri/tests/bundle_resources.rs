//! What each installer is configured to ship — checked without building one.
//!
//! The AI installer is the build that ships, and it is configured as the base
//! `tauri.conf.json` with `tauri.ai.conf.json` laid over it. The overlay lists its
//! own `bundle.resources` (ONNX Runtime, DirectML) and does *not* repeat LibRaw.
//! Whether LibRaw survives depends entirely on how Tauri combines the two: a
//! merge keeps it, a replace drops it — and then raw develop is dead in exactly
//! the installer most people use, while the base build everyone tests works
//! fine.
//!
//! Tauri merges `--config` overlays as an RFC 7396 merge patch, with the
//! `json-patch` crate. So does this test — the same crate, the same function —
//! which is what makes it an answer rather than a guess about Tauri's rules. A
//! real AI build confirmed it (every resource below present in the generated
//! installer script); this keeps a future edit to either file from quietly
//! undoing it. `scripts/verify-installer.sh` checks the built artifact itself.

use serde_json::Value;

fn config(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// The installed file names a configuration bundles: the values of the
/// `bundle.resources` map, which is how this project declares them.
fn installed(config: &Value) -> Vec<String> {
    let resources = config
        .pointer("/bundle/resources")
        .and_then(Value::as_object)
        .expect("bundle.resources should be a source -> destination map");
    let mut names: Vec<String> = resources
        .values()
        .map(|v| v.as_str().expect("destination is a file name").to_string())
        .collect();
    names.sort();
    names
}

/// LibRaw and the licence files every build owes its users. Dropping
/// `libraw.dll` kills raw develop; dropping a licence breaks the terms the
/// library is used under (LibRaw is LGPL; the rest are listed in the notice).
/// `LICENSE.txt` is SourceLight's own MPL-2.0: shipped as a file rather than
/// as an installer page, because there is nothing for anyone to accept.
const BASE: [&str; 6] = [
    "COPYRIGHT.LibRaw.txt",
    "LICENSE.LibRaw.txt",
    "LICENSE.txt",
    // What THIRD-PARTY-LICENSES.txt refers its readers to.
    "NOTICE.txt",
    "THIRD-PARTY-LICENSES.txt",
    "libraw.dll",
];

/// What the AI build adds: the runtime it loads dynamically, its GPU provider,
/// and their notices.
const AI: [&str; 9] = [
    "DirectML.dll",
    "LICENSE.DirectML.txt",
    "LICENSE.OnnxRuntime.txt",
    "ThirdPartyNotices.DirectML.txt",
    "ThirdPartyNotices.OnnxRuntime.txt",
    // The face detector (ai_models::BUNDLED), where install_bundled looks.
    "models/faces/blendshapes.onnx",
    "models/faces/mesh.onnx",
    "models/faces/yunet.onnx",
    "onnxruntime.dll",
];

fn sorted(names: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    v.sort();
    v
}

#[test]
fn the_base_installer_ships_libraw_and_its_licences() {
    assert_eq!(installed(&config("tauri.conf.json")), sorted(&BASE));
}

#[test]
fn the_ai_installer_ships_everything_the_base_does_and_onnx_runtime() {
    let mut merged = config("tauri.conf.json");
    json_patch::merge(&mut merged, &config("tauri.ai.conf.json"));

    let expected: Vec<&str> = BASE.iter().chain(AI.iter()).copied().collect();
    assert_eq!(
        installed(&merged),
        sorted(&expected),
        "the AI overlay must add to the base resources, not replace them — \
         without libraw.dll the build that ships cannot develop a raw"
    );
}

/// Tauri lays `tauri.macos.conf.json` over the base on a Mac, before any
/// `--config`. There is no LibRaw for macOS yet, so the DLL must go — the
/// bundler fails outright on a resource that is not there — while the
/// licences stay, ready for the dylib that replaces it.
#[test]
fn the_mac_app_drops_the_windows_dll_and_keeps_the_licences() {
    let mut merged = config("tauri.conf.json");
    json_patch::merge(&mut merged, &config("tauri.macos.conf.json"));

    let expected: Vec<&str> = BASE
        .iter()
        .filter(|n| **n != "libraw.dll")
        .copied()
        .collect();
    assert_eq!(installed(&merged), sorted(&expected));
    // LibRaw goes in Contents/Frameworks instead, where its @rpath points.
    assert_eq!(
        merged.pointer("/bundle/macOS/frameworks"),
        Some(&serde_json::json!([LIBRAW_DYLIB]))
    );
}

/// The macOS LibRaw, as a `bundle.macOS.frameworks` entry.
const LIBRAW_DYLIB: &str = "vendor/libraw/lib/aarch64-apple-darwin/libraw.dylib";

/// The macOS AI build: base, then the Mac overlay, then the AI overlay, then
/// the Mac AI overlay — the order `tauri:build:ai:mac` passes them in. ONNX
/// Runtime is a framework there, not a resource: `ensure_ort_dylib` looks for
/// it in `Contents/Frameworks`, and DirectML does not exist on a Mac. A merge
/// patch replaces an array outright, so the overlay must list LibRaw again —
/// or the build that ships would lose Develop on a Mac, exactly the
/// failure the Windows test above guards against.
#[test]
fn the_mac_ai_app_ships_onnx_runtime_as_a_framework_and_no_windows_dlls() {
    let mut merged = config("tauri.conf.json");
    for overlay in [
        "tauri.macos.conf.json",
        "tauri.ai.conf.json",
        "tauri.ai.macos.conf.json",
    ] {
        json_patch::merge(&mut merged, &config(overlay));
    }

    let expected: Vec<&str> = BASE
        .iter()
        .chain(AI.iter())
        .filter(|n| !n.ends_with(".dll") && !n.contains("DirectML"))
        .copied()
        .collect();
    assert_eq!(installed(&merged), sorted(&expected));
    assert_eq!(
        merged.pointer("/bundle/macOS/frameworks"),
        Some(&serde_json::json!([
            LIBRAW_DYLIB,
            "vendor/onnxruntime/libonnxruntime.dylib"
        ]))
    );
    // Ad-hoc, over the whole bundle. Without it only the binary carries the
    // linker's signature, and a downloaded copy is refused as "damaged",
    // which has no Open Anyway.
    assert_eq!(
        merged.pointer("/bundle/macOS/signingIdentity"),
        Some(&serde_json::json!("-"))
    );
    // And without the hardened runtime, which Tauri turns on by default: its
    // library validation refuses ad-hoc-signed dylibs, so the app died at
    // launch on `libraw.dylib`. It only earns its keep with notarisation,
    // which needs a Developer ID.
    assert_eq!(
        merged.pointer("/bundle/macOS/hardenedRuntime"),
        Some(&serde_json::json!(false))
    );
}

/// `app.windows` is an array, and a merge patch replaces an array outright, so
/// the Mac's window is a whole copy of the base one plus its title-bar keys. A
/// size, minimum or background changed in `tauri.conf.json` alone would
/// silently not reach the Mac; this keeps the copies in step.
#[test]
fn the_mac_window_is_the_base_window_plus_its_title_bar() {
    let base = config("tauri.conf.json");
    let mac = config("tauri.macos.conf.json");
    let mut window = mac
        .pointer("/app/windows/0")
        .and_then(Value::as_object)
        .expect("tauri.macos.conf.json has the main window")
        .clone();
    for key in ["titleBarStyle", "hiddenTitle", "trafficLightPosition"] {
        assert!(window.remove(key).is_some(), "the Mac window sets {key}");
    }
    assert_eq!(
        Value::Object(window),
        base.pointer("/app/windows/0").cloned().unwrap(),
        "the Mac window must match tauri.conf.json's apart from its title bar"
    );
}

/// A release build ignores `SOURCELIGHT_DATA_DIR`, so the only thing that
/// keeps a test bundle away from someone's real catalogue is the identifier:
/// the catalogue, the models, the webview's storage, the window state and the
/// single-instance lock are all found through it. The test overlay must change
/// it, keep everything the build ships, and never update itself into the real
/// app.
#[test]
fn the_test_build_has_its_own_data_and_ships_what_the_real_one_does() {
    let mut real = config("tauri.conf.json");
    for overlay in [
        "tauri.macos.conf.json",
        "tauri.ai.conf.json",
        "tauri.ai.macos.conf.json",
    ] {
        json_patch::merge(&mut real, &config(overlay));
    }
    let mut test = real.clone();
    json_patch::merge(&mut test, &config("tauri.test.conf.json"));

    let (real_id, test_id) = (real["identifier"].as_str(), test["identifier"].as_str());
    assert!(real_id.is_some() && test_id.is_some());
    assert_ne!(
        real_id, test_id,
        "a test build must not open the real catalogue"
    );
    assert_ne!(
        real["productName"], test["productName"],
        "and must say it is a test"
    );
    assert_eq!(installed(&test), installed(&real));
    assert_eq!(
        test.pointer("/bundle/macOS/frameworks"),
        real.pointer("/bundle/macOS/frameworks")
    );
    assert!(test.pointer("/plugins/updater").is_none());
}
