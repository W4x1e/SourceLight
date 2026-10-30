//! Browsing and search: the photos and facets in view, the timeline, and
//! search by content.

use rusqlite::Connection;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Emitter, State};

use super::common::{ai_models_dir, count, err, wait_while_developing};
use crate::models::{Facets, Photo, PhotoFilter, ScanProgress, SearchVocabulary, GENRES};
use crate::sync::lock_recover;
use crate::{db, gpu_gate, hardware, models, search, tagger, AppState, SearchCache};

/// Photos to consider for a natural-language search before facets and paging trim
/// it — the fusion is bounded so the candidate universe (and the facet counts
/// computed over it) stays cheap.
const SEMANTIC_CANDIDATES: usize = 1000;

/// Resolves a natural-language query to a ranked photo-id list, memoising the
/// most recent one. A single `reload` asks for the same query twice — once for
/// the grid, once for the facet counts — and each resolve runs the CLIP text
/// encoder, reloads every embedding (~100 MB at 50k), and fuses over the lot. The
/// second caller wants the identical answer, so it is served from the cache
/// instead. The entry is captured against [`AppState::search_gen`] and dropped
/// the moment a write bumps it, so a resolve never survives a change to the
/// photos, embeddings, or captions it was built from.
///
/// `within`, when given, is the filter whose ordinary facets the query ranks
/// inside: "Anna at the beach last June" ranked "at the beach" over the
/// whole library and cut to the top 1,000 before Anna and June applied, so a
/// large library returned a handful of photos or none. The grid and the
/// rail's counts rank the same way, so the totals agree with the grid.
pub(crate) fn fused_search_ids(
    app: &AppHandle,
    state: &AppState,
    query: &str,
    within: Option<&PhotoFilter>,
) -> Result<Vec<i32>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    // The facets go into the cache key with the words: the same words over
    // other facets are another answer.
    let key = match within {
        Some(f) => {
            let facets = PhotoFilter {
                semantic: None,
                ..f.clone()
            };
            format!("{q}\u{0}{}", serde_json::to_string(&facets).map_err(err)?)
        }
        None => q.to_string(),
    };

    // Snapshot the generation *before* computing, so a write landing mid-resolve
    // invalidates the entry we are about to store rather than being masked by it.
    let generation = state.search_gen.load(Ordering::Relaxed);
    // Held through the resolve, not just the lookup. `get_photos` and
    // `get_facets` arrive together and run off the main thread side by side,
    // so without it both would miss and both would run the text encoder; with
    // it the second waits for the first and reads its answer.
    let mut guard = lock_recover(&state.search_cache);
    if let Some(hit) = guard.as_ref() {
        if hit.generation == generation && hit.query == key {
            return Ok(hit.ids.clone());
        }
    }

    let scope = match within {
        Some(f) => {
            let conn = state.conn()?;
            Some(
                db::ids_matching(&conn, f)
                    .map_err(err)?
                    .into_iter()
                    .collect::<std::collections::HashSet<i32>>(),
            )
        }
        None => None,
    };
    let ids = resolve_search_ids(app, state, q, scope.as_ref())?;

    *guard = Some(SearchCache {
        generation,
        query: key,
        ids: ids.clone(),
    });
    Ok(ids)
}

/// The uncached resolve: CLIP "looks like" fused with caption/OCR "mentions the
/// words", best first, bounded to [`SEMANTIC_CANDIDATES`]. The caption half needs
/// no model, so search still works (captions only) when CLIP isn't installed or a
/// photo set isn't indexed; if neither half has data the list is empty. Callers
/// go through [`fused_search_ids`]; `q` is already trimmed and non-empty.
fn resolve_search_ids(
    app: &AppHandle,
    state: &AppState,
    q: &str,
    scope: Option<&std::collections::HashSet<i32>>,
) -> Result<Vec<i32>, String> {
    let conn = state.conn()?;
    let inside = |id: i64| scope.is_none_or(|s| s.contains(&(id as i32)));

    // Caption / OCR full-text — always available, no model needed. Within a
    // scope, far more are asked for, since most may fall outside it.
    let asked = if scope.is_some() {
        100_000
    } else {
        SEMANTIC_CANDIDATES as u32
    };
    let caption: Vec<i64> = db::caption_search(&conn, q, asked)
        .map_err(err)?
        .into_iter()
        .map(i64::from)
        .filter(|&id| inside(id))
        .take(SEMANTIC_CANDIDATES)
        .collect();

    // CLIP semantic — needs the model; degrade to caption-only if it isn't there
    // rather than failing the whole search.
    let semantic: Vec<i64> = match with_embedder(app, state, |e| e.query(q).map_err(err)) {
        Ok(qvec) => {
            let scores = db::embedding_scores(&conn, &qvec).map_err(err)?;
            relevant_within(scores, scope)
        }
        Err(_) => Vec::new(),
    };

    let fused = search::fuse(&[&semantic, &caption], search::RRF_K);
    Ok(fused
        .into_iter()
        .take(SEMANTIC_CANDIDATES)
        .map(|id| id as i32)
        .collect())
}

/// The photos that match a content query by how they look, best first:
/// ranked within `scope` when there is one, so the relevance cut is relative
/// to the photos that could be shown, not the whole library. Only what
/// matches, not the nearest 1,000 whatever they are.
fn relevant_within(
    mut scores: Vec<(i64, f32)>,
    scope: Option<&std::collections::HashSet<i32>>,
) -> Vec<i64> {
    if let Some(s) = scope {
        scores.retain(|&(id, _)| s.contains(&(id as i32)));
    }
    search::relevant(scores, SEMANTIC_CANDIDATES)
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

/// The newest grid load asked for, as `page << 32 | seq`.
static NEWEST_VIEW: AtomicU64 = AtomicU64::new(0);

/// Makes `request` the newest load unless a later one from the same page is
/// already in, and returns its key. A reloaded page picks a new `page`, so
/// its first load wins whatever its count restarted at.
fn claim_view(newest: &AtomicU64, request: models::ViewRequest) -> u64 {
    let key = (u64::from(request.page) << 32) | u64::from(request.seq);
    let _ = newest.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
        let same_page = current >> 32 == u64::from(request.page);
        (!same_page || key > current).then_some(key)
    });
    key
}

/// A connection for a grid load that gives up once a newer one is asked for.
/// Typing a search starts a load at every pause. The page ignored
/// the answers it no longer wanted, but the catalogue still worked each of
/// them out, beside the one that counted.
fn view_conn(state: &AppState, request: Option<models::ViewRequest>) -> Result<Connection, String> {
    let conn = state.conn()?;
    if let Some(request) = request {
        let mine = claim_view(&NEWEST_VIEW, request);
        stop_when_superseded(&conn, &NEWEST_VIEW, mine).map_err(err)?;
    }
    Ok(conn)
}

/// Stops `conn`'s statements, with an "interrupted" error, once `newest` is
/// no longer `mine`. SQLite asks every few thousand steps.
fn stop_when_superseded(
    conn: &Connection,
    newest: &'static AtomicU64,
    mine: u64,
) -> rusqlite::Result<()> {
    conn.progress_handler(10_000, Some(move || newest.load(Ordering::Relaxed) != mine))
}

/// The trimmed non-empty natural-language query on a filter, if any.
pub(crate) fn semantic_query(filter: &PhotoFilter) -> Option<&str> {
    filter
        .semantic
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

#[tauri::command(async)]
#[specta::specta]
pub(crate) fn get_photos(
    app: AppHandle,
    state: State<'_, AppState>,
    filter: PhotoFilter,
    limit: i32,
    offset: i32,
    request: Option<models::ViewRequest>,
) -> Result<Vec<Photo>, String> {
    // A natural-language query orders by relevance and restricts to its ranked
    // candidates; everything else pages by date as before.
    if let Some(q) = semantic_query(&filter) {
        let ranked = fused_search_ids(&app, &state, q, Some(&filter))?;
        let conn = view_conn(&state, request)?;
        return db::query_photos_ranked(&conn, &filter, &ranked, limit, offset).map_err(err);
    }
    let conn = view_conn(&state, request)?;
    db::query_photos(&conn, &filter, limit, offset).map_err(err)
}

/// The names the search box can recognise in a query — see
/// `src/lib/queryReading.ts`.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn search_vocabulary(state: State<'_, AppState>) -> Result<SearchVocabulary, String> {
    let conn = state.conn()?;
    db::search_vocabulary(&conn).map_err(err)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn get_photo(state: State<'_, AppState>, id: i32) -> Result<Option<Photo>, String> {
    let conn = state.conn()?;
    db::get_photo(&conn, id).map_err(err)
}

/// Fetches specific photos by id, in the order asked for.
///
/// The grid only ever holds what the current filter matched, but a duplicate
/// group comes from the whole library — so the compare view needs a way to pull
/// rows that the filter excluded, without abandoning the filter.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn get_photos_by_ids(
    state: State<'_, AppState>,
    ids: Vec<i32>,
) -> Result<Vec<Photo>, String> {
    let conn = state.conn()?;
    db::photos_by_ids(&conn, &ids).map_err(err)
}

#[tauri::command(async)]
#[specta::specta]
pub(crate) fn get_facets(
    app: AppHandle,
    state: State<'_, AppState>,
    filter: PhotoFilter,
    request: Option<models::ViewRequest>,
) -> Result<Facets, String> {
    // When a natural-language query is active the facet counts describe the
    // search results, not the whole library, so they stay in step with the grid.
    if let Some(q) = semantic_query(&filter) {
        let ranked = fused_search_ids(&app, &state, q, Some(&filter))?;
        let conn = view_conn(&state, request)?;
        return db::facets(&conn, &filter, Some(&ranked)).map_err(err);
    }
    let conn = view_conn(&state, request)?;
    db::facets(&conn, &filter, None).map_err(err)
}

/// The rail's optional counts — camera and lens, people, the status
/// toggles — for the groups whose section is open. See [`db::extra_facets`].
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn get_extra_facets(
    app: AppHandle,
    state: State<'_, AppState>,
    filter: PhotoFilter,
    groups: models::FacetGroups,
) -> Result<models::ExtraFacets, String> {
    if let Some(q) = semantic_query(&filter) {
        let ranked = fused_search_ids(&app, &state, q, Some(&filter))?;
        let conn = state.conn()?;
        return db::extra_facets(&conn, &filter, Some(&ranked), groups).map_err(err);
    }
    let conn = state.conn()?;
    db::extra_facets(&conn, &filter, None, groups).map_err(err)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn get_genres() -> Vec<String> {
    GENRES.iter().map(|g| g.to_string()).collect()
}

/// What the photos in view say about how they were made.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn insights(
    state: State<'_, AppState>,
    filter: PhotoFilter,
) -> Result<models::Insights, String> {
    db::insights(&state.conn()?, &filter).map_err(err)
}

/// What each of these lenses took, among the photos in view.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn lens_keywords(
    state: State<'_, AppState>,
    filter: PhotoFilter,
    lenses: Vec<String>,
) -> Result<Vec<models::LensKeywords>, String> {
    db::lens_keywords(&state.conn()?, &filter, &lenses).map_err(err)
}

/// Photos in view by month, or by day between two dates.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn timeline(
    state: State<'_, AppState>,
    filter: PhotoFilter,
    day_from: Option<String>,
    day_to: Option<String>,
) -> Result<Vec<models::TimelineBar>, String> {
    let days = day_from.as_deref().zip(day_to.as_deref());
    db::timeline(&state.conn()?, &filter, days).map_err(err)
}

/// Runs `f` with the shared CLIP embedder, loading it (once) from the `clip`
/// model folder on first use. The lock is held for the duration, which
/// serialises indexing and search — fine, since both want the one model.
fn with_embedder<T>(
    app: &AppHandle,
    state: &AppState,
    f: impl FnOnce(&search::Embedder) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = lock_recover(&state.search_embedder);
    if guard.is_none() {
        let dir = ai_models_dir(app)
            .map_err(err)?
            .join(tagger::builtin::MODEL_DIR);
        *guard = Some(search::build_embedder(&dir).map_err(err)?);
    }
    let embedder = guard
        .as_ref()
        .ok_or("the search model failed to load; try re-downloading it in Settings")?;
    f(embedder)
}

/// Number of photos still needing a search embedding — a status line for the UI.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn search_index_status(state: State<'_, AppState>) -> Result<i32, String> {
    let conn = state.conn()?;
    db::unindexed_count(&conn).map_err(err)
}

/// Embeds every not-yet-indexed photo with CLIP for semantic search, storing the
/// vector on the row and reporting progress on `embed-progress`.
///
/// `command(async)` is load-bearing: a plain `#[tauri::command]` on a *sync* fn
/// runs on the **main thread**, so this pass — CLIP over the whole library —
/// froze the window until it finished, and Windows would eventually offer to
/// kill the app. The `(async)` form spawns the same synchronous body onto the
/// runtime instead. The three sibling passes below had the same problem.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn index_for_search(app: AppHandle, state: State<'_, AppState>) -> Result<u32, String> {
    // One run at a time: automatic analysis and a search's "Index
    // now" both started one on a new library, and the second re-embedded the
    // same photos beside the first, with Stop reaching only one of them. The
    // run already going covers these photos.
    if state.jobs.is_running("index") {
        return Ok(0);
    }
    let db_path = state.db_path.clone();
    let queue = {
        let conn = db::connect(&db_path).map_err(err)?;
        db::unindexed_photos(&conn, 1_000_000).map_err(err)?
    };
    let total = queue.len();
    // Cancellable between photos. Every photo indexed before the cancel
    // keeps its embedding; the rest are simply still unindexed, and the next
    // run picks them up — so stopping is an outcome, not a failure.
    let cancel = state.jobs.start("index");
    let mut done = 0usize;

    // Load the model up front, so a missing one fails the run rather than
    // every photo in it.
    with_embedder(&app, &state, |_| Ok(()))?;
    let conn = db::connect(&db_path).map_err(err)?;
    let mut model_gone = None;
    // This thread decodes each preview and takes part in each model run, so
    // it gives way to the window like the model's own threads do
    // (`develop::ai::cpu_threads`).
    let _below_normal = hardware::BackgroundPriority::begin();
    for (photo_id, image_path) in queue {
        wait_while_developing(&state, &cancel);
        if cancel.is_cancelled() {
            break;
        }
        // The model is taken a photo at a time, not for the whole run: a
        // search typed while the library indexes needs it too, and waited
        // for every photo left — with the window frozen, while searches ran
        // on the main thread.
        match std::fs::read(&image_path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                // Before the model's lock, so a search typed meanwhile does
                // not wait on the vision model too.
                let _gpu = gpu_gate::GATE.local("search index");
                with_embedder(&app, &state, |embedder| {
                    embedder
                        .image_blob(&bytes)
                        .map_err(|e| format!("embedding {photo_id}: {e:#}"))
                })
            }) {
            Ok(blob) => {
                if let Err(e) = db::set_embedding(&conn, photo_id, &blob) {
                    eprintln!("index: cannot store embedding for {photo_id}: {e}");
                }
            }
            // The model went mid-run — deleted in Settings, now that the run
            // no longer holds it: every photo left would fail the same way.
            Err(e) if lock_recover(&state.search_embedder).is_none() => {
                model_gone = Some(e);
                break;
            }
            // One unreadable/odd file must not abort the whole library.
            Err(e) => eprintln!("index: {e}"),
        }
        done += 1;
        let _ = app.emit(
            "embed-progress",
            ScanProgress {
                phase: "embed".into(),
                current: count(done),
                total: count(total),
                message: "Indexing for search".into(),
            },
        );
    }

    // New embeddings change what a natural-language search resolves to.
    state.bump_search_gen();

    let stopped = cancel.is_cancelled() || model_gone.is_some();
    let _ = app.emit(
        "embed-progress",
        ScanProgress {
            phase: "done".into(),
            current: count(done),
            total: count(total),
            message: if stopped { "Stopped" } else { "Indexed" }.into(),
        },
    );
    match model_gone {
        Some(e) => Err(e),
        None => Ok(done as u32),
    }
}

/// Ranks the library against a free-text query by CLIP similarity, returning the
/// best matches (most similar first). Empty query → empty result.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn semantic_search(
    app: AppHandle,
    state: State<'_, AppState>,
    query: String,
    limit: u32,
) -> Result<Vec<Photo>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let qvec = with_embedder(&app, &state, |embedder| embedder.query(q).map_err(err))?;

    let conn = state.conn()?;
    let scores = db::embedding_scores(&conn, &qvec).map_err(err)?;
    // Text search ranks the whole library: a query legitimately has no natural
    // similarity floor, so keep every match (0.0) and let `limit` bound it.
    let ranked = search::best(scores, limit.max(1) as usize, 0.0);
    let ids: Vec<i32> = ranked.iter().map(|(id, _)| *id as i32).collect();
    db::photos_by_ids(&conn, &ids).map_err(err)
}

/// The best `count` of `ids`, best first: scored against a prompt
/// ensemble of good and poor photographs over the stored SigLIP 2
/// embeddings, picked for variety, with everyone named in the set included.
/// The caller has already dropped rejects, blinks and soft frames. Photos not
/// yet indexed for search can't be scored and are left out.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn highlights(
    app: AppHandle,
    state: State<'_, AppState>,
    ids: Vec<i32>,
    count: u32,
) -> Result<Vec<i32>, String> {
    let direction = with_embedder(&app, &state, |embedder| {
        let embed = |prompts: &[&str]| -> Result<Vec<Vec<f32>>, String> {
            prompts
                .iter()
                .map(|p| embedder.query(p).map_err(err))
                .collect()
        };
        Ok(search::aesthetic_direction(
            &embed(&search::GOOD_PROMPTS)?,
            &embed(&search::BAD_PROMPTS)?,
        ))
    })?;
    let conn = state.conn()?;
    // Only the photos in question, not every embedding in the library.
    let candidates: Vec<(i32, Vec<f32>)> = db::embeddings_of(&conn, &ids)
        .map_err(err)?
        .into_iter()
        .filter(|(_, e)| e.len() == direction.len())
        .collect();
    let scores: Vec<f32> = candidates
        .iter()
        .map(|(_, e)| e.iter().zip(&direction).map(|(a, b)| a * b).sum())
        .collect();
    let embeddings: Vec<Vec<f32>> = candidates.iter().map(|(_, e)| e.clone()).collect();
    let mut chosen = search::pick_diverse(&scores, &embeddings, count as usize, 0.7);
    let ids_in: Vec<i32> = candidates.iter().map(|(id, _)| *id).collect();
    let by_photo = db::named_people_by_photo(&conn, &ids_in).map_err(err)?;
    let people: Vec<Vec<i32>> = ids_in
        .iter()
        .map(|id| by_photo.get(id).cloned().unwrap_or_default())
        .collect();
    search::cover_people(&mut chosen, &people, &scores);
    chosen.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
    Ok(chosen.into_iter().map(|i| candidates[i].0).collect())
}

/// Ranks the library by visual similarity to one photo (nearest first, the photo
/// itself excluded). Pure vector maths over the stored embeddings — needs no
/// model, only that the photos have been indexed.
///
/// `threshold` is the minimum cosine similarity to include: without it the
/// result is the entire indexed library reordered, which reads as "nothing was
/// filtered". The caller picks the cutoff (see `SIMILAR_THRESHOLD` on the JS
/// side).
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn similar_photos(
    state: State<'_, AppState>,
    photo_id: i32,
    limit: u32,
    threshold: f32,
) -> Result<Vec<Photo>, String> {
    let conn = state.conn()?;
    let (_, target) = db::embeddings_of(&conn, &[photo_id])
        .map_err(err)?
        .pop()
        .ok_or("this photo isn't indexed for search yet — run indexing first")?;

    let mut scores = db::embedding_scores(&conn, &target).map_err(err)?;
    scores.retain(|(id, _)| *id as i32 != photo_id);
    let ranked = search::best(scores, limit.max(1) as usize, threshold);
    let ids: Vec<i32> = ranked.iter().map(|(id, _)| *id as i32).collect();
    db::photos_by_ids(&conn, &ids).map_err(err)
}

/// Ranks photos by caption full-text match (best first), the keyword/OCR half of
/// search — reads signs, text and exact words the CLIP "looks like" search
/// can't. Returns nothing until photos have been captioned. Free-text query;
/// never errors on stray punctuation or FTS operators (see `fts::build_query`).
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn caption_search(
    state: State<'_, AppState>,
    query: String,
    limit: u32,
) -> Result<Vec<Photo>, String> {
    let conn = state.conn()?;
    let ids = db::caption_search(&conn, &query, limit.max(1)).map_err(err)?;
    db::photos_by_ids(&conn, &ids).map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grid load is the newest until a later one from its page comes in,
    /// however they arrive; a reloaded page's first load wins.
    #[test]
    fn the_newest_grid_load_is_the_one_that_counts() {
        let newest = AtomicU64::new(0);
        let view = |page, seq| models::ViewRequest { page, seq };
        let first = claim_view(&newest, view(7, 1));
        assert_eq!(newest.load(Ordering::SeqCst), first);
        let third = claim_view(&newest, view(7, 3));
        // The second arrives late: it doesn't take over from the third.
        let second = claim_view(&newest, view(7, 2));
        assert_ne!(second, third);
        assert_eq!(newest.load(Ordering::SeqCst), third);
        // The page reloads and counts from one again.
        let reloaded = claim_view(&newest, view(9, 1));
        assert_eq!(newest.load(Ordering::SeqCst), reloaded);
    }

    /// A superseded load's query stops where it is, and a current one runs.
    #[test]
    fn a_superseded_load_stops_its_query() {
        static NEWEST: AtomicU64 = AtomicU64::new(1);
        let conn = Connection::open_in_memory().unwrap();
        // Long enough for SQLite to ask many times.
        let long = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n \
                    WHERE i < 2000000) SELECT count(*) FROM n";
        stop_when_superseded(&conn, &NEWEST, 1).unwrap();
        let counted: i64 = conn.query_row(long, [], |r| r.get(0)).unwrap();
        assert_eq!(counted, 2_000_000);
        NEWEST.store(2, Ordering::SeqCst);
        let err = conn
            .query_row(long, [], |r| r.get::<_, i64>(0))
            .unwrap_err();
        assert!(err.to_string().contains("interrupted"), "{err}");
    }

    /// A content query ranks within the facets: a beach that is only
    /// middling across the whole library is still the best of Anna's June,
    /// and was cut before Anna and June applied.
    #[test]
    fn a_content_query_ranks_within_the_facets() {
        // Two hundred strong beaches elsewhere; Anna's best is weaker than
        // those, but clearly the beach among hers.
        let mut scores: Vec<(i64, f32)> = (1..=200).map(|id| (id, 0.30)).collect();
        scores.extend((201..=1000).map(|id| (id, 0.05)));
        scores.extend([(2001, 0.15), (2002, 0.07), (2003, 0.065), (2004, 0.06)]);
        let everywhere = relevant_within(scores.clone(), None);
        assert!(!everywhere.contains(&2001), "cut across the library");
        let annas: std::collections::HashSet<i32> = [2001, 2002, 2003, 2004].into();
        let within = relevant_within(scores, Some(&annas));
        assert_eq!(within, vec![2001]);
    }
}
