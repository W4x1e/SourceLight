//! Images the desktop sent, kept on the phone so the grid and the loupe work
//! offline and nothing is fetched twice: `thumbs/<id>.jpg` (512 px, about
//! 30 KB), `previews/<id>.jpg` (1600 px, about 200 KB), and
//! `edited/<id>_<tag>.jpg`, an edited photo's preview as edited, named by its
//! edits' tag so new edits are fetched anew.
//!
//! No size limit yet: a 20,000-photo library's thumbnails are about 600 MB,
//! and previews are only fetched for photos opened in the loupe. A limit and
//! "keep on phone" for chosen shoots come with the real library screens
//! (docs/MOBILE.md, "What syncs").

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Thumb,
    Preview,
}

pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    pub fn new(dir: &Path) -> Self {
        let cache = Cache {
            dir: dir.to_path_buf(),
        };
        for kind in [Kind::Thumb, Kind::Preview] {
            let _ = std::fs::create_dir_all(cache.folder(kind));
        }
        cache
    }

    fn folder(&self, kind: Kind) -> PathBuf {
        self.dir.join(match kind {
            Kind::Thumb => "thumbs",
            Kind::Preview => "previews",
        })
    }

    fn path(&self, kind: Kind, id: i64) -> PathBuf {
        self.folder(kind).join(format!("{id}.jpg"))
    }

    fn edited_folder(&self) -> PathBuf {
        self.dir.join("edited")
    }

    /// Photo `id`'s preview as edited, when its edits' tag is `tag`.
    pub fn get_edited(&self, id: i64, tag: &str) -> Option<Vec<u8>> {
        std::fs::read(self.edited_folder().join(format!("{id}_{tag}.jpg"))).ok()
    }

    /// Keeps photo `id`'s preview as edited, and lets go of the one for its
    /// older edits.
    pub fn put_edited(&self, id: i64, tag: &str, jpeg: &[u8]) {
        let folder = self.edited_folder();
        let _ = std::fs::create_dir_all(&folder);
        self.forget_edited(id);
        let path = folder.join(format!("{id}_{tag}.jpg"));
        let part = path.with_extension("part");
        if std::fs::write(&part, jpeg).is_ok() {
            let _ = std::fs::rename(&part, &path);
        }
    }

    fn forget_edited(&self, id: i64) {
        let prefix = format!("{id}_");
        for entry in std::fs::read_dir(self.edited_folder())
            .into_iter()
            .flatten()
            .flatten()
        {
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    pub fn get(&self, kind: Kind, id: i64) -> Option<Vec<u8>> {
        std::fs::read(self.path(kind, id)).ok()
    }

    /// Writes through a temporary name, so a crash mid-write never leaves a
    /// half image that would be served from then on.
    pub fn put(&self, kind: Kind, id: i64, jpeg: &[u8]) {
        let path = self.path(kind, id);
        let part = path.with_extension("part");
        if std::fs::write(&part, jpeg).is_ok() {
            let _ = std::fs::rename(&part, &path);
        }
    }

    /// Everything, as when the phone starts copying another library.
    pub fn clear(&self) {
        for kind in [Kind::Thumb, Kind::Preview] {
            let _ = std::fs::remove_dir_all(self.folder(kind));
            let _ = std::fs::create_dir_all(self.folder(kind));
        }
        let _ = std::fs::remove_dir_all(self.edited_folder());
    }

    /// One photo's images, as when the desktop removed it.
    pub fn forget(&self, id: i64) {
        for kind in [Kind::Thumb, Kind::Preview] {
            let _ = std::fs::remove_file(self.path(kind, id));
        }
        self.forget_edited(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_round_trip_and_clear() {
        let dir = std::env::temp_dir().join(format!("sl-cache-{}", std::process::id()));
        let cache = Cache::new(&dir);
        assert_eq!(cache.get(Kind::Thumb, 1), None);
        cache.put(Kind::Thumb, 1, b"thumb");
        cache.put(Kind::Preview, 1, b"preview");
        assert_eq!(cache.get(Kind::Thumb, 1).as_deref(), Some(&b"thumb"[..]));
        assert_eq!(
            cache.get(Kind::Preview, 1).as_deref(),
            Some(&b"preview"[..])
        );
        cache.forget(1);
        assert_eq!(cache.get(Kind::Preview, 1), None);
        cache.put(Kind::Thumb, 2, b"x");
        cache.clear();
        assert_eq!(cache.get(Kind::Thumb, 2), None);

        // New edits replace the old ones' copy.
        cache.put_edited(3, "aa", b"old");
        cache.put_edited(3, "bb", b"new");
        assert_eq!(cache.get_edited(3, "aa"), None);
        assert_eq!(cache.get_edited(3, "bb").as_deref(), Some(&b"new"[..]));
        cache.forget(3);
        assert_eq!(cache.get_edited(3, "bb"), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
