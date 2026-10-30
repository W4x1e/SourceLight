#![warn(clippy::undocumented_unsafe_blocks)]

// The develop core, shared with the phone app (src-tauri/crates/sl-develop).
// Under their old names, so the rest of this crate reads as it did.
pub use sl_develop::{exif_read, gpu, raster, test_support};

pub mod ai_models;
pub mod ai_setup;
pub mod backup;
mod commands;
pub mod crash;
pub mod db;
pub mod dedup;
pub mod denoise;
pub mod develop;
pub mod errors;
pub mod export;
pub mod faces;
pub mod folder_keywords;
pub mod fsutil;
pub mod fts;
pub mod full_preview;
pub mod geo;
pub mod gpu_gate;
pub mod gpx;
pub mod hardware;
pub mod hashing;
pub mod identity;
pub mod ingest;
pub mod jobs;
pub mod llama_pack;
pub mod mcp;
pub mod models;
pub mod palette;
pub mod people;
pub mod phone_link;
pub mod present;
pub mod quality;
pub mod raw;
pub mod scan;
pub mod search;
pub mod seed;
pub mod session_log;
pub mod shell;
pub mod sidecar;
pub mod sync;
pub mod tagger;
#[doc(hidden)]
pub mod trips;
pub mod updates;
pub mod vector;
pub mod vlm;
pub mod window_chrome;
pub mod xmp;

pub use commands::common::DATA_DIR_ENV;

use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tauri::{Emitter, Manager};

use commands::ai::{remove_ollama_leftovers, stop_vlm_server};
use commands::app::hold_quit_for_the_ui;
#[cfg(target_os = "macos")]
use commands::app::OPENED_FOLDERS;
use commands::common::{
    bundled_model_sources, data_dir, err, sweep_unused_fills, window_state_file,
};
use commands::protocols::{serve_full, serve_thumb};
#[cfg(feature = "libraw")]
use commands::render::serve_develop;

/// People suggestions and the [`db::people_fingerprint`] they were worked
/// out for.
type CachedSuggestions = (String, Vec<(i64, i64, f32)>);

/// The window's minimum size on a screen whose work area is `width` x
/// `height` logical pixels: the configured 940 x 620, shrunk to fit
/// a screen smaller than that, with room left for its own frame.
fn min_window_size(width: f64, height: f64) -> (f64, f64) {
    const MIN: (f64, f64) = (940.0, 620.0);
    const FRAME: f64 = 40.0;
    (
        MIN.0.min((width - FRAME).max(480.0)),
        MIN.1.min((height - FRAME).max(360.0)),
    )
}

/// SQLite is in WAL mode, so each command opening its own short-lived connection
/// is cheaper than serialising every query behind one shared mutex — a long scan
/// would otherwise block the UI's reads.
pub struct AppState {
    db_path: PathBuf,
    cache_dir: PathBuf,
    /// Set when startup found the catalogue damaged and restored a backup;
    /// taken once by the interface, which tells the user.
    recovery: std::sync::Mutex<Option<backup::Recovery>>,
    cancel_tagging: AtomicBool,
    cancel_captioning: AtomicBool,
    /// Set to stop the running export batch between files.
    cancel_export: AtomicBool,
    /// Set while a batch export runs: one at a time.
    exporting: AtomicBool,
    /// The `llama-server` the app runs the vision model on, and which model it
    /// has loaded (see `sidecar`). `None` when none is loaded.
    vlm_server: std::sync::Mutex<Option<(String, std::process::Child)>>,
    /// GPU context and the photo currently being developed. Built on first use
    /// — see `with_develop`.
    develop: std::sync::Mutex<Option<develop::DevelopState>>,
    /// The last few full-resolution embedded JPEGs served to a zoomed loupe —
    /// see [`full_preview`].
    full_previews: std::sync::Mutex<full_preview::Cache>,
    /// Model ids with a download in flight, so a second request for the same
    /// model (a double-click) is refused rather than letting two writers race on
    /// one `.part` file.
    active_downloads: std::sync::Mutex<std::collections::HashSet<String>>,
    /// The SigLIP 2 embedder for semantic search, loaded once and reused — the model
    /// is ~600 MB, so reloading it per search would make search unusable.
    search_embedder: std::sync::Mutex<Option<search::Embedder>>,
    face_engine: std::sync::Mutex<Option<faces::FaceEngine>>,
    /// NAFNet for AI noise reduction, loaded on first use.
    #[cfg(feature = "ai")]
    denoiser: std::sync::Mutex<Option<std::sync::Arc<denoise::DenoiseEngine>>>,
    /// LaMa for magic erase, loaded on first use.
    #[cfg(feature = "ai")]
    eraser: std::sync::Mutex<Option<std::sync::Arc<develop::erase::Inpainter>>>,
    /// Held while click to select encodes a photo, so a click that arrives
    /// while magic erase is readying one waits for it rather than encoding too.
    #[cfg(feature = "ai")]
    sam2_encoding: std::sync::Mutex<()>,
    /// Held for the whole of a face pass or a regroup, so they queue rather
    /// than overlap. People, the Settings row and automatic analysis can each
    /// start one. Overlapping, a second pass re-did the first one's queue, and
    /// a regroup's long write held the catalogue past the busy timeout so face
    /// rows failed to store ("database is locked").
    people_work: std::sync::Mutex<()>,
    /// Held while people are regrouped or edited — merged, "Not them", named
    /// into someone, taken out of photos — so an edit never lands in the
    /// middle of a regroup and is overwritten by it. Separate from
    /// `people_work`, which a face pass holds for minutes.
    people_edit: std::sync::Mutex<()>,
    /// The last merge, for its Undo. One step: a merge's toast offers
    /// it, and any other merge replaces it.
    last_merge: std::sync::Mutex<Option<db::MergeUndo>>,
    /// The last people suggestions, by [`db::people_fingerprint`]:
    /// every People load asked for them again, and nothing had changed.
    people_suggestions: std::sync::Mutex<Option<CachedSuggestions>>,
    /// Set while face portraits are being cut in the background.
    portraits_filling: AtomicBool,
    /// Bumped whenever a VLM batch starts or finishes. A deferred unload
    /// captures the value it saw and only frees the model if nothing has
    /// touched it since — see `unload_when_idle`.
    vlm_seq: AtomicU64,
    /// Vision-model batches running now. A deferred unload never fires while
    /// one is, however long it runs.
    vlm_busy: std::sync::atomic::AtomicUsize,
    /// Bumped by anything that changes what a natural-language search resolves
    /// to — new photos, new embeddings, new captions. `search_cache` carries the
    /// value it was computed at, so a stale entry is recomputed rather than
    /// served. See [`AppState::bump_search_gen`].
    search_gen: AtomicU64,
    /// Memoises the last resolved natural-language search so the two commands a
    /// single `reload` fires — `get_photos` and `get_facets`, dispatched together
    /// (`store/library.ts`) — don't each run the SigLIP 2 text encoder and the whole fusion
    /// over again for the identical query. Only the most recent query is kept:
    /// that is all the duplicate needs, and both callers ask for the same one.
    /// Locked for the whole resolve, so the second caller waits for the first.
    search_cache: std::sync::Mutex<Option<SearchCache>>,
    /// Long jobs that can be cancelled, by the id the interface tracks their
    /// progress under. See `jobs.rs`.
    jobs: jobs::Jobs,
    /// The phone link: its listener and any pairing in progress.
    phone: phone_link::State,
    /// This session's rating, flag and colour-label changes, newest last, each
    /// holding what it overwrote — so a mistaken keypress, or a verdict applied
    /// to the wrong selection, can be undone. Bounded by
    /// [`MAX_VERDICT_UNDO`] and not persisted: a verdict is cheap to redo by hand
    /// tomorrow; what hurts is losing a hundred of them to one stray key now.
    ///
    /// Settings copied onto other photos are steps here too, and so is a
    /// marker for each change Undo can't take back.
    verdict_undo: std::sync::Mutex<Vec<UndoStep>>,
    /// Develop opens asked for, in order, so one overtaken by a later
    /// request can stop before the develop lock.
    #[cfg(feature = "libraw")]
    develop_opens: std::sync::atomic::AtomicU64,
    /// Whether Develop is open, for automatic analysis to wait while it is
    /// ([`wait_while_developing`]).
    develop_open: std::sync::atomic::AtomicBool,
    /// Held while the vision model's runtime is installed or its model
    /// downloaded, so a second setup (automatic analysis, with a yes on file,
    /// while the one the user asked for is still downloading) waits and then
    /// finds them there, instead of writing the same staging folder and
    /// `.part` files at once.
    vision_setup: tokio::sync::Mutex<()>,
}

/// How many verdict changes can be undone in one session.
const MAX_VERDICT_UNDO: usize = 200;

/// One step Ctrl/⌘Z takes back, newest last.
enum UndoStep {
    /// Ratings, flags and colour labels.
    Verdicts(VerdictChange),
    /// Develop settings copied onto other photos: each photo's own, before.
    Settings {
        label: String,
        before: Vec<(i32, Option<develop::EditStack>)>,
    },
    /// A change Undo can't take back — keywords, a place, a collection — kept
    /// so Undo stops at it and says so, rather than reaching past it to undo
    /// an older verdict nobody asked about.
    Beyond { label: String },
}

/// One verdict change, as it can be taken back.
struct VerdictChange {
    /// What the change was, for the undo message — "Rating on 12 photos".
    label: String,
    /// Each photo's verdicts *before* the change.
    before: Vec<models::Verdict>,
}

/// A resolved natural-language search, valid only while `generation` still equals
/// [`AppState::search_gen`].
struct SearchCache {
    generation: u64,
    /// The query, and the facets it was ranked within.
    query: String,
    ids: Vec<i32>,
}

impl AppState {
    fn conn(&self) -> Result<Connection, String> {
        db::connect(&self.db_path).map_err(err)
    }

    /// Invalidates the resolved-search cache. Call after any write that changes
    /// the photo set, embeddings, or captions — the inputs `fused_search_ids`
    /// reads. Over-invalidating is safe (a needless recompute); under-invalidating
    /// serves stale results, so err towards calling it.
    fn bump_search_gen(&self) {
        self.search_gen.fetch_add(1, Ordering::Relaxed);
    }
}

/// The command surface, defined once and shared by the running app and the
/// binding generator (`bin/export_bindings.rs`) — so the generated TypeScript can
/// never describe a different set of commands than the app actually serves.
pub fn specta_builder() -> tauri_specta::Builder<tauri::Wry> {
    tauri_specta::Builder::<tauri::Wry>::new().commands(tauri_specta::collect_commands![
        commands::export::export_space_check,
        commands::backups::backup_catalogue_if_due,
        commands::app::on_battery,
        commands::app::take_opened_folders,
        commands::app::check_for_update,
        commands::app::install_update,
        commands::phone::phone_outbox_start,
        commands::phone::phone_outbox_queue,
        commands::phone::phone_import_folder,
        commands::phone::set_phone_import_folder,
        commands::phone::phone_link_status,
        commands::phone::set_phone_link,
        commands::app::edit_server_status,
        commands::app::set_edit_server,
        commands::phone::start_phone_pairing,
        commands::phone::cancel_phone_pairing,
        commands::phone::answer_phone_pairing,
        commands::phone::remove_phone,
        commands::app::quit_app,
        commands::app::stay_open,
        commands::export::drag_photos_out,
        commands::app::licence_text,
        commands::app::third_party_licences,
        commands::cull::undo_verdict,
        commands::app::log_ui_error,
        commands::app::diagnostics_summary,
        commands::app::open_log_folder,
        commands::app::unclean_exit,
        commands::app::cancel_job,
        commands::library::list_library_folders,
        commands::import::import_sources,
        commands::places::locations_status,
        commands::places::read_locations,
        commands::colours::colours_status,
        commands::colours::read_colours,
        commands::places::gpx_preview,
        commands::places::gpx_apply,
        commands::places::gpx_replace,
        commands::places::count_track_placed,
        commands::import::import_preview,
        commands::import::import_run,
        commands::import::finish_shoot_backup,
        commands::library::add_library_folder,
        commands::library::folder_is_card,
        commands::library::remove_library_folder,
        commands::library::forget_unfiled_photos,
        commands::library::relocate_library_folder,
        commands::library::scan_library,
        commands::browse::get_photos,
        commands::browse::get_photo,
        commands::browse::get_photos_by_ids,
        commands::browse::get_facets,
        commands::browse::get_extra_facets,
        commands::browse::get_genres,
        commands::metadata::add_tag,
        commands::metadata::remove_tag,
        commands::metadata::confirm_keywords,
        commands::metadata::ai_keywords,
        commands::metadata::add_folder_keywords,
        commands::metadata::remove_folder_keywords,
        commands::metadata::set_capture_time,
        commands::metadata::revert_capture_time,
        commands::colours::palette_status,
        commands::colours::read_palettes,
        commands::colours::similar_colours,
        commands::collections::list_collection_groups,
        commands::collections::create_collection_group,
        commands::collections::rename_collection_group,
        commands::collections::delete_collection_group,
        commands::collections::set_collection_group,
        commands::collections::move_in_collection,
        commands::browse::insights,
        commands::browse::lens_keywords,
        commands::browse::timeline,
        commands::metadata::photo_iptc,
        commands::metadata::iptc_selection,
        commands::metadata::set_iptc_field,
        commands::metadata::list_iptc_templates,
        commands::metadata::save_iptc_template,
        commands::metadata::delete_iptc_template,
        commands::metadata::apply_iptc_template,
        commands::metadata::use_sidecar_times,
        commands::export::render_for_clipboard,
        commands::export::render_edited_preview,
        commands::metadata::paste_location,
        commands::metadata::paste_keywords,
        commands::metadata::keyword_tree,
        commands::metadata::create_keyword,
        commands::metadata::move_keyword,
        commands::metadata::rename_keyword,
        commands::metadata::delete_keyword,
        commands::cull::set_rating,
        commands::cull::set_flag,
        commands::cull::set_flags,
        commands::cull::set_color_label,
        commands::collections::list_collections,
        commands::collections::create_collection,
        commands::collections::rename_collection,
        commands::collections::delete_collection,
        commands::collections::add_to_collection,
        commands::collections::remove_from_collection,
        commands::collections::list_smart_collections,
        commands::collections::save_smart_collection,
        commands::collections::delete_smart_collection,
        commands::sidecars::import_xmp,
        commands::sidecars::count_sidecars,
        commands::sidecars::export_xmp,
        commands::browse::search_vocabulary,
        commands::places::clear_place,
        commands::people::name_face,
        commands::develop::paste_settings_from,
        commands::develop::paste_settings,
        commands::develop::develop_go_to,
        commands::import::list_shoots,
        commands::import::delete_shoot,
        commands::places::list_places,
        commands::places::assign_place,
        commands::places::set_place_position,
        commands::places::name_trip,
        commands::places::rename_place,
        commands::places::merge_places,
        commands::places::delete_place,
        commands::places::place_thumbs,
        commands::places::get_trip_clusters,
        commands::ai::provider_status,
        commands::ai::ensure_ai_ready,
        commands::ai::get_ai_settings,
        commands::ai::set_ai_settings,
        commands::ai::turn_ai_on,
        commands::tagging::start_tagging,
        commands::tagging::tag_translations,
        commands::tagging::translate_tags,
        commands::tagging::cancel_tagging,
        commands::export::cancel_export,
        commands::tagging::reset_tagging,
        commands::develop::open_develop,
        commands::develop::develop_histogram,
        commands::render::set_develop_draft,
        commands::masks::ai_status,
        commands::masks::ai_select_subject,
        commands::masks::develop_denoise,
        commands::masks::develop_erase,
        commands::masks::develop_remove_erase,
        commands::masks::develop_prepare_erase,
        commands::develop::reset_develop,
        commands::masks::ai_select_points,
        commands::ai::ai_available_models,
        commands::ai::download_ai_model,
        commands::ai::delete_ai_model,
        commands::ai::hardware_report,
        commands::browse::search_index_status,
        commands::browse::index_for_search,
        commands::backups::backup_catalogue,
        commands::app::show_in_folder,
        commands::app::show_folder,
        commands::app::folder_exists,
        commands::cull::bin_rejects,
        commands::cull::delete_photos,
        commands::backups::list_catalogue_backups,
        commands::backups::restore_catalogue_backup,
        commands::backups::cancel_catalogue_restore,
        commands::backups::catalogue_restore_pending,
        commands::backups::catalogue_recovery,
        commands::backups::restore_newest_backup,
        commands::backups::restart_app,
        commands::cull::quality_status,
        commands::cull::score_quality,
        commands::people::faces_status,
        commands::people::analyze_faces,
        commands::people::group_people,
        commands::people::list_people,
        commands::people::set_person_name,
        commands::people::person_faces,
        commands::people::photo_faces,
        commands::people::reset_faces,
        commands::people::people_enabled,
        commands::people::set_people_enabled,
        commands::people::erase_face_data,
        commands::people::merge_people,
        commands::people::undo_merge,
        commands::people::detach_face,
        commands::people::set_cover_face,
        commands::people::unplaced_faces,
        commands::people::dismiss_face,
        commands::people::set_people_hidden,
        commands::people::add_photos_to_person,
        commands::people::add_photos_to_new_person,
        commands::people::remove_photos_from_person,
        commands::people::photo_people,
        commands::people::people_suggestions,
        commands::people::keep_people_apart,
        commands::people::set_eyes_override,
        commands::browse::semantic_search,
        commands::browse::similar_photos,
        commands::browse::highlights,
        commands::browse::caption_search,
        commands::tagging::caption_status,
        commands::tagging::start_captioning,
        commands::tagging::cancel_captioning,
        commands::ai::vlm_catalogue,
        commands::ai::local_vlm_storage,
        commands::ai::remove_local_vlm,
        commands::cull::duplicate_groups,
        commands::develop::set_develop_stack,
        commands::develop::reframe_develop,
        commands::develop::undo_develop,
        commands::develop::redo_develop,
        commands::develop::auto_tone_develop,
        commands::develop::auto_wb_develop,
        commands::develop::pick_wb_develop,
        commands::develop::auto_all_develop,
        commands::presets::import_lut,
        commands::presets::list_luts,
        commands::presets::set_lut_group,
        commands::presets::delete_lut,
        commands::presets::lut_usage,
        commands::presets::list_preset_groups,
        commands::presets::preset_stack,
        commands::presets::suggest_edits,
        commands::presets::builtin_preset_previews,
        commands::presets::preset_previews,
        commands::presets::lut_previews,
        commands::develop::save_develop_as_copy,
        commands::export::export_develop,
        commands::export::get_export_rights,
        commands::export::set_export_rights,
        commands::export::export_batch,
        commands::render::set_direct_rendering,
        commands::render::render_direct,
        commands::render::probe_direct,
        commands::develop::close_develop,
        commands::presets::list_presets,
        commands::presets::save_preset,
        commands::presets::delete_preset,
        commands::presets::apply_preset,
        commands::develop::copy_settings_to,
    ])
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let context = tauri::generate_context!();
    // An AI app starting the editing server as a command (`mcp.rs`). Before
    // the app proper, so it runs as a bridge to the open app instead of being
    // turned away as a second copy. The identifier finds the server's token.
    if std::env::args().nth(1).as_deref() == Some("--mcp") {
        return mcp::bridge_stdio(&context.config().identifier);
    }
    let builder = specta_builder();

    // Before anything uses the data folder: a library from before the rename
    // to SourceLight moves to the folder this build looks in.
    #[cfg(not(any(debug_assertions, feature = "dev-tools")))]
    identity::adopt_former_data(&context.config().identifier);
    #[cfg(any(debug_assertions, feature = "dev-tools"))]
    if std::env::var_os(DATA_DIR_ENV).is_none_or(|d| d.is_empty()) {
        identity::adopt_former_data(&context.config().identifier);
    }

    // First, so a second launch hands over to the running app before its own
    // setup opens the catalogue. Two copies writing one catalogue (and each
    // scanning the same folders) is never what anyone meant, and it is the
    // one thing SQLite's own safety cannot make sensible.
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        // The window comes back where it was left: size, position, maximised
        // or full screen. Not visibility — the window starts hidden and the UI
        // shows it once it has painted, and a restored "visible" would show it
        // blank first.
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_filename(window_state_file())
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::all()
                        - tauri_plugin_window_state::StateFlags::VISIBLE,
                )
                .build(),
        );
    // Only a build configured with an update key and endpoint gets the updater:
    // the plugin will not start without a key, and an optional feature must not
    // be able to stop the app opening. See `updates.rs`.
    let app = if updates::configured(context.config()) {
        app.plugin(tauri_plugin_updater::Builder::new().build())
    } else {
        app
    };

    // Asynchronous, like `full` below: a synchronous handler runs on the
    // webview's own thread, so a screen of thumbnails was served one at a
    // time — measured at about 3.5 ms apiece, 150-180 ms for the ~50 a scroll
    // or a jump asks for at once. On the blocking pool they are read side by
    // side.
    app.register_asynchronous_uri_scheme_protocol("thumb", |ctx, request, responder| {
        let app = ctx.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            responder.respond(serve_thumb(&app, request.uri()))
        });
    })
    // Asynchronous: reading a 25 MB raw (or asking LibRaw for a CR3's
    // preview) must not stall the webview's thread.
    .register_asynchronous_uri_scheme_protocol("full", |ctx, request, responder| {
        let app = ctx.app_handle().clone();
        std::thread::spawn(move || responder.respond(serve_full(&app, request.uri())));
    })
    // Asynchronous too. A synchronous handler runs on the thread that owns the
    // webview — on macOS that is AppKit's main thread — so every frame of a
    // slider drag held up menus, window drags and typing for as long as the
    // render took. Frames still go one at a time: the frontend keeps one in
    // flight (`useLatestFrame`), and rendering holds the develop lock.
    .register_asynchronous_uri_scheme_protocol("develop", |ctx, request, responder| {
        #[cfg(feature = "libraw")]
        {
            let app = ctx.app_handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                let state = app.state::<AppState>();
                responder.respond(serve_develop(&state, request.uri()))
            });
        }
        #[cfg(not(feature = "libraw"))]
        {
            let _ = (ctx, request);
            responder.respond(
                tauri::http::Response::builder()
                    .status(501)
                    .body(b"raw decoding is not compiled in".to_vec())
                    .unwrap_or_default(),
            )
        }
    })
    .invoke_handler(builder.invoke_handler())
    .setup(|app| {
        let dir = data_dir(app)?;
        std::fs::create_dir_all(&dir)?;

        // The title bar in the top bar's colours, so the two read as one.
        #[cfg(windows)]
        if let Some(window) = app.get_webview_window("main") {
            if let Ok(hwnd) = window.hwnd() {
                window_chrome::match_title_bar(hwnd.0 as isize);
            }
        }
        // The smallest window, no bigger than the screen it opens on:
        // at 175% scaling a 1080p laptop's work area is about 570 logical
        // pixels tall, and a fixed 620 put the bottom of every view off it.
        if let Some(window) = app.get_webview_window("main") {
            if let Ok(Some(monitor)) = window.current_monitor() {
                let area = monitor
                    .work_area()
                    .size
                    .to_logical::<f64>(monitor.scale_factor());
                let (w, h) = min_window_size(area.width, area.height);
                let _ = window.set_min_size(Some(tauri::LogicalSize::new(w, h)));
            }
        }

        // The session log first, so everything after it is kept: stderr,
        // stamped and on disk, and whether the last session ever exited
        // (`session_log`). Without it a release build has nowhere to print.
        let logs = dir.join("logs");
        let stamp = format!(
            "{}-{}",
            chrono::Local::now().format("%Y-%m-%dT%H-%M-%S"),
            std::process::id()
        );
        let session = session_log::begin(&logs, &stamp)
            .inspect(|s| {
                if let Err(e) = session_log::capture_stderr(&s.log) {
                    eprintln!("session log: cannot capture stderr: {e}");
                }
            })
            .inspect_err(|e| eprintln!("session log: {e}"))
            .ok();
        eprintln!(
            "session start: version {}, {} {}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        if let Some(previous) = session.as_ref().and_then(|s| s.unclean.as_ref()) {
            eprintln!(
                "the previous session never exited; its log is kept as {}",
                previous.display()
            );
        }

        // Then the panic hook, before anything below can panic, so a crash
        // during startup is still logged. The GPU line is filled in off the
        // main thread — probing adapters is not worth making the window wait
        // on — so an early crash simply logs without it.
        let diagnostics = crash::install(logs.clone());
        // One copy of the machine facts, shared by the panic hook, the
        // frontend error log and Settings' "Copy diagnostics".
        let shared = crash::Diagnostics::new(std::sync::Arc::clone(&diagnostics), logs);
        app.manage(match session {
            Some(s) => shared.with_session(s),
            None => shared,
        });
        std::thread::spawn(move || {
            let hw = hardware::probe();
            let line = format!(
                "gpu:     {} ({}, {})\nthreads: {}",
                hw.gpu_name, hw.gpu_kind, hw.backend, hw.cpu_threads
            );
            eprintln!(
                "{line}\nmemory:  {} MB video, {} MB system",
                hw.vram_mb.unwrap_or(0),
                hw.ram_mb.unwrap_or(0)
            );
            let mut d = crate::sync::lock_recover(&diagnostics);
            d.push_str(&format!("\n{line}"));
        });

        // CoreML keeps the models it compiles in the app's cache folder and
        // reuses them, rather than compiling each into the temp dir again and
        // leaving it there, as the app never drops its sessions
        // (`develop::coreml_cache`). What earlier runs left there goes now.
        #[cfg(all(feature = "ai", target_os = "macos"))]
        {
            let coreml = app.path().app_cache_dir().ok().map(|c| c.join("coreml"));
            if let Some(coreml) = coreml.clone() {
                develop::ai::set_coreml_cache(coreml);
            }
            // Click to select and Select Subject run on the CPU now
            // (`ai::commit_without_compile`): what CoreML compiled for them
            // goes, half a gigabyte for SAM 2's encoder alone.
            let models = dir.join("models");
            std::thread::spawn(move || {
                let removed = develop::coreml_cache::sweep_leftovers(&std::env::temp_dir());
                if removed > 0 {
                    eprintln!(
                        "ai: removed {removed} CoreML folders earlier runs left in the temp dir"
                    );
                }
                let off = [
                    models.join("sam2/encoder.onnx"),
                    models.join("sam2/decoder.onnx"),
                    models.join("birefnet.onnx"),
                ];
                if let Some(coreml) = coreml {
                    let removed = develop::coreml_cache::forget(&coreml, &off);
                    if removed > 0 {
                        eprintln!("ai: removed {removed} CoreML compiles no model uses now");
                    }
                }
            });
        }

        ai_models::remove_retired(&dir.join("models"));
        ai_models::remove_unused_variants(&dir.join("models"));
        remove_ollama_leftovers(&dir);
        // A server an earlier run left behind, and downloads it never
        // finished.
        sidecar::stop_orphan(&dir.join("llama-server.pid"));
        ai_models::remove_partial_downloads(&dir.join("models"));
        ai_models::remove_partial_downloads(&dir.join("llama-runtime"));
        let _ = std::fs::remove_dir_all(dir.join(llama_pack::STAGING));
        // The face detector ships in the installer: put it where the models
        // live, so culling finds it with nothing to download.
        if cfg!(feature = "ai") {
            let sources = bundled_model_sources(app.handle());
            for id in ai_models::BUNDLED {
                ai_models::install_bundled(&sources, &dir.join("models"), id);
            }
        }

        let cache_dir = dir.join("thumbs");
        std::fs::create_dir_all(&cache_dir)?;
        let db_path = dir.join("library.db");

        // Grant the asset protocol access to the thumbnail cache using the
        // exact resolved path. The declarative `$APPLOCALDATA/**` scope in
        // tauri.conf.json does NOT match on Windows — the pattern uses `/`
        // while the path uses `\`, so every thumbnail request came back
        // 403 Forbidden. Doing it here also means the grant can never drift
        // from the directory we actually write into.
        // Migrate once at startup; per-command connections skip it. A damaged
        // catalogue is swapped for its newest good backup here, and the
        // interface says so.
        let (_, recovery) = db::open_or_recover(&db_path)?;

        // The startup check reads only the schema. A full check runs once the
        // window is up, off the main thread, and asks the interface to offer
        // the newest backup if it finds damage deeper in.
        let checked = db_path.clone();
        let handle = app.handle().clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(20));
            if let Ok(Some(problem)) = db::quick_check(&checked) {
                eprintln!("catalogue check found damage: {problem}");
                let _ = handle.emit("catalogue-damaged", problem);
            }
        });

        // Magic erase's fills nothing names any more: an erase undone, or a
        // photo reset. Once at startup, when no history is held in memory.
        let swept = db_path.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(30));
            sweep_unused_fills(&swept);
        });

        app.manage(AppState {
            db_path,
            cache_dir,
            recovery: std::sync::Mutex::new(recovery),
            cancel_tagging: AtomicBool::new(false),
            cancel_captioning: AtomicBool::new(false),
            cancel_export: AtomicBool::new(false),
            exporting: AtomicBool::new(false),
            vlm_server: std::sync::Mutex::new(None),
            develop: std::sync::Mutex::new(None),
            full_previews: std::sync::Mutex::new(full_preview::Cache::new(6)),
            verdict_undo: std::sync::Mutex::new(Vec::new()),
            #[cfg(feature = "libraw")]
            develop_opens: std::sync::atomic::AtomicU64::new(0),
            develop_open: std::sync::atomic::AtomicBool::new(false),
            vision_setup: tokio::sync::Mutex::new(()),
            active_downloads: std::sync::Mutex::new(std::collections::HashSet::new()),
            search_embedder: std::sync::Mutex::new(None),
            face_engine: std::sync::Mutex::new(None),
            #[cfg(feature = "ai")]
            denoiser: std::sync::Mutex::new(None),
            #[cfg(feature = "ai")]
            eraser: std::sync::Mutex::new(None),
            #[cfg(feature = "ai")]
            sam2_encoding: std::sync::Mutex::new(()),
            people_work: std::sync::Mutex::new(()),
            people_edit: std::sync::Mutex::new(()),
            last_merge: std::sync::Mutex::new(None),
            people_suggestions: std::sync::Mutex::new(None),
            portraits_filling: AtomicBool::new(false),
            vlm_seq: AtomicU64::new(0),
            vlm_busy: std::sync::atomic::AtomicUsize::new(0),
            search_gen: AtomicU64::new(0),
            jobs: jobs::Jobs::default(),
            phone: phone_link::State::default(),
            search_cache: std::sync::Mutex::new(None),
        });

        // The phone link listens from launch if it was left on. A no-op in a
        // build without it.
        let handle = app.handle().clone();
        tauri::async_runtime::spawn(async move {
            let state = handle.state::<AppState>();
            if let Err(e) = phone_link::apply(&state.phone, &handle, &state.db_path).await {
                eprintln!("phone link did not start: {e}");
            }
            // The editing server too, if it was left on.
            mcp::apply(&handle).await;
        });

        // The window starts hidden and the interface shows it once it has
        // painted (`main.tsx`), so launching never shows an empty frame while
        // WebView2 starts. If the interface never gets that far, it must not
        // leave an invisible app running, so this shows it regardless.
        let handle = app.handle().clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(4));
            if let Some(window) = handle.get_webview_window("main") {
                if !window.is_visible().unwrap_or(true) {
                    let _ = window.show();
                }
            }
        });
        Ok(())
    })
    .build(context)
    .expect("error while building photo organiser")
    .run(|app_handle, event| {
        match &event {
            // A folder dropped on the Dock icon (the app declares folders as
            // a document type it can open, in `Info.plist`). Files are left
            // to `add_library_folder`, which says it wants a folder.
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Opened { urls } => {
                let paths: Vec<String> = urls
                    .iter()
                    .filter_map(|u| u.to_file_path().ok())
                    .map(|p| p.display().to_string())
                    .collect();
                if !paths.is_empty() {
                    crate::sync::lock_recover(&OPENED_FOLDERS).extend(paths);
                    let _ = app_handle.emit("folders-opened", ());
                }
            }
            tauri::RunEvent::ExitRequested {
                api, code: None, ..
            } if hold_quit_for_the_ui(app_handle) => {
                api.prevent_exit();
            }
            tauri::RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::CloseRequested { api, .. },
                ..
            } if label == "main" && hold_quit_for_the_ui(app_handle) => {
                api.prevent_close();
            }
            _ => {}
        }
        // Stop the vision model's server when the app exits, so it never
        // outlives the window that owns it (and on Windows the job it was
        // started in ends it even when the app does not exit cleanly).
        if let tauri::RunEvent::Exit = event {
            if let Some(state) = app_handle.try_state::<AppState>() {
                stop_vlm_server(&state);
            }
            // A normal exit: the next launch has nothing to report.
            if let Some(d) = app_handle.try_state::<crash::Diagnostics>() {
                eprintln!("session end");
                if let Some(log) = &d.session_log {
                    session_log::finish(&d.log_dir, log);
                }
            }
            // Last: ONNX Runtime must be released before `exit()` runs its
            // static destructors, or a Mac aborts on quit (see `release_ort`).
            #[cfg(feature = "ai")]
            develop::ai::release_ort();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The minimum window fits a scaled-up laptop screen.
    #[test]
    fn the_minimum_window_fits_the_screen() {
        assert_eq!(min_window_size(1920.0, 1040.0), (940.0, 620.0));
        // 1080p at 175%: about 1097 x 594 logical, less the taskbar.
        let (w, h) = min_window_size(1097.0, 571.0);
        assert!(w <= 1097.0 - 40.0 && h <= 571.0 - 40.0, "{w} x {h}");
    }
}
