//! Helpers the command areas share.
//!
//! Error text for the interface, counts it can hold, the undo stack, the
//! develop lock and where the app keeps its data.

use std::path::{Path, PathBuf};
use tauri::Manager;

use crate::{
    backup, db, develop, errors, gpu, identity, jobs, present, sync, AppState, UndoStep,
    MAX_VERDICT_UNDO,
};

/// Puts `step` on the undo stack, dropping the oldest past the bound.
pub(crate) fn push_undo(state: &AppState, step: UndoStep) {
    let mut undo = sync::lock_recover(&state.verdict_undo);
    undo.push(step);
    if undo.len() > MAX_VERDICT_UNDO {
        undo.remove(0);
    }
}

/// After photos leave the catalogue: Undo stops here. Their ids are
/// given to the next photos added, so an older step undone afterwards set the
/// rating or flag of whichever new photo now had the id.
pub(crate) fn photos_removed(state: &AppState, label: &str) {
    let mut undo = sync::lock_recover(&state.verdict_undo);
    undo.clear();
    undo.push(UndoStep::Beyond {
        label: label.to_string(),
    });
}

/// "{what} on 12 photos", as every undo label reads.
pub(crate) fn undo_label(what: &str, n: usize) -> String {
    format!("{what} on {n} photo{}", if n == 1 { "" } else { "s" })
}

/// Marks a change Undo can't take back, so it stops there. The
/// change's result is passed through untouched.
pub(crate) fn beyond_undo<T>(
    state: &AppState,
    what: &str,
    n: usize,
    result: Result<T, String>,
) -> Result<T, String> {
    if result.is_ok() && n > 0 {
        push_undo(
            state,
            UndoStep::Beyond {
                label: undo_label(what, n),
            },
        );
    }
    result
}

/// Formats an error for the UI, **including its cause chain**.
///
/// `to_string()` on an `anyhow::Error` prints only the outermost context and
/// silently discards everything underneath — so a failure wrapped in
/// "creating a swapchain on the window" arrives with the actual reason thrown
/// away, and the message names the thing that failed rather than why. The
/// alternate form walks the chain. Every command routes through here, so this
/// was costing detail everywhere, not just once.
///
/// Where the root cause is one the interface can word — a missing file, a full
/// disk, a busy catalogue — it goes classified, with the chain alongside, so the
/// user reads it in their own language (see [`errors`]). A `String` is passed
/// through as it is: it is either already described or an authored message.
pub(crate) fn err<E: std::fmt::Display + 'static>(e: E) -> String {
    let any = &e as &dyn std::any::Any;
    if let Some(e) = any.downcast_ref::<anyhow::Error>() {
        return errors::describe(e);
    }
    if let Some(e) = any.downcast_ref::<std::io::Error>() {
        return errors::describe_std(e);
    }
    if let Some(e) = any.downcast_ref::<rusqlite::Error>() {
        return errors::describe_std(e);
    }
    format!("{e:#}")
}

/// Narrows a count for the UI, which cannot represent 64-bit integers. Saturates
/// rather than wrapping: a progress bar pinned at its maximum is a far better
/// failure than one that silently restarts from zero.
pub(crate) fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Borrows the develop state, creating the GPU context on first use.
///
/// Lazy on purpose: someone who only browses their library never pays for
/// device creation, and a machine whose adapter fails to initialise still runs
/// the organiser — the failure surfaces when they try to develop, with a
/// message about the GPU rather than a dead app.
pub(crate) fn with_develop<T>(
    state: &AppState,
    f: impl FnOnce(&mut develop::DevelopState) -> Result<T, String>,
) -> Result<T, String> {
    // A render that panicked while holding the lock poisoned it, and every
    // Develop call then failed until a restart. The state it left is
    // dropped — outside the lock, since dropping a direct surface waits on
    // the GPU — and made afresh below: the edits are in the catalogue.
    let mut guard = match state.develop.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            eprintln!("develop: a render panicked; starting the editor afresh");
            state.develop.clear_poison();
            let mut guard = poisoned.into_inner();
            let broken = guard.take();
            drop(guard);
            drop(broken);
            gpu::forget_shared();
            state.develop.lock().map_err(|_| "develop state poisoned")?
        }
    };
    if guard.is_none() {
        let presenter = Box::new(present::readback::ReadbackPresenter::new());
        let mut created = develop::DevelopState::new(presenter).map_err(err)?;
        created.set_luts_dir(ensure_luts_dir(state));
        created.set_fills_dir(fills_dir(state));
        eprintln!("develop: {}", created.describe());
        *guard = Some(created);
    }
    f(guard.as_mut().expect("just created"))
}

/// Where imported `.cube` LUTs live — a `luts` folder beside the library DB.
pub(crate) fn luts_dir(state: &AppState) -> PathBuf {
    state
        .db_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default()
        .join("luts")
}

/// Where magic erase's fills are kept — a `patches` folder beside the
/// library DB, one file each (`develop::erase::FillStore`).
pub(crate) fn fills_dir(state: &AppState) -> PathBuf {
    fills_dir_for(&state.db_path)
}

fn fills_dir_for(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default()
        .join("patches")
}

/// Deletes the fills that neither the catalogue nor any backup kept of it
/// names, a day after they were made. A backup restored later still finds
/// its fills, so every one is read; if one can't be, or a restore is
/// waiting, nothing is deleted.
pub(crate) fn sweep_unused_fills(db_path: &Path) {
    let fills = fills_dir_for(db_path);
    if !fills.is_dir() || backup::pending_for(db_path).is_some() {
        return;
    }
    let mut in_use = match db::erase_fills_in(db_path) {
        Ok(names) => names,
        Err(e) => {
            eprintln!("erase: kept every fill, the catalogue couldn't be read: {e:#}");
            return;
        }
    };
    let backups = match backup::list(db_path) {
        Ok(backups) => backups,
        Err(e) => {
            eprintln!("erase: kept every fill, the backups couldn't be listed: {e:#}");
            return;
        }
    };
    for kept in backups {
        match db::erase_fills_in(Path::new(&kept.path)) {
            Ok(names) => in_use.extend(names),
            Err(e) => {
                eprintln!(
                    "erase: kept every fill, {} couldn't be read: {e:#}",
                    kept.path
                );
                return;
            }
        }
    }
    let removed = develop::erase::sweep(
        &fills,
        &in_use,
        std::time::Duration::from_secs(24 * 60 * 60),
    );
    if removed > 0 {
        eprintln!("erase: removed {removed} fills nothing uses");
    }
}

/// The LUTs folder, with the built-in look pack seeded into it once. Used by
/// every path that *reads* the folder, so a fresh library shows the built-in
/// looks immediately. `delete_lut` deliberately uses the plain [`luts_dir`] so
/// removing a look never triggers a re-seed.
pub(crate) fn ensure_luts_dir(state: &AppState) -> PathBuf {
    let dir = luts_dir(state);
    if let Err(e) = develop::lut_packs::seed_builtins(&dir) {
        eprintln!("luts: could not seed built-in pack: {e}");
    }
    dir
}

/// Writes the open photo's current edit to the catalogue and passes the info
/// through.
///
/// Develop saves as it goes. It used to keep edits in memory until Save
/// was pressed, so Esc, "← Library" and clicking another frame in the filmstrip
/// all threw them away without a word — the one thing every other raw editor
/// has taught people never to expect. Every command that changes the edit now
/// ends here, so there is no path that can forget to save. It is one small
/// `UPDATE`, cheap enough to run on every slider frame.
pub(crate) fn persist_develop(
    state: &AppState,
    info: develop::DevelopInfo,
) -> Result<develop::DevelopInfo, String> {
    let conn = state.conn()?;
    db::set_develop_settings(&conn, info.photo_id, &info.stack).map_err(err)?;
    Ok(info)
}

/// The environment variable that points a development build at another data
/// folder — see [`data_dir`].
pub const DATA_DIR_ENV: &str = "SOURCELIGHT_DATA_DIR";

/// The app's own data folder: catalogue, thumbnails, models and logs.
///
/// A development build (debug, or built with `dev-tools`) can be pointed at
/// another folder with `SOURCELIGHT_DATA_DIR`, so a seeded
/// 50,000-photo library can be opened and scrolled without touching the real
/// one. A product build ignores the variable: nobody's installed app should
/// move its library because of something in the environment.
pub(crate) fn data_dir<R: tauri::Runtime, M: Manager<R>>(app: &M) -> tauri::Result<PathBuf> {
    #[cfg(any(debug_assertions, feature = "dev-tools"))]
    if let Some(dir) = std::env::var_os(DATA_DIR_ENV).filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    // The folder from before the rename, when it couldn't be moved this
    // launch (see `identity`).
    if let Some(dir) = identity::stayed() {
        return Ok(dir.to_path_buf());
    }
    app.path().app_local_data_dir()
}

/// Where the window keeps its size and place. A development run pointed at
/// another data folder keeps it there. In the real folder it would overwrite
/// where the installed app's window was, and it is the file that once made
/// an unused folder look in use to the move to SourceLight. The plugin
/// joins this to its own folder, and joining an absolute path replaces it.
pub(crate) fn window_state_file() -> String {
    let file = tauri_plugin_window_state::DEFAULT_FILENAME;
    #[cfg(any(debug_assertions, feature = "dev-tools"))]
    if let Some(dir) = std::env::var_os(DATA_DIR_ENV).filter(|d| !d.is_empty()) {
        return PathBuf::from(dir).join(file).to_string_lossy().into_owned();
    }
    file.to_owned()
}

/// The models directory (created if missing).
/// Where the installer's copies of [`ai_models::BUNDLED`] may be: its
/// resources, or, running from source, the vendored files they are built from.
pub(crate) fn bundled_model_sources(app: &tauri::AppHandle) -> Vec<std::path::PathBuf> {
    let mut sources = Vec::new();
    if let Ok(dir) = app.path().resource_dir() {
        sources.push(dir.join("models"));
    }
    sources.push(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("vendor/models"));
    sources
}

pub(crate) fn ai_models_dir(app: &tauri::AppHandle) -> anyhow::Result<std::path::PathBuf> {
    let dir = data_dir(app)?.join("models");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Holds a search-index or face pass between photos while Develop is open,
/// until it closes or the pass is stopped.
///
/// Automatic analysis says it pauses in Develop, and only did between its
/// steps: a pass already going, a library's worth of photos on a first run,
/// carried on beside the editor. On a slower machine its model runs and
/// Develop's renders want the same cores and the same graphics, the ones an
/// integrated GPU shares with the CPU, and the editor stuttered for it.
pub(crate) fn wait_while_developing(state: &AppState, cancel: &jobs::CancelToken) {
    while state
        .develop_open
        .load(std::sync::atomic::Ordering::Relaxed)
        && !cancel.is_cancelled()
    {
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}
