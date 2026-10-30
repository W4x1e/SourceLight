//! The `thumb://` and `full://` schemes the webview loads images from.

use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[cfg(feature = "libraw")]
use super::common::with_develop;
use super::common::{data_dir, err};
#[cfg(feature = "libraw")]
use crate::raw;
use crate::{db, full_preview, sync, AppState};

/// Serves cached thumbnails and previews over a `thumb://` URI scheme.
///
/// This replaces Tauri's built-in asset protocol, whose scope matching rejected
/// every request with 403 on Windows: neither the declarative `$APPLOCALDATA/**`
/// pattern nor a runtime `allow_directory()` grant (tried with both `\` and `/`
/// separators) ever made `Scope::is_allowed` return true for a real thumbnail
/// path. Owning the handler removes the glob from the picture entirely, and the
/// containment check below is a stricter guarantee than a path pattern anyway.
pub(crate) fn serve_thumb(
    app: &AppHandle,
    uri: &tauri::http::Uri,
) -> tauri::http::Response<Vec<u8>> {
    let reply = |status: u16, body: &str| {
        tauri::http::Response::builder()
            .status(status)
            .header("content-type", "text/plain")
            .body(body.as_bytes().to_vec())
            .unwrap_or_default()
    };

    let raw = uri.path().trim_start_matches('/');
    let decoded = percent_encoding::percent_decode_str(raw).decode_utf8_lossy();
    let requested = PathBuf::from(decoded.as_ref());

    // Only thumbnails, previews, face portraits and clipboard renders:
    // the whole data folder held the catalogue, its backups and the logs.
    // Checked by the path's own words before anything touches the disk — a
    // network path (`\\host\share`) would otherwise have Windows connect to
    // the host just to canonicalise it — then canonicalised, so `..` and links
    // cannot escape either folder.
    let Ok(root) = data_dir(app) else {
        return reply(500, "no app data dir");
    };
    let allowed = [root.join("thumbs"), root.join("clipboard")];
    if !on_local_drive(&requested) {
        return reply(403, "not a thumbnail");
    }
    let Ok(target) = requested.canonicalize() else {
        return reply(404, "not found");
    };
    if !allowed
        .iter()
        .filter_map(|dir| dir.canonicalize().ok())
        .any(|dir| target.starts_with(dir))
    {
        return reply(403, "not a thumbnail");
    }

    // Thumbnails and previews are JPEGs; a render for the clipboard is a PNG,
    // which the page reads as data — hence the CORS header, since
    // `thumb://` is another origin to it. Only this app's webview can ask.
    let kind = match target.extension().and_then(|e| e.to_str()) {
        Some(e) if e.eq_ignore_ascii_case("png") => "image/png",
        _ => "image/jpeg",
    };
    match std::fs::read(&target) {
        Ok(bytes) => tauri::http::Response::builder()
            .status(200)
            .header("content-type", kind)
            .header("access-control-allow-origin", "*")
            // Content-addressed filenames (the photo's hash), so this is safe.
            .header("cache-control", "public, max-age=31536000, immutable")
            .body(bytes)
            .unwrap_or_default(),
        Err(_) => reply(404, "not found"),
    }
}

/// Whether `path` is absolute on a local drive (`C:\…`, or `/…` off Windows),
/// judged by its parsed parts rather than its spelling: `\/host/share`
/// passed a test for `\\` and `//`, and Windows reads it as a network path.
fn on_local_drive(path: &std::path::Path) -> bool {
    use std::path::{Component, Prefix};
    let mut parts = path.components();
    if cfg!(windows) {
        matches!(parts.next(), Some(Component::Prefix(p))
            if matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
            && parts.next() == Some(Component::RootDir)
    } else {
        parts.next() == Some(Component::RootDir)
    }
}

/// Serves `full://localhost/<photo id>`: the raw's embedded full-resolution
/// JPEG, for the Cull and Lightbox zoom. 204 when the file has nothing
/// bigger than the cached preview, which the page reads as "keep the preview".
pub(crate) fn serve_full(
    app: &AppHandle,
    uri: &tauri::http::Uri,
) -> tauri::http::Response<Vec<u8>> {
    let reply = |status: u16, body: &str| {
        tauri::http::Response::builder()
            .status(status)
            .header("content-type", "text/plain")
            .body(body.as_bytes().to_vec())
            .unwrap_or_default()
    };
    let Ok(id) = uri.path().trim_start_matches('/').parse::<i32>() else {
        return reply(400, "expected a photo id");
    };
    // The photo asked for last. A prefetch for any other, still
    // waiting for its turn, is no longer wanted: someone stepping through a
    // burst queued a sensor render for every frame they paused on, and the
    // zoom they wanted waited behind renders of frames already left.
    static NEWEST: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
    NEWEST.store(id, std::sync::atomic::Ordering::Relaxed);
    let prefetch = uri
        .query()
        .is_some_and(|q| q.split('&').any(|p| p == "prefetch=1"));
    let state = app.state::<AppState>();
    // Cached by content as well as id: see `full_preview::Cache`.
    let key = match state
        .conn()
        .and_then(|c| db::photo_hash(&c, id).map_err(err))
    {
        Ok(Some(hash)) => (id, hash),
        Ok(None) => return reply(404, "no such photo"),
        Err(e) => return reply(500, &e),
    };

    // One load at a time, re-checking the cache once it is this one's turn:
    // the page warms the cache ahead of a zoom, and a zoom that lands while
    // that render is still running must wait for it rather than start a
    // second, identical sensor decode behind it.
    static LOADING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let mut _turn = None;
    let mut cached = sync::lock_recover(&state.full_previews).get(&key);
    if cached.is_none() {
        _turn = Some(sync::lock_recover(&LOADING));
        cached = sync::lock_recover(&state.full_previews).get(&key);
        if cached.is_none() && prefetch && NEWEST.load(std::sync::atomic::Ordering::Relaxed) != id {
            return reply(204, "moved on");
        }
    }
    let jpeg = match cached {
        Some(hit) => hit,
        None => {
            // Only a photo in the catalogue can be served — the id is looked
            // up, never a path from the page.
            let photo = match state
                .conn()
                .and_then(|c| db::get_photo(&c, id).map_err(err))
            {
                Ok(Some(p)) => p,
                Ok(None) => return reply(404, "no such photo"),
                Err(e) => return reply(500, &e),
            };
            let path = std::path::Path::new(&photo.path);
            let embedded = full_preview::load(path, photo.orientation.max(0) as u32);
            // A small embedded preview (Sony, Panasonic) means rendering the
            // sensor data instead. A failed embedded read goes the same way:
            // LibRaw may still decode a file whose preview it cannot find.
            #[cfg(feature = "libraw")]
            let embedded = match embedded {
                Ok(Some(bytes)) => Ok(Some(bytes)),
                // A raster is already its full resolution: rendering
                // a small one (a phone or web save) could only give back its
                // own pixels, under the develop lock, on every zoom.
                _ if crate::raster::is_raster(path) => Ok(None),
                _ => with_develop(&state, |dev| {
                    let (gpu, pipeline) = dev.gpu_and_pipeline();
                    let decoder = raw::libraw::LibRawDecoder::new();
                    full_preview::render(gpu, pipeline, &decoder, path)
                        .map(Some)
                        .map_err(err)
                })
                .map_err(|e| anyhow::anyhow!(e)),
            };
            let loaded = match embedded {
                Ok(v) => std::sync::Arc::new(v),
                Err(e) => return reply(404, &e.to_string()),
            };
            sync::lock_recover(&state.full_previews).put(key, std::sync::Arc::clone(&loaded));
            loaded
        }
    };

    match jpeg.as_ref() {
        Some(bytes) => tauri::http::Response::builder()
            .status(200)
            .header("content-type", "image/jpeg")
            // Keyed by photo id, not content, and the file on disk can be
            // replaced, so the webview must not keep it past this session.
            .header("cache-control", "no-store")
            // Focus peaking reads these pixels with WebGL, which only
            // a CORS response allows.
            .header("access-control-allow-origin", "*")
            .body(bytes.clone())
            .unwrap_or_default(),
        None => tauri::http::Response::builder()
            .status(204)
            .body(Vec::new())
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::on_local_drive;
    use std::path::Path;

    /// A network path in any spelling is refused before anything touches the
    /// disk: looking one up has Windows connect to the host.
    #[test]
    fn only_local_absolute_paths_are_looked_up() {
        #[cfg(windows)]
        {
            for network in [
                r"\/host/share/x.jpg",
                r"/\host\share\x.jpg",
                r"\\host\share\x.jpg",
                "//host/share/x.jpg",
                r"\\?\UNC\host\share\x.jpg",
                r"\\.\pipe\x",
                r"C:x.jpg",
                r"\x.jpg",
            ] {
                assert!(!on_local_drive(Path::new(network)), "{network}");
            }
            assert!(on_local_drive(Path::new(r"C:\Users\a\thumbs\x.jpg")));
            assert!(on_local_drive(Path::new("C:/Users/a/thumbs/x.jpg")));
            assert!(on_local_drive(Path::new(r"\\?\C:\Users\a\thumbs\x.jpg")));
        }
        #[cfg(not(windows))]
        assert!(on_local_drive(Path::new("/Users/a/thumbs/x.jpg")));
        assert!(!on_local_drive(Path::new(r"\/host/share/x.jpg")));
        assert!(!on_local_drive(Path::new("thumbs/x.jpg")));
    }
}
