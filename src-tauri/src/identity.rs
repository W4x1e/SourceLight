//! The move to SourceLight's own identifier.
//!
//! The identifier names the app's data folder: the catalogue, its backups,
//! the previews, the downloaded models and, on Windows, the webview's own
//! storage, where the interface keeps its settings. Builds before the rename
//! used a working identifier, so the first launch under the new one moves
//! that folder across, once, and rewrites the preview paths the catalogue
//! keeps into it. Without this every existing library would look empty.
//!
//! It runs before the app is built, because the window, and with it the
//! webview's storage in the new folder, exists before the setup hook. So the
//! folders are found by hand, as `bin/seed_library.rs` finds the catalogue.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The identifier each build used before the rename, by the one it has now.
const FORMER: &[(&str, &str)] = &[
    ("app.sourcelight", "com.jamestully.photoorganiser"),
    ("app.sourcelight.test", "com.jamestully.photoorganiser.test"),
];

/// The old folder, when it could not be moved this launch (another copy of
/// an older build has files in it open, say): the app uses it where it is,
/// and the next launch tries again.
static STAYED: OnceLock<PathBuf> = OnceLock::new();

/// The data folder to use instead of the new one, if the move failed.
pub fn stayed() -> Option<&'static Path> {
    STAYED.get().map(PathBuf::as_path)
}

/// What [`adopt_former_data`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum Adopted {
    /// No older folder, or a catalogue already lives in the new one.
    Nothing,
    /// The old folder is now the new one.
    Moved,
    /// The old folder couldn't be moved and is used where it is.
    Stayed,
}

/// Moves the data folder of the build before the rename to where this one
/// looks, if there is one and no catalogue is in the new folder yet.
pub fn adopt_former_data(identifier: &str) -> Adopted {
    let Some(former) = FORMER
        .iter()
        .find(|(now, _)| *now == identifier)
        .map(|(_, was)| *was)
    else {
        return Adopted::Nothing;
    };
    let Some(base) = local_data_base() else {
        return Adopted::Nothing;
    };
    let adopted = adopt(
        &base.join(former),
        &base.join(identifier),
        former,
        identifier,
    );
    if adopted == Adopted::Stayed {
        let _ = STAYED.set(base.join(former));
    }
    let moved = adopted == Adopted::Moved;
    // The config folder, where the window keeps its size and place, is
    // another folder on Windows (roaming) and Linux. On a Mac it is the data
    // folder, moved above.
    if let Some(config) = config_base().filter(|c| *c != base) {
        let _ = adopt_beside(&config.join(former), &config.join(identifier), moved);
    }
    // WebKit keeps a Mac app's web storage, the interface's settings among
    // it, in a folder of its own named by the identifier.
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        let webkit = PathBuf::from(home).join("Library/WebKit");
        let _ = adopt_beside(&webkit.join(former), &webkit.join(identifier), moved);
    }
    adopted
}

/// The data folders a build named `identifier` may be using, found without
/// Tauri: its own, then the one from before the rename, which a launch keeps
/// using when it could not move it ([`stayed`]).
pub fn data_folders(identifier: &str) -> Vec<PathBuf> {
    let Some(base) = local_data_base() else {
        return Vec::new();
    };
    std::iter::once(identifier)
        .chain(
            FORMER
                .iter()
                .filter(|(now, _)| *now == identifier)
                .map(|(_, was)| *was),
        )
        .map(|name| base.join(name))
        .collect()
}

/// The parent of Tauri's `app_config_dir`.
fn config_base() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        local_data_base()
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }
}

/// The folder the OS keeps apps' local data in, the parent of Tauri's
/// `app_local_data_dir`.
fn local_data_base() -> Option<PathBuf> {
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    }
}

/// `old` moved to `new`, and the catalogue's paths into it rewritten.
fn adopt(old: &Path, new: &Path, former: &str, identifier: &str) -> Adopted {
    match adopt_folder(old, new) {
        Ok(false) => Adopted::Nothing,
        Ok(true) => {
            if let Err(e) = rebase_catalogue(&new.join("library.db"), former, identifier) {
                // The previews are then found again by the next scan, which
                // remakes any it can't find.
                eprintln!("moved the data folder, but could not update the catalogue's preview paths: {e:#}");
            }
            eprintln!(
                "moved the data folder from {} to {}",
                old.display(),
                new.display()
            );
            Adopted::Moved
        }
        Err(e) => {
            eprintln!(
                "could not move the data folder from {} to {}, using it where it is: {e}",
                old.display(),
                new.display()
            );
            Adopted::Stayed
        }
    }
}

/// Moves the data folder `old` to `new` when there is an `old` and no
/// catalogue lives in `new`. `Ok(false)` when there was nothing to do.
///
/// A new folder without a catalogue can still hold things. Each run of this
/// build writes the window's size and place there on a Mac, and the
/// webview's storage there on Windows and Linux. That includes a development
/// run pointed at another catalogue, and a launch that used the old folder
/// where it was. Neither may stop the library moving, or the app opens an
/// empty one.
fn adopt_folder(old: &Path, new: &Path) -> std::io::Result<bool> {
    if !old.is_dir() || new.join("library.db").exists() {
        return Ok(false);
    }
    replace(old, new)?;
    Ok(true)
}

/// Moves a folder beside the data folder (the window's or the interface's
/// settings) from `old` to `new`. Once the library has moved, they follow it
/// over whatever a run of this build left in their place; otherwise only into
/// a `new` that holds nothing.
fn adopt_beside(old: &Path, new: &Path, library_moved: bool) -> std::io::Result<bool> {
    if !old.is_dir() || (!library_moved && holds_anything(new)) {
        return Ok(false);
    }
    replace(old, new)?;
    Ok(true)
}

/// Files a run leaves that hold nothing worth keeping: where the window was,
/// which the old folder has its own of, and Finder's view of the folder.
const LEFTOVERS: &[&str] = &[".window-state.json", ".DS_Store"];

/// Renames `old` to `new`, clearing the way first. [`LEFTOVERS`] are deleted;
/// anything else in `new` is set aside beside it under a dated name, never
/// deleted.
fn replace(old: &Path, new: &Path) -> std::io::Result<()> {
    if new.is_dir() {
        for name in LEFTOVERS {
            match std::fs::remove_file(new.join(name)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        if holds_anything(new) {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let mut aside = new.as_os_str().to_os_string();
            aside.push(format!(".set-aside-{secs}"));
            std::fs::rename(new, aside)?;
        } else {
            std::fs::remove_dir(new)?;
        }
    }
    std::fs::rename(old, new)
}

fn holds_anything(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some())
}

/// Rewrites the previews' paths, which the catalogue keeps whole, from the
/// old folder to the new. Matched on the identifier as one folder of the path,
/// whichever separator and however the folder above it is spelled.
fn rebase_catalogue(db: &Path, former: &str, identifier: &str) -> anyhow::Result<usize> {
    if !db.is_file() {
        return Ok(0);
    }
    let mut conn = rusqlite::Connection::open(db)?;
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    let tx = conn.transaction()?;
    let mut changed = 0;
    for sep in ['\\', '/'] {
        let from = format!("{sep}{former}{sep}");
        let to = format!("{sep}{identifier}{sep}");
        for column in ["thumb_path", "preview_path"] {
            changed += tx.execute(
                &format!(
                    "UPDATE photos SET {column} = replace({column}, ?1, ?2)
                      WHERE instr({column}, ?1) > 0"
                ),
                rusqlite::params![from, to],
            )?;
        }
    }
    tx.commit()?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Scratch;

    const OLD: &str = "com.jamestully.photoorganiser";
    const NEW: &str = "app.sourcelight";

    fn catalogue_at(dir: &Path, thumb: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let conn = rusqlite::Connection::open(dir.join("library.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE photos (id INTEGER PRIMARY KEY, thumb_path TEXT, preview_path TEXT)",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO photos (thumb_path, preview_path) VALUES (?1, NULL)",
            [thumb],
        )
        .unwrap();
    }

    #[test]
    fn the_old_folder_moves_and_its_preview_paths_follow() {
        let base = Scratch::new("identity-move");
        let old = base.path().join(OLD);
        let new = base.path().join(NEW);
        let thumb = format!(r"C:\Users\a\AppData\Local\{OLD}\thumbs\ab_t.jpg");
        catalogue_at(&old, &thumb);
        std::fs::write(old.join("settings.txt"), "kept").unwrap();

        assert_eq!(adopt(&old, &new, OLD, NEW), Adopted::Moved);
        assert!(!old.exists());
        assert_eq!(
            std::fs::read_to_string(new.join("settings.txt")).unwrap(),
            "kept"
        );
        let conn = rusqlite::Connection::open(new.join("library.db")).unwrap();
        let now: String = conn
            .query_row("SELECT thumb_path FROM photos", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            now,
            format!(r"C:\Users\a\AppData\Local\{NEW}\thumbs\ab_t.jpg")
        );
    }

    #[test]
    fn an_empty_new_folder_is_no_obstacle() {
        let base = Scratch::new("identity-empty");
        let old = base.path().join(OLD);
        let new = base.path().join(NEW);
        catalogue_at(&old, &format!("/home/a/.local/share/{OLD}/thumbs/x.jpg"));
        std::fs::create_dir_all(&new).unwrap();
        assert_eq!(adopt(&old, &new, OLD, NEW), Adopted::Moved);
        assert!(new.join("library.db").is_file());
    }

    /// What a run of the new build left, its catalogue elsewhere: a
    /// development run, or a launch that used the old folder where it was.
    #[test]
    fn where_the_window_was_is_no_obstacle() {
        let base = Scratch::new("identity-window-state");
        let old = base.path().join(OLD);
        let new = base.path().join(NEW);
        catalogue_at(&old, "x");
        std::fs::write(old.join(".window-state.json"), "old").unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join(".window-state.json"), "dev").unwrap();
        std::fs::write(new.join(".DS_Store"), "").unwrap();

        assert_eq!(adopt(&old, &new, OLD, NEW), Adopted::Moved);
        assert!(new.join("library.db").is_file());
        assert_eq!(
            std::fs::read_to_string(new.join(".window-state.json")).unwrap(),
            "old"
        );
        assert!(set_aside(base.path()).is_empty(), "nothing worth keeping");
    }

    #[test]
    fn a_webviews_storage_is_set_aside_not_lost() {
        let base = Scratch::new("identity-webview");
        let old = base.path().join(OLD);
        let new = base.path().join(NEW);
        catalogue_at(&old, "x");
        let storage = Path::new("EBWebView/Default/Local Storage/leveldb");
        std::fs::create_dir_all(new.join(storage)).unwrap();
        std::fs::write(new.join(storage).join("000003.log"), "settings").unwrap();

        assert_eq!(adopt(&old, &new, OLD, NEW), Adopted::Moved);
        assert!(new.join("library.db").is_file());
        assert!(!new.join("EBWebView").exists());
        let aside = set_aside(base.path());
        assert_eq!(aside.len(), 1);
        assert_eq!(
            std::fs::read_to_string(aside[0].join(storage).join("000003.log")).unwrap(),
            "settings"
        );
    }

    #[test]
    fn a_new_folder_in_use_is_left_alone() {
        let base = Scratch::new("identity-in-use");
        let old = base.path().join(OLD);
        let new = base.path().join(NEW);
        catalogue_at(&old, "x");
        catalogue_at(&new, "y");
        std::fs::write(new.join(".window-state.json"), "new").unwrap();
        assert_eq!(adopt(&old, &new, OLD, NEW), Adopted::Nothing);
        assert!(
            old.join("library.db").is_file(),
            "the old folder stays as it was"
        );
        assert!(new.join(".window-state.json").is_file());
        assert!(set_aside(base.path()).is_empty());
    }

    #[test]
    fn the_settings_beside_follow_the_library() {
        let base = Scratch::new("identity-beside");
        let old = base.path().join(OLD);
        let new = base.path().join(NEW);
        let settings = Path::new("WebsiteData/LocalStorage");
        for (dir, what) in [(&old, "old"), (&new, "dev")] {
            std::fs::create_dir_all(dir.join(settings)).unwrap();
            std::fs::write(dir.join(settings).join("x"), what).unwrap();
        }

        // Kept apart while the library stays where it is.
        assert!(!adopt_beside(&old, &new, false).unwrap());
        assert_eq!(
            std::fs::read_to_string(new.join(settings).join("x")).unwrap(),
            "dev"
        );

        assert!(adopt_beside(&old, &new, true).unwrap());
        assert!(!old.exists());
        assert_eq!(
            std::fs::read_to_string(new.join(settings).join("x")).unwrap(),
            "old"
        );
        assert_eq!(set_aside(base.path()).len(), 1);
    }

    /// The folders set aside beside the new one.
    fn set_aside(base: &Path) -> Vec<PathBuf> {
        let prefix = format!("{NEW}.set-aside-");
        std::fs::read_dir(base)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&prefix))
            })
            .collect()
    }

    #[test]
    fn nothing_to_move_is_nothing() {
        let base = Scratch::new("identity-none");
        assert_eq!(
            adopt(&base.path().join(OLD), &base.path().join(NEW), OLD, NEW),
            Adopted::Nothing
        );
    }

    #[test]
    fn only_the_renamed_builds_look_for_a_former_folder() {
        assert_eq!(adopt_former_data("com.example.other"), Adopted::Nothing);
    }
}
