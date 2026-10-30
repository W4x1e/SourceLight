//! On this phone (docs/MOBILE.md section 4): the phone's own photos, kept
//! apart from the synced library. Read where they are, through
//! `mobile/plugins/photos`; the app keeps only its own thumbnails and screen
//! sized copies, in its cache folder, where the phone may clear them.

use std::path::{Path, PathBuf};

use tauri::http::Response;
use tauri::Manager;
use tauri_plugin_photos::{Access, Album, Photo, PhotosExt};

/// Longest side of a grid thumbnail: a tile on a large phone, sharp.
const THUMB: u32 = 384;
/// Longest side of the copy the loupe shows: a phone screen, sharp.
const PREVIEW: u32 = 2048;

#[derive(Clone, Copy)]
pub enum Kind {
    Thumb,
    Preview,
}

#[tauri::command]
pub async fn device_access(app: tauri::AppHandle) -> Result<Access, String> {
    app.photos().access().await
}

#[tauri::command]
pub async fn device_request_access(app: tauri::AppHandle) -> Result<Access, String> {
    app.photos().request_access().await
}

#[tauri::command]
pub async fn device_albums(app: tauri::AppHandle) -> Result<Vec<Album>, String> {
    app.photos().albums().await
}

/// The phone's photos, newest first: all, or one album's.
#[tauri::command]
pub async fn device_photos(
    app: tauri::AppHandle,
    album: Option<String>,
) -> Result<Vec<Photo>, String> {
    app.photos().photos(album.as_deref()).await
}

/// `devthumb://localhost/<id>_<modified>` and `devpreview://…`: a phone
/// photo as a JPEG, from the cache or made by the phone and cached. The
/// modified time in the name means an edited photo is made again.
pub async fn serve(app: tauri::AppHandle, kind: Kind, path: String) -> Response<Vec<u8>> {
    let image = match (key(&path), cache_dir(&app)) {
        (Some(key), Some(dir)) => cached(&app, kind, key, &dir).await,
        _ => Err("not a phone photo".into()),
    };
    match image {
        Ok(jpeg) => Response::builder()
            .header("Content-Type", "image/jpeg")
            .header("Cache-Control", "max-age=31536000, immutable")
            .body(jpeg),
        Err(e) => Response::builder().status(404).body(e.into_bytes()),
    }
    .expect("a static response builds")
}

async fn cached(
    app: &tauri::AppHandle,
    kind: Kind,
    (id, modified): (i64, i64),
    dir: &Path,
) -> Result<Vec<u8>, String> {
    let (folder, size) = match kind {
        Kind::Thumb => ("thumb", THUMB),
        Kind::Preview => ("preview", PREVIEW),
    };
    let file = dir.join(folder).join(format!("{id}_{modified}.jpg"));
    if let Ok(jpeg) = std::fs::read(&file) {
        return Ok(jpeg);
    }
    app.photos().image(id, size, &file).await?;
    std::fs::read(&file).map_err(|e| e.to_string())
}

fn cache_dir(app: &tauri::AppHandle) -> Option<PathBuf> {
    Some(app.path().app_cache_dir().ok()?.join("device"))
}

/// `/<id>_<modified>` as numbers, or nothing for anything else.
fn key(path: &str) -> Option<(i64, i64)> {
    let (id, modified) = path.trim_start_matches('/').split_once('_')?;
    Some((id.parse().ok()?, modified.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::key;

    /// Only numbers make a cache file's name: nothing a page sends can reach
    /// outside the cache folder.
    #[test]
    fn a_photo_is_named_by_numbers_only() {
        assert_eq!(key("/41_1791019820"), Some((41, 1_791_019_820)));
        assert_eq!(key("41_7"), Some((41, 7)));
        assert_eq!(key("/../../x_1"), None);
        assert_eq!(key("/41"), None);
        assert_eq!(key("/41_1/2"), None);
    }
}
