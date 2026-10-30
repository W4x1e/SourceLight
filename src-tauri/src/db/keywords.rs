//! The keyword tree, and keywords made from folder names.
//!
//! Subject keywords can be filed under one another. Everything here addresses a
//! keyword by its name, as filters, saved views and the tagger already do:
//! names are unique among subjects, so a name is a keyword.

use super::*;

/// A keyword's name as the catalogue keeps it: trimmed and lower-cased. `|` is
/// refused, because it separates a path's levels in `lr:hierarchicalSubject`
/// and a name holding one would come back from a sidecar as two keywords.
fn keyword_name(name: &str) -> Result<String> {
    let name = name.trim().to_lowercase();
    if name.is_empty() {
        bail!("a keyword needs a name");
    }
    if name.contains('|') {
        bail!("a keyword's name can't contain “|”");
    }
    Ok(name)
}

/// What someone typed, as a keyword path, root first: `places > europe >
/// france` and `Places|Europe|France` alike. A plain keyword is one level.
pub fn keyword_path(input: &str) -> Vec<String> {
    input
        .split(['|', '>'])
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// A subject keyword's id, if the catalogue has one by that name.
fn keyword_id(conn: &Connection, name: &str) -> Result<Option<i32>> {
    Ok(conn
        .prepare_cached("SELECT id FROM tags WHERE category = 'subject' AND name = ?1")?
        .query_row(params![name], |r| r.get(0))
        .optional()?)
}

fn existing_keyword(conn: &Connection, name: &str) -> Result<i32> {
    let name = keyword_name(name)?;
    keyword_id(conn, &name)?.ok_or_else(|| anyhow::anyhow!("there is no keyword “{name}”"))
}

fn keyword_parent(conn: &Connection, id: i32) -> Result<Option<i32>> {
    Ok(conn.query_row(
        "SELECT parent_id FROM tags WHERE id = ?1",
        params![id],
        |r| r.get(0),
    )?)
}

/// Whether keyword `id` is `branch` or filed anywhere beneath it.
fn in_branch(conn: &Connection, id: i32, branch: i32) -> Result<bool> {
    Ok(conn.query_row(
        "WITH RECURSIVE up(id, depth) AS (
             SELECT ?1, 0
             UNION
             SELECT t.parent_id, up.depth + 1 FROM tags t JOIN up ON t.id = up.id
              WHERE t.parent_id IS NOT NULL AND up.depth < 64)
         SELECT EXISTS (SELECT 1 FROM up WHERE id = ?2)",
        params![id, branch],
        |r| r.get(0),
    )?)
}

/// Where a keyword is filed, root first and ending with it — or `None` when it
/// is outside the tree altogether, at the top with nothing beneath it. A
/// sidecar keeps one of those in the flat keyword list alone, as it always has.
pub(super) fn keyword_filed_path(conn: &Connection, name: &str) -> Result<Option<Vec<String>>> {
    let Some(id) = keyword_id(conn, name)? else {
        return Ok(None);
    };
    let has_children: bool = conn
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM tags WHERE parent_id = ?1)")?
        .query_row(params![id], |r| r.get(0))?;
    let mut stmt = conn.prepare_cached(
        "WITH RECURSIVE up(id, name, parent_id, depth) AS (
             SELECT id, name, parent_id, 0 FROM tags WHERE id = ?1
             UNION ALL
             SELECT t.id, t.name, t.parent_id, up.depth + 1 FROM tags t JOIN up ON t.id = up.parent_id
              WHERE up.depth < 64)
         SELECT name FROM up ORDER BY depth DESC",
    )?;
    let path = stmt
        .query_map(params![id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok((path.len() > 1 || has_children).then_some(path))
}

/// Every keyword the tree shows, each with the one it is filed under: the ones
/// photos have, and the ones the user made or filed, which stay put with no
/// photos yet.
pub fn keyword_tree(conn: &Connection) -> Result<Vec<KeywordNode>> {
    let mut stmt = conn.prepare(
        "SELECT t.name, pt.name, t.kept FROM tags t
           LEFT JOIN tags pt ON pt.id = t.parent_id
          WHERE t.category = 'subject'
            AND (t.kept = 1 OR t.parent_id IS NOT NULL
                 OR EXISTS (SELECT 1 FROM tags c WHERE c.parent_id = t.id)
                 OR EXISTS (SELECT 1 FROM photo_tags x WHERE x.tag_id = t.id))
          ORDER BY t.name",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(KeywordNode {
            name: r.get(0)?,
            parent: r.get(1)?,
            kept: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Makes a keyword, under `parent` or at the top, for photos to have later.
///
/// A name in use is refused, saying where it is, rather than moved: the user
/// asked for a new keyword, and the old one's photos would go with it. A word
/// the tagger once used and no photo has any more is invisible, though, and
/// that one is simply taken over.
pub fn create_keyword(conn: &Connection, name: &str, parent: Option<&str>) -> Result<()> {
    let name = keyword_name(name)?;
    let parent_id = parent.map(|p| existing_keyword(conn, p)).transpose()?;
    if let Some(id) = keyword_id(conn, &name)? {
        // Shown by the tree, by the same test `keyword_tree` makes.
        let (shown, under): (bool, Option<String>) = conn.query_row(
            "SELECT t.kept = 1 OR t.parent_id IS NOT NULL
                    OR EXISTS (SELECT 1 FROM tags c WHERE c.parent_id = t.id)
                    OR EXISTS (SELECT 1 FROM photo_tags x WHERE x.tag_id = t.id),
                    (SELECT name FROM tags WHERE id = t.parent_id)
               FROM tags t WHERE t.id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if shown {
            match under {
                Some(p) => bail!("“{name}” already exists, under “{p}”"),
                None => bail!("“{name}” already exists"),
            }
        }
        conn.execute(
            "UPDATE tags SET parent_id = ?2, kept = 1 WHERE id = ?1",
            params![id, parent_id],
        )?;
        return Ok(());
    }
    conn.execute(
        "INSERT INTO tags (name, category, parent_id, kept) VALUES (?1, 'subject', ?2, 1)",
        params![name, parent_id],
    )?;
    Ok(())
}

/// Makes sure a path of keywords exists, root first, and returns its last
/// level. A level that is missing is made under the one before it; one that
/// exists but is not filed anywhere is filed there. One already filed
/// elsewhere stays where it is — somebody put it there, and a keyword is in
/// one place — and the rest of the path hangs from it.
pub(super) fn ensure_keyword_path(conn: &Connection, path: &[String]) -> Result<Option<String>> {
    let mut parent: Option<i32> = None;
    for level in path {
        let name = keyword_name(level)?;
        let id = tag_id(conn, &name, "subject")?;
        if let Some(pid) = parent {
            // Filing it under its own branch would make a loop.
            if keyword_parent(conn, id)?.is_none() && !in_branch(conn, pid, id)? {
                conn.execute(
                    "UPDATE tags SET parent_id = ?2 WHERE id = ?1",
                    params![id, pid],
                )?;
            }
        }
        // Only when it changes: unconditional, it rewrote and
        // committed a page for every keyword level of every sidecar read.
        conn.execute(
            "UPDATE tags SET kept = 1 WHERE id = ?1 AND kept = 0",
            params![id],
        )?;
        parent = Some(id);
    }
    Ok(path.last().cloned())
}

/// Files a keyword under `parent`, or at the top for `None`. Its branch goes
/// with it. Filing one inside its own branch is refused.
pub fn move_keyword(conn: &Connection, name: &str, parent: Option<&str>) -> Result<()> {
    let id = existing_keyword(conn, name)?;
    let parent_id = parent.map(|p| existing_keyword(conn, p)).transpose()?;
    if let Some(pid) = parent_id {
        if in_branch(conn, pid, id)? {
            bail!("a keyword can't go inside itself or anything beneath it");
        }
    }
    conn.execute(
        "UPDATE tags SET parent_id = ?2, kept = 1 WHERE id = ?1",
        params![id, parent_id],
    )?;
    Ok(())
}

/// Renames a keyword and returns the name it ends up with. Renaming it to
/// another keyword's name **merges** the two: that is what it means when both
/// name the same thing ("dogs" into "dog"). Saved views that named it follow.
pub fn rename_keyword(conn: &mut Connection, name: &str, new_name: &str) -> Result<String> {
    let tx = conn.transaction()?;
    let from = existing_keyword(&tx, name)?;
    let old: String = tx.query_row("SELECT name FROM tags WHERE id = ?1", params![from], |r| {
        r.get(0)
    })?;
    let new = keyword_name(new_name)?;
    if new == old {
        return Ok(old);
    }
    match keyword_id(&tx, &new)? {
        Some(into) => merge_keyword_ids(&tx, from, into)?,
        None => {
            tx.execute(
                "UPDATE tags SET name = ?2 WHERE id = ?1",
                params![from, new],
            )?;
        }
    }
    rename_in_saved_views(&tx, &old, &new)?;
    tx.commit()?;
    Ok(new)
}

/// Folds keyword `from` into `into`: its photos get `into` (as the user's if
/// either link was), what was filed under it is filed under `into`, and it
/// goes. When `into` was somewhere in `from`'s branch it first moves up to
/// where `from` was, or the branch would end up filed inside itself.
fn merge_keyword_ids(conn: &Connection, from: i32, into: i32) -> Result<()> {
    if from == into {
        return Ok(());
    }
    if in_branch(conn, into, from)? {
        conn.execute(
            "UPDATE tags SET parent_id = (SELECT parent_id FROM tags WHERE id = ?1) WHERE id = ?2",
            params![from, into],
        )?;
    }
    conn.execute(
        "UPDATE tags SET parent_id = ?2 WHERE parent_id = ?1 AND id != ?2",
        params![from, into],
    )?;
    conn.execute(
        "INSERT INTO photo_tags (photo_id, tag_id, confidence, source)
         SELECT photo_id, ?2, confidence, source FROM photo_tags WHERE tag_id = ?1
         ON CONFLICT(photo_id, tag_id) DO UPDATE SET
             source = CASE WHEN 'manual' IN (photo_tags.source, excluded.source) THEN 'manual'
                           WHEN 'folder' IN (photo_tags.source, excluded.source) THEN 'folder'
                           ELSE photo_tags.source END,
             confidence = MAX(COALESCE(photo_tags.confidence, 0), COALESCE(excluded.confidence, 0))",
        params![from, into],
    )?;
    conn.execute(
        "UPDATE tags SET kept = MAX(kept, (SELECT kept FROM tags WHERE id = ?1)) WHERE id = ?2",
        params![from, into],
    )?;
    // A tagger's word turned down under the old name stays turned down.
    conn.execute(
        "INSERT OR IGNORE INTO rejected_tags (photo_id, tag_id)
         SELECT photo_id, ?2 FROM rejected_tags WHERE tag_id = ?1
            AND NOT EXISTS (SELECT 1 FROM photo_tags x WHERE x.photo_id = rejected_tags.photo_id AND x.tag_id = ?2)",
        params![from, into],
    )?;
    conn.execute("DELETE FROM photo_tags WHERE tag_id = ?1", params![from])?;
    conn.execute("DELETE FROM tags WHERE id = ?1", params![from])?;
    Ok(())
}

/// Deletes a keyword: it comes off every photo, and whatever was filed under
/// it moves up a level. Returns how many photos had it. Saved views that name
/// it are left alone — dropping it from one would widen the view to photos it
/// never meant.
pub fn delete_keyword(conn: &mut Connection, name: &str) -> Result<usize> {
    let tx = conn.transaction()?;
    let id = existing_keyword(&tx, name)?;
    let photos: i64 = tx.query_row(
        "SELECT COUNT(*) FROM photo_tags WHERE tag_id = ?1",
        params![id],
        |r| r.get(0),
    )?;
    tx.execute(
        "UPDATE tags SET parent_id = (SELECT parent_id FROM tags WHERE id = ?1) WHERE parent_id = ?1",
        params![id],
    )?;
    tx.execute("DELETE FROM photo_tags WHERE tag_id = ?1", params![id])?;
    tx.execute("DELETE FROM tags WHERE id = ?1", params![id])?;
    tx.commit()?;
    Ok(photos as usize)
}

/// Points saved views that chose, or left out, keyword `old` at `new` instead,
/// so renaming or merging a keyword doesn't quietly change a smart collection.
/// A view whose JSON can't be read is left exactly as it is.
fn rename_in_saved_views(conn: &Connection, old: &str, new: &str) -> Result<()> {
    let views: Vec<(i32, String)> = {
        let mut stmt = conn.prepare("SELECT id, filter FROM smart_collections")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (id, json) in views {
        let Ok(mut filter) = serde_json::from_str::<serde_json::Value>(&json) else {
            continue;
        };
        let mut changed = false;
        // Chosen or left out, a keyword is named the same way.
        for list in ["subjects", "excludeSubjects"] {
            let Some(names) = filter.get_mut(list).and_then(|s| s.as_array_mut()) else {
                continue;
            };
            if !names.iter().any(|s| s.as_str() == Some(old)) {
                continue;
            }
            let mut renamed: Vec<serde_json::Value> = Vec::new();
            for s in names.iter() {
                let s = if s.as_str() == Some(old) {
                    serde_json::Value::from(new)
                } else {
                    s.clone()
                };
                if !renamed.contains(&s) {
                    renamed.push(s);
                }
            }
            *names = renamed;
            changed = true;
        }
        if changed {
            conn.execute(
                "UPDATE smart_collections SET filter = ?2 WHERE id = ?1",
                params![id, filter.to_string()],
            )?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Keywords from folder names
// ---------------------------------------------------------------------------

/// Where keywords made from folder names are filed, until the user moves them.
pub const FOLDERS_ROOT: &str = "folders";

/// The photos a folder-keyword pass is over: one library folder's, one
/// shoot's, or — both `None` — every photo. `IN (…)` rather than a join, so a
/// photo in no library folder still comes back, with no root.
fn folder_keyword_scope(
    conn: &Connection,
    folder_id: Option<i32>,
    shoot_id: Option<i32>,
) -> Result<Vec<(i32, String, Option<String>)>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.path, lf.path FROM photos p
           LEFT JOIN library_folders lf ON lf.id = p.folder_id
          WHERE (?1 IS NULL OR p.folder_id = ?1)
            AND (?2 IS NULL OR p.path IN (SELECT path FROM shoot_files WHERE shoot_id = ?2))",
    )?;
    let rows = stmt.query_map(params![folder_id, shoot_id], |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The folder names above a photo, outermost first: its library folder's own
/// name, then every folder between that and the file. A photo in no library
/// folder has just the one it is in. Split on both separators, so a Windows
/// path reads the same on any machine.
fn folder_names(path: &str, root: Option<&str>) -> Vec<String> {
    let parts = |s: &str| -> Vec<String> {
        s.split(['\\', '/'])
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect()
    };
    let mut dirs = parts(path);
    dirs.pop(); // the file
    match root {
        Some(root) if path.len() > root.len() && path.starts_with(root) => {
            let root_parts = parts(root);
            let inside = dirs.len().saturating_sub(root_parts.len());
            let mut out: Vec<String> = root_parts.last().cloned().into_iter().collect();
            out.extend(dirs.into_iter().rev().take(inside).rev());
            out
        }
        _ => dirs.pop().into_iter().collect(),
    }
}

/// The keyword for a folder name: the one of that name wherever it is filed,
/// or a new one under [`FOLDERS_ROOT`].
fn folder_keyword_id(conn: &Connection, name: &str) -> Result<i32> {
    if let Some(id) = keyword_id(conn, name)? {
        return Ok(id);
    }
    let root = tag_id(conn, FOLDERS_ROOT, "subject")?;
    conn.execute(
        "UPDATE tags SET kept = 1 WHERE id = ?1 AND kept = 0",
        params![root],
    )?;
    conn.execute(
        "INSERT INTO tags (name, category, parent_id, kept) VALUES (?1, 'subject', ?2, 1)",
        params![name, root],
    )?;
    Ok(conn.last_insert_rowid() as i32)
}

/// Turns the photos' folder names into keywords — see
/// [`crate::folder_keywords`] for which names say something. Linked as
/// `folder`, so they can be told from the user's and the tagger's and taken
/// back off together, and never onto a photo the user took that keyword off.
/// Only when asked: a rescan never runs this, so a keyword deleted stays
/// deleted.
pub fn add_folder_keywords(
    conn: &mut Connection,
    folder_id: Option<i32>,
    shoot_id: Option<i32>,
) -> Result<FolderKeywords> {
    let tx = conn.transaction()?;
    let photos = folder_keyword_scope(&tx, folder_id, shoot_id)?;
    let mut ids: std::collections::HashMap<String, i32> = std::collections::HashMap::new();
    let mut touched = std::collections::HashSet::new();
    let mut used = std::collections::HashSet::new();
    {
        let mut link = tx.prepare(
            "INSERT OR IGNORE INTO photo_tags (photo_id, tag_id, confidence, source)
             SELECT ?1, ?2, 1.0, 'folder'
              WHERE NOT EXISTS (SELECT 1 FROM rejected_tags WHERE photo_id = ?1 AND tag_id = ?2)",
        )?;
        for (photo, path, root) in &photos {
            let dirs = folder_names(path, root.as_deref());
            let dirs: Vec<&str> = dirs.iter().map(String::as_str).collect();
            for k in crate::folder_keywords::keywords(&dirs) {
                let tid = match ids.get(&k) {
                    Some(&id) => id,
                    None => {
                        let id = folder_keyword_id(&tx, &k)?;
                        ids.insert(k.clone(), id);
                        id
                    }
                };
                if link.execute(params![photo, tid])? > 0 {
                    touched.insert(*photo);
                    used.insert(tid);
                }
            }
        }
    }
    tx.commit()?;
    Ok(FolderKeywords {
        photos: touched.len() as u32,
        keywords: used.len() as u32,
    })
}

/// Takes keywords made from folder names back off these photos (all of them
/// for `None`), and deletes the ones that were made for it and are now on no
/// photo. A keyword the user or the tagger also put on a photo stays there.
/// Returns how many photos lost one.
pub fn remove_folder_keywords(conn: &mut Connection, folder_id: Option<i32>) -> Result<usize> {
    let tx = conn.transaction()?;
    let scope = "source = 'folder'
            AND (?1 IS NULL OR photo_id IN (SELECT id FROM photos WHERE folder_id = ?1))";
    let photos: i64 = tx.query_row(
        &format!("SELECT COUNT(DISTINCT photo_id) FROM photo_tags WHERE {scope}"),
        params![folder_id],
        |r| r.get(0),
    )?;
    tx.execute(
        &format!("DELETE FROM photo_tags WHERE {scope}"),
        params![folder_id],
    )?;
    // Under "folders", on nothing, holding nothing: made by the pass, now empty.
    tx.execute(
        "DELETE FROM tags WHERE category = 'subject'
            AND parent_id = (SELECT id FROM tags WHERE category = 'subject' AND name = ?1)
            AND NOT EXISTS (SELECT 1 FROM photo_tags x WHERE x.tag_id = tags.id)
            AND NOT EXISTS (SELECT 1 FROM tags c WHERE c.parent_id = tags.id)",
        params![FOLDERS_ROOT],
    )?;
    tx.execute(
        "DELETE FROM tags WHERE category = 'subject' AND name = ?1
            AND NOT EXISTS (SELECT 1 FROM photo_tags x WHERE x.tag_id = tags.id)
            AND NOT EXISTS (SELECT 1 FROM tags c WHERE c.parent_id = tags.id)",
        params![FOLDERS_ROOT],
    )?;
    tx.commit()?;
    Ok(photos as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::*;

    fn parent_of(conn: &Connection, name: &str) -> Option<String> {
        keyword_tree(conn)
            .unwrap()
            .into_iter()
            .find(|k| k.name == name)
            .and_then(|k| k.parent)
    }

    /// Photo 1 a dog, 2 a cat, 3 a car; dog and cat filed under animals.
    fn animals(conn: &mut Connection) {
        add_photos(conn, 3);
        add_manual_tag_many(conn, &[1], "dog", "subject").unwrap();
        add_manual_tag_many(conn, &[2], "cat", "subject").unwrap();
        add_manual_tag_many(conn, &[3], "car", "subject").unwrap();
        create_keyword(conn, "Animals", None).unwrap();
        move_keyword(conn, "dog", Some("animals")).unwrap();
        move_keyword(conn, "cat", Some("animals")).unwrap();
    }

    #[test]
    fn choosing_a_keyword_finds_everything_filed_beneath_it() {
        let mut conn = open_memory().unwrap();
        animals(&mut conn);
        assert_eq!(matching(&conn, &subjects_filter(&["animals"])), vec![1, 2]);
        assert_eq!(matching(&conn, &subjects_filter(&["dog"])), vec![1]);
        // A level deeper still counts.
        create_keyword(&conn, "pets", Some("animals")).unwrap();
        move_keyword(&conn, "dog", Some("pets")).unwrap();
        assert_eq!(matching(&conn, &subjects_filter(&["animals"])), vec![1, 2]);
        assert_eq!(matching(&conn, &subjects_filter(&["pets"])), vec![1]);
        assert_eq!(
            matching(&conn, &subjects_filter(&["pets", "car"])),
            vec![1, 3]
        );
    }

    #[test]
    fn a_branch_counts_each_photo_once_under_the_filter() {
        let mut conn = open_memory().unwrap();
        animals(&mut conn);
        // Photo 1 is a dog *and* a cat, and 4 has the parent itself.
        add_manual_tag_many(&mut conn, &[1], "cat", "subject").unwrap();
        add_photos_from(&conn, 4);
        add_manual_tag_many(&mut conn, &[4], "animals", "subject").unwrap();
        let all = PhotoFilter::default();
        assert_eq!(subject_count(&conn, &all, "animals"), 3, "1, 2 and 4");
        assert_eq!(subject_count(&conn, &all, "cat"), 2);
        assert_eq!(subject_count(&conn, &all, "car"), 1);

        // Under another filter, counted within it.
        add_manual_tag_many(&mut conn, &[1, 3], "outdoors", "condition").unwrap();
        let outdoors = PhotoFilter {
            conditions: vec!["outdoors".into()],
            ..Default::default()
        };
        assert_eq!(subject_count(&conn, &outdoors, "animals"), 1);
        // And a keyword's own choice doesn't collapse its branch's count.
        assert_eq!(
            subject_count(&conn, &subjects_filter(&["car"]), "animals"),
            3
        );
    }

    #[test]
    fn a_typed_path_files_the_keyword_and_tags_its_last_level() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 2);
        add_manual_tag_many(&mut conn, &[1, 2], "Places > Europe > France", "subject").unwrap();
        add_manual_tag_many(&mut conn, &[2], "places|europe|spain", "subject").unwrap();
        assert_eq!(
            get_photo(&conn, 1).unwrap().unwrap().subjects,
            vec!["france"]
        );
        assert_eq!(parent_of(&conn, "france").as_deref(), Some("europe"));
        assert_eq!(parent_of(&conn, "spain").as_deref(), Some("europe"));
        assert_eq!(parent_of(&conn, "europe").as_deref(), Some("places"));
        assert_eq!(parent_of(&conn, "places"), None);
        assert_eq!(matching(&conn, &subjects_filter(&["places"])), vec![1, 2]);
        assert_eq!(keyword_path(" a >  b|c "), vec!["a", "b", "c"]);
        assert!(keyword_path(" > | ").is_empty());
    }

    /// A keyword already filed somewhere stays there: the path hangs from it.
    #[test]
    fn a_typed_path_never_moves_a_filed_keyword_or_makes_a_loop() {
        let mut conn = open_memory().unwrap();
        animals(&mut conn);
        add_manual_tag_many(&mut conn, &[1], "pets > dog > puppy", "subject").unwrap();
        assert_eq!(parent_of(&conn, "dog").as_deref(), Some("animals"));
        assert_eq!(parent_of(&conn, "puppy").as_deref(), Some("dog"));
        // "dog > animals" would put animals inside its own branch.
        add_manual_tag_many(&mut conn, &[1], "dog > animals", "subject").unwrap();
        assert_eq!(parent_of(&conn, "animals"), None);
    }

    #[test]
    fn made_keywords_show_before_any_photo_has_them() {
        let conn = open_memory().unwrap();
        create_keyword(&conn, "To print", None).unwrap();
        create_keyword(&conn, "A3", Some("to print")).unwrap();
        let tree = keyword_tree(&conn).unwrap();
        assert_eq!(
            tree,
            vec![
                KeywordNode {
                    name: "a3".into(),
                    parent: Some("to print".into()),
                    kept: true
                },
                KeywordNode {
                    name: "to print".into(),
                    parent: None,
                    kept: true
                },
            ]
        );
        let err = create_keyword(&conn, "a3", None).unwrap_err().to_string();
        assert!(err.contains("under “to print”"), "{err}");
        assert!(create_keyword(&conn, "a|b", None).is_err());
        assert!(create_keyword(&conn, "  ", None).is_err());
        assert!(create_keyword(&conn, "x", Some("nowhere")).is_err());
    }

    /// A word of the tagger's that no photo has any more is out of sight, so
    /// making a keyword of that name takes it over rather than refusing.
    #[test]
    fn making_a_keyword_takes_over_an_unused_tagger_word() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 1);
        add_manual_tag_many(&mut conn, &[1], "boat", "subject").unwrap();
        remove_tag(&conn, 1, "boat").unwrap();
        assert!(keyword_tree(&conn).unwrap().is_empty());
        create_keyword(&conn, "boat", None).unwrap();
        assert_eq!(keyword_tree(&conn).unwrap().len(), 1);
    }

    #[test]
    fn a_keyword_cannot_go_inside_its_own_branch() {
        let mut conn = open_memory().unwrap();
        animals(&mut conn);
        assert!(move_keyword(&conn, "animals", Some("dog")).is_err());
        assert!(move_keyword(&conn, "animals", Some("animals")).is_err());
        move_keyword(&conn, "dog", None).unwrap();
        assert_eq!(parent_of(&conn, "dog"), None);
        // Now it can.
        move_keyword(&conn, "animals", Some("dog")).unwrap();
        assert_eq!(parent_of(&conn, "animals").as_deref(), Some("dog"));
    }

    #[test]
    fn renaming_to_an_existing_keyword_merges_them() {
        let mut conn = open_memory().unwrap();
        animals(&mut conn);
        add_photos_from(&conn, 4);
        // "dogs" from the tagger on 1 and 4, with a keyword of its own below.
        let ai = |photo: i32, name: &str| {
            let tid = tag_id(&conn, name, "subject").unwrap();
            conn.execute(
                "INSERT INTO photo_tags (photo_id, tag_id, confidence, source) VALUES (?1, ?2, 0.4, 'ai')",
                params![photo, tid],
            )
            .unwrap();
        };
        ai(1, "dogs");
        ai(4, "dogs");
        create_keyword(&conn, "terrier", Some("dogs")).unwrap();
        let view =
            save_smart_collection(&conn, "Dogs", &subjects_filter(&["dogs", "car"])).unwrap();

        assert_eq!(rename_keyword(&mut conn, "Dogs", "DOG").unwrap(), "dog");
        assert_eq!(matching(&conn, &subjects_filter(&["dog"])), vec![1, 4]);
        assert!(keyword_tree(&conn)
            .unwrap()
            .iter()
            .all(|k| k.name != "dogs"));
        assert_eq!(parent_of(&conn, "terrier").as_deref(), Some("dog"));
        // Photo 1 had "dog" by hand: the merge keeps it the user's.
        let source: String = conn
            .query_row(
                "SELECT pt.source FROM photo_tags pt JOIN tags t ON t.id = pt.tag_id
                  WHERE pt.photo_id = 1 AND t.name = 'dog'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(source, "manual");
        // The saved view follows.
        let saved = list_smart_collections(&conn).unwrap();
        let saved = saved.iter().find(|v| v.id == view).unwrap();
        assert_eq!(saved.filter.subjects, vec!["dog", "car"]);
    }

    #[test]
    fn a_plain_rename_keeps_the_keyword_its_photos_and_its_place() {
        let mut conn = open_memory().unwrap();
        animals(&mut conn);
        rename_keyword(&mut conn, "dog", "hound").unwrap();
        assert_eq!(matching(&conn, &subjects_filter(&["hound"])), vec![1]);
        assert_eq!(parent_of(&conn, "hound").as_deref(), Some("animals"));
        assert!(rename_keyword(&mut conn, "hound", "a|b").is_err());
        assert!(rename_keyword(&mut conn, "nothing", "b").is_err());
    }

    /// Merging a keyword into one filed beneath it must not leave its branch
    /// filed inside itself.
    #[test]
    fn merging_into_a_keyword_below_lifts_it_out_first() {
        let mut conn = open_memory().unwrap();
        animals(&mut conn);
        create_keyword(&conn, "pets", Some("animals")).unwrap();
        move_keyword(&conn, "dog", Some("pets")).unwrap();
        rename_keyword(&mut conn, "animals", "dog").unwrap();
        assert_eq!(parent_of(&conn, "dog"), None);
        assert_eq!(parent_of(&conn, "pets").as_deref(), Some("dog"));
        assert_eq!(parent_of(&conn, "cat").as_deref(), Some("dog"));
        // The tree still resolves: no loop to chase.
        assert_eq!(matching(&conn, &subjects_filter(&["dog"])), vec![1, 2]);
    }

    #[test]
    fn deleting_a_keyword_lifts_its_branch_and_untags_its_photos() {
        let mut conn = open_memory().unwrap();
        animals(&mut conn);
        add_manual_tag_many(&mut conn, &[1, 2], "animals", "subject").unwrap();
        assert_eq!(delete_keyword(&mut conn, "animals").unwrap(), 2);
        assert_eq!(parent_of(&conn, "dog"), None);
        assert_eq!(get_photo(&conn, 1).unwrap().unwrap().subjects, vec!["dog"]);
        assert!(delete_keyword(&mut conn, "animals").is_err());
    }

    /// A filed keyword leaves as its whole path, with its parents in the flat
    /// list, and comes back into an empty catalogue as the same tree — with
    /// the photo tagged only with what it had.
    #[test]
    fn the_tree_round_trips_through_a_sidecar() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 1);
        add_manual_tag_many(&mut conn, &[1], "places > europe > france", "subject").unwrap();
        add_manual_tag_many(&mut conn, &[1], "pier", "subject").unwrap();
        create_keyword(&conn, "animals", None).unwrap();
        add_manual_tag_many(&mut conn, &[1], "animals", "subject").unwrap();
        create_keyword(&conn, "dog", Some("animals")).unwrap();
        let photo = get_photo(&conn, 1).unwrap().unwrap();
        let meta = xmp_meta(&conn, &photo).unwrap();
        for kw in ["france", "europe", "places", "pier", "animals"] {
            assert!(
                meta.keywords.contains(&kw.to_string()),
                "{kw}: {:?}",
                meta.keywords
            );
        }
        // A parent the photo has goes out as a path of its own; an unfiled
        // keyword stays in the flat list alone.
        assert!(meta.hierarchy.contains(&"places|europe|france".to_string()));
        assert!(meta.hierarchy.contains(&"animals".to_string()));
        assert!(!meta.hierarchy.iter().any(|h| h.contains("pier")));

        let packet = crate::xmp::write(&meta);
        let mut empty = open_memory().unwrap();
        add_photos(&empty, 1);
        import_xmp_meta(&mut empty, 1, &crate::xmp::read(&packet).unwrap()).unwrap();
        let mut subjects = get_photo(&empty, 1).unwrap().unwrap().subjects;
        subjects.sort();
        assert_eq!(subjects, vec!["animals", "france", "pier"]);
        assert_eq!(parent_of(&empty, "france").as_deref(), Some("europe"));
        assert_eq!(parent_of(&empty, "europe").as_deref(), Some("places"));
    }

    fn all_of(names: &[&str]) -> PhotoFilter {
        PhotoFilter {
            subjects_all: true,
            ..subjects_filter(names)
        }
    }

    #[test]
    fn keywords_chosen_together_can_mean_any_or_all() {
        let mut conn = open_memory().unwrap();
        beach_and_dog(&mut conn);
        assert_eq!(
            matching(&conn, &subjects_filter(&["beach", "dog"])),
            vec![1, 2, 3]
        );
        assert_eq!(matching(&conn, &all_of(&["beach", "dog"])), vec![1]);
        // Each still takes in its branch.
        assert_eq!(matching(&conn, &all_of(&["beach", "animals"])), vec![1]);
        // One keyword is the same either way.
        assert_eq!(matching(&conn, &all_of(&["beach"])), vec![1, 2]);
    }

    /// With "all", each keyword's count is what adding it would leave; with
    /// "any" the choice is lifted, as every facet's own is.
    #[test]
    fn counts_follow_any_and_all() {
        let mut conn = open_memory().unwrap();
        beach_and_dog(&mut conn);
        let any = subjects_filter(&["beach"]);
        assert_eq!(subject_count(&conn, &any, "dog"), 2);
        assert_eq!(subject_count(&conn, &any, "car"), 1);
        let all = all_of(&["beach"]);
        assert_eq!(subject_count(&conn, &all, "dog"), 1);
        assert_eq!(subject_count(&conn, &all, "animals"), 1);
        assert_eq!(subject_count(&conn, &all, "car"), 0);
        assert_eq!(subject_count(&conn, &all, "beach"), 2);
    }

    #[test]
    fn a_left_out_keyword_takes_its_branch_with_it() {
        let mut conn = open_memory().unwrap();
        beach_and_dog(&mut conn);
        let not = |names: &[&str]| PhotoFilter {
            exclude_subjects: names.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        assert_eq!(matching(&conn, &not(&["dog"])), vec![2, 4]);
        assert_eq!(matching(&conn, &not(&["animals"])), vec![2, 4]);
        // "beach, not dog".
        let beach_not_dog = PhotoFilter {
            exclude_subjects: vec!["dog".into()],
            ..subjects_filter(&["beach"])
        };
        assert_eq!(matching(&conn, &beach_not_dog), vec![2]);
        // Counts leave the excluded photos out, even the facet's own.
        assert_eq!(subject_count(&conn, &not(&["dog"]), "beach"), 1);
        assert_eq!(subject_count(&conn, &not(&["dog"]), "dog"), 0);
    }

    #[test]
    fn genres_and_conditions_can_be_left_out_too() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 3);
        add_manual_tag_many(&mut conn, &[1, 2], "landscape", "genre").unwrap();
        add_manual_tag_many(&mut conn, &[2, 3], "overcast", "condition").unwrap();
        let f = PhotoFilter {
            exclude_genres: vec!["landscape".into()],
            ..Default::default()
        };
        assert_eq!(matching(&conn, &f), vec![3]);
        let f = PhotoFilter {
            genres: vec!["landscape".into()],
            exclude_conditions: vec!["overcast".into()],
            ..Default::default()
        };
        assert_eq!(matching(&conn, &f), vec![1]);
        let genre_counts: Vec<(String, i32)> = facets(&conn, &f, None)
            .unwrap()
            .genres
            .into_iter()
            .map(|c| (c.value, c.count))
            .collect();
        assert_eq!(genre_counts, vec![("landscape".to_string(), 1)]);
    }

    #[test]
    fn a_rename_follows_views_that_leave_the_keyword_out() {
        let mut conn = open_memory().unwrap();
        beach_and_dog(&mut conn);
        let view = PhotoFilter {
            exclude_subjects: vec!["dog".into()],
            ..subjects_filter(&["beach"])
        };
        let id = save_smart_collection(&conn, "Empty beaches", &view).unwrap();
        rename_keyword(&mut conn, "dog", "hound").unwrap();
        let saved = list_smart_collections(&conn).unwrap();
        let saved = &saved.iter().find(|v| v.id == id).unwrap().filter;
        assert_eq!(saved.exclude_subjects, vec!["hound"]);
        assert_eq!(saved.subjects, vec!["beach"]);
        assert_eq!(matching(&conn, saved), vec![2]);
    }

    #[test]
    fn a_merge_keeps_what_was_turned_down() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 1);
        tagger_says(&mut conn, 1, &["dogs"]);
        remove_tag(&conn, 1, "dogs").unwrap();
        create_keyword(&conn, "dog", None).unwrap();
        rename_keyword(&mut conn, "dogs", "dog").unwrap();
        tagger_says(&mut conn, 1, &["dog"]);
        assert!(photo_of(&conn, 1).subjects.is_empty());
    }

    // --- Keywords from folder names -----------------------------------

    #[test]
    fn a_photos_folder_names_start_at_its_library_folder() {
        assert_eq!(
            folder_names(
                r"C:\Photos\2024\2024-05 Karoo trip\a.arw",
                Some(r"C:\Photos")
            ),
            vec!["Photos", "2024", "2024-05 Karoo trip"]
        );
        assert_eq!(
            folder_names("/Users/me/Pictures/Skye/b.arw", Some("/Users/me/Pictures")),
            vec!["Pictures", "Skye"]
        );
        // In no library folder: only the folder it is in.
        assert_eq!(
            folder_names(r"D:\Misc\Iceland 2019\c.arw", None),
            vec!["Iceland 2019"]
        );
        assert_eq!(
            folder_names(r"C:\Photos\d.arw", Some(r"C:\Photos")),
            vec!["Photos"]
        );
    }

    /// Photos 1 and 2 in "Weddings/2024-05-01 Smith wedding", 3 in
    /// "2024/DCIM/100MSDCF", 4 in another library folder's "Skye"; 5 in none.
    fn foldered(conn: &Connection) {
        conn.execute_batch(
            r"INSERT INTO library_folders (id, path) VALUES (1, 'C:\Photos'), (2, 'E:\Trips');
              INSERT INTO photos (id, path, filename, file_hash, indexed_at, folder_id) VALUES
                (1, 'C:\Photos\Weddings\2024-05-01 Smith wedding\a.arw', 'a', 'h1', '2026-01-01', 1),
                (2, 'C:\Photos\Weddings\2024-05-01 Smith wedding\b.arw', 'b', 'h2', '2026-01-01', 1),
                (3, 'C:\Photos\2024\DCIM\100MSDCF\c.arw', 'c', 'h3', '2026-01-01', 1),
                (4, 'E:\Trips\Skye\d.arw', 'd', 'h4', '2026-01-01', 2),
                (5, 'F:\Loose\Mountains\e.arw', 'e', 'h5', '2026-01-01', NULL);",
        )
        .unwrap();
    }

    #[test]
    fn folder_names_become_keywords_filed_under_folders() {
        let mut conn = open_memory().unwrap();
        foldered(&conn);
        create_keyword(&conn, "Scotland", None).unwrap();
        create_keyword(&conn, "Skye", Some("scotland")).unwrap();
        let got = add_folder_keywords(&mut conn, None, None).unwrap();
        assert_eq!(
            got,
            FolderKeywords {
                photos: 4,
                keywords: 5
            }
        );
        assert_eq!(sorted_subjects(&conn, 1), vec!["smith wedding", "weddings"]);
        assert!(
            photo_of(&conn, 3).subjects.is_empty(),
            "a camera's folders say nothing"
        );
        assert_eq!(sorted_subjects(&conn, 4), vec!["skye", "trips"]);
        assert_eq!(sorted_subjects(&conn, 5), vec!["mountains"]);
        // New ones under "folders"; one the user had already filed stays put.
        assert_eq!(
            parent_of(&conn, "smith wedding").as_deref(),
            Some("folders")
        );
        assert_eq!(parent_of(&conn, "skye").as_deref(), Some("scotland"));
        let source: String = conn
            .query_row(
                "SELECT pt.source FROM photo_tags pt JOIN tags t ON t.id = pt.tag_id
                  WHERE pt.photo_id = 1 AND t.name = 'weddings'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(source, "folder");
        // Asked again, nothing new.
        assert_eq!(
            add_folder_keywords(&mut conn, None, None).unwrap(),
            FolderKeywords::default()
        );
    }

    #[test]
    fn folder_keywords_cover_one_folder_or_one_shoot() {
        let mut conn = open_memory().unwrap();
        foldered(&conn);
        let got = add_folder_keywords(&mut conn, Some(2), None).unwrap();
        assert_eq!(got.photos, 1);
        assert!(photo_of(&conn, 1).subjects.is_empty());
        create_shoot(
            &mut conn,
            "Smith",
            "F:",
            r"C:\Photos",
            &[r"C:\Photos\Weddings\2024-05-01 Smith wedding\b.arw".to_string()],
        )
        .unwrap();
        let shoot: i32 = conn
            .query_row("SELECT id FROM shoots", [], |r| r.get(0))
            .unwrap();
        add_folder_keywords(&mut conn, None, Some(shoot)).unwrap();
        assert!(photo_of(&conn, 1).subjects.is_empty());
        assert_eq!(sorted_subjects(&conn, 2), vec!["smith wedding", "weddings"]);
    }

    #[test]
    fn a_folder_keyword_taken_off_stays_off_and_counts_as_the_users() {
        let mut conn = open_memory().unwrap();
        foldered(&conn);
        add_folder_keywords(&mut conn, None, None).unwrap();
        remove_tag(&conn, 1, "weddings").unwrap();
        add_folder_keywords(&mut conn, None, None).unwrap();
        assert_eq!(sorted_subjects(&conn, 1), vec!["smith wedding"]);
        // Not the tagger's: they show with only the user's keywords shown.
        let own = PhotoFilter {
            own_keywords: true,
            ..Default::default()
        };
        assert_eq!(subject_count(&conn, &own, "smith wedding"), 2);
        assert_eq!(subject_count(&conn, &own, "folders"), 4);
    }

    #[test]
    fn folder_keywords_come_back_off_together() {
        let mut conn = open_memory().unwrap();
        foldered(&conn);
        add_folder_keywords(&mut conn, None, None).unwrap();
        // The user gives one by hand too, on a photo of its own.
        add_manual_tag_many(&mut conn, &[3], "skye", "subject").unwrap();
        assert_eq!(remove_folder_keywords(&mut conn, Some(1)).unwrap(), 2);
        assert!(photo_of(&conn, 1).subjects.is_empty());
        assert_eq!(
            sorted_subjects(&conn, 4),
            vec!["skye", "trips"],
            "another folder's stay"
        );
        assert_eq!(remove_folder_keywords(&mut conn, None).unwrap(), 2);
        assert_eq!(photo_of(&conn, 3).subjects, vec!["skye"]);
        // What was made for the pass and is now on nothing is gone; "skye"
        // is on a photo still, so it and "folders" above it stay.
        let names: Vec<String> = keyword_tree(&conn)
            .unwrap()
            .into_iter()
            .map(|k| k.name)
            .collect();
        assert_eq!(names, vec!["folders", "skye"]);
    }
}
