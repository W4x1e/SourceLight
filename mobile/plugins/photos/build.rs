// No commands are exposed to the webview: the app's Rust core calls the
// Android and iOS sides directly (`run_mobile_plugin_async`), which needs no
// ACL.
const COMMANDS: &[&str] = &[];

fn main() {
    tauri_plugin::Builder::new(COMMANDS)
        .android_path("android")
        .ios_path("ios")
        .build();
}
