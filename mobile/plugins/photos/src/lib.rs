//! The phone's own photos, for On this phone (docs/MOBILE.md section 4):
//! whether the app may see them, the albums, the photos, and JPEGs of them at
//! a size.
//!
//! Android: MediaStore and the platform's decoders, so HEIC and every format
//! the phone reads work without anything third-party. iOS: PhotoKit and
//! ImageIO, likewise (`ios/`); PhotoKit's string identifiers are numbered
//! there, as the app keys photos by number. On a desktop, where the phone app
//! runs during development, every call says it needs a phone.
//!
//! The app's Rust core calls this directly; nothing is exposed to the webview.

use std::path::Path;

use serde::{Deserialize, Serialize};
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

/// Whether the app may see the phone's photos: all of them, only those the
/// person chose (Android 14), or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    All,
    Some,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Album {
    pub id: String,
    pub name: String,
    pub count: u32,
}

/// A photo fetched for editing: see [`Photos::original`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Original {
    /// Its file name in the gallery.
    pub name: String,
    pub taken_ms: i64,
    /// Its own bytes, as opposed to a decoded copy.
    pub copied: bool,
}

/// A photo's file copied as stored: see [`Photos::file`].
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredFile {
    pub name: String,
    pub taken_ms: i64,
    pub mime: String,
}

/// One photo: `[id, takenMs, modified, width, height, raw]`. `modified`
/// changes when the photo does; width and height are upright; `raw` is 1 for
/// a DNG.
pub type Photo = (i64, i64, i64, u32, u32, u8);

#[cfg_attr(not(any(target_os = "android", target_os = "ios")), allow(dead_code))]
#[derive(Deserialize)]
struct AccessReply {
    access: Access,
}

#[cfg_attr(not(any(target_os = "android", target_os = "ios")), allow(dead_code))]
#[derive(Deserialize)]
struct AlbumsReply {
    albums: Vec<Album>,
}

#[cfg_attr(not(any(target_os = "android", target_os = "ios")), allow(dead_code))]
#[derive(Deserialize)]
struct PhotosReply {
    photos: Vec<Photo>,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
const NEEDS_A_PHONE: &str = "This needs a phone.";

pub struct Photos<R: Runtime> {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    handle: tauri::plugin::PluginHandle<R>,
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    _runtime: std::marker::PhantomData<fn() -> R>,
}

impl<R: Runtime> Photos<R> {
    pub async fn access(&self) -> Result<Access, String> {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            let r: AccessReply = self
                .handle
                .run_mobile_plugin_async("access", serde_json::json!({}))
                .await
                .map_err(|e| e.to_string())?;
            Ok(r.access)
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        Err(NEEDS_A_PHONE.into())
    }

    /// Asks the person, or with chosen photos only, lets them choose again.
    pub async fn request_access(&self) -> Result<Access, String> {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            let r: AccessReply = self
                .handle
                .run_mobile_plugin_async("requestAccess", serde_json::json!({}))
                .await
                .map_err(|e| e.to_string())?;
            Ok(r.access)
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        Err(NEEDS_A_PHONE.into())
    }

    pub async fn albums(&self) -> Result<Vec<Album>, String> {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            let r: AlbumsReply = self
                .handle
                .run_mobile_plugin_async("albums", serde_json::json!({}))
                .await
                .map_err(|e| e.to_string())?;
            Ok(r.albums)
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        Err(NEEDS_A_PHONE.into())
    }

    /// The photos, newest first: all of them, or one album's.
    pub async fn photos(&self, album: Option<&str>) -> Result<Vec<Photo>, String> {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            let r: PhotosReply = self
                .handle
                .run_mobile_plugin_async("photos", serde_json::json!({ "album": album }))
                .await
                .map_err(|e| e.to_string())?;
            Ok(r.photos)
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            let _ = album;
            Err(NEEDS_A_PHONE.into())
        }
    }

    /// Writes photo `id` to `to` for editing: its own bytes when it is a
    /// JPEG, PNG or WebP no larger than `max_edge`, otherwise decoded by the
    /// phone, upright, down to `max_edge`, as a high-quality JPEG.
    pub async fn original(&self, id: i64, max_edge: u32, to: &Path) -> Result<Original, String> {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            self.handle
                .run_mobile_plugin_async(
                    "original",
                    serde_json::json!({ "id": id, "maxEdge": max_edge, "path": to.to_string_lossy() }),
                )
                .await
                .map_err(|e| e.to_string())
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            let _ = (id, max_edge, to);
            Err(NEEDS_A_PHONE.into())
        }
    }

    /// Copies photo `id`'s file to `to` exactly as stored, location included
    /// when Android allows it: what Add to library sends.
    pub async fn file(&self, id: i64, to: &Path) -> Result<StoredFile, String> {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            self.handle
                .run_mobile_plugin_async(
                    "file",
                    serde_json::json!({ "id": id, "path": to.to_string_lossy() }),
                )
                .await
                .map_err(|e| e.to_string())
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            let _ = (id, to);
            Err(NEEDS_A_PHONE.into())
        }
    }

    /// Copies photo `id`'s camera details, capture time and location into
    /// the JPEG at `to`, marked upright. `taken_ms` stands in for a capture
    /// time the original's file doesn't give.
    pub async fn copy_metadata(&self, id: i64, to: &Path, taken_ms: i64) -> Result<(), String> {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            let _: serde_json::Value = self
                .handle
                .run_mobile_plugin_async(
                    "copyMetadata",
                    serde_json::json!({ "id": id, "path": to.to_string_lossy(), "takenMs": taken_ms }),
                )
                .await
                .map_err(|e| e.to_string())?;
            Ok(())
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            let _ = (id, to, taken_ms);
            Err(NEEDS_A_PHONE.into())
        }
    }

    /// Writes a JPEG of photo `id`, upright and no larger than `size` on its
    /// longer side, to `to`.
    pub async fn image(&self, id: i64, size: u32, to: &Path) -> Result<(), String> {
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            let _: serde_json::Value = self
                .handle
                .run_mobile_plugin_async(
                    "image",
                    serde_json::json!({ "id": id, "size": size, "path": to.to_string_lossy() }),
                )
                .await
                .map_err(|e| e.to_string())?;
            Ok(())
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            let _ = (id, size, to);
            Err(NEEDS_A_PHONE.into())
        }
    }
}

/// `app.photos()`.
pub trait PhotosExt<R: Runtime> {
    fn photos(&self) -> &Photos<R>;
}

impl<R: Runtime, T: Manager<R>> PhotosExt<R> for T {
    fn photos(&self) -> &Photos<R> {
        self.state::<Photos<R>>().inner()
    }
}

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_photos);

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("photos")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                let handle =
                    api.register_android_plugin("app.sourcelight.photos", "PhotosPlugin")?;
                app.manage(Photos { handle });
            }
            #[cfg(target_os = "ios")]
            {
                let handle = api.register_ios_plugin(init_plugin_photos)?;
                app.manage(Photos { handle });
            }
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                let _ = api;
                app.manage(Photos::<R> {
                    _runtime: std::marker::PhantomData,
                });
            }
            Ok(())
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Kotlin and Swift sides' replies, as they write them.
    #[test]
    fn replies_read_as_written() {
        let a: AccessReply = serde_json::from_str(r#"{"access":"some"}"#).unwrap();
        assert_eq!(a.access, Access::Some);
        let p: PhotosReply =
            serde_json::from_str(r#"{"photos":[[41,1791019815000,1791019820,3000,4000,0]]}"#)
                .unwrap();
        assert_eq!(
            p.photos,
            vec![(41, 1_791_019_815_000, 1_791_019_820, 3000, 4000, 0)]
        );
        let al: AlbumsReply =
            serde_json::from_str(r#"{"albums":[{"id":"-1739773001","name":"Camera","count":12}]}"#)
                .unwrap();
        assert_eq!(al.albums[0].name, "Camera");
    }
}
