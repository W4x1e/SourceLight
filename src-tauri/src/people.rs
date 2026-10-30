//! People grouping: cluster face embeddings into identities.
//!
//! Face detection and the AuraFace embedder (in [`crate::faces`]) turn each
//! face into a 512-d unit vector; here those vectors are grouped into people by
//! cosine similarity. Pure and unit-tested — no model, no `ai` gate — so the
//! grouping logic is verifiable on its own, and the thresholds are calibrated
//! against real embeddings separately.
//!
//! # Why this is not single-link clustering any more
//!
//! It used to be: union any pair at ≥ 0.45 and take the connected components.
//! That is single linkage, and single linkage **chains** — A links to B, B links
//! to C, and A and C end up the same person even though nothing ever compared
//! them favourably. One bad embedding is enough to bridge two strangers, and
//! bad embeddings are exactly what a photo library produces: faces 30 pixels
//! across, profiles, motion blur, someone half out of frame.
//!
//! Two changes fix it. Faces that cannot produce a trustworthy embedding are
//! kept out of the vote entirely ([`is_groupable`]) — they are still detected,
//! counted and eye-scored, they just do not get to decide who anyone is. And
//! linking happens in two stages: tight cores at a high threshold, then a merge
//! by **average linkage**, so joining two groups needs their members to agree
//! on average rather than through their two closest members.
//!
//! # Why average linkage, not centroids (2026-09-24)
//!
//! The merge used to compare re-normalised centroids. That snowballs: once a
//! group holds two people, their vectors partly cancel, normalising the sum
//! inflates its cosine to everyone, and the group absorbs more strangers. On
//! 3,443 LFW faces from 2,132 people it collapsed thousands of faces into a
//! few mixed groups (pairwise precision 0.001). Average linkage uses the same
//! sums without the normalisation, so a mixed group is *less* attractive, not
//! more: precision 0.95, recall 0.97, 1.04 groups per person. Measured in
//! `scripts/model-eval/` and written up in `docs/AI-MODEL-SPIKES.md`.
//!
//! # Voting and attaching
//!
//! Profiles, occluded faces and weak detections embed badly, and bad
//! embeddings are what link strangers. So only faces that pass [`votes`] form
//! and merge groups. The rest of the groupable faces then [`attach`] to the
//! group they agree with best, if they agree well enough, and cannot link two
//! groups. What is still split is offered to the user by [`suggest`].

/// Legacy single threshold, kept so callers and tests that only care about the
/// same/different boundary have a name for it. [`Thresholds::default`] is what
/// grouping actually uses.
pub const PERSON_THRESHOLD: f32 = 0.45;

/// Detector confidence below which a face does not vote on identity. YuNet
/// scores a clean face well above 0.9; the tail below this is mostly texture
/// that happens to look face-shaped, and its embedding is noise.
pub const MIN_DETECTION_SCORE: f32 = 0.60;

/// Shortest face-box edge, below which a face does not vote — in pixels of the
/// **1600px preview the detector runs on**, not of the capture.
///
/// That distinction is the whole story of this constant. It was 64, which
/// sounds conservative until you notice it means 4% of the frame's long edge:
/// on a beach or a coast path, essentially every face in the library failed it,
/// went ungrouped, and turned into its own one-photo "person". A wall of
/// singletons is a worse answer than a slightly wrong cluster.
///
/// 32px is where the embedder's 112×112 aligned crop becomes more than 3×
/// upsampling, which is the point at which the embedding really does stop
/// meaning anything. The gate is a floor against garbage, not a quality bar —
/// bridging is prevented by [`Thresholds::core`], which does it by geometry
/// rather than by hoping.
pub const MIN_FACE_EDGE_PX: f32 = 32.0;

/// Detector confidence a face needs to *vote*: to found or merge a group.
/// Below it (but above [`MIN_DETECTION_SCORE`]) a face may still attach.
pub const MIN_VOTE_SCORE: f32 = 0.75;

/// How far off frontal a face may be and still vote, as [`pose_offset`]
/// measures it. A three-quarter view sits around 0.3–0.5; past 0.6 the face
/// is a profile, or the detector found the back of a head.
pub const MAX_VOTE_POSE: f32 = 0.6;

/// Lowest average cosine at which two groups are offered to the user as
/// possibly the same person. On the labelled library the real splits scored
/// 0.27–0.35 and the closest pair of strangers 0.22.
pub const SUGGEST_FLOOR: f32 = 0.25;

/// The bar for suggesting two groups when one of them is a single face. One
/// face is one observation, and at 0.25 a lone stranger was offered as
/// "Same person?" on a real library; two groups that each agree internally
/// are far stronger evidence at the same score.
pub const SUGGEST_FLOOR_SINGLE: f32 = 0.35;

/// The most people [`suggest`] compares with everyone else: named
/// people first, then the biggest groups. Every pair of P people was P²/2
/// comparisons on one thread on every People load, about 15 s at 30,000
/// people. Strict cores leave most strangers as single faces, and two single
/// faces are never offered anyway unless one of them is someone the user
/// cares about.
pub const SUGGEST_ANCHORS: usize = 500;

/// How good a person's cover this face makes: its size (the portrait is cut
/// from it, so bigger is sharper), discounted for turning away from the
/// camera and for a weak detection. Unknown score or pose count as fine.
pub fn cover_quality(score: Option<f32>, w: f32, h: f32, pose: Option<f32>) -> f32 {
    let size = w.min(h).max(0.0);
    let facing = 1.0 / (1.0 + 2.0 * pose.unwrap_or(0.0).max(0.0));
    size * facing * score.unwrap_or(1.0)
}

/// How faces are linked into people.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Thresholds {
    /// Two voting faces are directly linked at or above this cosine. This is
    /// the only stage that chains, so it is set where only near-certain pairs
    /// (the same face in a burst, or a clean frontal match) link directly.
    pub core: f32,
    /// Two groups merge when the **average** cosine between their members
    /// reaches this. See the module docs for why it is not a centroid cosine.
    /// Also the bar a non-voting face must clear to [`attach`].
    pub merge: f32,
    /// A face whose average cosine to its own group falls below this is
    /// evicted to a singleton: the group as a whole does not support it.
    pub member_floor: f32,
}

impl Default for Thresholds {
    /// Measured with AuraFace on two sets (2026-09-24):
    ///
    /// - **LFW**, 3,443 faces from 2,132 people through this pipeline:
    ///   pairwise precision 0.95, recall 0.97, 1.04 groups per person.
    ///   Loosening `merge` to 0.36 drops precision to 0.85. That is where
    ///   strangers start to merge at scale.
    /// - **A real library** with sunglasses, hats and profiles: every group
    ///   was a single person. The one person still split (with and without
    ///   sunglasses) was the top [`suggest`]ion.
    ///
    /// They lean toward splitting a person (which the user merges in one
    /// click, prompted by a suggestion) over merging two strangers (which the
    /// user has no good way to undo photo by photo).
    fn default() -> Self {
        Self {
            core: 0.75,
            merge: 0.40,
            member_floor: 0.20,
        }
    }
}

/// How far a face turns from frontal, from its five landmarks: the nose's
/// sideways offset from the midpoint of the eyes, over the distance between
/// the eyes. About 0 face-on, 0.3–0.5 three-quarter, 1+ a profile.
pub fn pose_offset(landmarks: &[[f32; 2]; 5]) -> f32 {
    let [re, le, nose, _, _] = *landmarks;
    let eyes = ((le[0] - re[0]).powi(2) + (le[1] - re[1]).powi(2)).sqrt();
    if eyes < 1e-3 {
        return f32::INFINITY;
    }
    (nose[0] - (re[0] + le[0]) / 2.0).abs() / eyes
}

/// Whether a groupable face may found or merge a group. Stricter than
/// [`is_groupable`]: a weak detection or a turned head still belongs to
/// someone, but its embedding is not reliable enough to decide *who* for a
/// whole group. Unknown score or pose (rows from before they were recorded)
/// count as passing, as in [`is_groupable`].
pub fn votes(score: Option<f32>, w: f32, h: f32, pose: Option<f32>) -> bool {
    is_groupable(score, w, h)
        && score.is_none_or(|s| s >= MIN_VOTE_SCORE)
        && pose.is_none_or(|p| p <= MAX_VOTE_POSE)
}

/// Whether a detected face is trustworthy enough to decide who someone is.
///
/// A face that fails this is still a face: it is detected, counted, eye-scored
/// and shown. It simply does not vote, because a bad embedding does not just
/// misplace itself — it drags two real identities together.
pub fn is_groupable(score: Option<f32>, w: f32, h: f32) -> bool {
    // A missing score comes from a row written before detection confidence was
    // stored. Judge those on size alone rather than discarding the library's
    // entire existing face table.
    score.is_none_or(|s| s >= MIN_DETECTION_SCORE) && w.min(h) >= MIN_FACE_EDGE_PX
}

/// L2-normalise in place, so cosine similarity is a plain dot product. A no-op
/// on a zero vector.
pub fn l2_normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-8 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Cosine similarity of two equal-length vectors — a dot product when both are
/// unit length (embeddings are). Returns 0 on a length mismatch.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    crate::vector::dot(a, b)
}

/// The plain sum of a group's embeddings. Average linkage between two groups is
/// `dot(sum_a, sum_b) / (n_a · n_b)`, the mean cosine over every cross pair,
/// so keeping sums makes each merge test one dot product.
fn sum_of(embeddings: &[Vec<f32>], group: &[usize]) -> Vec<f32> {
    let Some(first) = group.first().and_then(|&i| embeddings.get(i)) else {
        return Vec::new();
    };
    let mut sum = vec![0.0f32; first.len()];
    for &i in group {
        for (s, v) in sum.iter_mut().zip(&embeddings[i]) {
            *s += v;
        }
    }
    sum
}

/// Mean cosine between the members of two groups, from their sums.
fn average_link(sum_a: &[f32], n_a: usize, sum_b: &[f32], n_b: usize) -> f32 {
    cosine(sum_a, sum_b) / (n_a * n_b).max(1) as f32
}

fn find(parent: &mut [usize], i: usize) -> usize {
    let mut root = i;
    while parent[root] != root {
        root = parent[root];
    }
    // Path compression.
    let mut cur = i;
    while parent[cur] != root {
        let next = parent[cur];
        parent[cur] = root;
        cur = next;
    }
    root
}

fn union(parent: &mut [usize], a: usize, b: usize) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb {
        parent[ra] = rb;
    }
}

/// Relabels group memberships to compact ids in first-appearance order.
fn relabel(n: usize, groups: &[Vec<usize>]) -> Vec<usize> {
    let mut of = vec![usize::MAX; n];
    for (g, group) in groups.iter().enumerate() {
        for &m in group {
            of[m] = g;
        }
    }
    // Renumber by the order members first appear, so the ids are stable and
    // readable rather than reflecting the order groups happened to merge in.
    let mut seen = std::collections::HashMap::new();
    let mut next = 0usize;
    of.into_iter()
        .map(|g| {
            *seen.entry(g).or_insert_with(|| {
                let v = next;
                next += 1;
                v
            })
        })
        .collect()
}

/// Group embeddings into people. Returns a person id per input, 0-based in
/// first-appearance order.
///
/// Three stages, in order: tight cores by direct linkage at `t.core`; merge
/// groups whose members agree on average at `t.merge`, repeated to a fixed
/// point; then evict any member whose average agreement with its final group
/// is below `t.member_floor`. See the module docs for why it is not one stage.
pub fn cluster(embeddings: &[Vec<f32>], t: Thresholds) -> Vec<usize> {
    cluster_with(embeddings, t, &|| false, &mut |_, _| {}).expect("never asked to stop")
}

/// Rows of the pairwise pass compared in parallel at a time. Small enough that
/// a stop is heard within a fraction of a second at 20,000 faces, large enough
/// to keep every core busy.
pub const CORE_BLOCK: usize = 64;

/// Faces a block of rows meets at a time in the pairwise pass: 256 of 512
/// floats is half a megabyte, which stays in cache for the whole block.
const CORE_TILE: usize = 256;

/// Groups left to search below which a merge looks for its partner on one
/// thread: handing a short search to the thread pool costs more than it saves.
const MERGE_PARALLEL_FROM: usize = 2048;

/// [`cluster`], stoppable: `stop` is asked between blocks of
/// [`CORE_BLOCK`] rows of the pairwise pass and between merges — the two
/// stages whose cost grows with the square of the library — and `None` comes
/// back once it says yes. `progress` hears `(rows done, rows)` for every row
/// of the pairwise pass, a block at a time, which is most of the time.
pub fn cluster_with(
    embeddings: &[Vec<f32>],
    t: Thresholds,
    stop: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(usize, usize),
) -> Option<Vec<usize>> {
    let n = embeddings.len();
    if n == 0 {
        return Some(Vec::new());
    }

    // Stage 1 — cores. Every face against every other is where the time goes,
    // so a block of rows is compared in parallel, its links gathered and then
    // joined; the groups are the same whatever order links are joined in. The
    // block meets the faces after it a tile at a time, so a tile is read from
    // memory once for the block rather than once per row — 20,000 faces are
    // 40 MB, more than the cache holds.
    use rayon::prelude::*;
    let mut parent: Vec<usize> = (0..n).collect();
    let mut start = 0;
    while start < n {
        if stop() {
            return None;
        }
        let rows = start..(start + CORE_BLOCK).min(n);
        let links: Vec<(usize, usize)> = (rows.start + 1..n)
            .into_par_iter()
            .step_by(CORE_TILE)
            .flat_map_iter(|from| {
                let tile = from..(from + CORE_TILE).min(n);
                rows.clone().flat_map(move |i| {
                    tile.clone()
                        .filter(move |&j| j > i && cosine(&embeddings[i], &embeddings[j]) >= t.core)
                        .map(move |j| (i, j))
                })
            })
            .collect();
        for (i, j) in links {
            union(&mut parent, i, j);
        }
        for i in rows.clone() {
            progress(i, n);
        }
        start = rows.end;
    }
    let mut groups: Vec<Vec<usize>> = {
        let mut by_root: std::collections::HashMap<usize, Vec<usize>> =
            std::collections::HashMap::new();
        for i in 0..n {
            let r = find(&mut parent, i);
            by_root.entry(r).or_default().push(i);
        }
        let mut out: Vec<Vec<usize>> = by_root.into_values().collect();
        // Deterministic order: by first member, so two runs over the same input
        // produce the same ids.
        out.sort_by_key(|g| g[0]);
        out
    };

    // Stage 2 — average-linkage merge, to a fixed point. Only the surviving
    // group's sum is updated, so a merge costs one vector add.
    let mut sums: Vec<Vec<f32>> = groups.iter().map(|g| sum_of(embeddings, g)).collect();
    loop {
        let mut merged = false;
        let mut a = 0;
        while a < groups.len() {
            if stop() {
                return None;
            }
            let mut from = a + 1;
            while let Some(b) = next_to_join(&groups, &sums, a, from, t.merge) {
                let moved = groups.remove(b);
                let moved_sum = sums.remove(b);
                groups[a].extend(moved);
                groups[a].sort_unstable();
                for (s, v) in sums[a].iter_mut().zip(&moved_sum) {
                    *s += v;
                }
                merged = true;
                from = b;
            }
            a += 1;
        }
        if !merged {
            break;
        }
    }

    // Stage 3 — evict members the group as a whole does not support. Judged
    // against the group as it stood *before* any eviction, so removing one
    // outlier cannot cascade into dismantling a real person.
    let mut evicted: Vec<usize> = Vec::new();
    for (g, group) in groups.iter_mut().enumerate() {
        if group.len() < 3 {
            // A pair has no majority to disagree with — its two members are all
            // the evidence there is, and they already passed `core`.
            continue;
        }
        let (sum, n) = (&sums[g], group.len());
        group.retain(|&m| {
            if average_link(&embeddings[m], 1, sum, n) >= t.member_floor {
                true
            } else {
                evicted.push(m);
                false
            }
        });
    }
    for m in evicted {
        groups.push(vec![m]);
    }

    groups.sort_by_key(|g| g[0]);
    Some(relabel(n, &groups))
}

/// The first group from `from` on whose members agree with group `a`'s on
/// average at `merge` — looked for in parallel when there are many, but always
/// the first in order, so each merge is the one a scan from left to right
/// makes, against the same sum.
fn next_to_join(
    groups: &[Vec<usize>],
    sums: &[Vec<f32>],
    a: usize,
    from: usize,
    merge: f32,
) -> Option<usize> {
    use rayon::prelude::*;
    let joins =
        |b: &usize| average_link(&sums[a], groups[a].len(), &sums[*b], groups[*b].len()) >= merge;
    let rest = from..groups.len();
    if rest.len() < MERGE_PARALLEL_FROM {
        rest.into_iter().find(joins)
    } else {
        rest.into_par_iter().find_first(joins)
    }
}

/// Places faces that were not allowed to vote: each joins the group whose
/// members it agrees with best on average, if that reaches `threshold`, and
/// otherwise stays unplaced (`None`). A face that attaches never moves a
/// group or links two groups; it only joins one.
///
/// `labels` are [`cluster`]'s over `voters`; the result is one entry per
/// `candidates` face.
pub fn attach(
    voters: &[Vec<f32>],
    labels: &[usize],
    candidates: &[Vec<f32>],
    threshold: f32,
) -> Vec<Option<usize>> {
    let (sums, sizes) = group_sums(voters, labels);
    attach_to_sums(&sums, &sizes, candidates, threshold)
}

/// Each group's sum of member vectors, and its size, for a labelling.
pub fn group_sums(embeddings: &[Vec<f32>], labels: &[usize]) -> (Vec<Vec<f32>>, Vec<usize>) {
    let groups = count(labels);
    let dim = embeddings.first().map_or(0, Vec::len);
    let mut sums = vec![vec![0.0f32; dim]; groups];
    let mut sizes = vec![0usize; groups];
    for (e, &g) in embeddings.iter().zip(labels) {
        for (s, v) in sums[g].iter_mut().zip(e) {
            *s += v;
        }
        sizes[g] += 1;
    }
    (sums, sizes)
}

/// [`attach`] against groups given by their sums and sizes: how new faces
/// join the people already there without clustering the library again.
/// In parallel, since a pass over a big library meets thousands of
/// people.
pub fn attach_to_sums(
    sums: &[Vec<f32>],
    sizes: &[usize],
    candidates: &[Vec<f32>],
    threshold: f32,
) -> Vec<Option<usize>> {
    use rayon::prelude::*;
    candidates
        .par_iter()
        .map(|c| {
            (0..sums.len())
                .filter(|&g| sizes[g] > 0)
                .map(|g| (g, average_link(c, 1, &sums[g], sizes[g])))
                .filter(|&(_, s)| s >= threshold)
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(g, _)| g)
        })
        .collect()
}

/// Pairs of groups that may be one person split in two, best first: the
/// average cosine between their members is at least `floor` (at least
/// [`SUGGEST_FLOOR_SINGLE`] when either is a single face) but was not enough
/// to merge them. At most `limit` pairs, and never a pair for which
/// `apart(a, b)` says the user already answered "not the same".
///
/// Returns `(group_a, group_b, score)` with `group_a < group_b`, in the ids of
/// `labels`.
///
/// Only pairs with at least one side among the anchors are looked at:
/// the groups `named` says the user named, then the biggest groups of more
/// than one face, [`SUGGEST_ANCHORS`] in all, each against every group, in
/// parallel.
pub fn suggest(
    embeddings: &[Vec<f32>],
    labels: &[usize],
    floor: f32,
    limit: usize,
    apart: &(dyn Fn(usize, usize) -> bool + Sync),
    named: &(dyn Fn(usize) -> bool + Sync),
) -> Vec<(usize, usize, f32)> {
    use rayon::prelude::*;
    let (sums, sizes) = group_sums(embeddings, labels);
    let groups = sums.len();
    let mut anchors: Vec<usize> = (0..groups).filter(|&g| named(g) || sizes[g] > 1).collect();
    anchors.sort_by_key(|&g| (!named(g), std::cmp::Reverse(sizes[g]), g));
    anchors.truncate(SUGGEST_ANCHORS);
    let mut is_anchor = vec![false; groups];
    for &g in &anchors {
        is_anchor[g] = true;
    }
    let mut out: Vec<(usize, usize, f32)> = anchors
        .par_iter()
        .flat_map_iter(|&a| {
            let (sums, sizes, is_anchor) = (&sums, &sizes, &is_anchor);
            // A pair of anchors is looked at once, from its lower side.
            (0..groups)
                .filter(move |&b| b != a && !(is_anchor[b] && b < a))
                .filter_map(move |b| {
                    let s = average_link(&sums[a], sizes[a], &sums[b], sizes[b]);
                    let bar = if sizes[a].min(sizes[b]) == 1 {
                        floor.max(SUGGEST_FLOOR_SINGLE)
                    } else {
                        floor
                    };
                    (s >= bar && !apart(a, b)).then_some((a.min(b), a.max(b), s))
                })
        })
        .collect();
    out.sort_by(|x, y| y.2.total_cmp(&x.2).then((x.0, x.1).cmp(&(y.0, y.1))));
    out.truncate(limit);
    out
}

/// Forces the user's own "these two are the same person" decisions onto a
/// clustering, then compacts the ids again.
///
/// Applied after clustering rather than inside it: a person the user merged by
/// hand must stay merged through every future regroup, whatever the embeddings
/// say, and the cleanest way to guarantee that is to let the algorithm run
/// unaware and then override it.
pub fn apply_links(labels: &[usize], links: &[(usize, usize)]) -> Vec<usize> {
    let n = labels.len();
    if n == 0 {
        return Vec::new();
    }
    let mut parent: Vec<usize> = (0..n).collect();
    // Re-link everything the clustering already grouped...
    let mut first_of: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for (i, &l) in labels.iter().enumerate() {
        match first_of.get(&l) {
            Some(&f) => union(&mut parent, i, f),
            None => {
                first_of.insert(l, i);
            }
        }
    }
    // ...then the user's links on top.
    for &(a, b) in links {
        if a < n && b < n {
            union(&mut parent, a, b);
        }
    }

    let mut by_root: std::collections::HashMap<usize, Vec<usize>> =
        std::collections::HashMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        by_root.entry(r).or_default().push(i);
    }
    let mut groups: Vec<Vec<usize>> = by_root.into_values().collect();
    for g in groups.iter_mut() {
        g.sort_unstable();
    }
    groups.sort_by_key(|g| g[0]);
    relabel(n, &groups)
}

/// Number of distinct people in a [`cluster`] labelling.
pub fn count(labels: &[usize]) -> usize {
    labels.iter().copied().max().map_or(0, |m| m + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(v: &[f32]) -> Vec<f32> {
        let mut v = v.to_vec();
        l2_normalize(&mut v);
        v
    }

    /// Two well-separated identities, four faces each, plus a controllable
    /// bridge. Built in 3-D so the geometry is easy to reason about.
    fn two_identities() -> Vec<Vec<f32>> {
        vec![
            unit(&[1.0, 0.10, 0.0]),
            unit(&[1.0, 0.05, 0.0]),
            unit(&[0.98, 0.12, 0.02]),
            unit(&[0.99, 0.08, 0.01]),
            unit(&[0.0, 0.10, 1.0]),
            unit(&[0.02, 0.05, 1.0]),
            unit(&[0.0, 0.12, 0.98]),
            unit(&[0.01, 0.08, 0.99]),
        ]
    }

    #[test]
    fn l2_normalize_makes_unit_vectors_and_ignores_zero() {
        let mut v = vec![3.0, 4.0];
        l2_normalize(&mut v);
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-6);
        let mut z = vec![0.0, 0.0];
        l2_normalize(&mut z);
        assert_eq!(z, vec![0.0, 0.0]);
    }

    #[test]
    fn cosine_is_one_for_parallel_zero_for_orthogonal() {
        let a = unit(&[1.0, 1.0]);
        assert!((cosine(&a, &a) - 1.0).abs() < 1e-6);
        let b = unit(&[1.0, -1.0]);
        assert!(cosine(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn cluster_groups_two_identities() {
        let labels = cluster(&two_identities(), Thresholds::default());
        assert_eq!(count(&labels), 2, "got {labels:?}");
        assert!(labels[0..4].iter().all(|&l| l == labels[0]));
        assert!(labels[4..8].iter().all(|&l| l == labels[4]));
        assert_ne!(labels[0], labels[4]);
    }

    /// The regression this rewrite exists for.
    #[test]
    fn a_bridging_face_does_not_merge_two_identities() {
        // One extra face sitting exactly between the two identities — the
        // profile shot, the 30-pixel bystander, the motion-blurred frame. It is
        // similar enough to *both* to link to both under single linkage, which
        // is how two strangers used to become one person.
        let mut e = two_identities();
        e.push(unit(&[0.72, 0.10, 0.72]));

        let bridge = e.len() - 1;
        // The bridge really does link to a member of each identity at the old
        // 0.45 threshold — otherwise this test would prove nothing. It reaches
        // ~0.71 to both, which is well past anything a real bridging face
        // manages, so this is a harder case than the field produces.
        assert!(cosine(&e[bridge], &e[0]) >= PERSON_THRESHOLD);
        assert!(cosine(&e[bridge], &e[4]) >= PERSON_THRESHOLD);

        let old = {
            // What single linkage at the old threshold would have done.
            let mut parent: Vec<usize> = (0..e.len()).collect();
            for i in 0..e.len() {
                for j in i + 1..e.len() {
                    if cosine(&e[i], &e[j]) >= PERSON_THRESHOLD {
                        union(&mut parent, i, j);
                    }
                }
            }
            let mut roots: Vec<usize> = (0..e.len()).map(|i| find(&mut parent, i)).collect();
            roots.sort_unstable();
            roots.dedup();
            roots.len()
        };
        assert_eq!(
            old, 1,
            "the old algorithm collapsed everyone into one person"
        );

        let labels = cluster(&e, Thresholds::default());
        assert_ne!(
            labels[0], labels[4],
            "the two identities must stay apart despite the bridge: {labels:?}"
        );
    }

    #[test]
    fn an_average_link_merge_reunites_a_person_split_by_pose() {
        // Two tight clusters of the same person — say a front-on set and a
        // three-quarter set — that never quite reach `core` across the divide,
        // but whose averages plainly agree. Single linkage would leave these as
        // two people forever.
        let e = vec![
            unit(&[1.0, 0.80, 0.0]),
            unit(&[1.0, 0.82, 0.0]),
            unit(&[1.0, 0.0, 0.80]),
            unit(&[1.0, 0.0, 0.82]),
        ];
        let t = Thresholds::default();
        assert!(
            cosine(&e[0], &e[2]) < t.core,
            "the two poses must not link directly, or this proves nothing"
        );
        assert_eq!(count(&cluster(&e, t)), 1);
    }

    #[test]
    fn a_strict_core_with_no_merge_splits_everyone_apart() {
        let e = two_identities();
        // Above 1.0, so no pair of unit vectors can reach either stage.
        let labels = cluster(
            &e,
            Thresholds {
                core: 1.01,
                merge: 1.01,
                member_floor: 0.0,
            },
        );
        assert_eq!(count(&labels), e.len());
    }

    /// Unit vectors with a chosen Gram matrix: a shared component `common`,
    /// plus `pair` more shared by the first two only, plus a private
    /// component each. Every pair scores `common`, except the first two,
    /// which score `common + pair`.
    fn crowd(n: usize, common: f32, pair: f32) -> Vec<Vec<f32>> {
        let dim = n + 2;
        (0..n)
            .map(|i| {
                let mut v = vec![0.0f32; dim];
                v[0] = common.sqrt();
                let private = if i < 2 {
                    v[1] = pair.sqrt();
                    1.0 - common - pair
                } else {
                    1.0 - common
                };
                v[2 + i] = private.sqrt();
                v
            })
            .collect()
    }

    /// The snowball this rewrite exists for. Two strangers agree at 0.45 and
    /// merge; everyone else agrees with everyone at 0.35, below the bar. A
    /// re-normalised centroid of the merged pair scores 0.41 against each of
    /// the others, over the bar, and each merge makes the next easier, until
    /// one "person" holds everybody. The average agreement stays at 0.35, so
    /// average linkage keeps them apart.
    #[test]
    fn a_merged_pair_does_not_snowball_into_everyone() {
        let e = crowd(6, 0.35, 0.10);
        assert!((cosine(&e[0], &e[1]) - 0.45).abs() < 1e-5);
        assert!((cosine(&e[0], &e[2]) - 0.35).abs() < 1e-5);
        let t = Thresholds {
            core: 0.99,
            merge: 0.40,
            member_floor: 0.0,
        };

        // The old criterion, reproduced: the normalised centroid of 0+1
        // against face 2 clears 0.40.
        let mut pair = sum_of(&e, &[0, 1]);
        l2_normalize(&mut pair);
        assert!(
            cosine(&pair, &e[2]) > t.merge,
            "or this test proves nothing"
        );

        let labels = cluster(&e, t);
        assert_eq!(labels[0], labels[1], "the pair itself does merge");
        assert_eq!(count(&labels), 5, "and nobody else joins it: {labels:?}");
    }

    #[test]
    fn an_outlier_is_evicted_from_a_group_that_does_not_support_it() {
        // Three faces that agree, plus one that only got in via a single link
        // to one of them and disagrees with the group's average.
        let e = vec![
            unit(&[1.0, 0.0, 0.0]),
            unit(&[1.0, 0.02, 0.0]),
            unit(&[1.0, 0.04, 0.0]),
            unit(&[0.3, 0.95, 0.0]),
        ];
        let labels = cluster(
            &e,
            Thresholds {
                core: 0.29, // loose enough to drag the outlier in
                merge: 0.99,
                member_floor: 0.8,
            },
        );
        assert_ne!(labels[3], labels[0], "the outlier should be evicted");
        assert_eq!(labels[0], labels[1]);
    }

    #[test]
    fn a_pair_is_never_dismantled_by_eviction() {
        // Two faces linked at `core` are all the evidence there is about each
        // other; there is no majority for them to disagree with.
        let e = vec![unit(&[1.0, 0.6, 0.0]), unit(&[1.0, 0.62, 0.0])];
        let labels = cluster(
            &e,
            Thresholds {
                core: 0.9,
                merge: 0.99,
                member_floor: 0.999,
            },
        );
        assert_eq!(count(&labels), 1);
    }

    #[test]
    fn a_stopped_clustering_returns_nothing_and_reports_its_progress() {
        let e = two_identities();
        let mut seen = Vec::new();
        let full = cluster_with(&e, Thresholds::default(), &|| false, &mut |d, n| {
            seen.push((d, n))
        });
        assert_eq!(full, Some(cluster(&e, Thresholds::default())));
        assert_eq!(seen.first(), Some(&(0, e.len())));
        assert_eq!(seen.len(), e.len());

        // Asked to stop partway through the pairwise pass: after its first
        // block, which is all it reports.
        let e: Vec<Vec<f32>> = (0..3 * CORE_BLOCK)
            .map(|i| unit(&[1.0, i as f32, (i * i) as f32 % 7.0]))
            .collect();
        let calls = std::cell::Cell::new(0);
        let stop = || {
            calls.set(calls.get() + 1);
            calls.get() > 1
        };
        let mut seen = Vec::new();
        assert_eq!(
            cluster_with(&e, Thresholds::default(), &stop, &mut |d, n| {
                seen.push((d, n))
            }),
            None
        );
        assert_eq!(seen.len(), CORE_BLOCK);
        assert_eq!(seen.last(), Some(&(CORE_BLOCK - 1, e.len())));
    }

    /// The pairwise pass runs in blocks of rows against tiles of columns, in
    /// parallel. A face must still find its double wherever the two fall:
    /// the same block, neighbouring tiles, or the far ends of the library.
    #[test]
    fn every_pair_is_found_across_blocks_and_tiles() {
        let n = 2 * CORE_TILE + CORE_BLOCK + 5;
        let mut seed = 0x5EEDu64;
        let mut e: Vec<Vec<f32>> = (0..n)
            .map(|_| {
                let v: Vec<f32> = (0..512)
                    .map(|_| {
                        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                        (seed >> 40) as f32 / (1u64 << 24) as f32 - 0.5
                    })
                    .collect();
                unit(&v)
            })
            .collect();
        let pairs = [
            (0, n - 1),
            (CORE_BLOCK - 1, CORE_BLOCK),
            (CORE_BLOCK, CORE_TILE + 2),
            (CORE_TILE, CORE_TILE + 1),
            (3, 4),
        ];
        for &(a, b) in &pairs {
            e[b] = e[a].clone();
        }
        let labels = cluster(&e, Thresholds::default());
        for &(a, b) in &pairs {
            assert_eq!(labels[a], labels[b], "{a} and {b} were not grouped");
        }
        // Everyone else is a stranger: n faces, one fewer group per pair (the
        // second and third pairs share a face, so they chain into one).
        assert_eq!(count(&labels), n - pairs.len());
    }

    #[test]
    fn empty_input_is_no_people() {
        assert_eq!(cluster(&[], Thresholds::default()), Vec::<usize>::new());
        assert_eq!(count(&[]), 0);
    }

    #[test]
    fn clustering_is_deterministic() {
        let e = two_identities();
        assert_eq!(
            cluster(&e, Thresholds::default()),
            cluster(&e, Thresholds::default())
        );
    }

    #[test]
    fn is_groupable_rejects_only_what_cannot_be_embedded() {
        assert!(is_groupable(Some(0.95), 120.0, 140.0));
        assert!(!is_groupable(Some(0.3), 120.0, 140.0), "low confidence");
        assert!(!is_groupable(Some(0.95), 12.0, 140.0), "too small to embed");
        // A face stored before detection confidence was recorded is judged on
        // size alone, rather than discarding an existing library's face table.
        assert!(is_groupable(None, 120.0, 140.0));
        assert!(!is_groupable(None, 12.0, 12.0));
    }

    /// The regression that produced a wall of one-photo people.
    #[test]
    fn an_ordinary_face_in_a_wide_shot_still_counts() {
        // 48px in a 1600px preview is 3% of the long edge — a person standing a
        // few metres away in a landscape frame. That is most of a holiday
        // library, and the old 64px floor rejected all of it.
        assert!(is_groupable(Some(0.9), 48.0, 56.0));
        assert!(is_groupable(Some(0.75), 34.0, 40.0));
    }

    #[test]
    fn pose_offset_reads_frontal_three_quarter_and_profile() {
        // Eyes level at x = 40 and 60, nose centred: face-on.
        let frontal = [
            [40.0, 50.0],
            [60.0, 50.0],
            [50.0, 60.0],
            [42.0, 70.0],
            [58.0, 70.0],
        ];
        assert!(pose_offset(&frontal) < 0.01);
        // Nose 8px off the eyes' midpoint over a 20px eye span: turned.
        let turned = [
            [40.0, 50.0],
            [60.0, 50.0],
            [58.0, 60.0],
            [45.0, 70.0],
            [60.0, 70.0],
        ];
        assert!((pose_offset(&turned) - 0.4).abs() < 1e-5);
        // Eyes on top of each other: no pose can be read, so it never votes.
        let collapsed = [
            [50.0, 50.0],
            [50.0, 50.0],
            [50.0, 60.0],
            [45.0, 70.0],
            [55.0, 70.0],
        ];
        assert!(pose_offset(&collapsed).is_infinite());
    }

    #[test]
    fn only_clear_faces_vote() {
        assert!(votes(Some(0.9), 80.0, 90.0, Some(0.1)));
        assert!(!votes(Some(0.7), 80.0, 90.0, Some(0.1)), "weak detection");
        assert!(!votes(Some(0.9), 80.0, 90.0, Some(1.2)), "a profile");
        assert!(!votes(Some(0.9), 20.0, 90.0, Some(0.1)), "too small at all");
        // Rows from before score and pose were stored are judged on size.
        assert!(votes(None, 80.0, 90.0, None));
        // Everything that votes is groupable; not the other way round.
        assert!(is_groupable(Some(0.7), 80.0, 90.0));
    }

    #[test]
    fn a_face_attaches_to_the_group_it_agrees_with_and_no_other() {
        let voters = two_identities();
        let labels = cluster(&voters, Thresholds::default());
        let a_ish = unit(&[0.9, 0.3, 0.2]);
        let nobody = unit(&[0.0, 1.0, 0.0]);
        let got = attach(&voters, &labels, &[a_ish, nobody], 0.40);
        assert_eq!(got, vec![Some(labels[0]), None]);
    }

    #[test]
    fn suggestions_are_near_misses_best_first_and_respect_not_the_same() {
        // Three groups: 0 and 1 close but not merged, 2 far from both.
        let e = vec![
            unit(&[1.0, 0.0, 0.0]),
            unit(&[1.0, 0.02, 0.0]),
            unit(&[0.5, 0.85, 0.0]),
            unit(&[0.52, 0.85, 0.0]),
            unit(&[0.0, 0.0, 1.0]),
        ];
        let labels = vec![0, 0, 1, 1, 2];
        let never = |_: usize, _: usize| false;
        let s = suggest(&e, &labels, 0.25, 10, &never, &|_| false);
        assert_eq!(s.len(), 1, "only 0-1 is close enough: {s:?}");
        assert_eq!((s[0].0, s[0].1), (0, 1));
        assert!(s[0].2 > 0.25 && s[0].2 < 0.75);

        let told_apart = |a: usize, b: usize| (a, b) == (0, 1);
        assert!(suggest(&e, &labels, 0.25, 10, &told_apart, &|_| false).is_empty());
        assert!(
            suggest(&e, &labels, 0.25, 0, &never, &|_| false).is_empty(),
            "limit"
        );
    }

    #[test]
    fn apply_links_merges_what_the_user_says_is_one_person() {
        // Two identities the algorithm kept apart; the user says faces 0 and 4
        // are the same person.
        let labels = cluster(&two_identities(), Thresholds::default());
        assert_eq!(count(&labels), 2);
        let merged = apply_links(&labels, &[(0, 4)]);
        assert_eq!(count(&merged), 1, "got {merged:?}");
    }

    #[test]
    fn apply_links_pulls_whole_groups_together_not_just_the_linked_pair() {
        let labels = cluster(&two_identities(), Thresholds::default());
        let merged = apply_links(&labels, &[(0, 4)]);
        // Every member of both original groups must come along, or a merge
        // would only move the one face the user happened to click.
        assert!(merged.iter().all(|&l| l == merged[0]));
    }

    #[test]
    fn apply_links_ignores_links_to_faces_that_are_gone() {
        // Links survive in the database across regroups; a face deleted with
        // its photo must not panic the next grouping run.
        let labels = vec![0, 0, 1];
        assert_eq!(apply_links(&labels, &[(0, 99)]), vec![0, 0, 1]);
        assert_eq!(apply_links(&[], &[(0, 1)]), Vec::<usize>::new());
    }

    #[test]
    fn a_single_face_needs_a_stronger_match_to_be_suggested() {
        // A pair of faces and a lone face, agreeing at 0.30: enough between
        // two groups, not enough when one side is a single observation.
        let a = unit(&[1.0, 0.0, 0.0]);
        let b = unit(&[0.30, 0.954, 0.0]);
        let never = |_: usize, _: usize| false;
        let groups = suggest(
            &[a.clone(), a.clone(), b.clone(), b.clone()],
            &[0, 0, 1, 1],
            0.25,
            5,
            &never,
            &|_| false,
        );
        assert_eq!(groups.len(), 1);
        let lone = suggest(&[a.clone(), a, b], &[0, 0, 1], 0.25, 5, &never, &|_| false);
        assert!(lone.is_empty(), "{lone:?}");
    }

    /// Two single faces are only compared when one of them is someone the
    /// user named: every stranger against every other was P² work on
    /// every People load.
    #[test]
    fn two_single_faces_are_compared_only_when_one_is_named() {
        let a = unit(&[1.0, 0.0, 0.0]);
        let b = unit(&[0.9, 0.436, 0.0]);
        let never = |_: usize, _: usize| false;
        assert!(
            suggest(&[a.clone(), b.clone()], &[0, 1], 0.25, 5, &never, &|_| {
                false
            })
            .is_empty()
        );
        let named = suggest(&[a, b], &[0, 1], 0.25, 5, &never, &|g| g == 1);
        assert_eq!(
            named.iter().map(|s| (s.0, s.1)).collect::<Vec<_>>(),
            vec![(0, 1)]
        );
    }

    /// New faces join the person whose stored sum they agree with, as
    /// they would have in a regroup.
    #[test]
    fn a_new_face_attaches_to_the_stored_sums() {
        let ids = two_identities();
        let labels = cluster(&ids, Thresholds::default());
        let (sums, sizes) = group_sums(&ids, &labels);
        let placed = attach_to_sums(&sums, &sizes, &[ids[0].clone(), ids[4].clone()], 0.45);
        assert_eq!(placed, vec![Some(labels[0]), Some(labels[4])]);
    }

    #[test]
    fn the_cover_is_a_big_face_looking_at_the_camera() {
        let frontal_big = cover_quality(Some(0.9), 120.0, 130.0, Some(0.05));
        let back_of_head = cover_quality(Some(0.7), 150.0, 150.0, Some(1.4));
        let frontal_small = cover_quality(Some(0.9), 40.0, 45.0, Some(0.05));
        assert!(frontal_big > back_of_head);
        assert!(frontal_big > frontal_small);
    }
}
