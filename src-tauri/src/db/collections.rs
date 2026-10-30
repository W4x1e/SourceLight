//! Collections, their groups, and smart collections, which are saved
//! filters.

use super::*;

/// Every collection with its current member count, newest first.
pub fn list_collections(conn: &Connection) -> Result<Vec<Collection>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, c.name,
                (SELECT COUNT(*) FROM collection_photos cp WHERE cp.collection_id = c.id) AS count,
                c.group_id
         FROM collections c
         ORDER BY c.created_at DESC, c.id DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(Collection {
            id: r.get("id")?,
            name: r.get("name")?,
            count: r.get("count")?,
            group_id: r.get("group_id")?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Creates a collection and returns its id. Names are unique.
pub fn create_collection(conn: &Connection, name: &str) -> Result<i32> {
    let name = name.trim();
    if name.is_empty() {
        bail!("a collection needs a name");
    }
    conn.execute(
        "INSERT INTO collections (name, created_at) VALUES (?1, ?2)",
        params![name, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(conn.last_insert_rowid() as i32)
}

pub fn rename_collection(conn: &Connection, id: i32, name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        bail!("a collection needs a name");
    }
    conn.execute(
        "UPDATE collections SET name = ?2 WHERE id = ?1",
        params![id, name],
    )?;
    Ok(())
}

/// Deletes a collection. `collection_photos` rows cascade, so a photo is only
/// removed from the grouping — the photo itself is untouched.
pub fn delete_collection(conn: &Connection, id: i32) -> Result<()> {
    conn.execute("DELETE FROM collections WHERE id = ?1", params![id])?;
    Ok(())
}

/// Adds photos to a collection, ignoring any already present. One transaction
/// for the whole selection rather than a commit per photo.
pub fn add_to_collection(conn: &mut Connection, id: i32, photo_ids: &[i32]) -> Result<()> {
    if photo_ids.is_empty() {
        return Ok(());
    }
    let when = chrono::Utc::now().to_rfc3339();
    let tx = conn.transaction()?;
    {
        // New photos go at the end of the collection's own order.
        let mut stmt = tx.prepare(
            "INSERT OR IGNORE INTO collection_photos (collection_id, photo_id, added_at, position)
             VALUES (?1, ?2, ?3,
                     (SELECT COALESCE(MAX(position), 0) + 1 FROM collection_photos WHERE collection_id = ?1))",
        )?;
        for pid in photo_ids {
            stmt.execute(params![id, pid, when])?;
        }
    }
    tx.commit()?;
    Ok(())
}

pub fn remove_from_collection(conn: &mut Connection, id: i32, photo_ids: &[i32]) -> Result<()> {
    if photo_ids.is_empty() {
        return Ok(());
    }
    let tx = conn.transaction()?;
    {
        let mut stmt =
            tx.prepare("DELETE FROM collection_photos WHERE collection_id = ?1 AND photo_id = ?2")?;
        for pid in photo_ids {
            stmt.execute(params![id, pid])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Every collection group, by name.
pub fn list_collection_groups(conn: &Connection) -> Result<Vec<CollectionGroup>> {
    let mut stmt =
        conn.prepare("SELECT id, name FROM collection_groups ORDER BY name COLLATE NOCASE")?;
    let rows = stmt.query_map([], |r| {
        Ok(CollectionGroup {
            id: r.get(0)?,
            name: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn create_collection_group(conn: &Connection, name: &str) -> Result<i32> {
    let name = name.trim();
    if name.is_empty() {
        bail!("a group needs a name");
    }
    conn.execute(
        "INSERT INTO collection_groups (name, created_at) VALUES (?1, ?2)",
        params![name, chrono::Utc::now().to_rfc3339()],
    )
    .map_err(|e| match e {
        rusqlite::Error::SqliteFailure(f, _)
            if f.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            anyhow::anyhow!("there's already a group called “{name}”")
        }
        e => e.into(),
    })?;
    Ok(conn.last_insert_rowid() as i32)
}

pub fn rename_collection_group(conn: &Connection, id: i32, name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        bail!("a group needs a name");
    }
    conn.execute(
        "UPDATE collection_groups SET name = ?2 WHERE id = ?1",
        params![id, name],
    )?;
    Ok(())
}

/// Deletes a group; what was in it moves up a level.
pub fn delete_collection_group(conn: &Connection, id: i32) -> Result<()> {
    conn.execute("DELETE FROM collection_groups WHERE id = ?1", params![id])?;
    Ok(())
}

/// Files a collection, or a saved view, in a group — or takes it out, with none.
pub fn set_collection_group(
    conn: &Connection,
    id: i32,
    smart: bool,
    group: Option<i32>,
) -> Result<()> {
    let table = if smart {
        "smart_collections"
    } else {
        "collections"
    };
    conn.execute(
        &format!("UPDATE {table} SET group_id = ?2 WHERE id = ?1"),
        params![id, group],
    )?;
    Ok(())
}

/// Moves photos within a collection's own order: the ones given, in the
/// order they are now, go just before `before`, or to the end without it. The
/// whole order is written again, 1 to n, so it never drifts.
pub fn move_in_collection(
    conn: &mut Connection,
    collection: i32,
    photo_ids: &[i32],
    before: Option<i32>,
) -> Result<()> {
    let tx = conn.transaction()?;
    let order: Vec<i32> = tx
        .prepare(
            "SELECT photo_id FROM collection_photos WHERE collection_id = ?1
              ORDER BY position ASC NULLS LAST, photo_id ASC",
        )?
        .query_map(params![collection], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let moving: Vec<i32> = order
        .iter()
        .copied()
        .filter(|id| photo_ids.contains(id))
        .collect();
    let mut rest: Vec<i32> = order
        .into_iter()
        .filter(|id| !photo_ids.contains(id))
        .collect();
    let at = before
        .and_then(|b| rest.iter().position(|id| *id == b))
        .unwrap_or(rest.len());
    rest.splice(at..at, moving);
    {
        let mut set = tx.prepare(
            "UPDATE collection_photos SET position = ?3 WHERE collection_id = ?1 AND photo_id = ?2",
        )?;
        for (i, id) in rest.iter().enumerate() {
            set.execute(params![collection, id, i as i64 + 1])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Every saved filter, newest first. A row whose JSON no longer parses (a filter
/// shape from a future build) is skipped rather than failing the whole list —
/// the same tolerance the preset loader has.
pub fn list_smart_collections(conn: &Connection) -> Result<Vec<SmartCollection>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, filter, group_id FROM smart_collections ORDER BY created_at DESC, id DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i32>("id")?,
            r.get::<_, String>("name")?,
            r.get::<_, String>("filter")?,
            r.get::<_, Option<i32>>("group_id")?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, name, json, group_id) = row?;
        match read_saved_filter(conn, &json) {
            Ok(filter) => out.push(SmartCollection {
                id,
                name,
                filter,
                group_id,
            }),
            Err(e) => eprintln!("skipping smart collection {name:?}: {e}"),
        }
    }
    Ok(out)
}

/// Who a saved view's people are, by something that outlives a regroup.
///
/// A person id does not: `regroup_people` rebuilds the `people` table, so a
/// view saved with Ada's id came back showing someone else, or nobody. Faces
/// keep their ids through a regroup, so a view records a few of each
/// person's faces, and their name for when those faces are gone (a photo
/// removed, or analysed again). Kept in the saved JSON beside the filter,
/// under a key `PhotoFilter` ignores, so no migration is needed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
struct PersonAnchor {
    /// The id the person had when the view was saved.
    id: i32,
    faces: Vec<i64>,
    name: Option<String>,
}

const PERSON_ANCHORS: &str = "personAnchors";

fn anchor_people(conn: &Connection, filter: &PhotoFilter) -> Result<Vec<PersonAnchor>> {
    let mut faces = conn.prepare(
        "SELECT id FROM faces WHERE person_id = ?1 ORDER BY manual, COALESCE(score, 0) DESC, id LIMIT 5",
    )?;
    let mut ids: Vec<i32> = filter.person_ids.clone();
    ids.extend(filter.person_id);
    ids.dedup();
    let mut out = Vec::new();
    for id in ids {
        let name: Option<String> = conn
            .query_row("SELECT name FROM people WHERE id = ?1", params![id], |r| {
                r.get(0)
            })
            .optional()?
            .flatten();
        let faces: Vec<i64> = faces
            .query_map(params![id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        out.push(PersonAnchor { id, faces, name });
    }
    Ok(out)
}

/// The person an anchor names now: whoever most of its faces belong to, else
/// someone with its name, else nobody — 0, which no person has, so the view
/// honestly shows no one rather than whoever inherited the old id.
fn resolve_anchor(conn: &Connection, a: &PersonAnchor) -> Result<i32> {
    let mut tally: std::collections::HashMap<i32, usize> = std::collections::HashMap::new();
    for face in &a.faces {
        let person: Option<i32> = conn
            .query_row(
                "SELECT person_id FROM faces WHERE id = ?1",
                params![face],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        if let Some(p) = person {
            *tally.entry(p).or_default() += 1;
        }
    }
    if let Some((p, _)) = tally.into_iter().max_by_key(|&(p, n)| (n, -p)) {
        return Ok(p);
    }
    if let Some(name) = &a.name {
        let named: Option<i32> = conn
            .query_row(
                "SELECT id FROM people WHERE name = ?1 COLLATE NOCASE ORDER BY size DESC, id LIMIT 1",
                params![name],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(p) = named {
            return Ok(p);
        }
    }
    Ok(0)
}

/// A saved filter, with its people as they are now.
fn read_saved_filter(conn: &Connection, json: &str) -> Result<PhotoFilter> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    let anchors: Vec<PersonAnchor> = value
        .get(PERSON_ANCHORS)
        .map(|a| serde_json::from_value(a.clone()))
        .transpose()?
        .unwrap_or_default();
    let mut filter: PhotoFilter = serde_json::from_value(value)?;
    if anchors.is_empty() {
        return Ok(filter);
    }
    let now = |old: i32| -> Result<i32> {
        match anchors.iter().find(|a| a.id == old) {
            Some(a) => resolve_anchor(conn, a),
            None => Ok(old),
        }
    };
    filter.person_ids = filter
        .person_ids
        .iter()
        .map(|&id| now(id))
        .collect::<Result<_>>()?;
    filter.person_id = filter.person_id.map(now).transpose()?;
    Ok(filter)
}

/// Creates or replaces a saved filter by name, returning its id.
pub fn save_smart_collection(conn: &Connection, name: &str, filter: &PhotoFilter) -> Result<i32> {
    let name = name.trim();
    if name.is_empty() {
        bail!("a smart collection needs a name");
    }
    let mut value = serde_json::to_value(filter)?;
    let anchors = anchor_people(conn, filter)?;
    if !anchors.is_empty() {
        if let Some(map) = value.as_object_mut() {
            map.insert(PERSON_ANCHORS.into(), serde_json::to_value(anchors)?);
        }
    }
    let json = value.to_string();
    conn.execute(
        "INSERT INTO smart_collections(name, filter, created_at) VALUES(?1, ?2, ?3)
         ON CONFLICT(name) DO UPDATE SET filter = excluded.filter",
        params![name, json, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM smart_collections WHERE name = ?1",
        params![name],
        |r| r.get(0),
    )?)
}

pub fn delete_smart_collection(conn: &Connection, id: i32) -> Result<()> {
    conn.execute("DELETE FROM smart_collections WHERE id = ?1", params![id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::*;

    /// A view saved with a person follows them through a regroup that
    /// renumbers everyone — by their faces, then by name, and shows
    /// no one rather than whoever inherited the old id once they're gone.
    #[test]
    fn a_saved_view_keeps_its_person_through_a_regroup() {
        let mut conn = open_memory().unwrap();
        let (ada, bo) = ada_and_bo(&mut conn, 3);
        let view = PhotoFilter {
            person_ids: vec![ada],
            ..Default::default()
        };
        save_smart_collection(&conn, "Ada", &view).unwrap();
        // What a regroup can do: everyone renumbered, and Bo given Ada's old id.
        conn.execute_batch(&format!(
            "UPDATE faces SET person_id = person_id + 100 WHERE person_id IS NOT NULL;
             UPDATE people SET id = id + 100;
             UPDATE faces SET person_id = {ada} WHERE person_id = {bo} + 100;
             UPDATE people SET id = {ada} WHERE id = {bo} + 100;"
        ))
        .unwrap();
        let saved = || {
            list_smart_collections(&conn).unwrap()[0]
                .filter
                .person_ids
                .clone()
        };
        assert_eq!(saved(), vec![ada + 100]);

        conn.execute("DELETE FROM faces WHERE person_id = ?1", params![ada + 100])
            .unwrap();
        assert_eq!(saved(), vec![ada + 100], "by name once her faces are gone");
        conn.execute("DELETE FROM people WHERE name = 'Ada'", [])
            .unwrap();
        assert_eq!(saved(), vec![0], "nobody, not Bo");
    }

    #[test]
    fn a_saved_filter_from_before_145_still_loads() {
        // What a smart collection stored before these fields existed.
        let json = r#"{"dateFrom":null,"dateTo":null,"genres":["landscape"],"subjects":[],
            "conditions":[],"placeIds":[],"minRating":null,"flags":[],"colorLabels":[],
            "collectionId":null,"untaggedOnly":false,"search":null}"#;
        let f: PhotoFilter = serde_json::from_str(json).unwrap();
        assert_eq!(f.genres, vec!["landscape".to_string()]);
        assert!(f.cameras.is_empty() && f.person_ids.is_empty() && !f.edited_only);
        assert_eq!(f.sort, PhotoSort::Newest);
    }

    #[test]
    fn photos_are_arranged_by_hand_and_new_ones_go_last() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 5);
        let c = create_collection(&conn, "Book").unwrap();
        add_to_collection(&mut conn, c, &[1, 2, 3, 4]).unwrap();
        assert_eq!(in_order(&conn, c), vec![1, 2, 3, 4]);

        // 4 and 2, dragged together before 1, keep their own order.
        move_in_collection(&mut conn, c, &[4, 2], Some(1)).unwrap();
        assert_eq!(in_order(&conn, c), vec![2, 4, 1, 3]);
        // To the end, with nothing to go before.
        move_in_collection(&mut conn, c, &[2], None).unwrap();
        assert_eq!(in_order(&conn, c), vec![4, 1, 3, 2]);
        add_to_collection(&mut conn, c, &[5]).unwrap();
        assert_eq!(in_order(&conn, c), vec![4, 1, 3, 2, 5]);

        // Custom without a collection is simply the newest first.
        let f = PhotoFilter {
            sort: PhotoSort::Custom,
            ..Default::default()
        };
        assert_eq!(query_photos(&conn, &f, 10, 0).unwrap().len(), 5);
    }

    #[test]
    fn a_group_holds_collections_and_views_and_leaves_them_when_deleted() {
        let conn = open_memory().unwrap();
        let g = create_collection_group(&conn, "Books").unwrap();
        assert!(
            create_collection_group(&conn, "books").is_err(),
            "names are unique"
        );
        let c = create_collection(&conn, "Summer").unwrap();
        let v = save_smart_collection(&conn, "Picks", &PhotoFilter::default()).unwrap();
        set_collection_group(&conn, c, false, Some(g)).unwrap();
        set_collection_group(&conn, v, true, Some(g)).unwrap();
        assert_eq!(list_collections(&conn).unwrap()[0].group_id, Some(g));
        assert_eq!(list_smart_collections(&conn).unwrap()[0].group_id, Some(g));
        rename_collection_group(&conn, g, "Photo books").unwrap();
        assert_eq!(
            list_collection_groups(&conn).unwrap()[0].name,
            "Photo books"
        );

        delete_collection_group(&conn, g).unwrap();
        assert!(list_collection_groups(&conn).unwrap().is_empty());
        assert_eq!(
            list_collections(&conn).unwrap()[0].group_id,
            None,
            "up a level"
        );
        assert_eq!(list_smart_collections(&conn).unwrap()[0].group_id, None);
    }
}
