//! The app itself: diagnostics, updates, quitting, licences, the file
//! manager and the editing server.

use std::path::Path;
use tauri::{AppHandle, Emitter, Manager, State};

use super::common::err;
use crate::{crash, db, hardware, mcp, shell, sync, updates, AppState};

/// Asks a running long job to stop — a model download, the local AI
/// runtime install, a vision-model pull, search indexing or face analysis — by
/// the id its progress is tracked under. False when nothing is running under
/// that id, which is not an error: it may have finished a moment ago.
#[tauri::command]
#[specta::specta]
pub(crate) fn cancel_job(state: State<'_, AppState>, id: String) -> bool {
    state.jobs.cancel(&id)
}

/// Writes an interface error — one an error boundary caught, or one nothing did —
/// beside the crash logs. Best-effort by nature: failing to log an error
/// must never become a second error the user sees.
#[tauri::command]
#[specta::specta]
pub(crate) fn log_ui_error(
    diagnostics: State<'_, crash::Diagnostics>,
    surface: String,
    message: String,
    stack: String,
) -> Result<(), String> {
    crash::log_ui_error(&diagnostics, &surface, &message, &stack)
        .map(|_| ())
        .map_err(err)
}

/// The text Settings' "Copy diagnostics" puts on the clipboard: version, OS,
/// GPU, where the catalogue and logs live, and the newest logs.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn diagnostics_summary(
    state: State<'_, AppState>,
    diagnostics: State<'_, crash::Diagnostics>,
) -> String {
    let text = sync::lock_recover(&diagnostics.text).clone();
    let logs = crash::recent_logs(&diagnostics.log_dir, 10);
    crash::summary(&text, &state.db_path, &diagnostics.log_dir, &logs)
}

/// The log of a previous session that never exited (the PC froze, lost power,
/// or the app was killed), by file name, for the interface to say so once. A
/// second call returns nothing.
#[tauri::command]
#[specta::specta]
pub(crate) fn unclean_exit(diagnostics: State<'_, crash::Diagnostics>) -> Option<String> {
    sync::lock_recover(&diagnostics.unclean_exit)
        .take()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
}

/// Opens the folder crash and error logs are written to.
#[tauri::command]
#[specta::specta]
pub(crate) fn open_log_folder(diagnostics: State<'_, crash::Diagnostics>) -> Result<(), String> {
    crash::open_folder(&diagnostics.log_dir).map_err(err)
}

/// Whether a newer release is available — see `updates.rs`. An error is
/// the caller's to show or swallow: the automatic check at launch stays silent,
/// a check the user asked for says what went wrong.
#[tauri::command]
#[specta::specta]
pub(crate) async fn check_for_update(app: AppHandle) -> Result<updates::UpdateCheck, String> {
    updates::check(&app).await
}

/// Whether AI apps can edit the open photo through the editing server
/// (`mcp.rs`), and the address to give them.
#[tauri::command]
#[specta::specta]
pub(crate) fn edit_server_status(
    state: State<'_, AppState>,
) -> Result<mcp::EditServerStatus, String> {
    mcp::status(&state.conn()?).map_err(err)
}

/// Turns the editing server on or off, and starts or stops it to match.
#[tauri::command]
#[specta::specta]
pub(crate) async fn set_edit_server(
    app: AppHandle,
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<mcp::EditServerStatus, String> {
    mcp::set_enabled(&state.conn()?, enabled).map_err(err)?;
    mcp::apply(&app).await;
    mcp::status(&state.conn()?).map_err(err)
}

/// The answer "quit" to `quit-requested`: an explicit exit, which the quit
/// guard below lets through.
#[tauri::command]
#[specta::specta]
pub(crate) fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// Folders the Finder handed the app — dropped on its Dock icon, or opened
/// with it — waiting for the interface to take them. Held rather than only
/// sent, because a drop that launches the app arrives before the interface
/// is listening.
pub(crate) static OPENED_FOLDERS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// The folders handed over since the interface last asked (`folders-opened`),
/// for it to add to the library as it does a folder dropped on the window.
#[tauri::command]
#[specta::specta]
pub(crate) fn take_opened_folders() -> Vec<String> {
    std::mem::take(&mut *crate::sync::lock_recover(&OPENED_FOLDERS))
}

/// Set while the UI is deciding whether a quit may go ahead (`quit-requested`).
/// A second ⌘Q or close in that time is let through, so a UI that never
/// answers can never trap anyone in the app.
static QUIT_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The answer "stay open" to `quit-requested`.
#[tauri::command]
#[specta::specta]
pub(crate) fn stay_open() {
    QUIT_PENDING.store(false, std::sync::atomic::Ordering::Release);
}

/// Holds a quit the user started — ⌘Q, the menu's Quit, the window's close
/// button — and asks the UI first, which knows whether an export, a card
/// import or an analysis would be cut off and says so. Returns whether it held
/// it. An exit with a code (`quit_app`, the updater's restart) always goes.
pub(crate) fn hold_quit_for_the_ui(app: &AppHandle) -> bool {
    use std::sync::atomic::Ordering;
    if app.get_webview_window("main").is_none() || QUIT_PENDING.swap(true, Ordering::AcqRel) {
        return false;
    }
    let _ = app.emit("quit-requested", ());
    true
}

/// SourceLight's own licence, for Settings → About. Compiled in rather than
/// read from the install folder like the notices below: it is small, and it
/// can then always be shown, whatever a packager left beside the program.
///
/// Nothing asks for it to be accepted (a first-run agreement used to): the licence grants
/// rights to anyone who receives the program and asks nothing of someone
/// who only runs it.
#[tauri::command]
#[specta::specta]
pub(crate) fn licence_text() -> String {
    include_str!("../../../LICENSE").to_string()
}

/// The licences of everything the app bundles, from the file the installer
/// ships beside it. Read at runtime rather than compiled in: it is ~440 KB,
/// and it is already on disk.
#[tauri::command]
#[specta::specta]
pub(crate) fn third_party_licences(app: AppHandle) -> Result<String, String> {
    let bundled = app
        .path()
        .resource_dir()
        .map(|d| d.join("THIRD-PARTY-LICENSES.txt"))
        .ok()
        .filter(|p| p.is_file());
    // A dev build runs from the tree, where the file sits beside Cargo.toml.
    let path = bundled.unwrap_or_else(|| {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("THIRD-PARTY-LICENSES.txt")
    });
    std::fs::read_to_string(&path)
        .map_err(|e| err(anyhow::Error::new(e).context(format!("cannot read {}", path.display()))))
}

/// Downloads, verifies and runs the newer release. On Windows the installer
/// takes over and this process exits.
#[tauri::command]
#[specta::specta]
pub(crate) async fn install_update(app: AppHandle) -> Result<(), String> {
    updates::install(&app).await
}

/// Whether the machine is on its battery (`hardware::on_battery`); unknown is
/// mains. Automatic analysis asks before each vision-model batch.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn on_battery() -> bool {
    hardware::on_battery().unwrap_or(false)
}

/// Whether a folder is there now: a remembered backup drive, say.
#[tauri::command]
#[specta::specta]
pub(crate) fn folder_exists(path: String) -> bool {
    Path::new(&path).is_dir()
}

/// Opens the file manager at a folder an export wrote into.
#[tauri::command]
#[specta::specta]
pub(crate) fn show_folder(path: String) -> Result<(), String> {
    shell::reveal(Path::new(&path)).map_err(err)
}

/// Opens the file manager with a photo's file selected.
#[tauri::command]
#[specta::specta]
pub(crate) fn show_in_folder(state: State<'_, AppState>, photo_id: i32) -> Result<(), String> {
    let conn = state.conn()?;
    let photo = db::get_photo(&conn, photo_id)
        .map_err(err)?
        .ok_or_else(|| format!("photo {photo_id} not found"))?;
    shell::reveal(Path::new(&photo.path)).map_err(err)
}
