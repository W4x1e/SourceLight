//! Saves photos sent from the desktop into the phone's gallery (docs/MOBILE.md,
//! "Send to phone").
//!
//! Android: MediaStore, into Pictures/SourceLight, where every gallery app
//! finds them. Android 10 and later need no permission for that; older
//! versions ask for storage access the first time. iOS: the Photos library,
//! with add-only access, asked the first time (`ios/`). On a desktop, where
//! the phone app runs during development, files go to the Pictures folder's
//! SourceLight folder.
//!
//! The app's Rust core calls this directly; nothing is exposed to the webview.

use std::path::Path;

use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

#[cfg_attr(not(any(target_os = "android", target_os = "ios")), allow(dead_code))]
#[derive(serde::Deserialize)]
struct Saved {
    name: String,
}

pub struct Gallery<R: Runtime> {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    handle: tauri::plugin::PluginHandle<R>,
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    app: tauri::AppHandle<R>,
}

/// The MIME type the gallery files a photo under, from its extension.
pub fn mime_type(name: &str) -> Option<&'static str> {
    let ext = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "tif" | "tiff" => "image/tiff",
        _ => return None,
    })
}

impl<R: Runtime> Gallery<R> {
    /// Copies the file at `path` into the gallery as `name`, and returns the
    /// name it got (`name (1)` when one was taken). The gallery dates it by
    /// its EXIF. The file itself is left for the caller to delete.
    pub async fn save(&self, path: &Path, name: &str) -> Result<String, String> {
        let mime = mime_type(name).ok_or_else(|| format!("{name} is not a photo"))?;
        #[cfg(any(target_os = "android", target_os = "ios"))]
        {
            let saved: Saved = self
                .handle
                .run_mobile_plugin_async(
                    "save",
                    serde_json::json!({
                        "path": path.to_string_lossy(),
                        "name": name,
                        "mime": mime,
                    }),
                )
                .await
                .map_err(|e| e.to_string())?;
            Ok(saved.name)
        }
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            let _ = mime;
            let dir = self
                .app
                .path()
                .picture_dir()
                .map_err(|e| e.to_string())?
                .join("SourceLight");
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let to = unused_name(&dir, name);
            std::fs::copy(path, &to).map_err(|e| e.to_string())?;
            Ok(to
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| name.to_string()))
        }
    }
}

/// `name` in `dir`, or `name (2)` and so on if that is taken: the gallery
/// keeps both rather than replacing one.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn unused_name(dir: &Path, name: &str) -> std::path::PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let p = Path::new(name);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let ext = p.extension().and_then(|e| e.to_str());
    (2..)
        .map(|n| match ext {
            Some(ext) => dir.join(format!("{stem} ({n}).{ext}")),
            None => dir.join(format!("{stem} ({n})")),
        })
        .find(|c| !c.exists())
        .expect("some number is free")
}

/// `app.gallery()`.
pub trait GalleryExt<R: Runtime> {
    fn gallery(&self) -> &Gallery<R>;
}

impl<R: Runtime, T: Manager<R>> GalleryExt<R> for T {
    fn gallery(&self) -> &Gallery<R> {
        self.state::<Gallery<R>>().inner()
    }
}

#[cfg(target_os = "ios")]
tauri::ios_plugin_binding!(init_plugin_gallery);

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("gallery")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            {
                let handle =
                    api.register_android_plugin("app.sourcelight.gallery", "GalleryPlugin")?;
                app.manage(Gallery { handle });
            }
            #[cfg(target_os = "ios")]
            {
                let handle = api.register_ios_plugin(init_plugin_gallery)?;
                app.manage(Gallery { handle });
            }
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                let _ = api;
                app.manage(Gallery { app: app.clone() });
            }
            Ok(())
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photos_get_their_mime_type() {
        assert_eq!(mime_type("IMG_0001.JPG"), Some("image/jpeg"));
        assert_eq!(mime_type("a.jpeg"), Some("image/jpeg"));
        assert_eq!(mime_type("scan.tiff"), Some("image/tiff"));
        assert_eq!(mime_type("x.webp"), Some("image/webp"));
        assert_eq!(mime_type("notes.txt"), None);
        assert_eq!(mime_type("no-extension"), None);
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    #[test]
    fn a_taken_name_gets_a_number() {
        let dir = std::env::temp_dir().join(format!("sl-gallery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unused_name(&dir, "a.jpg"), dir.join("a.jpg"));
        std::fs::write(dir.join("a.jpg"), b"1").unwrap();
        assert_eq!(unused_name(&dir, "a.jpg"), dir.join("a (2).jpg"));
        std::fs::write(dir.join("a (2).jpg"), b"2").unwrap();
        assert_eq!(unused_name(&dir, "a.jpg"), dir.join("a (3).jpg"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
