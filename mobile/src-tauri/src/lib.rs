//! SourceLight on phones and tablets (docs/MOBILE.md).
//!
//! The phone is a companion to the desktop: the desktop keeps the catalogue
//! and does the heavy work. Here: pairing and syncing over the link (`link`),
//! the phone's copy of the library (`replica`), the images it has been sent
//! (`cache`), the phone's own photos (`device`) and editing them (`edit`,
//! `edits`), and stand-in thumbnails for measuring scrolling (`thumbs`).

mod cache;
mod device;
mod edit;
mod edits;
mod gpu_test;
mod link;
mod replica;
mod thumbs;
mod track;

use tauri::http::Response;
use tauri::Manager;

use crate::cache::Kind;

/// `thumb://localhost/<id>` and `preview://localhost/<id>`: a photo's images,
/// from the cache or fetched from the desktop. A 404 when neither has it, which
/// the interface shows as an empty tile.
fn serve_image(
    app: &tauri::AppHandle,
    kind: Kind,
    path: &str,
) -> impl std::future::Future<Output = Response<Vec<u8>>> {
    let app = app.clone();
    let id: Option<i64> = path.trim_start_matches('/').parse().ok();
    async move {
        let image = match id {
            Some(id) => app.state::<link::Link>().image(kind, id).await,
            None => None,
        };
        match image {
            Some(jpeg) => Response::builder()
                .header("Content-Type", "image/jpeg")
                .header("Cache-Control", "max-age=31536000, immutable")
                .body(jpeg),
            None => Response::builder().status(404).body(Vec::new()),
        }
        .expect("a static response builds")
    }
}

/// `edited://localhost/<id>_<tag>`: a library photo's preview as edited, the
/// tag being its edits', so new edits are a new address.
fn serve_edited(
    app: &tauri::AppHandle,
    path: &str,
) -> impl std::future::Future<Output = Response<Vec<u8>>> {
    let app = app.clone();
    let asked = path
        .trim_start_matches('/')
        .split_once('_')
        .and_then(|(id, tag)| {
            let id: i64 = id.parse().ok()?;
            let plain = !tag.is_empty() && tag.chars().all(|c| c.is_ascii_hexdigit());
            plain.then(|| (id, tag.to_string()))
        });
    async move {
        let image = match asked {
            Some((id, tag)) => app.state::<link::Link>().edited(id, &tag).await,
            None => None,
        };
        match image {
            Some(jpeg) => Response::builder()
                .header("Content-Type", "image/jpeg")
                .header("Cache-Control", "max-age=31536000, immutable")
                .body(jpeg),
            None => Response::builder().status(404).body(Vec::new()),
        }
        .expect("a static response builds")
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_shoot_track::init())
        .plugin(tauri_plugin_qr_scan::init())
        .plugin(tauri_plugin_gallery::init())
        .plugin(tauri_plugin_photos::init());
    builder
        .register_asynchronous_uri_scheme_protocol("standin", |_ctx, request, responder| {
            // Off the webview's thread: the first request builds the pool.
            std::thread::spawn(move || responder.respond(thumbs::serve(&request)));
        })
        .register_asynchronous_uri_scheme_protocol("thumb", |ctx, request, responder| {
            let image = serve_image(ctx.app_handle(), Kind::Thumb, request.uri().path());
            tauri::async_runtime::spawn(async move { responder.respond(image.await) });
        })
        .register_asynchronous_uri_scheme_protocol("preview", |ctx, request, responder| {
            let image = serve_image(ctx.app_handle(), Kind::Preview, request.uri().path());
            tauri::async_runtime::spawn(async move { responder.respond(image.await) });
        })
        .register_asynchronous_uri_scheme_protocol("edited", |ctx, request, responder| {
            let image = serve_edited(ctx.app_handle(), request.uri().path());
            tauri::async_runtime::spawn(async move { responder.respond(image.await) });
        })
        .register_asynchronous_uri_scheme_protocol("devthumb", |ctx, request, responder| {
            let image = device::serve(
                ctx.app_handle().clone(),
                device::Kind::Thumb,
                request.uri().path().to_string(),
            );
            tauri::async_runtime::spawn(async move { responder.respond(image.await) });
        })
        .register_asynchronous_uri_scheme_protocol("devpreview", |ctx, request, responder| {
            let image = device::serve(
                ctx.app_handle().clone(),
                device::Kind::Preview,
                request.uri().path().to_string(),
            );
            tauri::async_runtime::spawn(async move { responder.respond(image.await) });
        })
        .register_asynchronous_uri_scheme_protocol("editframe", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let path = request.uri().path().to_string();
            let query = request.uri().query().unwrap_or_default().to_string();
            std::thread::spawn(move || responder.respond(edit::serve_frame(&app, &path, &query)));
        })
        .register_asynchronous_uri_scheme_protocol("gputest", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let path = request.uri().path().to_string();
            std::thread::spawn(move || responder.respond(gpu_test::serve(&app, &path)));
        })
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            // The link makes the data folder the editor's database lives in.
            app.manage(link::Link::load(dir.clone())?);
            app.manage(edit::Editor::load(&dir)?);
            app.manage(gpu_test::GpuTest::default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            link::link_status,
            link::link_waiting,
            link::pair,
            link::connect,
            link::ping,
            link::sync,
            link::photos,
            link::set_flag,
            link::set_rating,
            link::library_vocabulary,
            link::library_photo_name,
            link::library_search,
            link::tracks,
            link::track_start,
            link::track_stop,
            link::scan_qr,
            link::forget,
            gpu_test::gpu_test,
            device::device_access,
            device::device_request_access,
            device::device_albums,
            device::device_photos,
            edit::edit_warm,
            edit::edit_open,
            edit::edit_open_library,
            edit::edit_commit,
            edit::edit_done,
            edit::edit_undo,
            edit::edit_redo,
            edit::edit_presets,
            edit::edit_looks,
            edit::edit_preset,
            edit::edit_auto,
            edit::edit_pick_wb,
            edit::edit_paste,
            edit::edit_edited,
            edit::edit_close,
            edit::edit_save,
            edit::device_add,
            edit::device_adds,
        ])
        .run(tauri::generate_context!())
        .expect("error while running SourceLight");
}
