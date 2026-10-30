//! People: finding faces, grouping them into people, and naming and
//! editing them.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, State};

use super::common::{ai_models_dir, count, err, wait_while_developing};
use crate::models::ScanProgress;
use crate::sync::lock_recover;
use crate::{ai_models, backup, db, faces, gpu_gate, hardware, jobs, models, AppState};

/// Lazily loads the face models (YuNet, the eye models) from `<models>/faces`, then runs
/// `f`. Mirrors [`with_embedder`].
fn with_face_engine<T>(
    app: &AppHandle,
    state: &AppState,
    f: impl FnOnce(&faces::FaceEngine) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = lock_recover(&state.face_engine);
    if guard.is_none() {
        let dir = ai_models_dir(app).map_err(err)?.join("faces");
        *guard = Some(faces::FaceEngine::load(&dir).map_err(err)?);
    }
    let engine = guard
        .as_ref()
        .ok_or("the face models failed to load; try re-downloading them in Settings")?;
    f(engine)
}

/// How much of the face pass is left: photos never analysed, and photos whose
/// faces still have no identity.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn faces_status(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<models::FacesStatus, String> {
    let conn = state.conn()?;
    Ok(models::FacesStatus {
        // Photos whose eyes still need reading with the face mesh count
        // as work for the face pass, so it runs for them too.
        to_detect: db::unanalysed_count(&conn).map_err(err)?
            + db::eyes_pending_count(&conn).map_err(err)?,
        to_recognise: db::unembedded_count(&conn).map_err(err)?,
        recogniser_installed: people_model_installed(&app),
    })
}

/// Whether the People recogniser has been downloaded: without it, the
/// face pass detects and scores eyes, and identities wait for it.
fn people_model_installed(app: &AppHandle) -> bool {
    ai_models_dir(app)
        .map(|dir| {
            ai_models::statuses(&dir)
                .iter()
                .any(|m| m.id == "people" && m.installed)
        })
        .unwrap_or(false)
}

/// Analyses every not-yet-analysed photo for faces (box + focus + eye-open),
/// storing the result and reporting progress on `faces-progress`. Needs the face
/// models installed. Sync so the model work runs off the async reactor.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn analyze_faces(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<models::FacesSummary, String> {
    // Taken before reading the queue, so a pass that waited behind another
    // sees what that one left rather than repeating it.
    let _one_at_a_time = lock_recover(&state.people_work);
    let db_path = state.db_path.clone();
    let (queue, eyes_queue, recognise) = {
        let conn = db::connect(&db_path).map_err(err)?;
        // Recognising who is in a photo waits for People's yes;
        // finding the faces, for culling, does not.
        let recognise = db::people_enabled(&conn).map_err(err)?;
        let mut q = db::unanalysed_photos(&conn, 1_000_000).map_err(err)?;
        // Photos analysed before the recogniser arrived: re-running them
        // embeds their faces, and store_faces keeps who anyone was.
        if recognise && people_model_installed(&app) {
            q.extend(db::unembedded_photos(&conn).map_err(err)?);
        }
        // Faces found before the face mesh read eyes, minus photos
        // the detection above is about to redo anyway.
        let redone: std::collections::HashSet<i32> = q.iter().map(|(id, _)| *id).collect();
        let eyes: Vec<db::EyesToRead> = db::photos_needing_eyes(&conn)
            .map_err(err)?
            .into_iter()
            .filter(|(id, _, _)| !redone.contains(id))
            .collect();
        (q, eyes, recognise)
    };
    let total = queue.len() + eyes_queue.len();
    // Cancellable between photos, like search indexing: what was analysed
    // stays, and the rest waits for the next run.
    let cancel = state.jobs.start("faces");
    let mut done = 0usize;
    let mut summary = models::FacesSummary {
        photos: 0,
        faces: 0,
        embedded: 0,
        embed_failed: 0,
        first_error: None,
        stopped: false,
    };

    // Below normal priority, like the face models' own threads
    // (`develop::ai::cpu_threads`): this one decodes and takes part too.
    let _below_normal = hardware::BackgroundPriority::begin();
    with_face_engine(&app, &state, |engine| {
        for (photo_id, image_path) in queue {
            wait_while_developing(&state, &cancel);
            if cancel.is_cancelled() {
                break;
            }
            let result = std::fs::read(&image_path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    // Decoded once, for finding the faces, embedding them and
                    // the size they are measured against. Each of the three
                    // used to read the preview again.
                    let rgb = image::load_from_memory(&bytes)
                        .map_err(|e| {
                            format!("faces {photo_id}: decoding image for face detection: {e}")
                        })?
                        .to_rgb8();
                    // Every face model for this photo in one turn.
                    let _gpu = gpu_gate::GATE.local("faces");
                    let found = engine
                        .detect_rgb(&rgb)
                        .map_err(|e| format!("faces {photo_id}: {e:#}"))?;
                    // Embed each face for people grouping when the embedder is
                    // present; a failed embed stores NULL for that face. It is
                    // counted and reported, never swallowed: a pass whose every
                    // embed failed once finished looking like a success, with
                    // an empty People page and nothing to say why.
                    let embeddings: Vec<Vec<f32>> =
                        if recognise && engine.can_embed() && !found.is_empty() {
                            let marks: Vec<[[f32; 2]; 5]> =
                                found.iter().map(|f| f.landmarks).collect();
                            match engine.embed_all_rgb(&rgb, &marks) {
                                Ok(v) => {
                                    summary.embedded += count(v.len());
                                    v
                                }
                                Err(e) => {
                                    eprintln!("faces: cannot embed for {photo_id}: {e:#}");
                                    summary.embed_failed += count(found.len());
                                    summary.first_error.get_or_insert_with(|| err(e));
                                    Vec::new()
                                }
                            }
                        } else {
                            Vec::new()
                        };
                    summary.faces += count(found.len());
                    // The size of the image the detector just measured against
                    // — the 1600px preview, not the capture — because a box
                    // without it can only be placed by guessing which
                    // rendition it belongs to.
                    let size = rgb.dimensions();
                    Ok((found, embeddings, size))
                });
            match result {
                Ok((found, embeddings, size)) => {
                    let mut conn = db::connect(&db_path).map_err(err)?;
                    if let Err(e) = db::store_faces(&mut conn, photo_id, &found, &embeddings, size)
                    {
                        eprintln!("faces: cannot store for {photo_id}: {e}");
                    }
                }
                // One odd file (or the pending decode) must not abort the run.
                Err(e) => eprintln!("faces: {e}"),
            }
            done += 1;
            let _ = app.emit(
                "faces-progress",
                ScanProgress {
                    phase: "faces".into(),
                    current: count(done),
                    total: count(total),
                    message: "Analysing faces".into(),
                },
            );
        }

        // Then the eyes of faces found before the face mesh, from the
        // boxes already stored: no re-detection, so who anyone is stays put.
        if engine.can_read_eyes() {
            for (photo_id, image_path, faces) in eyes_queue {
                wait_while_developing(&state, &cancel);
                if cancel.is_cancelled() {
                    break;
                }
                let boxes: Vec<[f32; 4]> = faces.iter().map(|(_, b)| *b).collect();
                match std::fs::read(&image_path)
                    .map_err(anyhow::Error::from)
                    .and_then(|bytes| engine.read_eyes(&bytes, &boxes))
                {
                    Ok(eyes) => {
                        let readings: Vec<(i64, Option<f32>)> =
                            faces.iter().map(|(id, _)| *id).zip(eyes).collect();
                        let mut conn = db::connect(&db_path).map_err(err)?;
                        if let Err(e) = db::store_eye_readings(&mut conn, photo_id, &readings) {
                            eprintln!("faces: cannot store eyes for {photo_id}: {e:#}");
                        }
                    }
                    Err(e) => eprintln!("faces: cannot read eyes for {photo_id}: {e:#}"),
                }
                done += 1;
                let _ = app.emit(
                    "faces-progress",
                    ScanProgress {
                        phase: "faces".into(),
                        current: count(done),
                        total: count(total),
                        message: "Reading eyes".into(),
                    },
                );
            }
        }
        Ok(())
    })?;
    // The face models go with the pass: only this pass uses them, and kept
    // they held about half a gigabyte (AuraFace's weights and its working
    // memory, measured on the CPU) for the rest of the session, on a PC
    // that may have 8 GB. The next pass loads them again, in half a second
    // on the CPU and a second or two on DirectML.
    *lock_recover(&state.face_engine) = None;

    let stopped = cancel.is_cancelled();
    let _ = app.emit(
        "faces-progress",
        ScanProgress {
            phase: "done".into(),
            current: count(done),
            total: count(total),
            message: if stopped { "Stopped" } else { "Faces analysed" }.into(),
        },
    );
    summary.photos = count(done);
    summary.stopped = stopped;
    Ok(summary)
}

/// Clusters every embedded face into people, rebuilding the `people` table.
/// Returns how many people were found. Run after [`analyze_faces`] has embedded
/// faces (an `--features ai` build with the AuraFace embedder downloaded).
///
/// Cancellable as `people`, with `people-progress` for the Activity
/// panel: clustering is quadratic in the number of faces, so a big library
/// takes long enough to want stopping. A stopped regroup writes nothing and
/// returns [`jobs::CANCELLED`].
///
/// After a face pass (`full` false) the new faces join the people already
/// there when they can ([`db::place_new_faces`]); every face is
/// clustered again only when asked, or when there are too many new ones.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn group_people(
    app: AppHandle,
    state: State<'_, AppState>,
    full: bool,
) -> Result<u32, String> {
    let _one_at_a_time = lock_recover(&state.people_work);
    let _no_edits = lock_recover(&state.people_edit);
    let mut conn = db::connect(&state.db_path).map_err(err)?;
    // Nothing to group until People is on.
    if !db::people_enabled(&conn).map_err(err)? {
        return Ok(0);
    }
    if !full {
        if let Some(n) = db::place_new_faces(&mut conn).map_err(err)? {
            let _ = app.emit(
                "people-progress",
                ScanProgress {
                    phase: "done".into(),
                    current: 0,
                    total: 0,
                    message: "People grouped".into(),
                },
            );
            return Ok(n as u32);
        }
    }
    let cancel = state.jobs.start("people");
    let emit = |phase: &str, current: usize, total: usize, message: &str| {
        let _ = app.emit(
            "people-progress",
            ScanProgress {
                phase: phase.into(),
                current: count(current),
                total: count(total),
                message: message.into(),
            },
        );
    };
    // A face at a time would be thousands of events; a percent at a time is
    // all a progress bar can show.
    let mut last = usize::MAX;
    let result =
        db::regroup_people_with(&mut conn, &|| cancel.is_cancelled(), &mut |done, total| {
            let step = (done * 100) / total.max(1);
            if step != last {
                last = step;
                emit("grouping", done, total, "Grouping people");
            }
        });
    emit(
        "done",
        0,
        0,
        if cancel.is_cancelled() {
            "Stopped"
        } else {
            "People grouped"
        },
    );
    match result.map_err(err)? {
        Some(n) => Ok(n as u32),
        None => Err(jobs::CANCELLED.into()),
    }
}

/// The clustered people for the People workspace, most-photographed first,
/// each with a portrait of its cover face. Async: the first visit after a
/// regroup cuts a portrait per person.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn list_people(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<models::Person>, String> {
    let conn = state.conn()?;
    let mut people = db::list_people(&conn).map_err(err)?;
    let sources: std::collections::HashMap<i32, (String, [f32; 4])> = db::face_sources(&conn, None)
        .map_err(err)?
        .into_iter()
        .map(|(id, path, bx)| (id, (path, bx)))
        .collect();
    let mut missing = Vec::new();
    for p in &mut people {
        if let Some((path, bx)) = sources.get(&p.id) {
            p.portrait_path = cut_portrait(&state.cache_dir, path, *bx, &mut missing);
        }
    }
    fill_portraits(&app, &state, missing);
    Ok(people)
}

/// A face's portrait if it has been cut already; otherwise `None`, with the
/// face added to `missing` for [`fill_portraits`]. The page crops the
/// thumbnail meanwhile.
fn cut_portrait(
    cache_dir: &Path,
    preview: &str,
    face: [f32; 4],
    missing: &mut Vec<(String, [f32; 4])>,
) -> Option<String> {
    let out = portrait_file(cache_dir, preview, face)?;
    if out.exists() {
        Some(out.to_string_lossy().into_owned())
    } else {
        missing.push((preview.to_string(), face));
        None
    }
}

/// Cuts the portraits a list came back without, on another thread, a
/// preview per core at a time, then says so with `people-portraits` so the
/// page asks again. Cutting them before answering kept People empty
/// for minutes after a big face pass: a 1600 px preview decoded per person,
/// one after another. One fill at a time; a list that asks meanwhile gets
/// what is missing when the page asks again.
fn fill_portraits(app: &AppHandle, state: &AppState, missing: Vec<(String, [f32; 4])>) {
    if missing.is_empty() || state.portraits_filling.swap(true, Ordering::SeqCst) {
        return;
    }
    let mut by_preview: std::collections::HashMap<String, Vec<[f32; 4]>> =
        std::collections::HashMap::new();
    for (preview, face) in missing {
        by_preview.entry(preview).or_default().push(face);
    }
    let (app, cache_dir) = (app.clone(), state.cache_dir.clone());
    std::thread::spawn(move || {
        use rayon::prelude::*;
        let cut = by_preview
            .par_iter()
            .filter(|(preview, faces)| {
                face_portraits(&cache_dir, preview, faces)
                    .iter()
                    .any(Option::is_some)
            })
            .count();
        app.state::<AppState>()
            .portraits_filling
            .store(false, Ordering::SeqCst);
        // Only when something was cut: a preview that will not decode would
        // otherwise have the page ask, and this cut, again for ever.
        if cut > 0 {
            let _ = app.emit("people-portraits", ());
        }
    });
}

/// The cached portrait for a face box in a preview, cutting it on first use.
/// Named from the preview's own content-addressed name and the box, so it is
/// never stale (the image server marks these immutable) and a face row that
/// is re-detected in the same place reuses it. `None` if it cannot be made;
/// the page then crops the thumbnail as before.
fn face_portrait(cache_dir: &Path, preview: &str, face: [f32; 4]) -> Option<String> {
    face_portraits(cache_dir, preview, &[face]).pop().flatten()
}

/// Where a face's portrait is cached: named from the preview's own
/// content-addressed name and the box.
fn portrait_file(cache_dir: &Path, preview: &str, face: [f32; 4]) -> Option<PathBuf> {
    let stem = Path::new(preview)
        .file_stem()?
        .to_string_lossy()
        .into_owned();
    let [x, y, w, h] = face.map(|v| v.round() as i64);
    Some(
        cache_dir
            .join("faces")
            .join(format!("{stem}_{x}_{y}_{w}_{h}.jpg")),
    )
}

/// [`face_portrait`] for several faces in one preview, decoding it at most once
/// — a group shot in a burst is a handful of faces per frame.
fn face_portraits(cache_dir: &Path, preview: &str, faces: &[[f32; 4]]) -> Vec<Option<String>> {
    let dir = cache_dir.join("faces");
    let outs: Vec<Option<PathBuf>> = faces
        .iter()
        .map(|face| portrait_file(cache_dir, preview, *face))
        .collect();
    let mut decoded: Option<Option<image::RgbImage>> = None;
    outs.iter()
        .zip(faces)
        .map(|(out, face)| {
            let out = out.as_ref()?;
            if !out.exists() {
                let img = decoded
                    .get_or_insert_with(|| image::open(preview).ok().map(|i| i.to_rgb8()))
                    .as_ref()?;
                let portrait = faces::portrait(img, *face);
                std::fs::create_dir_all(&dir).ok()?;
                let file = std::fs::File::create(out).ok()?;
                let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(
                    std::io::BufWriter::new(file),
                    90,
                );
                if enc.encode_image(&portrait).is_err() {
                    let _ = std::fs::remove_file(out);
                    return None;
                }
            }
            Some(out.to_string_lossy().into_owned())
        })
        .collect()
}

/// The detected faces in each photo, largest first, each with its portrait —
/// Cull's close-ups and Compare's faces across a burst.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn photo_faces(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
) -> Result<Vec<db::PhotoFace>, String> {
    let conn = state.conn()?;
    let rows = db::photo_faces(&conn, &photo_ids).map_err(err)?;
    drop(conn);
    // Grouped by preview, so each is decoded once however many faces it holds.
    let mut out: Vec<db::PhotoFace> = Vec::with_capacity(rows.len());
    let mut i = 0;
    while i < rows.len() {
        let preview = rows[i].1.clone();
        let end = i + rows[i..].iter().take_while(|r| r.1 == preview).count();
        let portraits = match &preview {
            Some(p) => face_portraits(
                &state.cache_dir,
                p,
                &rows[i..end].iter().map(|r| r.2).collect::<Vec<_>>(),
            ),
            None => vec![None; end - i],
        };
        for (row, portrait) in rows[i..end].iter().zip(portraits) {
            out.push(db::PhotoFace {
                portrait_path: portrait,
                ..row.0.clone()
            });
        }
        i = end;
    }
    Ok(out)
}

/// Says who a detected face is, from the Lightbox — see
/// [`db::name_face`].
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn name_face(
    state: State<'_, AppState>,
    face_id: i32,
    name: String,
) -> Result<(), String> {
    let _no_regroup = lock_recover(&state.people_edit);
    let mut conn = state.conn()?;
    db::name_face(&mut conn, face_id, &name)
        .map(|_| ())
        .map_err(err)
}

/// Rename a person (blank clears it back to unnamed).
#[tauri::command]
#[specta::specta]
pub(crate) fn set_person_name(
    state: State<'_, AppState>,
    id: i32,
    name: String,
) -> Result<(), String> {
    let conn = state.conn()?;
    db::set_person_name(&conn, id, &name).map_err(err)
}

/// Detects every photo's faces again, keeping merges, names and the faces
/// added by hand (see [`db::reset_faces`]), after a backup. Returns the
/// number of photos re-queued.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn reset_faces(state: State<'_, AppState>) -> Result<u32, String> {
    let conn = state.conn()?;
    backup::create(&conn, &state.db_path, "auto-before-redetect").map_err(|e| {
        format!("couldn't back up the catalogue first, so nothing was changed: {e:#}")
    })?;
    let _ = backup::prune(&state.db_path);
    db::reset_faces(&conn).map(count).map_err(err)
}

/// Whether People is on: recognising who is in photos.
#[tauri::command]
#[specta::specta]
pub(crate) fn people_enabled(state: State<'_, AppState>) -> Result<bool, String> {
    let conn = state.conn()?;
    db::people_enabled(&conn).map_err(err)
}

/// Turns People on, the user's yes to recognising faces, or off,
/// keeping what was built.
#[tauri::command]
#[specta::specta]
pub(crate) fn set_people_enabled(state: State<'_, AppState>, on: bool) -> Result<(), String> {
    let conn = state.conn()?;
    db::set_people_enabled(&conn, on).map_err(err)
}

/// What erasing face data did to the backups.
#[derive(serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FaceDataErased {
    /// Automatic backups the face data was erased from.
    scrubbed: u32,
    /// Backups the user made, and damaged catalogues set aside: untouched,
    /// and still holding face data.
    kept: u32,
}

/// Turns People off and erases everything it built: every face
/// template, person, name and merge, the faces added by hand, the portraits
/// cut from previews, and the same in every automatic backup. Detected faces
/// stay, without templates, for culling.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn erase_face_data(state: State<'_, AppState>) -> Result<FaceDataErased, String> {
    let _one_at_a_time = lock_recover(&state.people_work);
    let _no_edits = lock_recover(&state.people_edit);
    let mut conn = state.conn()?;
    db::erase_face_data(&mut conn).map_err(err)?;
    let _ = std::fs::remove_dir_all(state.cache_dir.join("faces"));
    *lock_recover(&state.people_suggestions) = None;
    let (scrubbed, kept) = backup::scrub_face_data(&state.db_path).map_err(err)?;
    Ok(FaceDataErased {
        scrubbed: count(scrubbed),
        kept: count(kept),
    })
}

/// The faces attributed to a person, for the "is this really them?" grid,
/// each with its portrait.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn person_faces(
    app: AppHandle,
    state: State<'_, AppState>,
    person_id: i32,
) -> Result<Vec<db::PersonFace>, String> {
    let conn = state.conn()?;
    let mut faces = db::person_faces(&conn, person_id).map_err(err)?;
    let sources: std::collections::HashMap<i32, (String, [f32; 4])> =
        db::face_sources(&conn, Some(person_id))
            .map_err(err)?
            .into_iter()
            .map(|(id, path, bx)| (id, (path, bx)))
            .collect();
    let mut missing = Vec::new();
    for f in &mut faces {
        if let Some((path, bx)) = sources.get(&f.face_id) {
            f.portrait_path = cut_portrait(&state.cache_dir, path, *bx, &mut missing);
        }
    }
    fill_portraits(&app, &state, missing);
    Ok(faces)
}

/// Records that several clusters are one person and regroups. The threshold
/// deliberately leans toward splitting a person in two rather than merging two
/// strangers, on the grounds that this is the repair — so it has to exist.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn merge_people(state: State<'_, AppState>, ids: Vec<i32>) -> Result<u32, String> {
    let _no_regroup = lock_recover(&state.people_edit);
    let mut conn = state.conn()?;
    let (n, undo) = db::merge_people_undoably(&mut conn, &ids).map_err(err)?;
    *lock_recover(&state.last_merge) = undo;
    Ok(count(n))
}

/// Takes back the last merge. False when there is none to take back.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn undo_merge(state: State<'_, AppState>) -> Result<bool, String> {
    let _no_regroup = lock_recover(&state.people_edit);
    let Some(undo) = lock_recover(&state.last_merge).take() else {
        return Ok(false);
    };
    let mut conn = state.conn()?;
    db::unmerge_people(&mut conn, &undo).map_err(err)?;
    Ok(true)
}

/// Two people who may be one person split in two, and how alike they are.
#[derive(serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PeopleSuggestion {
    a: i32,
    b: i32,
    /// Average similarity between their faces, 0–1.
    score: f32,
}

/// People the grouping kept apart but who may be the same person, best first,
/// for the user to merge or tell apart. At most a handful at a time.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn people_suggestions(
    state: State<'_, AppState>,
) -> Result<Vec<PeopleSuggestion>, String> {
    let conn = state.conn()?;
    // Worked out again only when the people changed: every People
    // load and every action asked, and most changed nothing they depend on.
    let mark = db::people_fingerprint(&conn).map_err(err)?;
    let cached = lock_recover(&state.people_suggestions)
        .as_ref()
        .filter(|(m, _)| *m == mark)
        .map(|(_, found)| found.clone());
    let found = match cached {
        Some(found) => found,
        None => {
            let found = db::people_suggestions(&conn, 5).map_err(err)?;
            *lock_recover(&state.people_suggestions) = Some((mark, found.clone()));
            found
        }
    };
    Ok(found
        .into_iter()
        .map(|(a, b, score)| PeopleSuggestion {
            a: a as i32,
            b: b as i32,
            score,
        })
        .collect())
}

/// The user's "not the same person": never suggest these two again.
#[tauri::command]
#[specta::specta]
pub(crate) fn keep_people_apart(state: State<'_, AppState>, a: i32, b: i32) -> Result<(), String> {
    let conn = state.conn()?;
    db::keep_people_apart(&conn, a, b).map_err(err)
}

/// Pulls one face out of the person it was grouped into, and keeps it out.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn detach_face(state: State<'_, AppState>, face_id: i32) -> Result<u32, String> {
    let _no_regroup = lock_recover(&state.people_edit);
    let mut conn = state.conn()?;
    db::detach_face(&mut conn, face_id).map(count).map_err(err)
}

/// Makes a face the cover of its person, kept across regroups.
#[tauri::command]
#[specta::specta]
pub(crate) fn set_cover_face(state: State<'_, AppState>, face_id: i32) -> Result<(), String> {
    let mut conn = state.conn()?;
    db::set_cover_face(&mut conn, face_id).map_err(err)
}

/// Faces grouping could not place, largest first, each with its portrait,
/// and how many there are in all.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn unplaced_faces(
    state: State<'_, AppState>,
) -> Result<(Vec<db::PersonFace>, u32), String> {
    let conn = state.conn()?;
    let mut faces = db::unplaced_faces(&conn, 120).map_err(err)?;
    let ids: Vec<i32> = faces.iter().map(|f| f.face_id).collect();
    let sources: std::collections::HashMap<i32, (String, [f32; 4])> =
        db::face_sources_of(&conn, &ids)
            .map_err(err)?
            .into_iter()
            .map(|(id, path, bx)| (id, (path, bx)))
            .collect();
    for f in &mut faces {
        if let Some((path, bx)) = sources.get(&f.face_id) {
            f.portrait_path = face_portrait(&state.cache_dir, path, *bx);
        }
    }
    let total = db::unplaced_face_count(&conn).map_err(err)?;
    Ok((faces, total))
}

/// Says a detection is not a face; it leaves the unplaced queue.
#[tauri::command]
#[specta::specta]
pub(crate) fn dismiss_face(state: State<'_, AppState>, face_id: i32) -> Result<(), String> {
    let conn = state.conn()?;
    db::dismiss_face(&conn, face_id).map_err(err)
}

/// Hides people the user does not care about, or shows them again.
#[tauri::command]
#[specta::specta]
pub(crate) fn set_people_hidden(
    state: State<'_, AppState>,
    ids: Vec<i32>,
    hidden: bool,
) -> Result<(), String> {
    let conn = state.conn()?;
    db::set_people_hidden(&conn, &ids, hidden).map_err(err)
}

/// Puts a person in these photos by hand, without asking the detector.
/// Returns how many photos they were added to (those they were already in
/// are skipped).
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn add_photos_to_person(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
    person_id: i32,
) -> Result<u32, String> {
    let mut conn = state.conn()?;
    db::add_photos_to_person(&mut conn, &photo_ids, person_id)
        .map(count)
        .map_err(err)
}

/// A new person, in these photos by hand: someone the detector never found.
/// Returns the new person's id.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn add_photos_to_new_person(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
    name: String,
) -> Result<i32, String> {
    let mut conn = state.conn()?;
    db::add_photos_to_new_person(&mut conn, &photo_ids, &name).map_err(err)
}

/// Takes a person out of these photos.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn remove_photos_from_person(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
    person_id: i32,
) -> Result<(), String> {
    let _no_regroup = lock_recover(&state.people_edit);
    let mut conn = state.conn()?;
    db::remove_photos_from_person(&mut conn, &photo_ids, person_id).map_err(err)
}

/// Who is in a photo, for the selection panel.
#[tauri::command]
#[specta::specta]
pub(crate) fn photo_people(
    state: State<'_, AppState>,
    photo_id: i32,
) -> Result<Vec<db::PhotoPerson>, String> {
    let conn = state.conn()?;
    db::photo_people(&conn, photo_id).map_err(err)
}

/// Overrules the eye-open model on these photos: `None` defers to it again,
/// `Some(0)` says the eyes are not closed, `Some(1)` confirms they are.
#[tauri::command]
#[specta::specta]
pub(crate) fn set_eyes_override(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
    value: Option<i32>,
) -> Result<u32, String> {
    let conn = state.conn()?;
    db::set_eyes_override(&conn, &photo_ids, value)
        .map(count)
        .map_err(err)
}
