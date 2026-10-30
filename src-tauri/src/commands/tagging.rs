//! Tagging and captioning photos with the vision model, and translating
//! tags.

use std::path::Path;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, State};

use super::ai::{ai_config, warm_engine, UnloadWhenDone};
use super::common::{count, err};
use crate::models::{ScanProgress, TagResult};
use crate::{backup, db, search, tagger, AppState};

/// Re-queues every photo for tagging — needed after the prompt or model changes,
/// since otherwise improvements only ever reach newly imported photos.
// Async: it backs the whole catalogue up first, seconds at scale, and
// on the main thread the window waited for it.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn reset_tagging(state: State<'_, AppState>) -> Result<u32, String> {
    let conn = state.conn()?;
    // Backed up first. The re-tag run this queues replaces every photo's
    // AI tags as it reaches it, and cannot be undone photo by photo — so this is
    // the moment worth a copy, restorable from the catalogue backups list. Named
    // `auto-…` so it rotates with the other automatic backups rather than
    // accumulating. Unlike the copy before a migration, a failure here stops the
    // reset: nothing is lost by trying again, and a lot could be by going ahead.
    backup::create(&conn, &state.db_path, "auto-before-retag").map_err(|e| {
        format!("Could not back up the catalogue first, so nothing was changed: {e:#}")
    })?;
    let _ = backup::prune(&state.db_path);
    db::reset_tagging(&conn).map(count).map_err(err)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn cancel_tagging(state: State<'_, AppState>) {
    state.cancel_tagging.store(true, Ordering::Relaxed);
}

/// Runs `work` over `items` with at most `limit` jobs in flight, handing each
/// result to `on_done` — on this task, in completion order — as it lands.
///
/// This is what lets a batch keep several photos at a local model at once: a
/// server like `llama-server` runs parallel requests together for far more throughput
/// than one at a time, and even at a limit of 1 the next photo is read while
/// the model works. No new job starts once `cancelled()` is true or `on_done`
/// returns `false`; jobs already running finish and are still reported, so no
/// completed work is thrown away.
async fn run_bounded<I, T, Fut>(
    items: Vec<I>,
    limit: usize,
    cancelled: impl Fn() -> bool,
    work: impl Fn(I) -> Fut,
    mut on_done: impl FnMut(T) -> bool,
) where
    T: Send + 'static,
    Fut: std::future::Future<Output = T> + Send + 'static,
{
    let mut jobs = tokio::task::JoinSet::new();
    let mut items = items.into_iter();
    let mut stopping = false;
    loop {
        while !stopping && jobs.len() < limit.max(1) {
            if cancelled() {
                stopping = true;
                break;
            }
            match items.next() {
                Some(item) => {
                    jobs.spawn(work(item));
                }
                None => break,
            }
        }
        match jobs.join_next().await {
            Some(Ok(result)) => {
                if !on_done(result) {
                    stopping = true;
                }
            }
            Some(Err(e)) => eprintln!("batch: a job did not finish: {e}"),
            None => break,
        }
    }
}

/// What tagging one photo came back with.
enum TagOutcome {
    /// The preview file could not be read; not the model's fault.
    Unreadable(String),
    /// Tags, plus a search embedding when one was freshly computed.
    Tagged(TagResult, Option<Vec<f32>>),
    Failed(String),
}

/// Tags one photo. With `use_stored`, a photo the search index already
/// embedded is tagged straight from that vector — no file read, no image
/// model — which makes re-tagging an indexed library near-instant.
async fn tag_one(
    engine: &dyn tagger::Tagger,
    db_path: &Path,
    photo_id: i32,
    image_path: &str,
    use_stored: bool,
) -> TagOutcome {
    if use_stored {
        let stored = db::connect(db_path)
            .ok()
            .and_then(|c| db::get_embedding(&c, photo_id).ok().flatten())
            .and_then(|b| search::decode_embedding(&b));
        if let Some(result) = stored.and_then(|v| engine.tag_from_embedding(&v)) {
            return TagOutcome::Tagged(result, None);
        }
    }
    let bytes = match tokio::fs::read(image_path).await {
        Ok(b) => b,
        Err(e) => return TagOutcome::Unreadable(format!("cannot read {image_path}: {e}")),
    };
    match engine.tag_and_embed(&bytes).await {
        Ok((result, embedding)) => TagOutcome::Tagged(result, embedding),
        Err(e) => TagOutcome::Failed(format!("{e:#}")),
    }
}

/// The tag names kept in `language` (a code such as "de"): English name → the
/// name the interface shows. The ones a fixed table covers are not here — see
/// [`translate_tags`].
#[tauri::command]
#[specta::specta]
pub(crate) fn tag_translations(
    state: State<'_, AppState>,
    language: String,
) -> Result<std::collections::HashMap<String, String>, String> {
    db::tag_translations(&state.conn()?, &language).map_err(err)
}

/// Names tags in `language` with the local vision model, and keeps the
/// answers: the tags it wrote freely, which the interface's fixed tables cannot
/// name (docs/I18N.md). Only `names` not already kept are asked, forty to a
/// request. Returns what is now kept for `names`.
///
/// Opportunistic, never an error for want of a model: with no vision model set
/// up, or none running, it answers with what is already kept, and the
/// interface shows the rest in English. The interface asks after a tagging or
/// describing run, when the model is loaded, and on a change of language.
#[tauri::command]
#[specta::specta]
pub(crate) async fn translate_tags(
    state: State<'_, AppState>,
    language: String,
    names: Vec<String>,
) -> Result<std::collections::HashMap<String, String>, String> {
    let kept = |state: &AppState| -> Result<std::collections::HashMap<String, String>, String> {
        let all = db::tag_translations(&state.conn()?, &language).map_err(err)?;
        Ok(names
            .iter()
            .filter_map(|n| all.get(n).map(|t| (n.clone(), t.clone())))
            .collect())
    };
    let Some(language_name) = tagger::language_name(&language) else {
        return Ok(std::collections::HashMap::new());
    };
    let known = kept(&state)?;
    let mut missing: Vec<String> = names
        .iter()
        .filter(|n| !known.contains_key(*n) && !n.trim().is_empty())
        .cloned()
        .collect();
    missing.sort();
    missing.dedup();
    let cfg = ai_config(&state)?;
    if missing.is_empty() || cfg.provider != tagger::Provider::Vision {
        return Ok(known);
    }
    let Ok(engine) = tagger::build(&cfg) else {
        return Ok(known);
    };
    if engine.check().await.is_err() {
        return Ok(known);
    }
    for chunk in missing.chunks(40) {
        match engine.translate_terms(chunk, language_name).await {
            Ok(answers) => {
                let pairs: Vec<(String, String)> = chunk
                    .iter()
                    .zip(answers)
                    .filter_map(|(name, answer)| answer.map(|a| (name.clone(), a)))
                    .collect();
                let mut conn = state.conn()?;
                db::set_tag_translations(&mut conn, &language, &pairs).map_err(err)?;
            }
            Err(e) => {
                // One bad reply should not cost the rest; a dead model will
                // fail the next chunk as quickly.
                eprintln!("translate tags: {e:#}");
            }
        }
    }
    kept(&state)
}

/// `language` is the interface's (a code such as "de"): descriptions written
/// while tagging are in it. Tags are English whatever it is — see
/// `tagger::tag_and_caption_prompt`.
#[tauri::command]
#[specta::specta]
pub(crate) async fn start_tagging(
    app: AppHandle,
    state: State<'_, AppState>,
    language: Option<String>,
) -> Result<u32, String> {
    state.cancel_tagging.store(false, Ordering::Relaxed);

    let db_path = state.db_path.clone();
    let cfg = tagger::ProviderConfig {
        caption_language: language.as_deref().and_then(tagger::language_name),
        ..ai_config(&state)?
    };
    // Claim the model, so a deferred unload doesn't free it out from under
    // this run, and free it after the idle time however the run ends.
    let _unload = UnloadWhenDone::claim(&app, &cfg);
    // Fail up front with a real message rather than looping over every photo
    // and logging the same error N times.
    let engine = tagger::build(&cfg).map_err(err)?;

    // Snapshot the queue up front so progress has a stable denominator.
    let queue = {
        let conn = db::connect(&db_path).map_err(err)?;
        db::untagged_photos(&conn, 100_000).map_err(err)?
    };
    // The vision model tags from a small image (0.3 s a photo on a discrete
    // GPU, 2.5 s on an M4), so tagging stays tags only: the filters fill
    // sooner, and descriptions, which need the full-size image, follow in
    // their own pass (which tags any photo still untagged as it describes).
    let total = queue.len();
    eprintln!(
        "tagging: {total} photos with {:?} {}",
        cfg.provider, cfg.model
    );
    warm_engine(&app, "tag-progress", &cfg, engine.as_ref(), total).await;
    let engine: std::sync::Arc<dyn tagger::Tagger> = engine.into();
    // Only the built-in tagger speaks CLIP embeddings, so only it can reuse the
    // one the search index already stored instead of looking at the image.
    let use_stored = cfg.provider == tagger::Provider::Builtin;
    let mut done = 0usize;
    // A revoked key or a wrong model id fails on every photo. Without this, a
    // 5000-photo batch would fire 5000 doomed (and, on cloud providers,
    // billable) requests before anyone noticed.
    let mut consecutive_failures = 0usize;
    const MAX_CONSECUTIVE_FAILURES: usize = 5;
    let mut aborted: Option<String> = None;
    let mut indexed_any = false;

    run_bounded(
        queue,
        engine.max_concurrency(),
        || state.cancel_tagging.load(Ordering::Relaxed),
        |(photo_id, image_path)| {
            let engine = engine.clone();
            let db_path = db_path.clone();
            async move {
                let outcome =
                    tag_one(engine.as_ref(), &db_path, photo_id, &image_path, use_stored).await;
                (photo_id, outcome)
            }
        },
        |(photo_id, outcome)| {
            match outcome {
                TagOutcome::Unreadable(e) => eprintln!("tag: {e}"),
                TagOutcome::Tagged(result, embedding) => {
                    consecutive_failures = 0;
                    match db::connect(&db_path) {
                        Ok(mut conn) => {
                            if let Err(e) = db::apply_ai_tags(&mut conn, photo_id, &result) {
                                eprintln!("tag: cannot store tags for {photo_id}: {e}");
                            }
                            // Tagging computed the search embedding anyway, so
                            // keep it — the index pass then skips this photo.
                            if let Some(v) = embedding {
                                let blob = search::encode_embedding(&v);
                                match db::set_embedding(&conn, photo_id, &blob) {
                                    Ok(()) => indexed_any = true,
                                    Err(e) => {
                                        eprintln!("tag: cannot store embedding for {photo_id}: {e}")
                                    }
                                }
                            }
                        }
                        Err(e) => eprintln!("tag: cannot open the catalogue: {e}"),
                    }
                }
                TagOutcome::Failed(e) => {
                    eprintln!("tag: model failed on {photo_id}: {e}");
                    consecutive_failures += 1;
                    if consecutive_failures >= MAX_CONSECUTIVE_FAILURES && aborted.is_none() {
                        aborted = Some(format!(
                            "Stopped after {consecutive_failures} consecutive failures: {e}"
                        ));
                    }
                }
            }

            done += 1;
            let _ = app.emit(
                "tag-progress",
                ScanProgress {
                    phase: "tag".into(),
                    current: count(done),
                    total: count(total),
                    message: "Tagging".into(),
                },
            );
            aborted.is_none()
        },
    )
    .await;
    eprintln!("tagging: ended after {done} of {total}");

    // Embeddings change what a search resolves to.
    if indexed_any {
        state.bump_search_gen();
    }

    // The model's VRAM is freed once the machine has been idle a while (the
    // vision model; a no-op otherwise) rather than the instant this batch ends:
    // `_unload`, when this returns.
    drop(engine);

    // Tagging is what fills `photo_tags` — from nothing to ~9 links a photo — so
    // the planner's statistics for the tag filters are stale the moment it ends.
    if done > 0 {
        if let Err(e) = db::connect(&db_path).and_then(|c| db::refresh_planner_stats(&c)) {
            eprintln!("could not refresh query-planner statistics after tagging: {e:#}");
        }
    }

    let _ = app.emit(
        "tag-progress",
        ScanProgress {
            phase: "done".into(),
            current: count(done),
            total: count(total),
            message: aborted.clone().unwrap_or_else(|| "Finished".into()),
        },
    );

    // Surface a repeated failure as a real error so the UI can show it, rather
    // than reporting a "successful" run that tagged nothing.
    match aborted {
        Some(msg) => Err(msg),
        None => Ok(count(done)),
    }
}

/// How many photos still need a caption — the "N to caption" status line.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn caption_status(state: State<'_, AppState>) -> Result<u32, String> {
    let conn = state.conn()?;
    db::uncaptioned_count(&conn)
        .map(|n| n.max(0) as u32)
        .map_err(err)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn cancel_captioning(state: State<'_, AppState>) {
    state.cancel_captioning.store(true, Ordering::Relaxed);
}

/// Captions every uncaptioned photo with the configured VLM, storing the text
/// (which the migration-6 triggers index for search). Background/batch by nature
/// — VLM inference is seconds per image — so it reports through the same
/// progress-event path as tagging, under `caption-progress`. Mirrors
/// `start_tagging`; only the per-photo call and the store differ. A photo not
/// yet tagged is tagged in the same request, as tagging describes a photo not
/// yet described, so neither pass sends a photo the other will send again.
/// `limit` caps the batch (the rest stays queued); `None` is every photo.
#[tauri::command]
#[specta::specta]
pub(crate) async fn start_captioning(
    app: AppHandle,
    state: State<'_, AppState>,
    limit: Option<u32>,
    // The interface's language (a code such as "de"): descriptions are written in it.
    language: Option<String>,
) -> Result<u32, String> {
    state.cancel_captioning.store(false, Ordering::Relaxed);

    let db_path = state.db_path.clone();
    let cfg = tagger::ProviderConfig {
        caption_language: language.as_deref().and_then(tagger::language_name),
        ..ai_config(&state)?
    };
    // Claimed, and freed after the idle time however this ends.
    let _unload = UnloadWhenDone::claim(&app, &cfg);
    let engine = tagger::build(&cfg).map_err(err)?;

    let (queue, untagged) = {
        let conn = db::connect(&db_path).map_err(err)?;
        let untagged: std::collections::HashSet<i32> = db::untagged_photos(&conn, 100_000)
            .map_err(err)?
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        // A limit is automatic analysis asking for a batch at a time, so new
        // photos' quicker passes are not stuck behind a library's worth.
        let limit = limit.map_or(100_000, |n| n.clamp(1, 100_000) as i32);
        (db::uncaptioned_photos(&conn, limit).map_err(err)?, untagged)
    };
    let total = queue.len();
    eprintln!(
        "describing: {total} photos with {:?} {}",
        cfg.provider, cfg.model
    );
    warm_engine(&app, "caption-progress", &cfg, engine.as_ref(), total).await;
    let engine: std::sync::Arc<dyn tagger::Tagger> = engine.into();
    let mut done = 0usize;
    let mut tagged_any = false;
    let mut consecutive_failures = 0usize;
    const MAX_CONSECUTIVE_FAILURES: usize = 5;
    let mut aborted: Option<String> = None;

    run_bounded(
        queue,
        engine.max_concurrency(),
        || state.cancel_captioning.load(Ordering::Relaxed),
        |(photo_id, image_path)| {
            let engine = engine.clone();
            // A photo with no tags yet is tagged in the same request: the
            // model reads the image once for both (`Tagger::tag_and_caption`).
            let tag_too = untagged.contains(&photo_id);
            async move {
                let outcome = match tokio::fs::read(&image_path).await {
                    Err(e) => Err(format!("cannot read {image_path}: {e}")),
                    Ok(bytes) if tag_too => Ok(engine
                        .tag_and_caption(&bytes)
                        .await
                        .map(|(tags, caption)| (caption, Some(tags)))
                        .map_err(err)),
                    Ok(bytes) => Ok(engine
                        .caption(&bytes)
                        .await
                        .map(|caption| (caption, None))
                        .map_err(err)),
                };
                (photo_id, outcome)
            }
        },
        |(photo_id, outcome)| {
            match outcome {
                Err(unreadable) => eprintln!("caption: {unreadable}"),
                Ok(Ok((caption, tags))) => {
                    consecutive_failures = 0;
                    match db::connect(&db_path) {
                        Ok(mut conn) => {
                            if let Err(e) = db::set_caption(&conn, photo_id, Some(&caption)) {
                                eprintln!("caption: cannot store caption for {photo_id}: {e}");
                            }
                            if let Some(tags) = tags {
                                match db::apply_ai_tags(&mut conn, photo_id, &tags) {
                                    Ok(()) => tagged_any = true,
                                    Err(e) => {
                                        eprintln!("caption: cannot store tags for {photo_id}: {e}")
                                    }
                                }
                            }
                        }
                        Err(e) => eprintln!("caption: cannot open the catalogue: {e}"),
                    }
                }
                Ok(Err(e)) => {
                    eprintln!("caption: model failed on {photo_id}: {e}");
                    consecutive_failures += 1;
                    if consecutive_failures >= MAX_CONSECUTIVE_FAILURES && aborted.is_none() {
                        aborted = Some(format!(
                            "Stopped after {consecutive_failures} consecutive failures: {e}"
                        ));
                    }
                }
            }

            done += 1;
            let _ = app.emit(
                "caption-progress",
                ScanProgress {
                    phase: "caption".into(),
                    current: count(done),
                    total: count(total),
                    message: "Captioning".into(),
                },
            );
            aborted.is_none()
        },
    )
    .await;
    eprintln!("describing: ended after {done} of {total}");

    drop(engine);

    // Captions feed the caption/OCR half of search, so invalidate any resolve
    // cached against the old text.
    state.bump_search_gen();
    // Tags written on the way leave the tag filters' planner statistics stale,
    // as a tagging pass does.
    if tagged_any {
        if let Err(e) = db::connect(&db_path).and_then(|c| db::refresh_planner_stats(&c)) {
            eprintln!("could not refresh query-planner statistics after describing: {e:#}");
        }
    }

    let _ = app.emit(
        "caption-progress",
        ScanProgress {
            phase: "done".into(),
            current: count(done),
            total: count(total),
            message: aborted.clone().unwrap_or_else(|| "Finished".into()),
        },
    );

    match aborted {
        Some(msg) => Err(msg),
        None => Ok(count(done)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    /// Keeps up to `limit` jobs running at once, never more, and delivers every
    /// result.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn run_bounded_overlaps_jobs_up_to_the_limit() {
        use std::sync::atomic::AtomicUsize;
        use std::sync::Arc;
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut seen = Vec::new();
        run_bounded(
            (0..12).collect(),
            3,
            || false,
            |i: usize| {
                let (running, peak) = (running.clone(), peak.clone());
                async move {
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    running.fetch_sub(1, Ordering::SeqCst);
                    i
                }
            },
            |i| {
                seen.push(i);
                true
            },
        )
        .await;
        seen.sort_unstable();
        assert_eq!(seen, (0..12).collect::<Vec<_>>());
        assert_eq!(peak.load(Ordering::SeqCst), 3, "ran three at a time");
    }

    /// Stopping (a cancel, or `on_done` saying so) starts nothing new, but
    /// every job already running is still reported.
    #[tokio::test]
    async fn run_bounded_stops_scheduling_but_reports_what_was_running() {
        let mut seen = 0;
        run_bounded(
            (0..100).collect(),
            4,
            || false,
            |i: usize| async move { i },
            |_| {
                seen += 1;
                seen < 5
            },
        )
        .await;
        assert!(
            (5..=8).contains(&seen),
            "stopped soon after the 5th, got {seen}"
        );

        let cancel = AtomicBool::new(true);
        let mut ran = 0;
        run_bounded(
            (0..10).collect(),
            2,
            || cancel.load(Ordering::Relaxed),
            |i: usize| async move { i },
            |_| {
                ran += 1;
                true
            },
        )
        .await;
        assert_eq!(ran, 0, "a batch cancelled before it starts runs nothing");
    }
}
