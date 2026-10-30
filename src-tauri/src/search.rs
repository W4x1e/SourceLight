//! Semantic search over SigLIP 2 image embeddings.
//!
//! Each photo carries a SigLIP 2 *vision* embedding (computed by the built-in
//! tagger's model — [`crate::tagger::builtin`]); a text query is embedded by the
//! matching SigLIP 2 *text* encoder, and photos are ranked by cosine similarity. The
//! two encoders share an embedding space, so "a dog on a beach" lands near photos
//! that look like that — true visual search, entirely local, with no VLM needed.
//!
//! This module is the storage + ranking maths, kept free of any `ort`/model
//! dependency so it is fully unit-tested; the embedding itself (the one part that
//! needs the model) lives behind the `ai` feature in the tagger.

/// Serialises an embedding as raw little-endian `f32` bytes for the
/// `photos.embedding` blob. Fixed-width and self-describing by length, so
/// decoding needs no header.
pub fn encode_embedding(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for &x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

/// Reads an embedding blob back. Returns `None` if the byte length is not a
/// whole number of `f32`s — a corrupt or foreign blob is skipped, not trusted.
pub fn decode_embedding(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

/// Cosine similarity. The embeddings are stored L2-normalised, so this is a plain
/// dot product; a length guard keeps a mismatched pair from panicking (it scores
/// 0 and drops out of the ranking rather than taking the process down).
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    crate::vector::dot(a, b)
}

/// Ranks `candidates` (id, embedding blob) against a query embedding, returning
/// the best `limit` as (id, score), highest first.
///
/// Only candidates scoring `>= min_score` are kept, so a "find similar" caller
/// gets the photos that are actually alike rather than the whole library
/// reordered — pass `0.0` to rank everything (cosine is 0 for orthogonal and
/// negative for opposed, so `0.0` keeps every non-opposed candidate). Blobs that
/// don't decode or don't match the query width are skipped. Ordering is stable
/// on ties by id, so repeated searches don't shuffle equal-scoring photos.
pub fn rank(
    query: &[f32],
    candidates: &[(i64, Vec<u8>)],
    limit: usize,
    min_score: f32,
) -> Vec<(i64, f32)> {
    let mut scratch = Vec::new();
    let scored = candidates
        .iter()
        .filter_map(|(id, blob)| Some((*id, score_blob(query, blob, &mut scratch)?)))
        .collect();
    best(scored, limit, min_score)
}

/// The cosine of `query` with a stored embedding blob, decoded into `scratch`
/// so that scoring a whole library allocates once. `None` when the blob is not
/// an embedding of the query's width.
pub fn score_blob(query: &[f32], blob: &[u8], scratch: &mut Vec<f32>) -> Option<f32> {
    if blob.len() != query.len() * 4 || query.is_empty() {
        return None;
    }
    scratch.clear();
    scratch.extend(
        blob.chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])),
    );
    Some(cosine(query, scratch))
}

/// [`rank`] over scores already worked out — by
/// `db::embedding_scores`, straight off the catalogue.
pub fn best(mut scored: Vec<(i64, f32)>, limit: usize, min_score: f32) -> Vec<(i64, f32)> {
    scored.retain(|s| s.1 >= min_score);
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    scored.truncate(limit);
    scored
}

/// The least a content-search match may score (cosine, SigLIP 2) — see
/// [`rank_relevant`].
pub const RELEVANCE_FLOOR: f32 = 0.06;
/// How far above a query's own average across the library a match must stand,
/// in standard deviations — see [`rank_relevant`].
pub const RELEVANCE_SPREAD: f32 = 1.0;

/// Ranks like [`rank`], but keeps only the photos that match, not the
/// nearest `limit` whatever they are. Content search used to return the top
/// 1,000 for every query, so "a purple elephant playing the violin" filled the
/// grid, and so did a real search, down to photos of nothing like it.
///
/// A match must clear two bars:
/// - [`RELEVANCE_FLOOR`], an absolute score. Relevant photos score a median
///   0.13, and a query that matches nothing tops out around 0.02–0.08.
/// - [`RELEVANCE_SPREAD`] deviations above this query's mean over the whole
///   library. Some short queries ("asdf") score every photo alike, and high
///   enough to clear the floor; a match stands out from that crowd and a
///   flat score doesn't.
///
/// Measured on the labelled query set (`scripts/model-eval/queries.py`, 50
/// queries) over a real 1,250-photo library, against returning everything:
/// mAP 0.908 → 0.903 in English and 0.923 → 0.912 translated, recall 0.97;
/// a typical query returns 60 photos instead of all 1,250, the widest 220;
/// and eight nonsense queries return 0–29, except "asdf" at 170. The floor
/// is SigLIP 2's: a different model needs it measured again.
pub fn rank_relevant(
    query: &[f32],
    candidates: &[(i64, Vec<u8>)],
    limit: usize,
) -> Vec<(i64, f32)> {
    relevant(rank(query, candidates, usize::MAX, f32::MIN), limit)
}

/// [`rank_relevant`] over every photo's score, already worked out.
pub fn relevant(scored: Vec<(i64, f32)>, limit: usize) -> Vec<(i64, f32)> {
    let all = best(scored, usize::MAX, f32::MIN);
    if all.is_empty() {
        return all;
    }
    let n = all.len() as f32;
    let mean = all.iter().map(|s| s.1).sum::<f32>() / n;
    let spread = (all.iter().map(|s| (s.1 - mean).powi(2)).sum::<f32>() / n).sqrt();
    let cut = RELEVANCE_FLOOR.max(mean + RELEVANCE_SPREAD * spread);
    all.into_iter()
        .take_while(|s| s.1 >= cut)
        .take(limit)
        .collect()
}

/// A photo as the near-duplicate grouper consumes it: id, the stored
/// `capture_date` string (or `None` when unknown), and the raw embedding blob.
pub type DuplicateCandidate = (i64, Option<String>, Vec<u8>);

/// Groups photos whose embeddings are near-identical (cosine ≥ `threshold`) —
/// bursts and duplicate frames — for a cull assistant.
///
/// A burst is a run of frames taken seconds apart, so this only ever compares
/// photos that are **close in time**. That single constraint does two things at
/// once:
///
/// - **It is correct.** Two sunsets shot on different evenings are visually
///   near-identical but are not the same moment; a duplicate finder that
///   conflated them would be worse than useless. Grouping by capture time as
///   well as appearance is what a burst actually *is*.
/// - **It is linear.** The earlier version compared every photo against every
///   existing group seed, which is `O(n·groups)` — and `groups` converges on `n`
///   in a real library, because at a 0.92 threshold almost no two distinct
///   photographs match, so almost every photo becomes its own group. That made
///   it quadratic in practice: minutes, frozen, at fifty thousand photos.
///   Restricting comparisons to a small time window bounds the work each
///   photo does to the handful of frames beside it.
///
/// The window ([`BURST_GAP_SECS`]) is the gap between *adjacent* frames, and it
/// chains: a long continuous burst stays one group as long as each frame lands
/// within the window of the one before it. Visual similarity is still the real
/// gate — the window only rules out coincidental look-alikes far apart in time.
///
/// `candidates` is `(id, capture_date, embedding blob)`; `capture_date` is the
/// stored `YYYY-MM-DD HH:MM:SS` string, or `None` for a photo whose date is
/// unknown. Undated photos cannot be windowed, so they fall back to the old
/// all-pairs grouping *among themselves only* — quadratic, but bounded because
/// real photos carry an EXIF capture date and that set is normally empty.
///
/// Only groups of two or more are returned, each sorted by id so the *first*
/// member is a stable "keeper" for a keep-the-first cull. Blobs that don't
/// decode are skipped.
pub fn group_near_duplicates(candidates: &[DuplicateCandidate], threshold: f32) -> Vec<Vec<i64>> {
    group_near_duplicates_counted(candidates, threshold).0
}

/// [`group_near_duplicates`], also returning how many embedding pairs it
/// compared.
///
/// That count is the algorithm's real cost, and unlike a timing it is exact and
/// machine independent — the same input always does the same work. It exists so
/// `tests/scale.rs` can guard the fix (linear, not all-pairs) without a
/// wall-clock ratio that a scheduler hiccup decides.
pub fn group_near_duplicates_counted(
    candidates: &[DuplicateCandidate],
    threshold: f32,
) -> (Vec<Vec<i64>>, u64) {
    let mut comparisons: u64 = 0;

    // Decode once, and split by whether we know when the photo was taken.
    let mut dated: Vec<(i64, i64, Vec<f32>)> = Vec::new(); // (id, epoch secs, embedding)
    let mut undated: Vec<(i64, Vec<f32>)> = Vec::new();
    for (id, capture, blob) in candidates {
        let Some(emb) = decode_embedding(blob) else {
            continue;
        };
        match capture.as_deref().and_then(capture_epoch) {
            Some(t) => dated.push((*id, t, emb)),
            None => undated.push((*id, emb)),
        }
    }
    // The DB hands these over in capture order already; sort anyway so a caller
    // (or a test) that does not is still correct, and so eviction below can rely
    // on time being non-decreasing.
    dated.sort_by_key(|&(_, t, _)| t);

    let mut groups: Vec<Vec<i64>> = Vec::new();

    // Dated frames: sweep in time order holding only the seeds still inside the
    // window. Time is non-decreasing, so a seed that falls out can never come
    // back — eviction is a cheap forward-only filter, not a search.
    struct Seed {
        emb: Vec<f32>,
        last: i64,
        group: usize,
    }
    let mut active: Vec<Seed> = Vec::new();
    for (id, t, emb) in &dated {
        active.retain(|s| t - s.last <= BURST_GAP_SECS);
        let hit = active.iter_mut().find(|s| {
            comparisons += 1;
            s.emb.len() == emb.len() && cosine(&s.emb, emb) >= threshold
        });
        match hit {
            Some(s) => {
                groups[s.group].push(*id);
                s.last = *t; // slide the window along the burst
            }
            None => {
                let g = groups.len();
                groups.push(vec![*id]);
                active.push(Seed {
                    emb: emb.clone(),
                    last: *t,
                    group: g,
                });
            }
        }
    }

    // Undated frames: old all-pairs greedy, scoped to just this set. See the
    // doc comment: normally empty, and slow only in the one library shape
    // (tens of thousands of dateless photos) this deliberately does not rescue.
    let mut seeds: Vec<(usize, usize)> = Vec::new(); // (index in `undated`, group)
    for i in 0..undated.len() {
        let hit = seeds.iter().find(|&&(s, _)| {
            comparisons += 1;
            undated[i].1.len() == undated[s].1.len()
                && cosine(&undated[i].1, &undated[s].1) >= threshold
        });
        match hit {
            Some(&(_, g)) => groups[g].push(undated[i].0),
            None => {
                let g = groups.len();
                groups.push(vec![undated[i].0]);
                seeds.push((i, g));
            }
        }
    }

    let groups = groups
        .into_iter()
        .filter(|g| g.len() >= 2)
        .map(|mut g| {
            g.sort_unstable();
            g
        })
        .collect();
    (groups, comparisons)
}

/// Consecutive frames more than this far apart in capture time are never treated
/// as the same burst. See [`group_near_duplicates`] for why this bounds the cost
/// and why it chains between adjacent frames rather than capping the whole burst.
pub const BURST_GAP_SECS: i64 = 60;

/// Parses the catalogue's stored `capture_date` to epoch seconds.
///
/// The format string is the same contract `trips.rs` parses against; only the
/// *difference* between two of these is ever used here, so the UTC assumption is
/// harmless — a consistent offset cancels.
fn capture_epoch(s: &str) -> Option<i64> {
    chrono::NaiveDateTime::parse_from_str(s.trim(), "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|dt| dt.and_utc().timestamp())
}

/// The standard reciprocal-rank-fusion damping constant. Larger flattens the
/// weight curve (deep positions still count); 60 is the value the RRF paper uses
/// and a sane default here.
pub const RRF_K: f32 = 60.0;

/// Fuses several ranked id lists into one, best first, by **reciprocal-rank
/// fusion**.
///
/// The two halves of search score on incomparable scales — a SigLIP 2 cosine (~0..1)
/// and an FTS5 rank (an unbounded negative number) — so you can't just merge by
/// score. RRF sidesteps that by using only each id's *position*: every list
/// contributes `1 / (k + position)` to an id's total, summed across lists. An id
/// near the top of either list ranks well; one near the top of *both* ranks best,
/// so "looks like" and "mentions the words" reinforce rather than drown each
/// other. `k` damps how fast deep positions lose weight. Ties break by id so the
/// order is stable across identical searches.
pub fn fuse(lists: &[&[i64]], k: f32) -> Vec<i64> {
    use std::collections::HashMap;
    let mut score: HashMap<i64, f32> = HashMap::new();
    for list in lists {
        for (pos, &id) in list.iter().enumerate() {
            *score.entry(id).or_insert(0.0) += 1.0 / (k + pos as f32);
        }
    }
    let mut ids: Vec<(i64, f32)> = score.into_iter().collect();
    ids.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    ids.into_iter().map(|(id, _)| id).collect()
}

/// The SigLIP 2 embedder, wrapping the built-in tagger's model. Defined in both
/// builds — a real embedder under `--features ai`, an error-returning stub
/// otherwise — so the commands in `commands/` need no `#[cfg]` of their own.
#[cfg(feature = "ai")]
mod engine {
    use super::encode_embedding;
    use anyhow::Result;
    use std::path::Path;

    /// The model tagging uses too — one copy between them (see
    /// `BuiltinTagger::shared`).
    pub struct Embedder(std::sync::Arc<crate::tagger::builtin::BuiltinTagger>);

    pub fn build_embedder(clip_dir: &Path) -> Result<Embedder> {
        Ok(Embedder(crate::tagger::builtin::BuiltinTagger::shared(
            clip_dir,
        )?))
    }

    /// The model files changed: the next embedder loads them afresh, even
    /// while a tagging run still holds the old model.
    pub fn forget_model() {
        crate::tagger::builtin::BuiltinTagger::forget_shared();
    }

    impl Embedder {
        /// A photo's image embedding, ready to store in `photos.embedding`.
        pub fn image_blob(&self, image: &[u8]) -> Result<Vec<u8>> {
            Ok(encode_embedding(&self.0.embed_image(image)?))
        }
        /// A search query's text embedding, in the same space as the images.
        pub fn query(&self, query: &str) -> Result<Vec<f32>> {
            self.0.embed_query(query)
        }
    }
}

#[cfg(not(feature = "ai"))]
mod engine {
    use anyhow::{bail, Result};
    use std::path::Path;

    pub struct Embedder;

    pub fn build_embedder(_clip_dir: &Path) -> Result<Embedder> {
        bail!("semantic search needs the AI-enabled build")
    }

    pub fn forget_model() {}

    impl Embedder {
        pub fn image_blob(&self, _image: &[u8]) -> Result<Vec<u8>> {
            unreachable!("no Embedder is ever built without the ai feature")
        }
        pub fn query(&self, _query: &str) -> Result<Vec<f32>> {
            unreachable!("no Embedder is ever built without the ai feature")
        }
    }
}

pub use engine::{build_embedder, forget_model, Embedder};

// ------------------------------------------------------------------------
// Highlights: the best few of a set, and not ten of one sunset.
// ------------------------------------------------------------------------

/// What a good photograph looks like, to SigLIP 2, and what a poor one does.
/// A zero-shot ensemble: the spike (docs/AI-MODEL-SPIKES.md) ranked the owner's
/// library with these, and the top read as postcards and the bottom as grey
/// snapshots. It leans to scenic and colourful; it is not a craft critic.
pub const GOOD_PROMPTS: [&str; 6] = [
    "a beautiful photograph",
    "an award-winning photo",
    "a stunning, well-composed photograph",
    "a professional photo with beautiful light",
    "a photo worth printing and framing",
    "a striking photograph with a clear subject",
];
pub const BAD_PROMPTS: [&str; 6] = [
    "a boring snapshot",
    "a blurry, out of focus photo",
    "a badly composed photo",
    "an accidental photo taken by mistake",
    "a dull, poorly lit photo",
    "a photo of nothing in particular",
];

/// The direction in embedding space from poor to good: the mean of the good
/// prompts' embeddings less the mean of the bad. A photo's score is its dot
/// product with this, i.e. its mean similarity to the good prompts less its
/// mean similarity to the bad.
pub fn aesthetic_direction(good: &[Vec<f32>], bad: &[Vec<f32>]) -> Vec<f32> {
    let dim = good.first().or(bad.first()).map_or(0, Vec::len);
    let mean = |set: &[Vec<f32>]| -> Vec<f32> {
        let mut m = vec![0.0f32; dim];
        for v in set {
            for (a, b) in m.iter_mut().zip(v) {
                *a += b;
            }
        }
        let n = set.len().max(1) as f32;
        m.iter_mut().for_each(|a| *a /= n);
        m
    };
    let (g, b) = (mean(good), mean(bad));
    g.iter().zip(&b).map(|(x, y)| x - y).collect()
}

/// Picks `count` items, best first, trading score against sameness: maximal
/// marginal relevance. Each pick is the one whose score (as a z-score within
/// the set) most exceeds `(1 - lambda) * 10` times its similarity to the
/// closest item already picked. On the spike's busiest day this cut
/// near-duplicate pairs in a best 24 from 35 to 9.
pub fn pick_diverse(
    scores: &[f32],
    embeddings: &[Vec<f32>],
    count: usize,
    lambda: f32,
) -> Vec<usize> {
    let n = scores.len().min(embeddings.len());
    if n == 0 {
        return Vec::new();
    }
    let mean = scores[..n].iter().sum::<f32>() / n as f32;
    let sd = (scores[..n].iter().map(|s| (s - mean).powi(2)).sum::<f32>() / n as f32)
        .sqrt()
        .max(1e-6);
    let z: Vec<f32> = scores[..n].iter().map(|s| (s - mean) / sd).collect();
    let mut closest = vec![f32::NEG_INFINITY; n];
    let mut taken = vec![false; n];
    let mut chosen = Vec::with_capacity(count.min(n));
    while chosen.len() < count.min(n) {
        let pick = (0..n)
            .filter(|&i| !taken[i])
            .max_by(|&a, &b| {
                let v = |i: usize| lambda * z[i] - (1.0 - lambda) * 10.0 * closest[i].max(0.0);
                v(a).total_cmp(&v(b)).then(b.cmp(&a))
            })
            .expect("fewer picks than items");
        taken[pick] = true;
        chosen.push(pick);
        for i in 0..n {
            if !taken[i] {
                closest[i] = closest[i].max(cosine(&embeddings[i], &embeddings[pick]));
            }
        }
    }
    chosen
}

/// Makes sure everyone named in the set appears in the pick: for each
/// person no picked photo shows, their best-scoring photo replaces the
/// lowest-scoring pick that no one else depends on. `people[i]` is who is in
/// item `i`. Nothing is swapped when every pick is someone's only one.
pub fn cover_people(chosen: &mut [usize], people: &[Vec<i32>], scores: &[f32]) {
    let mut everyone: Vec<i32> = people.iter().flatten().copied().collect();
    everyone.sort_unstable();
    everyone.dedup();
    for person in everyone {
        let shown = chosen
            .iter()
            .any(|&i| people.get(i).is_some_and(|p| p.contains(&person)));
        if shown {
            continue;
        }
        let Some(best) = (0..people.len())
            .filter(|i| !chosen.contains(i) && people[*i].contains(&person))
            .max_by(|&a, &b| scores[a].total_cmp(&scores[b]))
        else {
            continue;
        };
        // A pick is spare when every person in it also appears elsewhere.
        let spare = |slot: usize| -> bool {
            let i = chosen[slot];
            people.get(i).is_none_or(|ps| {
                ps.iter().all(|p| {
                    chosen
                        .iter()
                        .enumerate()
                        .any(|(k, &j)| k != slot && people.get(j).is_some_and(|q| q.contains(p)))
                })
            })
        };
        if let Some(slot) = (0..chosen.len())
            .filter(|&s| spare(s))
            .min_by(|&a, &b| scores[chosen[a]].total_cmp(&scores[chosen[b]]))
        {
            chosen[slot] = best;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_round_trips_through_the_blob() {
        let v = vec![0.5f32, -0.25, 0.0, 1.0, -1.0];
        let blob = encode_embedding(&v);
        assert_eq!(blob.len(), v.len() * 4);
        assert_eq!(decode_embedding(&blob).unwrap(), v);
    }

    #[test]
    fn a_malformed_blob_is_skipped_not_trusted() {
        assert_eq!(decode_embedding(&[]), None);
        assert_eq!(decode_embedding(&[1, 2, 3]), None); // not a multiple of 4
        assert!(decode_embedding(&[0, 0, 0, 0]).is_some());
    }

    #[test]
    fn cosine_is_one_for_identical_and_zero_for_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert!((cosine(&a, &a) - 1.0).abs() < 1e-6);
        assert!(cosine(&a, &b).abs() < 1e-6);
        // A width mismatch scores 0 rather than panicking.
        assert_eq!(cosine(&a, &[1.0, 0.0]), 0.0);
    }

    #[test]
    fn rank_orders_by_similarity_and_respects_the_limit() {
        // Query points along +x; candidates at increasing angle from it.
        let query = vec![1.0f32, 0.0];
        let near = encode_embedding(&[1.0, 0.0]); // cos 1.0
        let mid = encode_embedding(&[
            std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
        ]); // cos ~0.707
        let far = encode_embedding(&[0.0, 1.0]); // cos 0.0
        let cands = vec![(10i64, far), (20, near), (30, mid)];

        let top2 = rank(&query, &cands, 2, 0.0);
        assert_eq!(top2.len(), 2);
        assert_eq!(top2[0].0, 20); // nearest first
        assert_eq!(top2[1].0, 30);
        assert!(top2[0].1 > top2[1].1);
    }

    /// A library of `n` photos along one axis, each scoring its value against
    /// a query along that axis.
    fn scored(values: &[f32]) -> Vec<(i64, Vec<u8>)> {
        values
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                let rest = (1.0 - v * v).max(0.0).sqrt();
                (i as i64 + 1, encode_embedding(&[v, rest]))
            })
            .collect()
    }

    #[test]
    fn rank_relevant_keeps_what_stands_out_and_clears_the_floor() {
        // Most of the library near 0.02; three real matches.
        let mut values = vec![0.02; 40];
        values.extend([0.15, 0.12, 0.09]);
        let got: Vec<i64> = rank_relevant(&[1.0, 0.0], &scored(&values), 1000)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(got, vec![41, 42, 43]);
    }

    #[test]
    fn rank_relevant_returns_nothing_for_a_query_that_matches_nothing() {
        // Below the floor however they spread.
        let low: Vec<f32> = (0..40).map(|i| i as f32 * 0.001).collect();
        assert!(rank_relevant(&[1.0, 0.0], &scored(&low), 1000).is_empty());
        // Above the floor, but everything alike ("asdf"): only the top of the
        // crowd is kept, not the library.
        let flat: Vec<f32> = (0..40).map(|i| 0.065 + i as f32 * 0.00025).collect();
        let kept = rank_relevant(&[1.0, 0.0], &scored(&flat), 1000);
        assert!(kept.len() < 10, "kept {} of 40", kept.len());
    }

    #[test]
    fn rank_drops_candidates_below_the_min_score() {
        // The bug behind "find similar shows everything": without a floor the far
        // photo comes back too, just ranked last. A threshold cuts it.
        let query = vec![1.0f32, 0.0];
        let near = encode_embedding(&[1.0, 0.0]); // cos 1.0
        let mid = encode_embedding(&[
            std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
        ]); // cos ~0.707
        let far = encode_embedding(&[0.0, 1.0]); // cos 0.0
        let cands = vec![(10i64, far), (20, near), (30, mid)];

        // No floor keeps all three; a 0.5 floor keeps only near + mid.
        assert_eq!(rank(&query, &cands, 10, 0.0).len(), 3);
        let kept = rank(&query, &cands, 10, 0.5);
        assert_eq!(kept.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [20, 30]);
    }

    #[test]
    fn rank_skips_blobs_that_dont_match_the_query_width() {
        let query = vec![1.0f32, 0.0, 0.0];
        let good = encode_embedding(&[1.0, 0.0, 0.0]);
        let wrong_width = encode_embedding(&[1.0, 0.0]);
        let corrupt = vec![1u8, 2, 3]; // not f32-aligned
        let cands = vec![(1i64, good), (2, wrong_width), (3, corrupt)];

        let ranked = rank(&query, &cands, 10, 0.0);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].0, 1);
    }

    /// A capture-date `n` seconds after a fixed base, in the stored format.
    fn at(secs: i64) -> Option<String> {
        let base = chrono::NaiveDate::from_ymd_opt(2025, 6, 1)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap();
        Some(
            (base + chrono::Duration::seconds(secs))
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
        )
    }

    #[test]
    fn near_duplicates_group_and_singletons_drop_out() {
        // Two near-identical pairs pointing almost the same way, plus a lone
        // photo pointing elsewhere. All within one burst window, so time does
        // not separate them — appearance does.
        let a1 = encode_embedding(&[1.0, 0.0]);
        let a2 = encode_embedding(&[0.999, 0.045]); // ~cos 0.999 with a1
        let b1 = encode_embedding(&[0.0, 1.0]);
        let b2 = encode_embedding(&[0.045, 0.999]); // ~cos 0.999 with b1
        let lone = encode_embedding(&[
            std::f32::consts::FRAC_1_SQRT_2,
            -std::f32::consts::FRAC_1_SQRT_2,
        ]);
        let cands = vec![
            (1i64, at(0), a1),
            (2, at(1), a2),
            (3, at(2), b1),
            (4, at(3), b2),
            (5, at(4), lone),
        ];

        let groups = group_near_duplicates(&cands, 0.99);
        assert_eq!(
            groups.len(),
            2,
            "two duplicate pairs, the singleton dropped"
        );
        assert!(groups.contains(&vec![1, 2]));
        assert!(groups.contains(&vec![3, 4]));
    }

    #[test]
    fn a_high_threshold_keeps_only_true_duplicates() {
        let a = encode_embedding(&[1.0, 0.0]);
        let close = encode_embedding(&[0.985, 0.174]); // ~cos 0.985
        let cands = vec![(1i64, at(0), a), (2, at(1), close)];
        // Above the pair's similarity → nothing groups.
        assert!(group_near_duplicates(&cands, 0.995).is_empty());
        // Below it → they group, first id first.
        assert_eq!(group_near_duplicates(&cands, 0.95), vec![vec![1, 2]]);
    }

    #[test]
    fn look_alikes_far_apart_in_time_are_not_duplicates() {
        // The same view, shot on two different days. Visually a near-perfect
        // match, but not the same moment — the whole point of the time window.
        let view = [0.8f32, 0.6];
        let day1 = encode_embedding(&view);
        let day2 = encode_embedding(&view);
        let cands = vec![(1i64, at(0), day1), (2, at(86_400), day2)];
        assert!(
            group_near_duplicates(&cands, 0.9).is_empty(),
            "identical-looking frames a day apart must not be called duplicates"
        );
    }

    #[test]
    fn a_long_burst_chains_into_one_group() {
        // Twenty frames, each half a minute after the last — no two are within
        // the window of a *fixed* start, but each is within the window of its
        // predecessor, so the burst must stay a single group rather than
        // fragmenting.
        let v = encode_embedding(&[1.0, 0.0]);
        let cands: Vec<_> = (0..20i64).map(|i| (i + 1, at(i * 30), v.clone())).collect();
        let groups = group_near_duplicates(&cands, 0.99);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 20);
    }

    #[test]
    fn undated_photos_still_group_among_themselves() {
        // No capture date at all (metadata-stripped export, say). These cannot
        // be windowed, but they must not silently stop grouping.
        let a = encode_embedding(&[1.0, 0.0]);
        let b = encode_embedding(&[0.999, 0.045]);
        let cands = vec![(1i64, None, a), (2, None, b)];
        assert_eq!(group_near_duplicates(&cands, 0.95), vec![vec![1, 2]]);
    }

    #[test]
    fn fuse_rewards_agreement_between_the_two_lists() {
        // `semantic` likes 1,2,3; `caption` likes 3,4,5. Id 3 is the only one
        // both rank, so it must come first even though it is neither list's top.
        let semantic = [1i64, 2, 3];
        let caption = [3i64, 4, 5];
        let fused = fuse(&[&semantic, &caption], RRF_K);
        assert_eq!(fused[0], 3, "the id both lists rank wins");
        // Every id from either list appears exactly once (union, de-duplicated).
        let mut sorted = fused.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, [1, 2, 3, 4, 5]);
        assert_eq!(fused.len(), 5);
    }

    #[test]
    fn fuse_falls_back_to_a_single_list_and_keeps_its_order() {
        // Caption search found nothing (no model / no captions): the fused order
        // is just the other list, unchanged.
        let semantic = [7i64, 4, 9];
        let fused = fuse(&[&semantic, &[]], RRF_K);
        assert_eq!(fused, [7, 4, 9]);
    }

    #[test]
    fn fuse_rewards_consistent_agreement_over_a_lone_top_hit() {
        // 20 is #2 in *both* lists; 10 is #1 in one but #4 in the other. At the
        // default k, 20's steady agreement (1/61 + 1/61 ≈ 0.0328) edges out 10's
        // one-list top placement (1/60 + 1/63 ≈ 0.0325) — the whole point of RRF.
        let a = [10i64, 20, 30, 40];
        let b = [50i64, 20, 30, 10];
        let fused = fuse(&[&a, &b], RRF_K);
        assert_eq!(fused[0], 20);
        assert!(fused.iter().position(|&x| x == 20) < fused.iter().position(|&x| x == 10));
    }

    #[test]
    fn ties_break_by_id_for_a_stable_order() {
        let query = vec![1.0f32, 0.0];
        let same = encode_embedding(&[1.0, 0.0]);
        let cands = vec![(30i64, same.clone()), (10, same.clone()), (20, same)];
        let ranked = rank(&query, &cands, 10, 0.0);
        assert_eq!(
            ranked.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            [10, 20, 30]
        );
    }

    #[test]
    fn the_aesthetic_direction_is_good_minus_bad() {
        let d = aesthetic_direction(&[vec![1.0, 0.0], vec![0.0, 1.0]], &[vec![0.0, 0.0]]);
        assert_eq!(d, vec![0.5, 0.5]);
    }

    /// Ten near-identical sunsets and two other scenes: a best three takes one
    /// sunset and both of the others, though every sunset scores higher.
    #[test]
    fn diversity_keeps_one_of_many_near_duplicates() {
        let mut embs: Vec<Vec<f32>> = (0..10).map(|k| vec![1.0, 0.01 * k as f32, 0.0]).collect();
        embs.push(vec![0.0, 1.0, 0.0]);
        embs.push(vec![0.0, 0.0, 1.0]);
        let mut scores: Vec<f32> = (0..10).map(|k| 1.0 - 0.01 * k as f32).collect();
        scores.extend([0.8, 0.7]);
        let got = pick_diverse(&scores, &embs, 3, 0.7);
        assert_eq!(got[0], 0, "the best first");
        assert!(got.contains(&10) && got.contains(&11), "{got:?}");
        // With no weight on sameness it is simply the top three.
        assert_eq!(pick_diverse(&scores, &embs, 3, 1.0), vec![0, 1, 2]);
    }

    #[test]
    fn everyone_named_gets_a_photo() {
        // 0 and 1 are Ada's (best scores), 2 is Bo's, 3 has nobody.
        let people = vec![vec![1], vec![1], vec![2], vec![]];
        let scores = [0.9, 0.8, 0.1, 0.5];
        let mut chosen = vec![0, 1, 3];
        cover_people(&mut chosen, &people, &scores);
        assert!(chosen.contains(&2), "Bo is in: {chosen:?}");
        assert!(chosen.contains(&0), "Ada keeps a photo");
        // When every pick is someone's only photo, nothing is swapped.
        let people = vec![vec![1], vec![2], vec![3]];
        let mut chosen = vec![0, 1];
        cover_people(&mut chosen, &people, &[0.9, 0.8, 0.1]);
        assert_eq!(chosen, vec![0, 1]);
    }
}
