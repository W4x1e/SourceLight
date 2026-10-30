//! Library folders: the folders the user added, the photos in each,
//! and following them when they move.

use super::*;

/// The SQL test "`path` is inside folder `?N`", separator included so that
/// `C:\Photos` does not claim `C:\Photos Old`. `n` is the parameter index.
///
/// A drive or volume root (`E:\`, `/`) already ends in its separator, so the
/// next character is the first of a name, not a separator.
fn under_folder_sql(n: usize) -> String {
    format!(
        "(substr(path, 1, length(?{n})) = ?{n} \
          AND (substr(path, length(?{n}) + 1, 1) IN ('\\', '/') OR substr(?{n}, -1) IN ('\\', '/')))"
    )
}

/// A library folder's path in the one form it is stored and compared in:
/// no trailing separator, except that a drive root keeps its own.
///
/// `E:\` trimmed to `E:` is not the drive's root but its *current directory*:
/// Windows joins `E:` and `Photos` as `E:Photos`, so a scan of the trimmed
/// form found every photo again under paths no other row had, and marked
/// the real ones missing.
pub fn normalise_folder_path(path: &str) -> String {
    let trimmed = path.trim_end_matches(['\\', '/']);
    let bytes = trimmed.as_bytes();
    if bytes.len() == 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        format!("{trimmed}\\")
    } else if trimmed.is_empty() && !path.is_empty() {
        // The Unix root, `/`.
        path[..1].to_string()
    } else {
        trimmed.to_string()
    }
}

/// Puts the separator back on drive-root library folders stored without it
/// by older builds. Run on every open; a no-op once done.
pub(super) fn repair_drive_roots(conn: &Connection) -> Result<()> {
    conn.execute(
        "UPDATE OR IGNORE library_folders SET path = path || '\\'
          WHERE length(path) = 2 AND substr(path, 2, 1) = ':'",
        [],
    )?;
    Ok(())
}

/// Every library folder with its photo counts, and how many photos belong to
/// no folder. `online` is left false; the caller checks the disk.
pub fn list_library_folders(conn: &Connection) -> Result<crate::models::LibraryFolders> {
    let mut stmt = conn.prepare(
        "SELECT f.id, f.path,
                (SELECT COUNT(*) FROM photos p WHERE p.folder_id = f.id AND p.variant = 0),
                (SELECT COUNT(*) FROM photos p
                  WHERE p.folder_id = f.id AND p.variant = 0 AND p.missing = 1)
           FROM library_folders f ORDER BY f.path",
    )?;
    let folders = stmt
        .query_map([], |r| {
            Ok(crate::models::LibraryFolder {
                id: r.get(0)?,
                path: r.get(1)?,
                photos: r.get(2)?,
                missing: r.get(3)?,
                online: false,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let unfiled: i32 = conn.query_row(
        "SELECT COUNT(*) FROM photos WHERE folder_id IS NULL AND variant = 0",
        [],
        |r| r.get(0),
    )?;
    Ok(crate::models::LibraryFolders { folders, unfiled })
}

/// The folder a path is, or `None`. Any spelling of it: `E:` and `E:\` alike.
pub fn folder_id_for(conn: &Connection, path: &str) -> Result<Option<i32>> {
    Ok(conn
        .query_row(
            "SELECT id FROM library_folders WHERE path = ?1",
            params![normalise_folder_path(path)],
            |r| r.get(0),
        )
        .optional()?)
}

/// The library folder a scan of `path` is of, and its path as stored —
/// which is what the scan walks, whatever spelling the page sent.
/// A folder not yet in the library is added, so a direct call still files
/// what it finds.
pub fn folder_for_scan(conn: &mut Connection, path: &str) -> Result<(i32, String)> {
    let id = match folder_id_for(conn, path)? {
        Some(id) => id,
        None => add_library_folder(conn, path)?,
    };
    Ok((id, folder_path(conn, id)?))
}

/// A library folder's stored path.
pub fn folder_path(conn: &Connection, id: i32) -> Result<String> {
    Ok(conn.query_row(
        "SELECT path FROM library_folders WHERE id = ?1",
        params![id],
        |r| r.get(0),
    )?)
}

/// Adds a folder to the library and returns its id.
///
/// Folders do not nest: a folder inside one already in the library is
/// refused (its photos are already there), and a folder that *contains*
/// library folders absorbs them — their photos move under it and the inner
/// rows go, so every photo belongs to exactly one folder.
pub fn add_library_folder(conn: &mut Connection, path: &str) -> Result<i32> {
    let path = normalise_folder_path(path);
    anyhow::ensure!(!path.is_empty(), "choose a folder");
    let new = Path::new(&path);
    let existing: Vec<(i32, String)> = conn
        .prepare("SELECT id, path FROM library_folders")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (id, p) in &existing {
        if Path::new(p) == new {
            return Ok(*id);
        }
        if new.starts_with(p) {
            anyhow::bail!("{path} is already in the library, inside {p}");
        }
    }
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO library_folders (path) VALUES (?1)",
        params![path],
    )?;
    let id = tx.last_insert_rowid() as i32;
    for (inner, p) in &existing {
        if Path::new(p).starts_with(new) {
            tx.execute(
                "UPDATE photos SET folder_id = ?1 WHERE folder_id = ?2",
                params![id, inner],
            )?;
            tx.execute("DELETE FROM library_folders WHERE id = ?1", params![inner])?;
        }
    }
    // Photos already in the catalogue from this folder — left unfiled by the
    // old single-folder library — belong to it now.
    tx.execute(
        &format!(
            "UPDATE photos SET folder_id = ?1 WHERE folder_id IS NULL AND {}",
            under_folder_sql(2)
        ),
        params![id, path],
    )?;
    tx.commit()?;
    Ok(id)
}

/// Files the photos under `root` to folder `id` — run after each scan, so the
/// photos it found count towards their folder.
pub fn assign_folder(conn: &Connection, id: i32, root: &str) -> Result<()> {
    conn.execute(
        &format!(
            "UPDATE photos SET folder_id = ?1
              WHERE (folder_id IS NULL OR folder_id != ?1) AND {}",
            under_folder_sql(2)
        ),
        params![id, root],
    )?;
    Ok(())
}

/// Deletes photo rows chosen by `condition` (SQL over `photos`, with `params`)
/// together with their faces — which do not cascade — and recounts the people
/// those faces made up. Returns the number of files forgotten.
fn forget_photos_where(
    tx: &Connection,
    condition: &str,
    params: &[&dyn rusqlite::ToSql],
) -> Result<usize> {
    let files: i64 = tx.query_row(
        &format!("SELECT COUNT(*) FROM photos WHERE variant = 0 AND {condition}"),
        params,
        |r| r.get(0),
    )?;
    tx.execute(
        &format!("DELETE FROM faces WHERE photo_id IN (SELECT id FROM photos WHERE {condition})"),
        params,
    )?;
    tx.execute(&format!("DELETE FROM photos WHERE {condition}"), params)?;
    recount_people(tx)?;
    Ok(files as usize)
}

/// The thumbnails and previews of the photos `condition` selects, to delete
/// once they are gone.
pub(super) fn derivatives_where(
    tx: &Connection,
    condition: &str,
    params: &[&dyn rusqlite::ToSql],
) -> Result<Vec<String>> {
    let mut stmt = tx.prepare(&format!(
        "SELECT thumb_path, preview_path FROM photos WHERE {condition}"
    ))?;
    let rows = stmt.query_map(params, |r| {
        Ok([
            r.get::<_, Option<String>>(0)?,
            r.get::<_, Option<String>>(1)?,
        ])
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.extend(row?.into_iter().flatten());
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// Takes a folder out of the library, forgetting its photos and all the work
/// on them — the files on disk are not touched. Returns how many photos went.
/// Their previews and face portraits go too.
pub fn remove_library_folder(conn: &mut Connection, id: i32) -> Result<usize> {
    let tx = conn.transaction()?;
    let derived = derivatives_where(&tx, "folder_id = ?1", &[&id])?;
    let n = forget_photos_where(&tx, "folder_id = ?1", &[&id])?;
    tx.execute("DELETE FROM library_folders WHERE id = ?1", params![id])?;
    tx.commit()?;
    for path in &derived {
        drop_unused_derivative(conn, path)?;
    }
    Ok(n)
}

/// Forgets photos that belong to no library folder.
pub fn forget_unfiled_photos(conn: &mut Connection) -> Result<usize> {
    let tx = conn.transaction()?;
    let derived = derivatives_where(&tx, "folder_id IS NULL", &[])?;
    let n = forget_photos_where(&tx, "folder_id IS NULL", &[])?;
    tx.commit()?;
    for path in &derived {
        drop_unused_derivative(conn, path)?;
    }
    Ok(n)
}

/// "Locate folder…": the folder moved somewhere the scan could not see
/// — another drive, another parent — so every photo in it is rewritten from
/// the old location to the new one, keeping all its work. The caller rescans
/// the folder afterwards, which unmarks what is really there.
/// How many of a folder's photos [`same_photos_at`] looks for.
const RELOCATE_SAMPLE: i64 = 24;

/// Refuses to point a folder's photos at `new_path` unless most of them are
/// there and are the same files. Locate took any folder: one with
/// last year's shoot, its DSC0xxxx names repeating, took over every pick,
/// keyword and edit, and a later bin threw away files that inherited reject
/// flags. A sample is checked by its quick hash — size, time and first
/// bytes — at the place it would now be.
fn same_photos_at(conn: &Connection, id: i32, old: &str, new_path: &str) -> Result<()> {
    let sample: Vec<(String, String)> = conn
        .prepare(
            "SELECT path, file_hash FROM photos WHERE folder_id = ?1 AND variant = 0
              ORDER BY random() LIMIT ?2",
        )?
        .query_map(params![id, RELOCATE_SAMPLE], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    if sample.is_empty() {
        return Ok(());
    }
    let mut same = 0;
    for (path, hash) in &sample {
        let Some(moved) = moved_path(path, old, new_path) else {
            continue;
        };
        if crate::scan::quick_hash(Path::new(&moved)).is_ok_and(|h| &h == hash) {
            same += 1;
        }
    }
    if same * 2 <= sample.len() {
        anyhow::bail!(
            "only {same} of {} of this folder's photos checked are at {new_path}, the same files — \
             choose the folder they were moved to",
            sample.len()
        );
    }
    Ok(())
}

/// Where the photo at `path`, inside folder `old`, is once the folder is at
/// `new`. Joined as paths, not strings: `F:` glued to the rest of
/// `E:\DSC1.ARW` gave `F:DSC1.ARW`, a path relative to F:'s current
/// directory. `None` for a path not inside `old`.
fn moved_path(path: &str, old: &str, new: &str) -> Option<String> {
    let rest = Path::new(path).strip_prefix(old).ok()?;
    Some(Path::new(new).join(rest).to_string_lossy().into_owned())
}

pub fn relocate_library_folder(conn: &mut Connection, id: i32, new_path: &str) -> Result<()> {
    let new_path = normalise_folder_path(new_path);
    let old: String = conn.query_row(
        "SELECT path FROM library_folders WHERE id = ?1",
        params![id],
        |r| r.get(0),
    )?;
    if Path::new(&old) == Path::new(&new_path) {
        return Ok(());
    }
    let clash: Option<String> = conn
        .prepare("SELECT path FROM library_folders WHERE id != ?1")?
        .query_map(params![id], |r| r.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .find(|p| Path::new(&new_path).starts_with(p) || Path::new(p).starts_with(&new_path));
    if let Some(p) = clash {
        anyhow::bail!("{new_path} overlaps {p}, which is already in the library");
    }
    same_photos_at(conn, id, &old, &new_path)?;
    move_folder(conn, id, &old, &new_path)
}

/// Rewrites folder `id` and its photos from `old` to `new`, unchecked.
fn move_folder(conn: &mut Connection, id: i32, old: &str, new_path: &str) -> Result<()> {
    let tx = conn.transaction()?;
    let paths: Vec<(i32, String)> = tx
        .prepare("SELECT id, path FROM photos WHERE folder_id = ?1")?
        .query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    {
        let mut update = tx.prepare("UPDATE photos SET path = ?2 WHERE id = ?1")?;
        for (photo, path) in &paths {
            // A row filed here but not under the path is left; the rescan
            // finds its file again by hash.
            if let Some(moved) = moved_path(path, old, new_path) {
                update.execute(params![photo, moved])?;
            }
        }
    }
    tx.execute(
        "UPDATE library_folders SET path = ?2 WHERE id = ?1",
        params![id, new_path],
    )?;
    tx.commit()?;
    Ok(())
}

/// The catalogue row a newly found file most likely *was*, before it moved:
/// the one master row with the same quick hash whose own file is gone.
///
/// The quick hash covers size, modified time and the first 64 KB, and a
/// rename or a move keeps all three — so a whole folder renamed in Explorer
/// matches back row for row. `None` when nothing matches, when more than one
/// row does (the same card imported twice, both copies moved: guessing would
/// swap their work), or when a row already sits at `new_path`.
/// Whether the place a photo was kept is reachable now: its library folder,
/// or, for a photo in none, the volume it was on.
fn where_it_was_is_online(path: &str, folder: Option<&str>) -> bool {
    match folder {
        Some(f) => Path::new(f).is_dir(),
        None => volume_root(Path::new(path)).is_some_and(|v| v.is_dir()),
    }
}

/// The root of the volume `path` is on: `F:\`, `\\server\share\`, or on a
/// Mac `/Volumes/<name>` (anything else under `/` is the system volume).
fn volume_root(path: &Path) -> Option<std::path::PathBuf> {
    use std::path::Component;
    let mut parts = path.components();
    let mut root = std::path::PathBuf::new();
    match parts.next()? {
        Component::Prefix(p) => {
            root.push(p.as_os_str());
            root.push(std::path::MAIN_SEPARATOR_STR);
        }
        Component::RootDir => {
            root.push("/");
            let mut rest = parts.clone();
            if rest.next().is_some_and(|c| c.as_os_str() == "Volumes") {
                root.push("Volumes");
                root.push(rest.next()?.as_os_str());
            }
        }
        _ => return None,
    }
    Some(root)
}

pub fn relink_candidate(conn: &Connection, new_path: &str, hash: &str) -> Result<Option<i32>> {
    let taken: i64 = conn.query_row(
        "SELECT COUNT(*) FROM photos WHERE path = ?1",
        params![new_path],
        |r| r.get(0),
    )?;
    if taken > 0 {
        return Ok(None);
    }
    // Gone means gone from a place that is there to look in: a photo
    // on a drive that is unplugged right now has not moved, and taking a copy
    // of it — the backup drive, say — for the photo carried years of work
    // over to the copy.
    let mut stmt = conn.prepare(
        "SELECT p.id, p.path, lf.path FROM photos p
           LEFT JOIN library_folders lf ON lf.id = p.folder_id
          WHERE p.file_hash = ?1 AND p.variant = 0 AND p.path != ?2",
    )?;
    let gone: Vec<i32> = stmt
        .query_map(params![hash, new_path], |r| {
            Ok((
                r.get::<_, i32>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?
        .filter_map(|r| r.ok())
        .filter(|(_, path, folder)| {
            !Path::new(path).exists() && where_it_was_is_online(path, folder.as_deref())
        })
        .map(|(id, _, _)| id)
        .collect();
    Ok(match gone.as_slice() {
        [one] => Some(*one),
        _ => None,
    })
}

/// Points a master row — and every virtual copy of it, which shares its file —
/// at the file's new location, keeping everything else: rating, flag, label,
/// tags, place, faces, edits, collections. Clears the missing mark.
pub fn relink(conn: &Connection, id: i32, new_path: &str, new_filename: &str) -> Result<()> {
    conn.execute(
        "UPDATE photos SET path = ?2, filename = ?3, missing = 0
         WHERE id = ?1 OR variant_of = ?1",
        params![id, new_path, new_filename],
    )?;
    Ok(())
}

/// Marks which photos under `root` the scan did and did not find, returning
/// how many are missing now.
///
/// `seen` is every raw file the scan's walk found. A row under `root` that is
/// not among them has gone — moved outside the library, deleted, or on a drive
/// that is not connected — and is marked missing, never deleted. One that is
/// found again (the drive plugged back in) is unmarked. Rows outside `root`
/// belong to another folder and are left as they are.
pub fn mark_missing(
    conn: &Connection,
    root: &Path,
    seen: &std::collections::HashSet<String>,
) -> Result<usize> {
    let rows: Vec<(String, bool)> = {
        let mut stmt = conn.prepare("SELECT path, missing FROM photos WHERE variant = 0")?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? != 0))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };
    let mut missing_now = 0;
    // One transaction for the lot: a drive left unplugged changed
    // every row under it, each its own commit.
    let tx = conn.unchecked_transaction()?;
    for (path, was_missing) in rows {
        if !Path::new(&path).starts_with(root) {
            continue;
        }
        let is_missing = !seen.contains(&path);
        if is_missing {
            missing_now += 1;
        }
        if is_missing != was_missing {
            tx.execute(
                "UPDATE photos SET missing = ?2 WHERE path = ?1",
                params![path, is_missing as i64],
            )?;
        }
    }
    tx.commit()?;
    Ok(missing_now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::*;

    fn folder_of(conn: &Connection, id: i32) -> Option<i32> {
        conn.query_row(
            "SELECT folder_id FROM photos WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap()
    }

    /// A Windows path in the host's own syntax — `C:\Photos\a.arw` becomes
    /// `/Volumes/C/Photos/a.arw` off Windows. Nesting goes through `Path`, which
    /// only splits on the host's separator, so a backslash path never nests on
    /// macOS; this lets one test cover both.
    fn native(windows: &str) -> String {
        if cfg!(windows) {
            return windows.to_string();
        }
        let (drive, rest) = windows.split_once(':').unwrap();
        format!("/Volumes/{drive}{}", rest.replace('\\', "/"))
    }

    /// Folders don't nest, and `C:\Photos` must not claim `C:\Photos Old`.
    #[test]
    fn library_folders_file_photos_and_refuse_to_nest() {
        let mut conn = open_memory().unwrap();
        photo_at(&conn, 1, &native(r"C:\Photos\2025\a.arw"));
        photo_at(&conn, 2, &native(r"C:\Photos Old\b.arw"));
        photo_at(&conn, 3, &native(r"D:\Archive\c.arw"));

        // Adding a folder claims the unfiled photos inside it, and only those.
        let photos = add_library_folder(&mut conn, &native(r"C:\Photos\")).unwrap();
        assert_eq!(folder_of(&conn, 1), Some(photos));
        assert_eq!(
            folder_of(&conn, 2),
            None,
            "a sibling with a longer name is not inside"
        );

        // Re-adding is a no-op; a folder inside it is refused.
        assert_eq!(
            add_library_folder(&mut conn, &native(r"C:\Photos")).unwrap(),
            photos
        );
        assert!(add_library_folder(&mut conn, &native(r"C:\Photos\2025")).is_err());

        // A parent absorbs an inner folder and its photos.
        let c = add_library_folder(&mut conn, &native(r"C:\")).unwrap();
        assert_eq!(folder_of(&conn, 1), Some(c));
        assert_eq!(folder_of(&conn, 2), Some(c));
        let all = list_library_folders(&conn).unwrap();
        assert_eq!(all.folders.len(), 1);
        assert_eq!((all.folders[0].photos, all.unfiled), (2, 1));
    }

    #[test]
    fn the_folder_and_missing_filters_count_against_the_rest() {
        let mut conn = open_memory().unwrap();
        photo_at(&conn, 1, r"C:\A\1.arw");
        photo_at(&conn, 2, r"C:\A\2.arw");
        photo_at(&conn, 3, r"D:\B\3.arw");
        let a = add_library_folder(&mut conn, r"C:\A").unwrap();
        let b = add_library_folder(&mut conn, r"D:\B").unwrap();
        conn.execute("UPDATE photos SET missing = 1 WHERE id IN (2, 3)", [])
            .unwrap();

        let only_a = PhotoFilter {
            folder_ids: vec![a],
            ..Default::default()
        };
        let ids: Vec<i32> = query_photos(&conn, &only_a, 100, 0)
            .unwrap()
            .iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids.len(), 2);
        let f = facets(&conn, &only_a, None).unwrap();
        // The folder facet ignores its own choice, so B still shows its count…
        let mut counts: Vec<(String, i32)> = f
            .folders
            .iter()
            .map(|c| (c.value.clone(), c.count))
            .collect();
        counts.sort();
        let mut want = vec![(a.to_string(), 2), (b.to_string(), 1)];
        want.sort();
        assert_eq!(counts, want);
        // …and Missing counts within folder A.
        assert_eq!(f.missing, 1);

        let missing = PhotoFilter {
            missing_only: true,
            ..Default::default()
        };
        assert_eq!(query_photos(&conn, &missing, 100, 0).unwrap().len(), 2);
    }

    #[test]
    fn removing_a_folder_forgets_its_photos_and_their_faces() {
        let mut conn = open_memory().unwrap();
        photo_at(&conn, 1, r"C:\A\1.arw");
        photo_at(&conn, 2, r"D:\B\2.arw");
        let a = add_library_folder(&mut conn, r"C:\A").unwrap();
        add_library_folder(&mut conn, r"D:\B").unwrap();
        conn.execute_batch(
            "INSERT INTO faces (id, photo_id, x, y, w, h, person_id) VALUES (10, 1, 0, 0, 1, 1, 7);
             INSERT INTO people (id, cover_face_id, size) VALUES (7, 10, 1);",
        )
        .unwrap();

        assert_eq!(remove_library_folder(&mut conn, a).unwrap(), 1);
        assert!(get_photo(&conn, 1).unwrap().is_none());
        assert!(get_photo(&conn, 2).unwrap().is_some());
        let (faces, people): (i64, i64) = conn
            .query_row(
                "SELECT (SELECT COUNT(*) FROM faces), (SELECT COUNT(*) FROM people)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((faces, people), (0, 0));
        assert_eq!(list_library_folders(&conn).unwrap().folders.len(), 1);
    }

    /// A file at `path` with `content`, its modified time `mtime` seconds
    /// after the epoch (a copy keeps the time; quick_hash reads it).
    fn file_with(path: &Path, content: &[u8], mtime: u64) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
        let f = std::fs::File::options().write(true).open(path).unwrap();
        f.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(mtime))
            .unwrap();
    }

    /// Locate folder…: a folder moved to another drive keeps every
    /// photo's work, rewritten to the new location — when the files there
    /// are the same files.
    #[test]
    fn relocating_a_folder_rewrites_its_photos() {
        let dir = crate::test_support::Scratch::new("relocate");
        let (old, moved, other) = (dir.join("E"), dir.join("F"), dir.join("G"));
        let old_file = old.join("Karoo").join("1.arw");
        file_with(&old_file, b"the karoo frame", 1_700_000_000);
        file_with(
            &moved.join("Karoo").join("1.arw"),
            b"the karoo frame",
            1_700_000_000,
        );
        // Another shoot, its counter at the same name.
        file_with(
            &other.join("Karoo").join("1.arw"),
            b"last year's frame",
            1_600_000_000,
        );
        let path = |p: &Path| p.to_string_lossy().to_string();

        let mut conn = open_memory().unwrap();
        photo_at(&conn, 1, &path(&old_file));
        let hash = crate::scan::quick_hash(&old_file).unwrap();
        conn.execute(
            "UPDATE photos SET rating = 5, missing = 1, file_hash = ?1",
            params![hash],
        )
        .unwrap();
        let id = add_library_folder(&mut conn, &path(&old)).unwrap();

        // The wrong folder: refused, and nothing moved.
        let err = relocate_library_folder(&mut conn, id, &path(&other)).unwrap_err();
        assert!(err.to_string().contains("choose the folder"), "{err}");
        assert_eq!(get_photo(&conn, 1).unwrap().unwrap().path, path(&old_file));

        relocate_library_folder(&mut conn, id, &path(&moved)).unwrap();
        let p = get_photo(&conn, 1).unwrap().unwrap();
        assert_eq!(p.path, path(&moved.join("Karoo").join("1.arw")));
        assert_eq!(p.rating, 5);
        assert_eq!(
            list_library_folders(&conn).unwrap().folders[0].path,
            path(&moved)
        );

        // Onto a folder already in the library: refused.
        add_library_folder(&mut conn, &native(r"G:\Other")).unwrap();
        assert!(relocate_library_folder(&mut conn, id, &native(r"G:\Other\Inside")).is_err());
    }

    /// Locate onto or from a drive root: the folder keeps the root's
    /// separator and each photo's path is joined on, so the rescan of the
    /// picked folder walks to the same paths and finds this folder.
    #[cfg(windows)]
    #[test]
    fn relocating_onto_or_from_a_drive_root_keeps_whole_paths() {
        for (old, new, photo, moved) in [
            (r"E:\", r"F:\", r"E:\DSC1.ARW", r"F:\DSC1.ARW"),
            (
                r"E:\Photos",
                r"F:\",
                r"E:\Photos\Karoo\1.arw",
                r"F:\Karoo\1.arw",
            ),
            (
                r"E:\",
                r"F:\Photos",
                r"E:\Karoo\1.arw",
                r"F:\Photos\Karoo\1.arw",
            ),
        ] {
            let mut conn = open_memory().unwrap();
            photo_at(&conn, 1, photo);
            let id = add_library_folder(&mut conn, old).unwrap();
            let old = folder_path(&conn, id).unwrap();
            move_folder(&mut conn, id, &old, &normalise_folder_path(new)).unwrap();
            assert_eq!(get_photo(&conn, 1).unwrap().unwrap().path, moved);
            assert_eq!(
                folder_for_scan(&mut conn, new).unwrap(),
                (id, normalise_folder_path(new))
            );
        }
        let mut conn = open_memory().unwrap();
        let id = add_library_folder(&mut conn, r"E:\Photos").unwrap();
        relocate_library_folder(&mut conn, id, r"F:\").unwrap();
        assert_eq!(folder_path(&conn, id).unwrap(), r"F:\");
    }

    /// A photo whose library folder is offline has not moved: its
    /// copy elsewhere — the backup drive — is not given its work.
    #[test]
    fn a_photo_on_an_offline_drive_is_not_relinked() {
        let dir = crate::test_support::Scratch::new("relink-offline");
        let copy = dir.join("backup").join("1.arw");
        file_with(&copy, b"frame", 1_700_000_000);
        let hash = crate::scan::quick_hash(&copy).unwrap();
        let mut conn = open_memory().unwrap();
        let offline = native(r"Q:\Unplugged\Shoots");
        let folder = add_library_folder(&mut conn, &offline).unwrap();
        conn.execute(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at, folder_id)
             VALUES (1, ?1, '1.arw', ?2, 'x', ?3)",
            params![
                format!("{offline}{}1.arw", std::path::MAIN_SEPARATOR),
                hash,
                folder
            ],
        )
        .unwrap();
        let copy = copy.to_string_lossy().to_string();
        assert_eq!(relink_candidate(&conn, &copy, &hash).unwrap(), None);

        // Online and the file gone: that is a move.
        let online = dir.join("main").to_string_lossy().to_string();
        std::fs::create_dir_all(&online).unwrap();
        conn.execute("UPDATE library_folders SET path = ?1", params![online])
            .unwrap();
        assert_eq!(relink_candidate(&conn, &copy, &hash).unwrap(), Some(1));
    }

    /// A drive root keeps its separator: `E:` alone is the drive's
    /// current directory, not its root.
    #[test]
    fn folder_paths_are_stored_in_one_form() {
        assert_eq!(normalise_folder_path("E:\\"), "E:\\");
        assert_eq!(normalise_folder_path("E:"), "E:\\");
        assert_eq!(normalise_folder_path("E:/"), "E:\\");
        assert_eq!(normalise_folder_path("E:\\Photos\\"), "E:\\Photos");
        assert_eq!(normalise_folder_path("/Volumes/Card/"), "/Volumes/Card");
        assert_eq!(normalise_folder_path("/"), "/");
        assert_eq!(
            normalise_folder_path("\\\\nas\\photos\\"),
            "\\\\nas\\photos"
        );
    }

    /// Added by its root, a drive is stored as `E:\`, found by either
    /// spelling, and files its photos — and only its own.
    #[test]
    fn a_drive_root_library_folder_files_its_photos() {
        let mut conn = open_memory().unwrap();
        let id = add_library_folder(&mut conn, "E:\\").unwrap();
        assert_eq!(folder_path(&conn, id).unwrap(), "E:\\");
        assert_eq!(folder_id_for(&conn, "E:").unwrap(), Some(id));
        assert_eq!(
            add_library_folder(&mut conn, "E:").unwrap(),
            id,
            "the same folder"
        );
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at)
             VALUES (1, 'E:\\Photos\\a.arw', 'a.arw', 'h1', 'x'),
                    (2, 'E:\\b.arw', 'b.arw', 'h2', 'x'),
                    (3, 'F:\\c.arw', 'c.arw', 'h3', 'x');",
        )
        .unwrap();
        assign_folder(&conn, id, "E:\\").unwrap();
        let filed: Vec<i32> = conn
            .prepare("SELECT id FROM photos WHERE folder_id = ?1 ORDER BY id")
            .unwrap()
            .query_map(params![id], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(filed, vec![1, 2]);
        // A folder inside it is already in the library. Windows only: a
        // backslash is no separator elsewhere, so on a Mac `E:\Photos` is
        // one name and inside nothing — a path no Mac can have anyway.
        #[cfg(windows)]
        assert!(add_library_folder(&mut conn, "E:\\Photos").is_err());
    }

    /// A catalogue whose drive root an older build stored as `E:` gets its
    /// separator back on open.
    #[test]
    fn a_trimmed_drive_root_is_repaired() {
        let conn = open_memory().unwrap();
        conn.execute("INSERT INTO library_folders (path) VALUES ('E:')", [])
            .unwrap();
        repair_drive_roots(&conn).unwrap();
        repair_drive_roots(&conn).unwrap();
        let paths: Vec<String> = conn
            .prepare("SELECT path FROM library_folders")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(paths, vec!["E:\\".to_string()]);
    }

    /// A moved photo is looked up by its quick hash for every new file a
    /// scan meets: by index, not by reading every photo.
    #[test]
    fn a_relink_lookup_searches_by_hash() {
        let conn = open_memory().unwrap();
        add_photos(&conn, 50);
        refresh_planner_stats(&conn).unwrap();
        let plan: String = conn
            .prepare(
                "EXPLAIN QUERY PLAN SELECT id, path FROM photos
                  WHERE file_hash = ?1 AND variant = 0 AND path != ?2",
            )
            .unwrap()
            .query_map(params!["h", "x"], |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .join(" | ");
        assert!(plan.contains("idx_photos_hash"), "{plan}");
    }
}
