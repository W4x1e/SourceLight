//! Byte-identical re-imports.
//!
//! Duplicate review groups by CLIP embedding, which answers "do these look the
//! same" — and so can miss the commonest real duplicate of all: the same card
//! imported twice, into two folders or onto two drives. The scanner cannot
//! catch it either, because its quick hash includes the file's mtime and the
//! catalogue keys on path.
//!
//! This finds files that are the same bytes, on demand, without hashing the
//! library: group by size (one `stat` each), then by the first 64 KB, and only
//! then SHA-256 whatever still collides. Compressed raws almost never share a
//! size, so the expensive step rarely runs; an uncompressed body whose files
//! are all one size falls through to the 64 KB check, which differs between
//! any two real exposures.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;

const HEAD_BYTES: usize = 64 * 1024;

/// Groups of photo ids whose files are byte-identical, each sorted ascending
/// so the first — the earliest import — is a stable keeper. Photos sharing a
/// *path* are one file, not duplicates of it (virtual copies do that), so a
/// path is only considered once. Unreadable files — an unplugged drive — are
/// skipped, not reported.
pub fn exact_groups(photos: &[(i32, PathBuf)]) -> Vec<Vec<i32>> {
    let mut seen_paths = std::collections::HashSet::new();
    let mut by_size: HashMap<u64, Vec<(i32, &PathBuf)>> = HashMap::new();
    for (id, path) in photos {
        if !seen_paths.insert(path) {
            continue;
        }
        if let Ok(md) = std::fs::metadata(path) {
            if md.is_file() && md.len() > 0 {
                by_size.entry(md.len()).or_default().push((*id, path));
            }
        }
    }

    let mut groups = Vec::new();
    for same_size in by_size.into_values().filter(|g| g.len() > 1) {
        for same_head in split_by(same_size, head) {
            for same_bytes in split_by(same_head, full_digest) {
                // One file reached by two paths — a NAS as `\\nas\photos`
                // and as `Z:\`, a hard link — is not its own duplicate:
                // resolving the pair would bin the only copy. Kept once, by
                // its first id.
                let mut same_bytes = same_bytes;
                same_bytes.sort_by_key(|(id, _)| *id);
                let mut files: Vec<same_file::Handle> = Vec::new();
                let mut ids = Vec::new();
                for (id, path) in same_bytes {
                    let Ok(handle) = same_file::Handle::from_path(path) else {
                        continue;
                    };
                    if !files.contains(&handle) {
                        files.push(handle);
                        ids.push(id);
                    }
                }
                if ids.len() > 1 {
                    groups.push(ids);
                }
            }
        }
    }
    groups.sort_by_key(|g| g[0]);
    groups
}

/// Splits a candidate group by `key`, keeping only sub-groups of two or more.
/// A file whose key cannot be read drops out.
fn split_by<K: Eq + std::hash::Hash>(
    group: Vec<(i32, &PathBuf)>,
    key: impl Fn(&PathBuf) -> Option<K>,
) -> Vec<Vec<(i32, &PathBuf)>> {
    let mut out: HashMap<K, Vec<(i32, &PathBuf)>> = HashMap::new();
    for (id, path) in group {
        if let Some(k) = key(path) {
            out.entry(k).or_default().push((id, path));
        }
    }
    out.into_values().filter(|g| g.len() > 1).collect()
}

fn head(path: &PathBuf) -> Option<Vec<u8>> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; HEAD_BYTES];
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(_) => return None,
        }
    }
    buf.truncate(n);
    Some(buf)
}

fn full_digest(path: &PathBuf) -> Option<[u8; 32]> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(_) => return None,
        }
    }
    Some(hasher.finalize().into())
}

/// Folds overlapping groups into one: an exact copy of a frame that is also
/// in a burst belongs in that burst's review, not in a second set of its own.
/// Groups keep the order of their first appearance, and each is sorted by id
/// so its first member stays the stable keeper.
pub fn merge_overlapping(groups: Vec<Vec<i32>>) -> Vec<Vec<i32>> {
    let mut merged: Vec<Vec<i32>> = Vec::new();
    for group in groups {
        let hits: Vec<usize> = merged
            .iter()
            .enumerate()
            .filter(|(_, m)| m.iter().any(|id| group.contains(id)))
            .map(|(i, _)| i)
            .collect();
        match hits.split_first() {
            None => merged.push(group),
            Some((&first, rest)) => {
                // Later hits fold into the first; remove from the back so
                // indices stay valid.
                for &i in rest.iter().rev() {
                    let absorbed = merged.remove(i);
                    merged[first].extend(absorbed);
                }
                merged[first].extend(group);
                let mut seen = std::collections::HashSet::new();
                merged[first].retain(|id| seen.insert(*id));
            }
        }
    }
    for g in &mut merged {
        g.sort_unstable();
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> crate::test_support::Scratch {
        crate::test_support::Scratch::new(name)
    }

    #[test]
    fn a_copy_on_another_path_is_found_and_a_lookalike_is_not() {
        let d = dir("dedup-exact");
        let body: Vec<u8> = (0..200_000u32).map(|i| (i * 7 % 251) as u8).collect();
        std::fs::create_dir_all(d.join("card-a")).unwrap();
        std::fs::create_dir_all(d.join("drive-b")).unwrap();
        std::fs::write(d.join("card-a/DSC1.ARW"), &body).unwrap();
        std::fs::write(d.join("drive-b/DSC1.ARW"), &body).unwrap();
        // Same size, same first 64 KB, one byte different near the end: the
        // case only the full digest can separate.
        let mut tail = body.clone();
        *tail.last_mut().unwrap() ^= 1;
        std::fs::write(d.join("drive-b/DSC2.ARW"), &tail).unwrap();
        // Same size, different head.
        let mut other = body.clone();
        other[0] ^= 1;
        std::fs::write(d.join("drive-b/DSC3.ARW"), &other).unwrap();

        let photos = vec![
            (1, d.join("card-a/DSC1.ARW")),
            (7, d.join("drive-b/DSC1.ARW")),
            (8, d.join("drive-b/DSC2.ARW")),
            (9, d.join("drive-b/DSC3.ARW")),
        ];
        assert_eq!(exact_groups(&photos), vec![vec![1, 7]]);
    }

    #[test]
    fn a_virtual_copy_is_not_its_own_duplicate() {
        let d = dir("dedup-variant");
        std::fs::write(d.join("a.ARW"), b"same bytes").unwrap();
        // Two catalogue rows, one file: the master and a virtual copy.
        let photos = vec![(1, d.join("a.ARW")), (2, d.join("a.ARW"))];
        assert!(exact_groups(&photos).is_empty());
    }

    #[test]
    fn a_missing_file_is_skipped_not_fatal() {
        let d = dir("dedup-missing");
        std::fs::write(d.join("a.ARW"), b"x").unwrap();
        std::fs::write(d.join("b.ARW"), b"x").unwrap();
        let photos = vec![
            (1, d.join("a.ARW")),
            (2, d.join("gone.ARW")),
            (3, d.join("b.ARW")),
        ];
        assert_eq!(exact_groups(&photos), vec![vec![1, 3]]);
    }

    #[test]
    fn overlapping_groups_merge_and_disjoint_ones_do_not() {
        let merged = merge_overlapping(vec![vec![4, 8], vec![5, 6], vec![2, 8], vec![3, 9]]);
        assert_eq!(merged, vec![vec![2, 4, 8], vec![5, 6], vec![3, 9]]);
        // A later group bridging two earlier ones folds all three together.
        let bridged = merge_overlapping(vec![vec![1, 2], vec![3, 4], vec![2, 3]]);
        assert_eq!(bridged, vec![vec![1, 2, 3, 4]]);
    }

    /// One file under two paths is not a duplicate of itself; a real
    /// copy beside it still is.
    #[test]
    fn one_file_under_two_paths_is_not_its_own_duplicate() {
        let d = dir("dedup-same-file");
        let body: Vec<u8> = (0..100_000u32).map(|i| (i * 13 % 251) as u8).collect();
        std::fs::write(d.join("DSC1.ARW"), &body).unwrap();
        // A second name for the same file, as a NAS share and its mapped
        // drive are.
        std::fs::hard_link(d.join("DSC1.ARW"), d.join("alias.ARW")).unwrap();
        let photos = vec![(1, d.join("DSC1.ARW")), (2, d.join("alias.ARW"))];
        assert!(exact_groups(&photos).is_empty());

        std::fs::write(d.join("copy.ARW"), &body).unwrap();
        let photos = vec![
            (1, d.join("DSC1.ARW")),
            (2, d.join("alias.ARW")),
            (3, d.join("copy.ARW")),
        ];
        assert_eq!(exact_groups(&photos), vec![vec![1, 3]]);
    }
}
