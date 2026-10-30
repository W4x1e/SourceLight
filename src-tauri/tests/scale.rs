//! What the catalogue actually costs at the size the app is built for.
//!
//! Until this existed the largest library the project had demonstrably run
//! against was 59 photos. That is not a small gap from "cull thousands per
//! shoot" — it is the difference between knowing and guessing, and the guess was
//! load-bearing for who the app could serve.
//!
//! ## What it runs by default
//!
//! **Always, with no environment variable to remember.** That is deliberate:
//! `SOURCELIGHT_TEST_RAW` and friends turn 31 integration tests into green
//! no-ops that prove nothing, and a benchmark nobody runs is worth
//! rather less than no benchmark, because it also claims to be covered.
//!
//! 10,000 rows in release, 2,000 in debug — see [`DEFAULT_ROWS`]. Either size is
//! well past the point where a missing index shows up.
//!
//! For a number worth writing down, ask for the real size, **in release**:
//!
//! ```text
//! SOURCELIGHT_SCALE=50000 cargo test --manifest-path src-tauri/Cargo.toml \
//!     --release --test scale -- --nocapture
//! ```
//!
//! A debug build compiles SQLite itself unoptimised and leaves the cosine loops
//! unvectorised, which moves individual measurements here by up to 25× — and
//! unevenly, so the debug numbers are not even proportionally useful.
//!
//! ## What it asserts, and what it only reports
//!
//! Wall-clock budgets on a developer's machine are a poor gate — they fail on a
//! busy laptop and pass on a fast one. So the timings are **reported**, with
//! ceilings set roughly an order of magnitude above the measured value: those
//! catch a query that has gone quadratic, not one that got 20% slower.
//!
//! The **query plans are asserted properly**, because they are machine
//! independent. A filter that stops using its index is the failure mode that
//! actually bites at scale, and it is invisible in a timing at 10,000 rows on a
//! machine with the whole database in page cache.
//!
//! The report is printed and also written to `target/scale-report.md`, so the
//! numbers survive a run without `--nocapture`.

use std::time::{Duration, Instant};

use rusqlite::Connection;
use sourcelight_lib::models::PhotoFilter;
use sourcelight_lib::seed::{seed, Shape};
use sourcelight_lib::test_support::Scratch;
use sourcelight_lib::{db, search};

/// Rows to seed unless `SOURCELIGHT_SCALE` says otherwise.
///
/// Smaller in a debug build, for two reasons. The measurements there are not
/// worth recording — an unoptimised SQLite and unoptimised cosine loops move
/// some of these by 25× — and `scripts/check.sh` runs the debug suite on every
/// commit, where forty seconds of benchmark nobody will read is exactly the kind
/// of cost that makes people stop running the checks.
///
/// It still *runs* in debug, on real data, asserting the same things. It just
/// does not pretend to be a measurement.
const DEFAULT_ROWS: usize = if cfg!(debug_assertions) {
    2_000
} else {
    10_000
};

/// Ceilings are profile-relative for the same reason. A debug build is not
/// slower by a constant factor across the board — SQL barely moves, float loops
/// move by 25× — so one number cannot serve both.
const CEILING: Duration = Duration::from_secs(if cfg!(debug_assertions) { 30 } else { 10 });

/// The grid's page size (`PAGE` in `store/library.ts`). Every filter change asks for this
/// many rows in one call, so it is the number that matters, not a screenful.
const PAGE: i32 = 20_000;

/// Bounded candidate set for a natural-language search
/// (`SEMANTIC_CANDIDATES` in `commands/browse.rs`).
const SEMANTIC_CANDIDATES: usize = 1_000;

fn rows() -> usize {
    std::env::var("SOURCELIGHT_SCALE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_ROWS)
}

/// Median of five, after one untimed warm-up.
///
/// The first call to any of these prepares a statement and faults the pages in;
/// timing that measures the cache, not the query. A median rather than a mean
/// because one scheduler hiccup should not move the number.
fn time<T>(mut f: impl FnMut() -> T) -> Duration {
    f();
    let mut runs: Vec<Duration> = (0..5)
        .map(|_| {
            let t = Instant::now();
            let out = f();
            let d = t.elapsed();
            drop(out);
            d
        })
        .collect();
    runs.sort();
    runs[2]
}

struct Report {
    rows: usize,
    lines: Vec<(String, Duration, String)>,
}

impl Report {
    fn add(&mut self, what: &str, d: Duration, note: &str) {
        println!("  {:>9.1} ms  {what}", d.as_secs_f64() * 1000.0);
        self.lines.push((what.into(), d, note.into()));
    }

    fn write(&self) {
        let mut md = format!(
            "| Operation | {} photos | Note |\n|---|--:|---|\n",
            self.rows
        );
        for (what, d, note) in &self.lines {
            md.push_str(&format!(
                "| {what} | {:.1} ms | {note} |\n",
                d.as_secs_f64() * 1000.0
            ));
        }
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("scale-report.md");
        let _ = std::fs::write(&out, md);
        println!("\nreport written to {}", out.display());
    }
}

/// A seeded catalogue and the scratch folder holding it, which is deleted when
/// this drops. Fields drop in order, so the connection closes first — Windows
/// will not delete a database that is still open.
struct Seeded {
    conn: Connection,
    _dir: Scratch,
}

impl std::ops::Deref for Seeded {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.conn
    }
}

impl std::ops::DerefMut for Seeded {
    fn deref_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }
}

/// An on-disk database, not `:memory:`.
///
/// The app runs against a file in WAL mode, and an in-memory database has no
/// page cache to miss and no journal to write. Measuring the wrong storage
/// engine would make every number here optimistic in a way that is invisible.
fn seeded(tag: &str, n: usize) -> Seeded {
    let dir = Scratch::new(&format!("scale-{tag}"));
    let path = dir.join("library.db");
    let mut conn = db::open(&path).expect("open catalogue");

    let t = Instant::now();
    let stats = seed(&mut conn, &Shape::library(n)).expect("seed");
    let elapsed = t.elapsed();

    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!(
        "\nseeded {} photos in {:.1}s — {} tag links, {} placed, {} faces over {} people, \
         {} captions, {} burst frames, {} virtual copies, {:.1} MB on disk",
        stats.photos,
        elapsed.as_secs_f64(),
        stats.tag_links,
        stats.placed,
        stats.faces,
        stats.people,
        stats.captions,
        stats.bursts,
        stats.variants,
        bytes as f64 / 1_048_576.0,
    );
    Seeded { conn, _dir: dir }
}

/// A filter with something set on every axis the rail exposes, which is the
/// expensive case: each facet count is computed with its own constraint lifted,
/// so the work is roughly one filtered count per facet, not one in total.
fn busy_filter() -> PhotoFilter {
    PhotoFilter {
        date_from: Some("2023-01-01".into()),
        date_to: Some("2025-12-31".into()),
        genres: vec!["landscape".into(), "wildlife".into()],
        subjects: vec!["tree".into(), "bird".into(), "mountain".into()],
        conditions: vec!["golden hour".into()],
        min_rating: Some(2),
        flags: vec![0, 1],
        color_labels: vec![1, 2, 3],
        ..Default::default()
    }
}

#[test]
fn catalogue_at_library_scale() {
    let n = rows();
    let mut conn = seeded("catalogue", n);
    let mut report = Report {
        rows: n,
        lines: Vec::new(),
    };

    println!("\n-- browsing --");

    let empty = PhotoFilter::default();
    report.add(
        "get_photos, no filter, first page",
        time(|| db::query_photos(&conn, &empty, PAGE, 0).unwrap()),
        "the whole library in one call — what opening the app does",
    );
    report.add(
        "get_photos, no filter, 200 rows",
        time(|| db::query_photos(&conn, &empty, 200, 0).unwrap()),
        "a screenful, for comparison",
    );

    // The grid does not receive `Vec<Photo>`; it receives JSON across the Tauri
    // bridge, parses it, and holds the result in a Zustand store. That crossing
    // is invisible from Rust and is paid on **every filter change**, so a
    // measurement that stops at `query_photos` is measuring the cheaper half.
    // This is the serialise side of it — the webview still has to parse the
    // same bytes.
    let page = db::query_photos(&conn, &empty, PAGE, 0).unwrap();
    let json = serde_json::to_string(&page).unwrap();
    report.add(
        "serialise that page to JSON for the bridge",
        time(|| serde_json::to_string(&page).unwrap()),
        &format!(
            "{:.1} MB for {} photos, and the webview parses it again",
            json.len() as f64 / 1_048_576.0,
            page.len()
        ),
    );

    // Clicking one genre in the filter rail — the most ordinary click in the
    // app. This used to be the one case with no index: a bare `SCAN p` running a
    // correlated EXISTS once per photo. It now drives from the tag through
    // idx_phototags_tag_photo and probes `photos` by primary key, asserted in
    // `the_filters_still_use_their_indexes`. Kept as its own line because it is
    // the click that has to stay fast, and the busy filter below hides it — the
    // date range there narrows through `idx_photos_capture` first.
    let one_genre = PhotoFilter {
        genres: vec!["landscape".into()],
        ..Default::default()
    };
    report.add(
        "get_photos, one genre and nothing else",
        time(|| db::query_photos(&conn, &one_genre, PAGE, 0).unwrap()),
        "drives from the tag index, not a full scan",
    );
    // Typing in the search box. A scan of the rows, as it always was, now
    // also looking in the caption and at who is in each photo.
    let typed = PhotoFilter {
        search: Some("gold".into()),
        ..Default::default()
    };
    report.add(
        "get_photos, typed text",
        time(|| db::query_photos(&conn, &typed, PAGE, 0).unwrap()),
        "filename, caption, tag, place and person, as typed",
    );
    report.add(
        "get_facets, typed text",
        time(|| db::facets(&conn, &typed, None).unwrap()),
        "the rail's counts under the same text",
    );
    report.add(
        "get_facets, one genre and nothing else",
        time(|| db::facets(&conn, &one_genre, None).unwrap()),
        "sibling facets batched; the filter applied once per photo, not per tag link",
    );

    let busy = busy_filter();
    report.add(
        "get_photos, every facet set",
        time(|| db::query_photos(&conn, &busy, PAGE, 0).unwrap()),
        "date + genre + subject + condition + rating + flag + colour",
    );
    report.add(
        "get_facets, no filter",
        time(|| db::facets(&conn, &empty, None).unwrap()),
        "one grouped count over photo_tags for every tag facet, no photos join",
    );
    report.add(
        "get_facets, every facet set",
        time(|| db::facets(&conn, &busy, None).unwrap()),
        "each count recomputed with its own constraint lifted",
    );
    // The rail's optional counts, asked for only while their section
    // is open. Every group at once is the worst case.
    let every_group = sourcelight_lib::models::FacetGroups {
        gear: true,
        people: true,
        status: true,
        kind: true,
        palette: true,
    };
    report.add(
        "get_extra_facets, every group, no filter",
        time(|| db::extra_facets(&conn, &empty, None, every_group).unwrap()),
        "gear, kinds and status in one grouped pass over the rows; people from faces",
    );
    report.add(
        "get_extra_facets, every group, one genre",
        time(|| db::extra_facets(&conn, &one_genre, None, every_group).unwrap()),
        "the genre narrows the pass to its photos",
    );

    // Library insights: one grouped pass, the busiest days, and a
    // closer look at the four most used lenses.
    report.add(
        "insights, no filter",
        time(|| db::insights(&conn, &empty).unwrap()),
        "every chart of the Insights view",
    );
    let top: Vec<String> = db::insights(&conn, &empty)
        .unwrap()
        .lenses
        .iter()
        .take(4)
        .map(|l| l.key.clone())
        .collect();
    report.add(
        "lens_keywords, four lenses, no filter",
        time(|| db::lens_keywords(&conn, &empty, &top).unwrap()),
        "what each lens took, filled in after the view opens",
    );
    report.add(
        "insights, one genre",
        time(|| db::insights(&conn, &one_genre).unwrap()),
        "the genre narrows every count",
    );

    // The timeline strip over the grid: one grouped count, redrawn with
    // every filter change.
    // What every launch rescan asks once per library folder: the
    // photos waiting for a town. The first call places the seeded positions;
    // the second is an unchanged library, which has none.
    db::place_from_gps(&mut conn).unwrap();
    report.add(
        "place_from_gps, nothing waiting",
        time(|| db::place_from_gps(&mut conn).unwrap()),
        "a rescan of an unchanged library, under the write lock",
    );
    report.add(
        "timeline by month, no filter",
        time(|| db::timeline(&conn, &empty, None).unwrap()),
        "read from the capture-date index alone",
    );
    report.add(
        "timeline by month, one genre",
        time(|| db::timeline(&conn, &one_genre, None).unwrap()),
        "the genre narrows it to its photos first",
    );

    // Colours: every photo given a palette of a few colours, then the
    // Palette counts and a ranking by likeness over all of them. Reading one
    // is timed from a thumbnail-sized JPEG, as the backfill reads them.
    {
        let ids: Vec<i32> = conn
            .prepare("SELECT id FROM photos")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let colours: [[u8; 3]; 6] = [
            [250, 120, 30],
            [110, 50, 140],
            [40, 110, 40],
            [30, 60, 220],
            [240, 240, 240],
            [20, 20, 22],
        ];
        conn.execute_batch("BEGIN").unwrap();
        for id in &ids {
            let a = colours[*id as usize % 6];
            let b = colours[(*id as usize / 6) % 6];
            let palette = sourcelight_lib::palette::read(
                std::iter::repeat_n(a, 60 + (*id as usize % 30)).chain(std::iter::repeat_n(b, 40)),
            );
            db::set_palette(&conn, *id, &palette).unwrap();
        }
        conn.execute_batch("COMMIT").unwrap();
        let palette_only = sourcelight_lib::models::FacetGroups {
            palette: true,
            ..Default::default()
        };
        report.add(
            "Palette counts, no filter",
            time(|| db::extra_facets(&conn, &empty, None, palette_only).unwrap()),
            "a bit per named colour, summed over every photo",
        );
        report.add(
            "similar colours to one photo",
            time(|| db::similar_colours(&conn, ids[0], 300, 0.55).unwrap()),
            "every signature compared, best 300 kept",
        );
        let mut jpeg = Vec::new();
        let img = image::RgbImage::from_fn(512, 341, |x, y| {
            image::Rgb([(x / 2) as u8, (y * 3 / 4) as u8, 120])
        });
        image::DynamicImage::ImageRgb8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut jpeg),
                image::ImageFormat::Jpeg,
            )
            .unwrap();
        report.add(
            "read one photo's colours from a thumbnail",
            time(|| sourcelight_lib::palette::read_image(&jpeg).unwrap()),
            "decode a 512 px JPEG, shrink, bin: the backfill's cost per photo",
        );
    }

    println!("\n-- search --");

    let embeddings = db::all_embeddings(&conn).unwrap();
    report.add(
        "all_embeddings",
        time(|| db::all_embeddings(&conn).unwrap()),
        "every CLIP vector into memory — runs twice per search, see below",
    );
    let qvec: Vec<f32> = {
        // Any unit vector will do; ranking cost does not depend on direction.
        let raw: Vec<f32> = (0..512).map(|i| ((i % 17) as f32) - 8.0).collect();
        let norm = raw.iter().map(|x| x * x).sum::<f32>().sqrt();
        raw.into_iter().map(|x| x / norm).collect()
    };
    report.add(
        "search::rank over every embedding",
        time(|| search::rank(&qvec, &embeddings, SEMANTIC_CANDIDATES, 0.0)),
        "cosine against the whole library",
    );
    report.add(
        "search::rank_relevant over every embedding",
        time(|| search::rank_relevant(&qvec, &embeddings, SEMANTIC_CANDIDATES)),
        "only what matches: every score, their mean and spread, one sort",
    );
    report.add(
        "caption_search (FTS5)",
        time(|| {
            db::caption_search(&conn, "golden hour mountain", SEMANTIC_CANDIDATES as u32).unwrap()
        }),
        "the half of search that needs no model",
    );

    let ranked: Vec<i32> = search::rank(&qvec, &embeddings, SEMANTIC_CANDIDATES, 0.0)
        .into_iter()
        .map(|(id, _)| id as i32)
        .collect();
    report.add(
        "query_photos_ranked over the candidates",
        time(|| db::query_photos_ranked(&conn, &busy, &ranked, PAGE, 0).unwrap()),
        "1,000-way `IN (…)` intersected with the facets",
    );
    report.add(
        "facets over the semantic whitelist",
        time(|| db::facets(&conn, &busy, Some(&ranked)).unwrap()),
        "the same 1,000-way `IN (…)`, once per facet",
    );

    // What one keystroke in the search box actually costs. `get_photos` and
    // `get_facets` are separate commands and each resolves the query from
    // scratch, so every ingredient below is paid for twice per interaction.
    let one_search = time(|| {
        for _ in 0..2 {
            let cands = db::all_embeddings(&conn).unwrap();
            let ranked: Vec<i32> = search::rank(&qvec, &cands, SEMANTIC_CANDIDATES, 0.0)
                .into_iter()
                .map(|(id, _)| id as i32)
                .collect();
            db::caption_search(&conn, "golden hour", SEMANTIC_CANDIDATES as u32).unwrap();
            db::query_photos_ranked(&conn, &busy, &ranked, PAGE, 0).unwrap();
            db::facets(&conn, &busy, Some(&ranked)).unwrap();
        }
    });
    report.add(
        "ONE natural-language search, end to end",
        one_search,
        "**get_photos and get_facets each resolve the query independently, \
         so everything above happens twice** — plus two CLIP text encodes, \
         which this cannot measure",
    );

    println!("\n-- cull and people --");

    let ids: Vec<i32> = db::all_photo_ids(&conn).unwrap();
    report.add(
        "all_photo_ids",
        time(|| db::all_photo_ids(&conn).unwrap()),
        "select-all in the grid",
    );
    let filmstrip: Vec<i32> = ids.iter().copied().take(PAGE as usize).collect();
    report.add(
        "photos_by_ids for a full filmstrip",
        time(|| db::photos_by_ids(&conn, &filmstrip).unwrap()),
        "the Lightbox filmstrip at the page cap",
    );
    report.add(
        "list_people",
        time(|| db::list_people(&conn).unwrap()),
        "the People workspace",
    );

    // Over the **whole** library, not a slice. Before the fix this could not finish
    // — it was quadratic — so it was measured on a slice and extrapolated. Now it
    // only compares frames within a burst window, so the whole library is cheap
    // and there is nothing to extrapolate.
    let dup_input = db::embeddings_for_grouping(&conn).unwrap();
    let groups = search::group_near_duplicates(&dup_input, 0.93);
    let grouped: usize = groups.iter().map(|g| g.len()).sum();
    report.add(
        &format!(
            "near-duplicate grouping, all {n} ({} frames in {} bursts)",
            grouped,
            groups.len()
        ),
        time(|| search::group_near_duplicates(&dup_input, 0.93)),
        "time-windowed — only frames within a minute of each other are compared",
    );

    report.write();

    // Ceilings, not budgets. Set about an order of magnitude above what these
    // measure so they catch a query that has gone quadratic — the failure this
    // whole file exists to find — rather than a slow afternoon.
    for (what, d, _) in &report.lines {
        assert!(
            *d < CEILING,
            "{what} took {:.1}s at {n} photos — that is pathological, not slow",
            d.as_secs_f64()
        );
    }
}

/// The bug was that `group_near_duplicates` was quadratic: it compared
/// every photo against every existing group seed, and in a real library almost
/// every photo is its own group, so `groups` converged on `n`. Restricting
/// comparisons to a burst-sized time window fixed it. This guards the fix.
///
/// It asserts on the **number of embedding comparisons**, not on wall-clock.
/// This used to be a timing ratio, and at these sizes a release build groups
/// 1,000 photos in about a millisecond — so one scheduler hiccup decided the
/// result, and under build load it failed more often than it passed while the
/// algorithm was, provably, doing the same work every run. The comparison count
/// is that work, measured exactly: the seeder is deterministic, so this is too.
///
/// Two guards, because each alone has a way out:
///
/// - **Doubling the input roughly doubles the comparisons.** An all-pairs
///   comparison quadruples them. Under 2.5x, against a theoretical 2x, leaves
///   room for the seeded bursts not landing evenly across the two prefixes.
/// - **The whole run averages under ten comparisons per photo** — thousands of
///   times below the `n²/2` that all-pairs against seeds costs once almost every
///   photo is its own group, the shape that was quadratic. A ratio alone would let a
///   near-all-pairs pass with lucky data through; this would not. The seeded
///   catalogue measures well under one per photo, since most frames have no
///   neighbour within the window at all.
#[test]
fn near_duplicate_grouping_stays_near_linear() {
    let conn = seeded("dups", 4_000);
    let all = db::embeddings_for_grouping(&conn).unwrap();

    let work = |k: usize| search::group_near_duplicates_counted(&all[..k], 0.93).1;
    let small = work(1_000);
    let large = work(2_000);
    let ratio = large as f64 / small.max(1) as f64;

    // It must still *find* the seeded bursts — an algorithm that compares
    // nothing and groups nothing would pass both work checks.
    let (groups, total) = search::group_near_duplicates_counted(&all, 0.93);
    let grouped: usize = groups.iter().map(|g| g.len()).sum();
    let n = all.len() as u64;
    let all_pairs = n * (n - 1) / 2;
    println!(
        "near-duplicate grouping: {small} comparisons at 1,000 -> {large} at 2,000 \
         ({ratio:.2}x for 2x); {total} at {n}, against {all_pairs} for all-pairs; \
         {grouped} of {n} frames grouped into {} bursts",
        groups.len(),
    );
    assert!(grouped > 0, "the seeded bursts were not found at all");

    assert!(
        ratio < 2.5,
        "doubling the input cost {ratio:.2}x the comparisons — that looks quadratic \
         again, which is exactly what was fixed. Did an all-pairs comparison \
         creep back?"
    );
    assert!(
        total < 10 * n,
        "grouping {n} photos made {total} comparisons — more than ten per photo, where \
         all-pairs would make {all_pairs}. The time window is no longer bounding the \
         work"
    );
}

/// Paging used to pay a full sort before the first row: the grid orders by
/// `capture_date DESC, COALESCE(variant_of, id) DESC, variant ASC` — virtual
/// copies sorting next to the photo they came from — and no index covered the
/// `COALESCE` expression, so the plan filled a temp b-tree over the whole library
/// (`USE TEMP B-TREE FOR LAST 2 TERMS OF ORDER BY`) before a single row came
/// back. Asking for 200 photos then cost almost what asking for 20,000 did.
///
/// `idx_photos_grid` (migration 14) is an expression index covering all three
/// terms, so the sort is read straight from it and a `LIMIT` stops after that
/// many rows. This is the inverted guard the fix asked for: if the index is
/// dropped or the ORDER BY drifts off it, the temp b-tree returns and this fails.
#[test]
fn paging_reads_the_grid_sort_from_an_index_not_a_full_sort() {
    let conn = seeded("paging", 20_000);

    // The paging case: 200 rows, which must not sort the whole library.
    let sql = format!(
        "EXPLAIN QUERY PLAN
        SELECT p.id FROM {GRID_FROM} WHERE 1 = 1
        ORDER BY p.capture_date DESC, COALESCE(p.variant_of, p.id) DESC, p.variant ASC
        LIMIT 200"
    );
    let mut stmt = conn.prepare(&sql).unwrap();
    let plan: String = stmt
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<String>>>()
        .unwrap()
        .join("\n");
    println!("{plan}");
    assert!(
        plan.contains("idx_photos_grid") && !plan.contains("TEMP B-TREE"),
        "the grid sort is filling a temp b-tree instead of reading idx_photos_grid — \
         paging pays a full sort again:\n{plan}"
    );

    // And the timing that motivated it: a small page must no longer cost what the
    // whole library costs. The ceiling is loose on purpose (see the module note);
    // the plan assertion above is the real gate.
    let empty = PhotoFilter::default();
    let one = time(|| db::query_photos(&conn, &empty, 1, 0).unwrap());
    let page = time(|| db::query_photos(&conn, &empty, 200, 0).unwrap());
    println!(
        "one row: {:.1} ms, two hundred rows: {:.1} ms",
        one.as_secs_f64() * 1000.0,
        page.as_secs_f64() * 1000.0,
    );
}

/// The assertion that is actually worth gating on.
///
/// Every timing above is a property of this machine on this afternoon. A query
/// plan is not: if the date filter stops using `idx_photos_capture`, it stops
/// using it everywhere, and at 10,000 rows with the whole file in page cache the
/// timing will not notice.
#[test]
fn the_filters_still_use_their_indexes() {
    // Small: a plan is a plan. The seeder refreshes the planner's statistics the
    // same way the app does, which is what lets it make the same choice here that
    // it makes at fifty thousand.
    let conn = seeded("plans", 2_000);

    let plan = |f: &PhotoFilter| -> String {
        // `build_where` is private, so go through the public query and ask
        // SQLite what it did — which is the more honest thing to assert anyway.
        let sql = explain_of(&conn, f);
        println!("{sql}");
        sql
    };

    // The date range must be driven by a capture_date-led index, not a full scan.
    // Since migration 14 that is idx_photos_grid — whose leading column is
    // capture_date — which the planner now prefers because the same index also
    // answers the grid sort, doing the range and the ORDER BY in one covering
    // pass. idx_photos_capture would also satisfy this; either is correct, a
    // `SCAN p` with no index is the regression.
    let by_date = PhotoFilter {
        date_from: Some("2024-01-01".into()),
        date_to: Some("2024-06-30".into()),
        ..Default::default()
    };
    let p = plan(&by_date);
    assert!(
        p.contains("idx_photos_capture") || p.contains("idx_photos_grid"),
        "the date filter is scanning the whole library instead of driving from a \
         capture_date index:\n{p}"
    );

    // A tag filter must drive from the tag — resolve it through
    // idx_tags_category_name, gather its photos from idx_phototags_tag_photo
    // without leaving the index (migration 15), then probe `photos` by primary
    // key. The failure this guards is a regression to the correlated EXISTS,
    // whose plan was a bare `SCAN p`: a full scan of the library running the
    // subquery once per row. `SEARCH p ... PRIMARY KEY` is fine; `SCAN p`
    // is the thing that must not come back.
    let by_tag = PhotoFilter {
        genres: vec!["landscape".into()],
        ..Default::default()
    };
    let p = plan(&by_tag);
    assert!(
        p.contains("COVERING INDEX idx_phototags_tag_photo") && !p.contains("SCAN p"),
        "the tag filter is scanning the whole library instead of driving from the tag:\n{p}"
    );

    let by_person = PhotoFilter {
        person_id: Some(1),
        ..Default::default()
    };
    let p = plan(&by_person);
    assert!(
        p.contains("idx_faces_photo") || p.contains("idx_faces_person"),
        "the person filter is scanning faces:\n{p}"
    );

    // The rail as it opens: the rating facet, the Missing count and
    // the folder counts read an index each, never the photo rows, which each
    // carry a ~3 KB embedding. As `facets` and `list_library_folders` build
    // them with no filter (`NO_FILTER` is `1 = 1`).
    let p = plan_of(
        &conn,
        "SELECT p.rating, COUNT(*) FROM photos p WHERE 1 = 1 GROUP BY p.rating",
    );
    assert!(
        p.contains("COVERING INDEX idx_photos_rating") && !p.contains("TEMP B-TREE"),
        "the rating facet is reading every photo row:\n{p}"
    );
    let p = plan_of(
        &conn,
        "SELECT COUNT(*) FROM photos p WHERE 1 = 1 AND p.missing = 1",
    );
    assert!(
        p.contains("idx_photos_missing"),
        "the Missing count is reading every photo row:\n{p}"
    );
    let p = plan_of(
        &conn,
        "SELECT COUNT(*) FROM photos p WHERE p.folder_id = 1 AND p.variant = 0 AND p.missing = 1",
    );
    assert!(
        p.contains("COVERING INDEX idx_photos_folder_counts"),
        "a folder's counts are reading photo rows:\n{p}"
    );

    // Typed text: the match is found once, from idx_photos_text, and
    // the photos looked up by id; not by testing every photo row. Mirrors
    // the search clause in `build_where`, its filename and caption part.
    let p = plan_of(
        &conn,
        "SELECT p.id FROM photos p WHERE p.id IN (SELECT s.id FROM photos s
           WHERE s.filename LIKE '%gold%' OR s.caption LIKE '%gold%'
              OR COALESCE(s.variant_of, s.id) IN (SELECT f.photo_id FROM faces f))",
    );
    assert!(
        p.contains("COVERING INDEX idx_photos_text") && p.contains("USING INTEGER PRIMARY KEY"),
        "typed text is testing every photo row:\n{p}"
    );

    // A rescan asks for the photos waiting for a town, and marks them
    // done, off a partial index: an unchanged library has none to read.
    let p = plan_of(
        &conn,
        "SELECT p.id, p.lat, p.lon FROM photos p
          WHERE p.lat IS NOT NULL AND p.place_auto = 0
            AND NOT EXISTS (SELECT 1 FROM photo_places pp WHERE pp.photo_id = p.id)",
    );
    assert!(
        p.contains("idx_photos_unplaced"),
        "the photos waiting for a town are found by reading every row:\n{p}"
    );
    let p = plan_of(
        &conn,
        "UPDATE photos SET place_auto = 1 WHERE lat IS NOT NULL AND place_auto = 0",
    );
    assert!(
        p.contains("idx_photos_unplaced"),
        "marking them done reads every row:\n{p}"
    );
}

/// `EXPLAIN QUERY PLAN` for a statement as written.
fn plan_of(conn: &Connection, sql: &str) -> String {
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let plan = stmt
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<String>>>()
        .unwrap()
        .join("\n");
    println!("{plan}");
    plan
}

/// What the grid query reads from — `PHOTO_FROM` in `db/query.rs`: the photo, and its
/// place joined in. Mirrored by hand like the filters below, so a plan asserted
/// here is the one the grid actually runs.
const GRID_FROM: &str = "photos p
    LEFT JOIN photo_places photo_place ON photo_place.photo_id = p.id
    LEFT JOIN places place_row ON place_row.id = photo_place.place_id";

/// `EXPLAIN QUERY PLAN` for the grid query under a given filter.
///
/// Rebuilt here rather than exported from `db`, because exporting the SQL
/// builder to make it testable would make it part of the library's surface for
/// the sake of one test.
fn explain_of(conn: &Connection, f: &PhotoFilter) -> String {
    let mut clauses: Vec<String> = Vec::new();
    if f.date_from.is_some() {
        clauses.push("p.capture_date >= '2024-01-01'".into());
    }
    if f.date_to.is_some() {
        clauses.push("p.capture_date <= '2024-06-30'".into());
    }
    if !f.genres.is_empty() {
        // Mirrors `tag_membership_clause` in `db/query.rs` — an uncorrelated semijoin,
        // not a correlated EXISTS. Kept in step by hand because `build_where` is
        // private; if the two drift the plan asserted here stops being real.
        clauses.push(
            "p.id IN (SELECT pt.photo_id FROM photo_tags pt JOIN tags t ON t.id = pt.tag_id
                      WHERE t.category = 'genre' AND t.name IN ('landscape'))"
                .into(),
        );
    }
    if f.person_id.is_some() {
        clauses.push(
            "EXISTS (SELECT 1 FROM faces fa
                     WHERE fa.photo_id = COALESCE(p.variant_of, p.id) AND fa.person_id = 1)"
                .into(),
        );
    }
    let where_sql = if clauses.is_empty() {
        "1 = 1".to_string()
    } else {
        clauses.join(" AND ")
    };
    let sql = format!(
        "EXPLAIN QUERY PLAN
         SELECT p.id FROM {GRID_FROM} WHERE {where_sql}
         ORDER BY p.capture_date DESC, COALESCE(p.variant_of, p.id) DESC, p.variant ASC
         LIMIT 20000"
    );
    let mut stmt = conn.prepare(&sql).unwrap();
    let rows: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    rows.join("\n")
}
