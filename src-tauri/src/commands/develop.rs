//! Develop: opening a photo, its edits and their history, and copying
//! settings to other photos.

use tauri::State;

use super::common::{count, err, persist_develop, push_undo, undo_label, with_develop};
use super::render::trace_frame;
#[cfg(feature = "libraw")]
use crate::raw;
#[cfg(feature = "ai")]
use crate::sync::lock_recover;
use crate::{db, develop, AppState, UndoStep};

#[tauri::command(async)]
#[specta::specta]
pub(crate) fn open_develop(
    state: State<'_, AppState>,
    photo_id: i32,
) -> Result<develop::DevelopInfo, String> {
    let conn = state.conn()?;
    let photo = db::get_photo(&conn, photo_id)
        .map_err(err)?
        .ok_or_else(|| format!("photo {photo_id} not found"))?;
    let stored = db::get_develop_settings(&conn, photo_id).map_err(err)?;
    if !std::path::Path::new(&photo.path).exists() {
        return Err(missing_file_message(&photo.path));
    }

    #[cfg(feature = "libraw")]
    {
        // Off the main thread, and the decode — the slow half — outside the
        // develop lock, so a preview or histogram isn't held up behind it.
        // Stepping fast asks for several photos before the first is
        // open: each is numbered, and one overtaken by a later request stops
        // before it takes the lock rather than decoding for nothing twice.
        let ticket = state
            .develop_opens
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1;
        let decoder = raw::libraw::LibRawDecoder::new();
        let source =
            crate::raster::load(std::path::Path::new(&photo.path), &decoder).map_err(err)?;
        if state
            .develop_opens
            .load(std::sync::atomic::Ordering::SeqCst)
            != ticket
        {
            return Err(OPEN_SUPERSEDED.to_string());
        }
        let info = with_develop(&state, |dev| {
            dev.open_source(photo_id, &source, stored).map_err(err)
        })?;
        // Open now, so automatic analysis waits between photos until it
        // closes (`wait_while_developing`). Only once it is: a photo that
        // will not open, with none open before it, leaves the interface
        // out of Develop, and nothing would close it again.
        state
            .develop_open
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(info)
    }
    #[cfg(not(feature = "libraw"))]
    {
        let _ = (photo, stored);
        Err("this build was compiled without raw decoding".to_string())
    }
}

/// What `open_develop` answers when a later open overtook it: the
/// page drops it quietly, since the photo it now wants is on its way.
#[cfg(feature = "libraw")]
const OPEN_SUPERSEDED: &str = "superseded by a later open";

/// What to say when a photo's file is not where the catalogue has it:
/// what happened, and the way back — not a decoder's "file not found".
fn missing_file_message(path: &str) -> String {
    format!(
        "This photo's file isn't at {path}. If it's on a drive that isn't connected, \
         connect it; if its folder was moved inside the library, the next scan finds it. \
         Its ratings, tags and edits are kept either way."
    )
}

/// An RGB histogram of the current edit — or of `draft`, the edit mid-drag —
/// for the develop panel. Rendered in Rust because the custom-scheme preview
/// cannot be read back through a canvas.
///
/// Off the main thread: during a drag it is asked for alongside every frame,
/// and its readback should not hold up the window.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn develop_histogram(
    state: State<'_, AppState>,
    draft: Option<develop::EditStack>,
) -> Result<develop::Histogram, String> {
    let started = std::time::Instant::now();
    let histogram = with_develop(&state, |dev| dev.histogram(draft.as_ref()).map_err(err));
    trace_frame("histogram", started);
    histogram
}

#[tauri::command]
#[specta::specta]
pub(crate) fn set_develop_stack(
    state: State<'_, AppState>,
    label: String,
    stack: develop::EditStack,
) -> Result<develop::DevelopInfo, String> {
    let stack = stack.sanitised();
    let info = with_develop(&state, |dev| {
        let stack = dev.with_own_erases(stack).map_err(err)?;
        dev.apply(&label, stack).map_err(err)
    })?;
    persist_develop(&state, info)
}

/// Puts the open photo's edit back to how a new photo opens, erases and all,
/// as one step in history. Not through [`set_develop_stack`], which keeps the
/// photo's erases whatever the page sends.
#[tauri::command]
#[specta::specta]
pub(crate) fn reset_develop(state: State<'_, AppState>) -> Result<develop::DevelopInfo, String> {
    let info = with_develop(&state, |dev| {
        dev.apply("Reset", develop::EditStack::default())
            .map_err(err)
    })?;
    persist_develop(&state, info)
}

/// A change of framing — crop, aspect, straighten, turn, flip — with the
/// masks and spots kept on the content rather than the frame.
/// Async: an AI mask is resampled, which is work for a pool thread.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn reframe_develop(
    state: State<'_, AppState>,
    label: String,
    stack: develop::EditStack,
) -> Result<develop::DevelopInfo, String> {
    let stack = stack.sanitised();
    let info = with_develop(&state, |dev| {
        let stack = dev.with_own_erases(stack).map_err(err)?;
        dev.reframe(&label, stack).map_err(err)
    })?;
    persist_develop(&state, info)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn undo_develop(state: State<'_, AppState>) -> Result<develop::DevelopInfo, String> {
    let info = with_develop(&state, |dev| dev.undo().map_err(err))?;
    persist_develop(&state, info)
}

/// Auto-tone the open photo: measure the scene and set a balanced
/// exposure/whites/blacks/highlights/shadows in one undoable step. Heuristic (no
/// model), so it's instant and needs nothing downloaded.
#[tauri::command]
#[specta::specta]
pub(crate) fn auto_tone_develop(
    state: State<'_, AppState>,
) -> Result<develop::DevelopInfo, String> {
    let info = with_develop(&state, |dev| dev.auto_tone().map_err(err))?;
    persist_develop(&state, info)
}

/// White balance from a point the user says is grey, in framed-image UV.
#[tauri::command]
#[specta::specta]
pub(crate) fn pick_wb_develop(
    state: State<'_, AppState>,
    x: f32,
    y: f32,
) -> Result<develop::DevelopInfo, String> {
    let info = with_develop(&state, |dev| dev.pick_wb(x, y).map_err(err))?;
    persist_develop(&state, info)
}

/// Auto white balance the open photo: estimate the cast from its near-neutral
/// midtones and set warmth/tint to counter it, in one undoable step. Heuristic
/// (gray-world), instant, no model.
#[tauri::command]
#[specta::specta]
pub(crate) fn auto_wb_develop(state: State<'_, AppState>) -> Result<develop::DevelopInfo, String> {
    let info = with_develop(&state, |dev| dev.auto_wb().map_err(err))?;
    persist_develop(&state, info)
}

/// Full auto (tone + white balance) in one undoable step. What the toolbar "Auto"
/// button runs.
#[tauri::command]
#[specta::specta]
pub(crate) fn auto_all_develop(state: State<'_, AppState>) -> Result<develop::DevelopInfo, String> {
    let info = with_develop(&state, |dev| dev.auto_all().map_err(err))?;
    persist_develop(&state, info)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn redo_develop(state: State<'_, AppState>) -> Result<develop::DevelopInfo, String> {
    let info = with_develop(&state, |dev| dev.redo().map_err(err))?;
    persist_develop(&state, info)
}

/// Keeps the edits on screen as a **virtual copy**, puts the original back to
/// how it was when it was opened, and moves Develop onto the copy.
///
/// A second row on the same raw file, carrying these edits and nothing else —
/// no rating, flag, label or tags — so a look tried on a photo can live beside
/// the version that is already finished rather than replacing it.
///
/// Develop saves as it goes, so by now the original's row already holds
/// the edits on screen; what it had before lives only in the session's
/// baseline. The copy is made from the row, the row goes back to the baseline,
/// and the session follows the copy so that further edits land on it rather
/// than quietly re-editing the original the user just asked to keep.
#[tauri::command]
#[specta::specta]
pub(crate) fn save_develop_as_copy(
    state: State<'_, AppState>,
    name: Option<String>,
) -> Result<develop::DevelopInfo, String> {
    let (photo_id, current, baseline) = with_develop(&state, |dev| {
        let id = dev.open_photo_id().ok_or("no photo is open")?;
        Ok((
            id,
            dev.current_stack().map_err(err)?,
            dev.baseline_stack().map_err(err)?,
        ))
    })?;

    let mut conn = state.conn()?;
    let copy =
        db::save_as_copy(&mut conn, photo_id, &current, &baseline, name.as_deref()).map_err(err)?;
    with_develop(&state, |dev| dev.rebind(copy).map_err(err))
}

/// Copies the chosen parts of the open photo's settings onto a selection,
/// and persists them; the rest of each photo's own edit is kept.
///
/// Writes directly rather than going through the develop session: the target
/// photos are not open, and decoding each one just to re-serialise a stack that
/// is already in hand would make a hundred-photo paste unusable.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn copy_settings_to(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
    groups: Vec<develop::stack::Group>,
) -> Result<u32, String> {
    let stack = with_develop(&state, |dev| dev.current_stack().map_err(err))?;
    let mut conn = state.conn()?;
    // What each photo had, so one Undo puts them all back: a copy to
    // a selection made earlier, and forgotten, overwrote forty careful edits.
    let before = photo_ids
        .iter()
        .map(|&id| Ok((id, db::get_develop_settings(&conn, id)?)))
        .collect::<anyhow::Result<Vec<_>>>()
        .map_err(err)?;
    let n = db::apply_groups_to(&mut conn, &photo_ids, &stack, &groups).map_err(err)?;
    push_undo(
        &state,
        UndoStep::Settings {
            label: undo_label("Copied settings", n),
            before,
        },
    );
    Ok(count(n))
}

/// Pastes the chosen parts of `stack` onto the open photo as one undoable
/// step: the settings as they were copied, not as their photo has
/// them now — an experiment made on it since doesn't come along.
#[tauri::command]
#[specta::specta]
pub(crate) fn paste_settings(
    state: State<'_, AppState>,
    stack: develop::EditStack,
    groups: Vec<develop::stack::Group>,
) -> Result<develop::DevelopInfo, String> {
    let info = with_develop(&state, |dev| {
        let current = dev.current_stack().map_err(err)?;
        dev.apply("Paste settings", stack.take_groups(&current, &groups))
            .map_err(err)
    })?;
    persist_develop(&state, info)
}

/// Pastes the chosen parts of another photo's saved settings onto the open
/// one, as one undoable step — "paste from previous".
#[tauri::command]
#[specta::specta]
pub(crate) fn paste_settings_from(
    state: State<'_, AppState>,
    photo_id: i32,
    groups: Vec<develop::stack::Group>,
) -> Result<develop::DevelopInfo, String> {
    let source = {
        let conn = state.conn()?;
        db::get_develop_settings(&conn, photo_id)
            .map_err(err)?
            .unwrap_or_default()
    };
    let info = with_develop(&state, |dev| {
        let current = dev.current_stack().map_err(err)?;
        dev.apply("Paste settings", source.take_groups(&current, &groups))
            .map_err(err)
    })?;
    persist_develop(&state, info)
}

/// Jumps to a step in the open photo's history.
#[tauri::command]
#[specta::specta]
pub(crate) fn develop_go_to(
    state: State<'_, AppState>,
    index: u32,
) -> Result<develop::DevelopInfo, String> {
    let info = with_develop(&state, |dev| dev.go_to(index as usize).map_err(err))?;
    persist_develop(&state, info)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn close_develop(state: State<'_, AppState>) -> Result<(), String> {
    state
        .develop_open
        .store(false, std::sync::atomic::Ordering::Relaxed);
    with_develop(&state, |dev| {
        dev.close();
        Ok(())
    })?;
    // The models only Develop uses go with it: Select Subject's,
    // magic erase's, and AI noise reduction's, which an export running now
    // keeps for as long as it needs it. The next use loads them again.
    #[cfg(feature = "ai")]
    {
        develop::ai::forget_subject_model();
        *lock_recover(&state.denoiser) = None;
        *lock_recover(&state.eraser) = None;
    }
    Ok(())
}

#[cfg(all(test, feature = "libraw"))]
mod tests {
    use super::*;
    use crate::present;

    /// Leaving a photo in Develop and coming back keeps its undo steps;
    /// a photo whose edit changed meanwhile starts afresh. A real raw.
    #[cfg(feature = "libraw")]
    #[test]
    fn develop_history_survives_leaving_the_photo() {
        use crate::raw::decoder::RawDecoder;
        let Some(dir) = crate::test_support::raw_dir() else {
            crate::test_support::skip(
                crate::test_support::Needs::Raw,
                "set SOURCELIGHT_TEST_RAW to a folder of raw files",
            );
            return;
        };
        let decoder = raw::libraw::LibRawDecoder::new();
        let exts = decoder.extensions();
        let Some(path) = crate::test_support::files_in(&dir, |e| exts.contains(&e))
            .into_iter()
            .next()
        else {
            return;
        };
        if crate::gpu::test_support::hardware_gpu().is_none() {
            return;
        }
        let mut dev =
            develop::DevelopState::new(Box::new(present::readback::ReadbackPresenter::new()))
                .unwrap();
        dev.open(1, &path, &decoder, None).unwrap();
        let brighter = develop::EditStack {
            exposure: 0.5,
            ..Default::default()
        };
        let edited = dev.apply("Exposure", brighter.clone()).unwrap();
        assert!(edited.can_undo);

        // Another photo, then back, with the edit as Develop saved it.
        dev.open(2, &path, &decoder, None).unwrap();
        let back = dev
            .open(1, &path, &decoder, Some(brighter.clone()))
            .unwrap();
        assert!(back.can_undo, "its steps are still there");
        assert_eq!(back.history.len(), 2);

        // Changed elsewhere in between (a copy to a selection): afresh.
        dev.open(2, &path, &decoder, None).unwrap();
        let other = develop::EditStack {
            exposure: -0.5,
            ..Default::default()
        };
        let changed = dev.open(1, &path, &decoder, Some(other)).unwrap();
        assert!(!changed.can_undo);
    }
}
