# What the catalogue costs at scale

Until 2026-09-07 the largest library this app had demonstrably run against was
**59 photos**, while the app is meant for people culling thousands per shoot. There
were no benchmarks of any kind. Scale behaviour was not "probably fine" — it was
unknown, and the guess was load-bearing for who the app could serve.

This was the measurement, and it was not reassuring. All four of its live
findings — near-duplicate grouping, the filter rail, grid paging
and the double-resolved search — have since been fixed; the table
below is kept as the dated baseline that drove them, and
**[After the fixes](#after-the-fixes-2026-09-08)** and
**[The rest of the filter rail](#the-rest-of-the-filter-rail-2026-09-22)** record
what they cost now.

## How to reproduce

```bash
cargo test --manifest-path src-tauri/Cargo.toml --release --test scale -- --nocapture
```

Runs at 10,000 photos by default, with no environment variable — deliberately, so
it cannot quietly stop running the way the 31 raw-fixture tests did.
For the full size:

```bash
SOURCELIGHT_SCALE=50000 cargo test --manifest-path src-tauri/Cargo.toml --release --test scale -- --nocapture
```

**Use `--release`.** A debug build compiles SQLite itself unoptimised, which
moves some of these by 50×, and it is not what ships.

The library is synthetic — `src-tauri/src/seed.rs` — with the proportions of a
real one: ~9 tag links per photo, 85% placed, a third with faces, ratings falling
off sharply above three stars, a CLIP embedding on every row. Fifty thousand rows
share a pool of derivatives on disk, because none of these code paths read sensor
data; the objection that 50,000 raws is 480 GB is correct and irrelevant.

**Measured on:** Windows 11, release build, warm page cache, app closed. The two
columns below are a single back-to-back pair, so they are comparable with each
other.

**Run-to-run variance is about ±40%** on this machine — `get_facets, one genre`
measured 3,294 ms in one run and 4,898 ms in the next. Absolute values are
therefore indicative, not precise, and no conclusion below rests on one. Where a
finding needs a sharp number it comes from a *within-run* comparison, which
machine load cannot distort.

**Planner statistics — the baseline flattered the app.** Until 2026-09-22 the
seeder ran a full `ANALYZE`, and the app never analysed anything. So every number
here before that date was measured on query plans a real catalogue could not get:
SQLite had real row counts in the benchmark and only its fixed default guesses in
the app. For a single-genre click that was worth about **4×** (100 ms benchmarked,
450 ms as shipped, same run). The app now refreshes its statistics with
`PRAGMA optimize` (`db::refresh_planner_stats` — on open, and after a scan or
tagging run), and the seeder calls the same function, so the two agree.

## The numbers

Median of five, after a warm-up. 50,000 is 5× the data, so **5× is linear**.

| | 10,000 photos | 50,000 photos | growth |
|---|--:|--:|:--|
| **Browsing** | | | |
| `get_photos`, no filter, first page (20,000 cap) | 223 ms | **1,003 ms** | 4.5× |
| `get_photos`, no filter, 200 rows | 47 ms | 205 ms | 4.4× |
| `get_photos`, one genre and nothing else | 58 ms | 319 ms | 5.5× |
| `get_photos`, every facet set | 8 ms | 61 ms | 7.5× |
| `get_facets`, no filter | 648 ms | **3,444 ms** | 5.3× |
| `get_facets`, one genre and nothing else | 810 ms | **4,898 ms** | 6.0× |
| `get_facets`, every facet set | 43 ms | 196 ms | 4.6× |
| serialise a full page to JSON for the bridge | 13 ms | 27 ms | 2.1× |
| **Search** | | | |
| `all_embeddings` | 60 ms | 255 ms | 4.3× |
| `search::rank` over every embedding | 6.7 ms | 36 ms | 5.3× |
| `caption_search` (FTS5) | 1.0 ms | 1.8 ms | 1.8× |
| `query_photos_ranked` over the candidates | 2.0 ms | 6.5 ms | 3.3× |
| `facets` over the semantic whitelist | 82 ms | 417 ms | 5.1× |
| **one natural-language search, end to end** | 299 ms | **1,386 ms** | 4.6× |
| **Cull and people** | | | |
| `all_photo_ids` | 38 ms | 196 ms | 5.2× |
| `photos_by_ids` for a full filmstrip | 179 ms | 461 ms | 2.6× |
| `list_people` | 0.1 ms | 0.5 ms | 5× |
| near-duplicate grouping, whole library — **before the fix** | ~21 s | ~8 min | quadratic |
| **near-duplicate grouping, whole library — after the fix** | **16 ms** | **67 ms** | linear |

Everything is now linear. The near-duplicate grouping *was* the one exception —
quadratic, minutes at 50,000 — and the fix was to compare only frames within
a burst-sized time window; the before/after rows above are the same operation.
The remaining costs are constants, which is the good news: constants are far
cheaper to fix than an algorithm.

Catalogue on disk: **74 MB** at 10,000, **370 MB** at 50,000 — roughly 100 MB of
that is CLIP embeddings, which `all_embeddings` loads into memory in full.

## After the fixes (2026-09-08)

The three browsing findings are addressed. Same machine, same synthetic 50,000,
release, warm cache — but a different afternoon, so read these against the ±40%
note above, not to three significant figures. The mechanism, not the millisecond,
is the point, and each is pinned by a query-plan assertion that a stopwatch
cannot.

| operation (50,000 photos) | baseline | now | why |
|---|--:|--:|---|
| `get_facets`, one genre and nothing else | 4,898 ms | **359 ms** | tag filter drives from the tag index, not a full scan |
| `get_photos`, no filter, 200 rows | 205 ms | **3 ms** | grid sort read from an index; a LIMIT now stops early |
| `get_photos`, no filter, first page (20,000 cap) | 1,003 ms | **300 ms** | same index, no temp b-tree — but still fetches 20,000 rows |
| `get_photos`, one genre and nothing else | 319 ms | **64 ms** | the tag filter again |
| `get_facets`, no filter | 3,444 ms | **1,709 ms** | see below — only half addressed |

Three things this table must not oversell:

- **The unfiltered rail is only half fixed.** `get_facets, no filter` fell to
  ~1.7 s, but the index did not do that — the residue is grouped counts over
  430,000 tag links, run once per facet, and that is untouched. It is the
  one-pass / return-facets-with-photos work, not a missing index, and the issue was left
  open for it. *(Since fixed — see
  [The rest of the filter rail](#the-rest-of-the-filter-rail-2026-09-22).)*
- **The busy multi-filter facet case rose**, 196 ms → ~511 ms. The uncorrelated
  semijoin materialises its id sets where the old correlated `EXISTS` was narrowed
  by the date index first. The plan stays fully index-driven (a bloom-filtered
  semijoin, no scan — verified at 50,000), so it is linear, not a landmine; it is
  a real trade for turning the 4.9 s single-genre click into a third of a second.
- **Fixing the double-resolved search does not move a row here.** The double resolve is collapsed at the command
  layer — `get_facets` reads the id list `get_photos` just resolved — and this
  harness reconstructs the search at the `db`/`search` level, below where the
  cache sits, so it cannot see it. The saving is one CLIP text encode and one
  fusion per interaction; the row that *would* show it is the one the harness
  cannot run.

## The rest of the filter rail (2026-09-22)

The first half of the filter-rail fix gave the tag filter an index. What it left was the
*counting*: every facet joined its tag links back through `photos` and took
`COUNT(DISTINCT p.id)` — a `photos` probe per link plus a temp b-tree for the
`DISTINCT`, over 430,000 links, once per facet. Three changes:

- **Count from the links, not through `photos`.** `photo_tags` has
  `PRIMARY KEY (photo_id, tag_id)` and `tags` has `UNIQUE(name, category)`, so a
  plain `COUNT(*)` grouped by tag id already counts distinct photos; foreign keys
  cascade a deleted photo's links away. When there is a filter it is applied as a
  semijoin on the photo id — evaluated once per photo, not once per tag link.
- **Count sibling facets together when they share a clause.** A facet's clause
  differs from the full filter only while that facet is active, so every inactive
  one is counted in the same statement. The rail as it opens — nothing selected —
  is one query for all three tag facets.
- **A covering tag index** (migration 15). `idx_phototags_tag` held `tag_id` and
  the rowid, so each link cost a second seek just to read `photo_id`;
  `idx_phototags_tag_photo (tag_id, photo_id)` replaces it (~5.5 MB at 50,000).

Plus the statistics fix described [above](#how-to-reproduce), which is worth
more than any of them for a real catalogue.

**Measured within a single run**, old and new implementations interleaved on the
same seeded 50,000-photo catalogue, statistics current — the machine was under
unrelated build load that afternoon, which is exactly the case a within-run
comparison survives and an absolute figure does not. The new output was compared
with the old for every case below and matched exactly.

| `get_facets` at 50,000 photos | old | new | |
|---|--:|--:|--:|
| no filter — the rail as it opens | 1,465 ms | **30 ms** | 49× |
| one genre | 298 ms | **73 ms** | 4.1× |
| one place | 1,419 ms | **18 ms** | 79× |
| two subjects | 1,241 ms | **200 ms** | 6.2× |
| a date range | 773 ms | **62 ms** | 12× |
| every facet set (the "busy" case) | 389 ms | **182 ms** | 2.1× |
| one person | 1,831 ms | **265 ms** | 6.9× |
| literal text search (`LIKE`) | 3,031 ms | **645 ms** | 4.7× |
| semantic whitelist + one genre | 54 ms | 59 ms | — |

The busy case is back under the 196 ms it measured before the first half of the fix
traded it up to ~511 ms.

**Regressed, then fixed again.** Two counts added to the rail after this
table, the rating facet and the Missing count, read every photo row, and with
them the no-filter case had grown to **275 ms**. Migration 43 gives each an index
(`idx_photos_rating`, and a partial `idx_photos_missing`), plus one for the
folder counts, and `tests/scale.rs` now asserts all three plans. Same machine and
seeded 50,000-photo catalogue, release, one run each:

| at 50,000 photos | before | after |
|---|--:|--:|
| `get_facets`, no filter | 275 ms | **24 ms** |
| `get_facets`, one genre | 129 ms | **45 ms** |
| `get_facets`, every facet set | 216 ms | 202 ms |
| `get_facets`, typed text | 976 ms | 942 ms |

Typed text is still the slow case, for the reason given below.

**Typed text, halved.** The search clause now finds its matches once per
statement, as a list, from `idx_photos_text` (migration 44: filename, caption
and `variant_of`), so it tests small index entries instead of photo rows. Each
row carries a ~3 KB embedding, and every one of the rail's statements read them
all. Same machine and catalogue, release:

| typed text at 50,000 photos | before | after |
|---|--:|--:|
| `get_photos` | 279 ms | **117 ms** |
| `get_facets` | 942 ms | **404 ms** |

The facets still run the list once per statement, about ten times. Finding it
once per command, or taking the embeddings off the photo row, is what's
left.

**The embedding off the photo row.** Migration 45 moves each photo's
search vector into `photo_embeddings` and drops the column, so a photo row is a
few hundred bytes instead of filling a 4 KB page by itself. Every read that a
covering index doesn't serve gets cheaper at once. Same machine and seeded
50,000-photo catalogue, release, one run each. The seed writes 2 KB vectors
where the app stores 3 KB, so a real catalogue gains more than this:

| at 50,000 photos | before | after |
|---|--:|--:|
| `get_photos`, no filter, first page | 177 ms | **88 ms** |
| `get_photos`, typed text | 117 ms | **73 ms** |
| `get_facets`, typed text | 404 ms | **208 ms** |
| `get_facets`, every facet set | 197 ms | **40 ms** |
| `get_extra_facets`, no filter | 296 ms | **155 ms** |
| insights, no filter | 273 ms | **133 ms** |
| facets over the semantic whitelist | 213 ms | **44 ms** |
| one natural-language search, end to end | 848 ms | **485 ms** |
| `all_photo_ids` | 110 ms | **2 ms** |
| `photos_by_ids` for a full filmstrip | 305 ms | **75 ms** |

Reading every vector (`all_embeddings`, 150 ms) is unchanged: it was always a
pass over the vectors themselves.

**What is left, deliberately.** Literal text search is the slowest facet case at
~0.65 s: `LIKE '%…%'` over tag and place names cannot use an index, and the six
facet statements each re-evaluate it. Evaluating a filter's shared part once and
reusing it across the facets would fix that, and would be the same work as one
command returning photos and facets together (the second half of the filter rail's original
list). Neither is needed to make the rail usable at 50,000 any more, so both are
left for when a measurement says otherwise.

## What this found

### 1. Near-duplicate grouping was quadratic and froze the app — fixed

**Original finding.** `search::group_near_duplicates` compared every photo
against every existing group seed — `O(n·groups)`. Near-duplicate means *the same
frame twice*, at a cosine of 0.92, and two different photographs of the same
mountain are nowhere near that, so almost every photo failed every seed, started
its own group, and `groups` converged on `n`. That made it `n²/2` cosine
comparisons of 512 floats: doubling the input cost ~4×, and 50,000 photos took
about **eight minutes**, on the UI thread, from three different buttons (Review
duplicates, Stack bursts, Select duplicates) with no cancel and no progress. This
is the cull assistant, one of the features the app is built around.

**Fix.** Grouping now only compares frames within a 60-second window of each
other — which is what a burst physically is. The whole library at 50,000 dropped
from ~8 minutes to **67 ms**, scaling linearly (1.9× for 2× the input). It also
fixed a latent false-positive: two look-alike frames shot on different days used
to group as duplicates and no longer do. See the follow-up in `search.rs` and the
`near_duplicate_grouping_stays_near_linear` guard in `tests/scale.rs`.

### 2. The filter rail costs seconds per click — fixed

**Original finding.** `get_facets` after clicking a single genre: **4.9 seconds**
at 50,000 (3.3 s in another run — see the variance note; either way it is
seconds, not milliseconds).

Facet counts are sibling-aware — each is computed with its own constraint lifted
— which is a genuinely good interaction design and the reason options never
collapse to zero. It also means one click runs four grouped counts plus two
totals over 430,000 tag links. Note that `get_photos` under the same filter is
319 ms: **the facets are fifteen times the cost of the photos.**

Two things make it worse than it needs to be:

- **A tag filter on its own has no index.** The plan is `SCAN p` plus a
  correlated `EXISTS` per row, then a temp b-tree for the sort. The "every facet
  set" row is *fast* (196 ms) only because its date range narrows through
  `idx_photos_capture` first, so the scan never happens. The cheap-looking case
  is the misleading one.
- **`get_facets` is a second round trip.** `reload` fires `getPhotos` and
  `getFacets` together ([`store/library.ts`](../src/store/library.ts)), so the whole filter
  is evaluated twice per interaction.

**Fix (the first half).** The tag membership test was a correlated `EXISTS`, so
the planner ran it once per row of a full `photos` scan. It is now an uncorrelated
semijoin (`p.id IN (SELECT …)`), and `idx_tags_category_name` (migration 13) lets
it begin at the small side — resolve the tag, gather its photos through
`idx_phototags_tag`, probe `photos` by primary key. `EXPLAIN QUERY PLAN` shows no
`SCAN` for a tag filter; the single-genre click fell **4.9 s → ~0.36 s**. Guarded
by `the_filters_still_use_their_indexes` (the plan) and
`a_tag_filter_counts_the_intersection_and_lifts_its_own_facet` (the counts, so the
rewrite cannot silently change them).

**Fix (the second half).** The index did nothing for the *unfiltered* rail —
grouped counts over 430,000 tag links, each joined back through `photos` — which
still cost ~1.7 s. Counting straight from the links, batching sibling facets that
share a clause, and a covering tag index took it to ~30 ms; see
[The rest of the filter rail](#the-rest-of-the-filter-rail-2026-09-22). Guarded by
`each_active_facet_lifts_only_its_own_constraint`,
`a_deleted_photo_leaves_the_facet_counts` and
`a_tag_lookup_is_answered_from_the_index_alone`.

### 3. Paging pays a full sort before the first row — fixed

At 20,000 photos, **one row costs 75.0 ms and two hundred rows cost 91.9 ms** —
200× the data for 1.2× the time. Both pay the same fixed sort before returning
anything.

That pair is measured inside a single test run, by
`paging_pays_a_full_sort_before_the_first_row`. It is quoted in preference to the
ratio between the two columns above because a within-run comparison is not at the
mercy of what else the machine was doing.

The grid orders by `capture_date DESC, COALESCE(variant_of, id) DESC, variant
ASC`, so a virtual copy sits next to the photo it came from. That behaviour is
right and worth keeping — but `COALESCE(variant_of, id)` is an expression no
index covers, and the plan confirms it: `USE TEMP B-TREE FOR LAST 2 TERMS OF
ORDER BY`.

The consequence that matters: **lowering `PAGE` from 20,000 would not help.**
That is the obvious fix and it is the wrong one.

**Fix.** SQLite indexes expressions and allows the rowid alias `id` inside one, so
`idx_photos_grid` (migration 14) covers all three ORDER BY terms directly — no
generated column (which cannot reference the rowid) and no maintenance triggers.
The plan is now `SCAN … USING COVERING INDEX idx_photos_grid` with **no temp
b-tree**, so a `LIMIT` reads that many ordered rows and stops: **200 rows at
50,000 went 205 ms → ~3 ms**, and the full 20,000-row page 1,003 ms → ~300 ms (it
still fetches 20,000 rows; the sort is what went away). The date filter now prefers
this index too — its leading column is `capture_date`, so one covering pass serves
the range and the sort. The old test asserted the *bad* behaviour so it could not
silently vanish; per the issue it is inverted, not deleted, and now gates on the
plan (`paging_reads_the_grid_sort_from_an_index_not_a_full_sort`).

### 4. Every natural-language search is computed twice — fixed

`get_photos` and `get_facets` are separate Tauri commands, and each calls
`fused_search_ids` from scratch ([`commands/browse.rs`](../src-tauri/src/commands/browse.rs)). So one
search runs the CLIP **text encoder** twice, loads all ~100 MB of embeddings
twice (255 ms each), ranks twice, and computes the facets twice.

Measured at **1.39 s** at 50,000 — and that excludes both text encodes, which
this harness cannot run.

**Fix.** `fused_search_ids` now memoises the most recent resolve, stamped with a
generation counter that `scan_library`, `index_for_search` and `start_captioning`
bump. The two commands are sync, so Tauri runs them back to back and `get_facets`
reads the id list `get_photos` just resolved — one CLIP text encode and one fusion
per interaction instead of two, with no API or frontend change. It does not show
in the table above: this harness reconstructs the search at the `db`/`search`
level, below the command layer where the cache lives.

### 5. What is genuinely fine

Worth saying, because most of it is:

- **Every filter that has an index uses it.** Dates ride `idx_photos_grid` (its
  leading column is `capture_date`, so it serves the range and the grid sort in
  one covering pass since the paging fix), people hit `idx_faces_photo` behind a Bloom
  filter, tag filters drive from `idx_phototags_tag_photo` — covering, so they
  never read the table (migration 15) — since the filter-rail fix. Asserted in
  `the_filters_still_use_their_indexes`, which is the part of this suite worth
  gating on — a plan is machine-independent in a way a stopwatch is not.
- **FTS5 caption search is essentially free** — 1.8 ms at 50,000.
- **Cosine ranking is not the bottleneck.** 36 ms over 50,000 vectors.
- **The IPC bridge is not the bottleneck either** — 27 ms to serialise a full
  page. Worth having measured, because it was the intuitive suspect, and it is
  the reason lowering `PAGE` is not the fix it looks like.
- **The UI is already virtualised** — grid, cull survey, cull filmstrip and
  Lightbox filmstrip all use `@tanstack/react-virtual`. DOM node count is not the
  problem at any of these sizes.
- **`list_people` is free.** 0.5 ms at 50,000.

## The UI at 20,000 photos (2026-09-23)

The catalogue numbers above stop at the IPC boundary. This is the other side:
how the React UI copes with a library at the app's page cap. It was measured with
the in-browser harness (`src/dev/mockBackend.ts`), set to
`localStorage["mock:photoCount"] = "20000"`: 20,000 fake photos over six years
of capture dates, with the grid grouped by date. A `longtask` observer records
every main-thread block over 50 ms, which is the threshold at which input starts
to feel delayed.

**Production build** (React's optimised bundle, with the harness forced on for
the measurement only), Chromium, 1500×940:

| Action | Time to result | Main-thread tasks over 50 ms |
| --- | --: | --: |
| Browse → Cull | 21 ms | 0 |
| Cull: step 10 frames | 6 ms | 0 |
| Cull: loupe → survey | 52 ms | 0 |
| Cull → Browse (date-grouped grid remounts) | 116 ms | 0 |
| Grid: scroll 30 screens | — | 0 |
| Open the Lightbox (20,000-frame filmstrip) | 21 ms | 0 |

**Development build** (`npm run dev`, StrictMode) at the same size shows one
100 ms task returning to Browse and one 74 ms task opening the survey; at 48
photos, none. That is development React: StrictMode renders twice, and the
unoptimised build is several times slower. It is worth knowing, because
`tauri dev` behaves like this, but it is not what ships. Grouping 20,000 cells
into date rows (`buildGridRows`) costs 9 ms a call, and is the largest single
piece.

Cull → Browse takes 116 ms of wall time without blocking: the grid measures its
own width (a `ResizeObserver`) before it places a cell, so it waits a frame or
two for layout rather than computing.

So at the page cap the UI stays responsive. WebView2's own painting, with real
512 px thumbnails, is measured next.

## The real app at 50,000 photos (2026-09-23)

The harness above runs in Chromium with tiny SVG thumbnails. This is the app
itself: a release build, WebView2, a seeded 50,000-photo catalogue, and 5,820
distinct real 512 px thumbnails (so no image repeats inside any one scroll and
the decode cache cannot flatter it). `scripts/bench-webview.mjs` attaches to the
webview over its debugging port and drives it the way a person would: wheel
scrolling, the mode rail, End and Enter on the grid. Window 1500×940 (grid area
1136×818), warm page cache, two runs each; the ranges are those two runs.

| Action | Before | After async `thumb://` | Main-thread tasks over 50 ms |
| --- | --: | --: | --: |
| Reload → first screen of thumbnails decoded | 879–885 ms | 873–896 ms | 0 |
| Wheel-scroll 30 screens: frame median / p95 | 8 / 9–16 ms | 8 / 9 ms | 0 |
| … frames over 33 ms | 0–1 of ~480 | 0 of ~445 | |
| Jump to the last photo: thumbnails decoded | 615–1,419 ms | **191–215 ms** | 0 |
| Browse → Cull | 20–24 ms | 23–24 ms | 0 |
| Cull → Browse (grid remounts) | 37–43 ms | 43 ms | 0 |
| Browse → People (cover thumbnails) | 328–331 ms | **138–145 ms** | 0 |
| Open the Lightbox (20,000-frame filmstrip) | 163–166 ms | 168–170 ms | 0 |

Times run from the input event to the last DOM change; "thumbnails decoded" is
until every image inside the grid's viewport has decoded.

**What it found.** `thumb://` was a *synchronous* scheme, and WebView2 runs a
synchronous handler on its own UI thread — so a screen's worth of thumbnails
was read one file at a time. Resource timing showed a batch of ~50 requests
going out together and coming back about 3.5 ms apart, 140–180 ms for the
batch. It is now registered asynchronously on Tauri's blocking pool, like the
`full` scheme beside it: the same batch completes in 46 ms, and the slowest
case, a jump to the far end of the library, is 3–6× faster.

**What is left.** Opening the library costs about 650 ms before the first grid
cell exists (the DOM is ready at ~30 ms). That is the first page, 20,000 rows,
fetched and sent over IPC — the `get_photos` first-page row above, ~300 ms of
query — and it happens once per launch or filter change, not per scroll.
Scrolling, workspace switches and the Lightbox never blocked the main thread.

### How to reproduce

Nothing here touches the real library. A development build (debug, or built
with `--features dev-tools`) opens `SOURCELIGHT_DATA_DIR` as its whole data
directory when that is set, and `seed_library --app` writes there too.

```bash
export SOURCELIGHT_DATA_DIR=/tmp/big-library
mkdir -p $SOURCELIGHT_DATA_DIR/grid-thumbs
# Distinct thumbnails: copy the real ones (the *_t.jpg files in the app's
# thumbs folder) under several prefixes until there are a few thousand.
cargo run --manifest-path src-tauri/Cargo.toml --features dev-tools --bin seed_library -- \
    --photos 50000 --app --yes --derivatives $SOURCELIGHT_DATA_DIR/grid-thumbs
npx tauri build --no-bundle --features dev-tools

# Launch with WebView2 isolated from the real profile and a debugging port.
WEBVIEW2_USER_DATA_FOLDER=$SOURCELIGHT_DATA_DIR/webview \
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 \
    src-tauri/target/release/sourcelight.exe &
# A fresh catalogue asks for the licence first: accept it in the window.
node scripts/bench-webview.mjs
```

The thumbnails must be inside the data directory: the app serves nothing from
outside it.

## Slower machines (2026-10-11)

Everything above is the catalogue. This is the on-device AI on the machines
most people own: a four-core laptop, integrated graphics, 8–16 GB of memory.
It measured each model alone and through the app's own code, fixed what that
found, and measured again.

### How it was measured

`src-tauri/src/bin/probe_ai.rs`, a release build (`--features ai,dev-tools`,
without the shipped profile's LTO, which the hot paths, ONNX Runtime and the
image crates, do not depend on). One mode per process, so each peak-memory
figure belongs to one model.

```bash
cargo build --manifest-path src-tauri/Cargo.toml --release --features ai,dev-tools --bin probe_ai
probe_ai adapters                                    # the GPUs DirectML can be given
probe_ai raw <model.onnx> --ep cpu --threads 4 --no-spin  # one model, these options
SOURCELIGHT_AI_DEVICE=cpu probe_ai index <models> <folder of 1600 px JPEGs> --n 300
SOURCELIGHT_AI_DEVICE=1   probe_ai faces <models> <folder> --n 150   # adapter 1
probe_ai subject|click|erase|denoise <models>
probe_ai jitter --secs 15                            # the window's stand-in
```

- **The machine:** Ryzen 7 7800X3D (8 cores, 16 threads), RTX 5070 Ti
  (16 GB), the CPU's own Radeon graphics (2 compute units, weaker than most
  laptops', as the denoise spike found), 32 GB, Windows 11, the shipped ONNX
  Runtime 1.22 with DirectML.
- **Shared and loaded.** Three other agents compiled and tested on it
  throughout. Every comparison ran before and after back to back,
  interleaved, at least twice; figures are medians of the runs' medians.
  Absolute times are pessimistic; the comparisons are the point.
- **Slower hardware, simulated.** `SOURCELIGHT_AI_DEVICE` (honoured by
  a development build only) keeps every model on the CPU (`cpu`), as on a PC
  with no usable graphics, or puts them on another adapter (`1`, the
  Radeon). **"4 cores"** is the process pinned to four physical cores
  (`start /affinity 55`). Told nothing, ONNX Runtime still starts one thread
  per physical core of the whole machine, eight, on those four: the case of
  a PC whose cores something else is using. The model table gives it four
  threads, as on a real four-core PC; the before-and-after tables leave it
  as the app did, and say so.
- **Inputs:** 1600 px JPEG previews made from the 13 CC0 raw fixtures'
  embedded previews (crops, flips, both orientations), the same with one to
  three LFW faces pasted in at 220–420 px, and a library of 300, a third
  with people, as `seed.rs` has it. Nothing from the owner's library.

### What each model costs

One run of the model alone (`probe_ai raw`), median, without spinning
(finding 4). Peak is the process's peak working set, on the CPU unless said.

| model | 8 cores, 16 threads | 4 cores | Radeon iGPU | RTX 5070 Ti | peak |
|---|--:|--:|--:|--:|--:|
| SigLIP 2 vision, fp16 (search index) | 83 ms | 117 ms | 160 ms | 7.3 ms | 503 MB |
| SigLIP 2 vision, fp32 | 49 ms | 79 ms | 156 ms | | 399 MB |
| SigLIP 2 text, fp16 (a search) | 47 ms | 59 ms | 46 ms | 3.8 ms | 949 MB |
| YuNet (finding faces) | 2.6 ms | 3.3 ms | 8.2 ms | 0.7 ms | 47 MB |
| face mesh (three times a face) | 6.8 ms | 6.5 ms | 7.9 ms | 2.1 ms | 44 MB |
| AuraFace (twice a face) | 30 ms | 49 ms | 42 ms | 4.7 ms | 505 MB |
| SAM 2 encoder (Click to Select) | 703 ms | 842 ms | 1,705 ms | 27 ms | 698 MB |
| BiRefNet fp32 (Select Subject) | 3,558 ms | 3,989 ms | out of memory | 1,526 ms | **6.3 GB**; 23 GB committed on the RTX |
| BiRefNet fp16 | 9,246 ms | | 7,477 ms | 88 ms | 0.5 GB on the RTX |
| LaMa fp32 (magic erase, a window) | 1,358 ms | 1,643 ms | fails | fails | 585 MB |
| NAFNet fp16 (denoise, a tile) | 1,090 ms | 1,252 ms | 622 ms | 6.7 ms | 982 MB |

### What it found and changed

**1. Magic erase never worked on Windows with a GPU.** DirectML accepts
LaMa's graph and fails its first pass ("The parameter is incorrect", at a
MatMul in the Fourier units) on the RTX and the Radeon alike. Erase took the
GPU on Windows, so its warm-up failed quietly and every erase with an error.
It now runs on the CPU on every platform, as a Mac already did.

**2. Select Subject was in the wrong precision for a GPU.** BiRefNet was
fp32 everywhere. On DirectML that took 1.5 s on the RTX with 23 GB of memory
committed, reset the GPU on a second run of real photos, and ran out of
memory on the Radeon, so on an integrated-graphics PC Select Subject failed
every time. The fp16 export from the same pinned revision (MIT, 115 MB)
runs in 88 ms in half a gigabyte, and runs on the Radeon. The CPU runs fp32
2.6 times faster than fp16, so a PC with a DirectML GPU now downloads fp16,
and the CPU and a Mac keep fp32 (`ai_models::Variant`). And a GPU that loads
a model but then fails a pass hands it to the CPU instead of failing.

Agreement with fp32 on the CPU, the reference, over the same photos, masks
cut at 50%:

| | IoU, mean (worst) | coverage difference, mean (worst) |
|---|--:|--:|
| fp16 on the RTX, 30 photos with people | 0.998 (0.988) | 0.09 (0.57) of 255 |
| fp16 on the Radeon, the same 30 | 0.997 (0.964) | 0.12 (0.92) of 255 |
| fp16 on the RTX, 30 scenes with no clear subject | 0.989 (0.898) | 0.08 (1.23) of 255 |

The lowest IoUs are scenes whose "subject" is a few faint fragments, where a
handful of pixels either side of 50% move the ratio; the coverage never
differs by more than 1.3 levels in 255 on average.

**3. The image work around the models took longer than some models.**
Preparing Select Subject's 1024 px frame took 106 ms and turning its matte
back into coverage 46 ms, around an 88 ms model, and Click to Select
prepares the same frame before every encode. The search index spent 15 of
its 23 ms a photo on the RTX decoding and resizing, and the face pass 27 ms
finding faces in a photo with none, nearly all of it resizing for the
detector. Much of it was `f32::round`, which x86-64's baseline cannot do in
an instruction, so each call went into the C library, several times a
pixel; the rest, the image crate's generic resize. `mask::round_to_byte`
rounds in two instructions, `resize_coverage` and `raster::resize_triangle`
work their weights out once per row and column, and Select Subject's
preparation resizes three channels in one pass. Each gives the same bytes
as the code it replaced, which tests check. Preparing for Select Subject
now takes 10 ms and post-processing 6; finding faces in a photo with none,
6 ms on the RTX and 9 on four cores.

**4. Spinning threads fight everything else.** By default ONNX Runtime's
threads spin while they wait for work: quickest on an idle machine, ruinous
on a busy one. SigLIP 2's vision tower in fp16, the shared machine:

| | median | p90 |
|---|--:|--:|
| 4 cores, 4 threads, spinning (as shipped) | 175 ms | 348 ms |
| 4 cores, 4 threads, not spinning | 125 ms | 127 ms |
| 4 cores, 8 threads, spinning | 858 ms | 1,142 ms |
| 4 cores, 8 threads, not spinning | 141 ms | 171 ms |
| 16 threads, spinning | 131 ms | 216 ms |
| 16 threads, not spinning | 91 ms | 104 ms |
| 16 threads, not spinning, below normal priority | 91 ms | 99 ms |
| 4 cores, 4 threads, spinning, the machine quieter | 111 ms | 112 ms |
| 4 cores, 4 threads, not spinning, the machine quieter | 117 ms | 130 ms |

With little else running, spinning is about 5% quicker (the last two
rows); with anything else running it is 40% slower with nearly three times
the tail, and with more threads than free cores, six times slower. A PC in
use is the second case far more often than the first. The big models were
all faster without it on the loaded machine (SAM 2's encoder 844 → 703 ms,
LaMa 2,100 → 1,358, NAFNet 1,394 → 1,090 on sixteen threads); the smallest
lost a little, YuNet 0.3 ms and the face mesh 1.3 ms a run. No session of
the app's own spins now. Leaving a core free (three threads on four cores)
cost 20%, so the count stays ONNX Runtime's.

**5. Automatic analysis gives way.** Its model threads, and the loop that
decodes each photo and drives them, run below normal priority on Windows.
When nothing else wants the cores it costs nothing: 91 ms either way on
sixteen threads above, and 79.4 against 79.8 ms for fp32 on four quiet
cores.
`probe_ai jitter` stands in for the window: sixty times a second a slice of
work about 2 ms long on an idle core, pinned to the same four cores as an
index pass of the library on the CPU:

| | the slice, median | p95 | p99 | slices over 16 ms, of 900 | waking late, p99 |
|---|--:|--:|--:|--:|--:|
| nothing else running | 2.2 ms | 2.5 ms | 2.7 ms | 0 | 0.5 ms |
| beside an index pass, before | 2.1 ms | 19.3 ms | 46.3 ms | 27 | 82 ms |
| beside an index pass, after | 2.2 ms | 2.5 ms | 5.7 ms | 3 | 2.4 ms |

Medians of three 15-second runs. Before, one frame in 33 missed its 16 ms
and the window's thread woke up to 80 ms late; after, it is all but the
machine at rest, while the pass beside it ran nine times as fast.

**6. A PC without a GPU runs SigLIP 2's vision tower in fp32.** fp16 on the
CPU is ONNX Runtime converting every layer; fp32 from the same pinned
revision (Apache-2.0) runs 1.5 times as fast on four cores and 1.7 on
sixteen (79 against 117 ms, 49 against 83), in 100 MB less memory,
for 186 MB more to download, and such a PC now downloads it. Over the
300-photo library its embeddings and the fp16 CPU path's agree to a cosine
of 0.999998 on average (0.99998 at worst); each photo's nearest neighbour is
the same for 299 of 300, and its ten nearest 99.8% the same. (fp16 on
DirectML and fp32 on the CPU agree to 0.99998, as before.)

**7. Analysis said it paused in Develop, and did only between passes.** A
pass already going, a whole library on a first run, carried on beside the
editor, which on integrated graphics wants the same GPU. The index and face
passes now wait between photos while Develop is open.

**8. Smaller things.** The face models are freed when a face pass ends
(about half a gigabyte on the CPU). The vision model for Detailed
descriptions recommends its 2B on a PC with no usable graphics, where it
tags in 4.7 s against the 4B's 7.2. A model download that breaks off
part-way asks for the rest instead of starting again. And Activity says how
long a pass has left, from its pace on this computer (`lib/timeLeft.ts`).

### A library of 300, before and after

`probe_ai index` and `faces` over the same previews, each pass as the app
runs it (read, decode, the models, per photo). *Before* is origin/main,
*after* this branch; photos a minute, and the two passes together as the
time a first run of 10,000 photos takes.

| where | search index | face pass | 10,000 photos, both | peak (index) |
|---|--:|--:|--:|--:|
| CPU, 16 threads | 537 → **741** a minute | 464 → **814** | 40 → **26 min** | 512 → 408 MB |
| CPU, 4 cores, as the app left ONNX Runtime | 77 → **668** | 86 → **559** | 4 h 7 min → **33 min** | 511 → 407 MB |
| Radeon iGPU | 340 → **354** | 470 → **550** | 51 → **47 min** | |
| RTX 5070 Ti | 2,633 → **3,681** | 1,267 → **2,152** | 12 → **7 min** | |

Per photo: on four cores the index went from 770 ms to 90, and the face
pass from 90 ms (p90 2.2 s) to 17 (p90 391 ms). The four-core "before" is
the case of a PC with fewer free cores than ONNX Runtime assumes. On a real
four-core PC with nothing else running, the model table puts the index
nearer 127 ms a photo before (fp16 spinning, 111 ms, and 16 ms of decoding
and resizing) against 90 after.

### Interactive tools, before and after

Median of each run's passes, two runs each (Select Subject on a 1024×683
frame; Click to Select's encode and then a click; one 512 px erase window;
denoise of a 1024×1024 image, nine tiles).

| tool | RTX 5070 Ti | Radeon iGPU | CPU, 4 cores |
|---|--:|--:|--:|
| Select Subject | 1,690 → **106 ms** | fails → **8.9 s** | 10.6 → **4.0 s** |
| … peak memory | 9.9 → **0.46 GB** | | 6.3 GB either way |
| Click to Select, encode | 132 → **38 ms** | 1,770 → **1,660 ms** | 3,409 → **867 ms** |
| … then a click | 26 → **9 ms** | 69 → **50 ms** | 163 → **28 ms** |
| Magic erase, a window | fails → **1.2 s** | fails → **1.8 s** | 6.9 → **1.7 s** |
| AI denoise, 1024 × 1024 | 74 → 74 ms | 5.66 → 5.64 s | 39 → **11.6 s** |

On the GPUs the models are the same, so what moved is around them, and
where a model failed. The CPU column is the four-core case above, so most
of its gain is spinning; with four threads and nothing else running it
would be nearer the model table's.

### Looked at and left

- **ONNX Runtime's thread count.** One per physical core is right; without
  spinning, more threads than cores no longer hurt (finding 4), and fewer
  cost speed. Not changed.
- **Graph optimisation level.** fp32 SigLIP at full optimisation was within
  noise of basic (53 against 57 ms); fp16 loads at full on ORT 1.22 on the
  CPU, unlike 1.24.4 in the spike, at the same speed. Not changed.
- **Memory patterns and the arena for BiRefNet on the CPU.** Memory
  patterns off took the peak from 6.3 to 5.2 GB and cost 11%; the CPU arena
  on took it to 10.9 GB. The 6.3 GB is the model's activations at 1024 px:
  on an 8 GB PC with no usable GPU, Select Subject will page. The cure is a
  smaller input, which needs a re-exported model (an open decision).
- **An integrated GPU against the CPU.** On this 2-CU Radeon the CPU wins
  most models (SigLIP 2: 160 ms against 49 in fp32 on eight cores;
  SAM 2: 1.7 s against 0.7), and DirectML's cost a run swamps the tiny face
  models (YuNet 8.2 ms against 2.6). Laptop graphics (Iris Xe, Radeon 780M)
  are several times stronger, and nothing measured here says where the line
  falls, so DirectML stays the choice wherever there is a GPU. A real laptop
  iGPU is the next thing to measure.
- **Caching optimised graphs on disk.** LaMa takes 4.7 s to load on the CPU
  (17,480 nodes), but the app loads it while a selection is being drawn, so
  nobody waits for it. Not done.
- **AuraFace without its mirror image.** Half the cost of a face (2 × 30 ms
  on sixteen threads), but grouping would have to be measured on labelled
  faces first, and a library would mix two kinds of embedding. Not done.
- **NAFNet in fp32 on a PC without a GPU.** Twice as fast on the CPU (564
  against 1,620 ms a tile in the spike), but only Qualcomm's zip has it, and
  hosting it is an open decision. Not done.

## What this deliberately does not measure

- **Scan throughput**, which genuinely needs the bytes. Time it against a real
  library and extrapolate. (The search index and face passes are timed on
  slower hardware in [Slower machines](#slower-machines-2026-10-11).)
- **A spinning disk or a network drive.** The WebView2 numbers above read
  thumbnails from a warm SSD. The async scheme reads them side by side, which
  helps a slow disk more, not less — but it was not measured on one.
- **Cold cache.** These are warm-cache numbers; a first open after boot will be
  worse, and a 370 MB catalogue does not fit anyone's spare RAM by accident.
