//! Scans the desktop's pairing QR code with the camera (docs/MOBILE.md,
//! "Pairing").
//!
//! Android: ZXing's scanner screen (zxing-android-embedded, Apache-2.0). This
//! replaces the official barcode-scanner plugin, whose Android side is
//! Google's closed-source ML Kit. iOS will use AVFoundation's own QR reader,
//! which needs nothing third-party. On a desktop every call says it needs a
//! phone; pairing there is by pasting the link.
//!
//! The app's Rust core calls this directly; nothing is exposed to the webview.

use serde::Deserialize;
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[derive(Deserialize)]
struct Scanned {
    #[serde(default)]
    content: Option<String>,
}

#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[derive(Deserialize)]
struct Permissions {
    camera: String,
}

pub struct QrScan<R: Runtime> {
    #[cfg(target_os = "android")]
    handle: tauri::plugin::PluginHandle<R>,
    #[cfg(not(target_os = "android"))]
    _runtime: std::marker::PhantomData<fn() -> R>,
}

impl<R: Runtime> QrScan<R> {
    /// Opens the camera on a QR code. `None` when the person backs out.
    pub async fn scan(&self, prompt: &str) -> Result<Option<String>, String> {
        #[cfg(target_os = "android")]
        {
            let p: Permissions = self
                .handle
                .run_mobile_plugin_async(
                    "requestPermissions",
                    serde_json::json!({ "permissions": ["camera"] }),
                )
                .await
                .map_err(|e| e.to_string())?;
            if p.camera != "granted" {
                return Err("Camera permission was not given. Paste the link instead.".into());
            }
            let scanned: Scanned = self
                .handle
                .run_mobile_plugin_async("scan", serde_json::json!({ "prompt": prompt }))
                .await
                .map_err(|e| e.to_string())?;
            Ok(scanned.content.filter(|c| !c.is_empty()))
        }
        #[cfg(not(target_os = "android"))]
        {
            let _ = prompt;
            Err("Scanning needs a phone's camera. Paste the link instead.".into())
        }
    }
}

/// `app.qr_scan()`.
pub trait QrScanExt<R: Runtime> {
    fn qr_scan(&self) -> &QrScan<R>;
}

impl<R: Runtime, T: Manager<R>> QrScanExt<R> for T {
    fn qr_scan(&self) -> &QrScan<R> {
        self.state::<QrScan<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("qr-scan")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                let handle =
                    api.register_android_plugin("app.sourcelight.qrscan", "QrScanPlugin")?;
                app.manage(QrScan { handle });
            }
            #[cfg(not(target_os = "android"))]
            {
                let _ = api;
                app.manage(QrScan::<R> {
                    _runtime: std::marker::PhantomData,
                });
            }
            Ok(())
        })
        .build()
}
