//! Records a GPS track during a shoot, with the screen off (docs/MOBILE.md,
//! "Shoot track").
//!
//! Android: a foreground service (`android/`) appends a fix every few seconds
//! to a file the app chose. iOS: CoreLocation, carrying on in the background
//! (`ios/`). On a desktop, where the phone app runs for development, every
//! call says it needs a phone.
//!
//! The app's Rust core calls this directly; nothing is exposed to the webview.

use serde::{Deserialize, Serialize};
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

/// Where a recording stands.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackStatus {
    pub recording: bool,
    /// The file being written, or last written.
    pub file: Option<String>,
    pub points: u32,
    /// When the newest fix was taken, UTC milliseconds; 0 for none yet.
    pub last_fix_ms: i64,
    /// How far off it may be, in metres.
    pub last_accuracy: f64,
}

/// What the phone said about a permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionState {
    Granted,
    Denied,
    Prompt,
    #[serde(rename = "prompt-with-rationale")]
    PromptWithRationale,
}

#[derive(Deserialize)]
#[cfg_attr(not(any(target_os = "android", target_os = "ios")), allow(dead_code))]
struct Permissions {
    location: PermissionState,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
const NEEDS_A_PHONE: &str = "Recording a track needs a phone.";

pub struct ShootTrack<R: Runtime> {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    handle: tauri::plugin::PluginHandle<R>,
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    _runtime: std::marker::PhantomData<fn() -> R>,
}

impl<R: Runtime> ShootTrack<R> {
    /// Asks for location (and on Android, the notification that shows a
    /// recording is on), showing the phone's prompt if it has not been
    /// answered. True when location is granted.
    pub async fn request_permissions(&self) -> Result<bool, String> {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            let p: Permissions = self
                .handle
                .run_mobile_plugin_async(
                    "requestPermissions",
                    serde_json::json!({ "permissions": ["location", "notifications"] }),
                )
                .await
                .map_err(|e| e.to_string())?;
            Ok(p.location == PermissionState::Granted)
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            Err(NEEDS_A_PHONE.into())
        }
    }

    /// Starts recording into `file`, asking for a fix every `interval_ms`.
    pub async fn start(&self, file: &str, interval_ms: u64) -> Result<TrackStatus, String> {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            self.handle
                .run_mobile_plugin_async(
                    "start",
                    serde_json::json!({ "file": file, "intervalMs": interval_ms }),
                )
                .await
                .map_err(|e| e.to_string())
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            let _ = (file, interval_ms);
            Err(NEEDS_A_PHONE.into())
        }
    }

    /// Stops recording. The status says which file it wrote.
    pub async fn stop(&self) -> Result<TrackStatus, String> {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            self.handle
                .run_mobile_plugin_async("stop", ())
                .await
                .map_err(|e| e.to_string())
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            Err(NEEDS_A_PHONE.into())
        }
    }

    pub async fn status(&self) -> Result<TrackStatus, String> {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            self.handle
                .run_mobile_plugin_async("status", ())
                .await
                .map_err(|e| e.to_string())
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            Ok(TrackStatus::default())
        }
    }
}

/// `app.shoot_track()`.
pub trait ShootTrackExt<R: Runtime> {
    fn shoot_track(&self) -> &ShootTrack<R>;
}

impl<R: Runtime, T: Manager<R>> ShootTrackExt<R> for T {
    fn shoot_track(&self) -> &ShootTrack<R> {
        self.state::<ShootTrack<R>>().inner()
    }
}

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_shoot_track);

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("shoot-track")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                let handle =
                    api.register_android_plugin("app.sourcelight.track", "ShootTrackPlugin")?;
                app.manage(ShootTrack { handle });
            }
            #[cfg(target_os = "ios")]
            {
                let handle = api.register_ios_plugin(init_plugin_shoot_track)?;
                app.manage(ShootTrack { handle });
            }
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                let _ = api;
                app.manage(ShootTrack::<R> {
                    _runtime: std::marker::PhantomData,
                });
            }
            Ok(())
        })
        .build()
}
