//! Places: named places and trips, positions from files and GPX tracks, and
//! pasting one photo's location on others.

use super::*;

/// The place with this name, ignoring case — "skye" is the "Skye" already
/// there, not a second place a letter apart.
fn place_named(conn: &Connection, name: &str) -> Result<Option<i32>> {
    Ok(conn
        .query_row(
            "SELECT id FROM places WHERE name = ?1 COLLATE NOCASE ORDER BY name = ?1 DESC LIMIT 1",
            params![name.trim()],
            |r| r.get(0),
        )
        .optional()?)
}

pub fn upsert_place(
    conn: &Connection,
    name: &str,
    lat: Option<f64>,
    lon: Option<f64>,
) -> Result<i32> {
    let name = name.trim();
    if let Some(id) = place_named(conn, name)? {
        // A position already there wins: it may be a pin the user put down or
        // moved by hand, and the next GPS pass naming the same town must not
        // snap it back.
        conn.execute(
            "UPDATE places SET lat = COALESCE(lat, ?2), lon = COALESCE(lon, ?3) WHERE id = ?1",
            params![id, lat, lon],
        )?;
        return Ok(id);
    }
    conn.execute(
        "INSERT INTO places(name, lat, lon) VALUES(?1, ?2, ?3)",
        params![name, lat, lon],
    )?;
    Ok(conn.last_insert_rowid() as i32)
}

/// Renames a place. When another place already goes by the new name nothing
/// is renamed and that place's id comes back instead, so the caller can ask
/// whether they are the same place and merge them — place names are unique,
/// so two can't simply share one.
pub fn rename_place(conn: &Connection, id: i32, name: &str) -> Result<Option<i32>> {
    let name = name.trim();
    if name.is_empty() {
        bail!("a place needs a name");
    }
    if let Some(other) = place_named(conn, name)?.filter(|&other| other != id) {
        return Ok(Some(other));
    }
    conn.execute(
        "UPDATE places SET name = ?2 WHERE id = ?1",
        params![id, name],
    )?;
    Ok(None)
}

/// Folds places into one: their photos move to `into` and they are deleted.
/// `into` keeps its own pin, or takes the first merged place's when it has
/// none — a hand-named place merged into a GPS town should not lose the map.
pub fn merge_places(conn: &mut Connection, from: &[i32], into: i32) -> Result<()> {
    let tx = conn.transaction()?;
    for &id in from.iter().filter(|&&id| id != into) {
        tx.execute(
            "UPDATE photo_places SET place_id = ?2 WHERE place_id = ?1",
            params![id, into],
        )?;
        tx.execute(
            "UPDATE trips SET place_id = ?2 WHERE place_id = ?1",
            params![id, into],
        )?;
        tx.execute(
            "UPDATE places SET
                lat = (SELECT lat FROM places WHERE id = ?1),
                lon = (SELECT lon FROM places WHERE id = ?1)
              WHERE id = ?2 AND lat IS NULL",
            params![id, into],
        )?;
        tx.execute("DELETE FROM places WHERE id = ?1", params![id])?;
    }
    tx.commit()?;
    Ok(())
}

/// Deletes a place. Its photos are kept and simply lose the place. One that
/// GPS named is not named again by the next "Read locations" — each photo is
/// considered once ([`place_from_gps`]), so taking a place away sticks.
pub fn delete_place(conn: &Connection, id: i32) -> Result<()> {
    // `photo_places` rows cascade.
    conn.execute("DELETE FROM places WHERE id = ?1", params![id])?;
    Ok(())
}

/// Pins a place to the map by hand, or moves its pin — for places named from
/// trips of a camera with no GPS, which otherwise have no position to draw.
/// Only the place moves: its photos' own positions are what their files said,
/// and stay that way.
pub fn set_place_position(conn: &Connection, place_id: i32, lat: f64, lon: f64) -> Result<()> {
    let lat = lat.clamp(-90.0, 90.0);
    let lon = lon.clamp(-180.0, 180.0);
    conn.execute(
        "UPDATE places SET lat = ?2, lon = ?3 WHERE id = ?1",
        params![place_id, lat, lon],
    )?;
    Ok(())
}

/// Names a trip: places the photos in it that have **no position of their
/// own**, and leaves the rest where their GPS put them. Naming a trip used to
/// re-place every photo in it, so the phone shots of a trip — already in
/// "Edinburgh, United Kingdom" from their own coordinates — became "Scotland"
/// along with the camera's. Picking photos in the grid and setting a place
/// ([`assign_place`]) is still a deliberate override and still wins.
///
/// A trip whose every photo carries a position creates no place at all.
pub fn name_trip(conn: &mut Connection, photo_ids: &[i32], name: &str) -> Result<TripNamed> {
    let tx = conn.transaction()?;
    let unlocated: Vec<i32> = {
        let mut stmt = tx.prepare("SELECT lat IS NULL FROM photos WHERE id = ?1")?;
        let mut out = Vec::new();
        for &id in photo_ids {
            if stmt
                .query_row(params![id], |r| r.get::<_, bool>(0))
                .unwrap_or(false)
            {
                out.push(id);
            }
        }
        out
    };
    // Of those, only the ones with no place yet, or in the trip's own place —
    // the one most of them share, which naming it again renames.
    // Others were placed by hand, one by one: "Rename" on a trip used to move
    // 60 frames carefully placed at Skye into the trip's new name.
    let current: Vec<(i32, Option<i32>)> = {
        let mut stmt = tx.prepare("SELECT place_id FROM photo_places WHERE photo_id = ?1")?;
        let mut out = Vec::new();
        for &id in &unlocated {
            out.push((id, stmt.query_row(params![id], |r| r.get(0)).optional()?));
        }
        out
    };
    let trip_place: Option<i32> = {
        let mut tally: std::collections::HashMap<i32, usize> = std::collections::HashMap::new();
        for (_, place) in &current {
            if let Some(p) = place {
                *tally.entry(*p).or_default() += 1;
            }
        }
        tally
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
            .map(|(p, _)| p)
    };
    let unlocated: Vec<i32> = current
        .iter()
        .filter(|(_, place)| place.is_none() || *place == trip_place)
        .map(|(id, _)| *id)
        .collect();
    let placed = unlocated.len() as i32;
    let kept_placed = current.len() as i32 - placed;
    let named = TripNamed {
        placed,
        kept: photo_ids.len() as i32 - placed - kept_placed,
        kept_placed,
    };
    if unlocated.is_empty() {
        return Ok(named);
    }
    let place_id = upsert_place(&tx, name, None, None)?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO photo_places(photo_id, place_id) VALUES(?1, ?2)
             ON CONFLICT(photo_id) DO UPDATE SET place_id = excluded.place_id",
        )?;
        for id in &unlocated {
            stmt.execute(params![id, place_id])?;
        }
    }
    tx.commit()?;
    Ok(named)
}

pub fn assign_place(conn: &mut Connection, photo_ids: &[i32], place_id: i32) -> Result<()> {
    let tx = conn.transaction()?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO photo_places(photo_id, place_id) VALUES(?1, ?2)
             ON CONFLICT(photo_id) DO UPDATE SET place_id = excluded.place_id",
        )?;
        for id in photo_ids {
            stmt.execute(params![id, place_id])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Takes the place off each photo. The place itself stays, with its
/// pin and any other photos; returns how many photos had one.
pub fn clear_place(conn: &mut Connection, photo_ids: &[i32]) -> Result<usize> {
    let tx = conn.transaction()?;
    let mut cleared = 0;
    {
        let mut stmt = tx.prepare("DELETE FROM photo_places WHERE photo_id = ?1")?;
        for id in photo_ids {
            cleared += stmt.execute(params![id])?;
        }
    }
    tx.commit()?;
    Ok(cleared)
}

/// Photos whose file has not yet been checked for a position — a library
/// catalogued before GPS was read. Files that are missing are left for
/// when they come back.
pub fn photos_needing_gps(conn: &Connection) -> Result<Vec<(i32, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, path FROM photos WHERE variant = 0 AND gps_read = 0 AND missing = 0",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Puts one photo's location on others: its named place on every one
/// of them, and its position on those with **none of their own** — a phone's
/// real coordinates are never overwritten by another photo's. A position given
/// this way is one a GPX track then leaves alone, since the track only places
/// photos with none. Virtual copies follow their originals.
pub fn paste_location(conn: &mut Connection, ids: &[i32], from: i32) -> Result<PastedLocation> {
    let (lat, lon, place): (Option<f64>, Option<f64>, Option<i32>) = conn
        .query_row(
            "SELECT p.lat, p.lon, pp.place_id FROM photos p
               LEFT JOIN photo_places pp ON pp.photo_id = p.id
              WHERE p.id = ?1",
            params![from],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("that photo is no longer in the library"))?;
    let position = lat.zip(lon);
    if position.is_none() && place.is_none() {
        bail!("that photo has no location to copy");
    }
    let tx = conn.transaction()?;
    let mut out = PastedLocation::default();
    for &id in ids.iter().filter(|&&id| id != from) {
        if let Some(place) = place {
            tx.execute(
                "INSERT INTO photo_places (photo_id, place_id) VALUES (?1, ?2)
                 ON CONFLICT(photo_id) DO UPDATE SET place_id = excluded.place_id",
                params![id, place],
            )?;
            out.placed += 1;
        }
        if let Some((lat, lon)) = position {
            // `place_auto`: a place pasted, or none, is not one for the next
            // "Read locations" to name over.
            let n = tx.execute(
                "UPDATE photos SET lat = ?2, lon = ?3, gps_read = 1, place_auto = 1
                  WHERE (id = ?1 OR variant_of = ?1) AND lat IS NULL",
                params![id, lat, lon],
            )?;
            if n > 0 {
                out.positioned += 1;
            }
        }
    }
    tx.commit()?;
    Ok(out)
}

/// Photos with a capture time and no position — the ones a GPX track can
/// place: `(id, capture_date)`.
pub fn photos_without_position(conn: &Connection) -> Result<Vec<(i32, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, capture_date FROM photos
          WHERE variant = 0 AND lat IS NULL AND capture_date IS NOT NULL",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Records what a file said about its position, on the photo and its copies.
/// It is the camera's, so no track moves it again.
pub fn set_gps(conn: &Connection, id: i32, gps: Option<(f64, f64)>) -> Result<()> {
    conn.execute(
        "UPDATE photos SET lat = ?2, lon = ?3, gps_read = 1, gps_track = NULL, gps_offset = NULL
          WHERE id = ?1 OR variant_of = ?1",
        params![id, gps.map(|g| g.0), gps.map(|g| g.1)],
    )?;
    Ok(())
}

/// How a set of GPX files is recorded on the photos they place: their
/// paths, sorted, one per line.
pub fn track_key(paths: &[String]) -> String {
    let mut sorted: Vec<&str> = paths.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    sorted.dedup();
    sorted.join("\n")
}

/// The GPX files a [`track_key`] names.
pub fn track_paths(key: &str) -> Vec<String> {
    key.lines().map(str::to_string).collect()
}

/// Records where a GPX track put a photo, on it and its copies, with
/// the track and the camera's offset from UTC used — so a later change to its
/// capture time can move it along the same track.
pub fn set_track_gps(
    conn: &Connection,
    id: i32,
    pos: (f64, f64),
    track: &str,
    offset_minutes: i32,
) -> Result<()> {
    conn.execute(
        "UPDATE photos SET lat = ?2, lon = ?3, gps_read = 1, gps_track = ?4, gps_offset = ?5
          WHERE id = ?1 OR variant_of = ?1",
        params![id, pos.0, pos.1, track, offset_minutes],
    )?;
    Ok(())
}

/// A photo a GPX track placed.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackPlaced {
    pub id: i32,
    pub capture_date: String,
    /// A [`track_key`].
    pub track: String,
    pub offset_minutes: i32,
}

/// The photos among these files (copies count as their originals) that a
/// GPX track placed, and have a time to place them by.
pub fn track_placed(conn: &Connection, ids: &[i32]) -> Result<Vec<TrackPlaced>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT id, capture_date, gps_track, gps_offset FROM photos
          WHERE variant = 0 AND gps_track IS NOT NULL AND capture_date IS NOT NULL
            AND path IN (SELECT path FROM photos WHERE id IN ({}))
          ORDER BY id",
        placeholders(ids.len())
    ))?;
    let rows = stmt.query_map(rusqlite::params_from_iter(ids), |r| {
        Ok(TrackPlaced {
            id: r.get(0)?,
            capture_date: r.get(1)?,
            track: r.get(2)?,
            offset_minutes: r.get::<_, Option<i32>>(3)?.unwrap_or(0),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Moves a photo its track placed to where the track now puts it, on
/// it and its copies. Its town follows only when it is still the one
/// automatic naming gave the old position: a place set by hand stays.
pub fn move_along_track(conn: &mut Connection, id: i32, pos: (f64, f64)) -> Result<()> {
    let tx = conn.transaction()?;
    let old: Option<(f64, f64)> = tx
        .query_row(
            "SELECT lat, lon FROM photos WHERE id = ?1 AND lat IS NOT NULL",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    tx.execute(
        "UPDATE photos SET lat = ?2, lon = ?3 WHERE id = ?1 OR variant_of = ?1",
        params![id, pos.0, pos.1],
    )?;
    let named = |p: Option<(f64, f64)>| p.and_then(|(lat, lon)| crate::geo::place_for(lat, lon));
    if let (Some((was, ..)), now) = (named(old), named(Some(pos))) {
        let rows: Vec<i32> = tx
            .prepare(
                "SELECT pp.photo_id FROM photo_places pp
                   JOIN places pl ON pl.id = pp.place_id
                   JOIN photos p ON p.id = pp.photo_id
                  WHERE (p.id = ?1 OR p.variant_of = ?1) AND pl.name = ?2",
            )?
            .query_map(params![id, was], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        if !rows.is_empty() {
            match now {
                Some((name, plat, plon)) if name != was => {
                    let place = upsert_place(&tx, &name, Some(plat), Some(plon))?;
                    for photo in &rows {
                        tx.execute(
                            "UPDATE photo_places SET place_id = ?2 WHERE photo_id = ?1",
                            params![photo, place],
                        )?;
                    }
                }
                Some(_) => {}
                None => {
                    for photo in &rows {
                        tx.execute(
                            "DELETE FROM photo_places WHERE photo_id = ?1",
                            params![photo],
                        )?;
                    }
                }
            }
        }
    }
    tx.commit()?;
    Ok(())
}

/// Gives every photo with a position, and no place yet, the town it was taken
/// in — named offline by `geo::place_for`. A place someone set by hand
/// is never replaced, and each photo is considered once: take its place off
/// and it stays off. Returns how many photos were placed.
pub fn place_from_gps(conn: &mut Connection) -> Result<usize> {
    let tx = conn.transaction()?;
    let todo: Vec<(i32, f64, f64)> = tx
        .prepare(
            "SELECT p.id, p.lat, p.lon FROM photos p
              WHERE p.lat IS NOT NULL AND p.place_auto = 0
                AND NOT EXISTS (SELECT 1 FROM photo_places pp WHERE pp.photo_id = p.id)",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut ids: std::collections::HashMap<String, i32> = std::collections::HashMap::new();
    let mut placed = 0;
    for (photo, lat, lon) in &todo {
        if let Some((name, plat, plon)) = crate::geo::place_for(*lat, *lon) {
            let place = match ids.get(&name) {
                Some(id) => *id,
                None => {
                    let id = upsert_place(&tx, &name, Some(plat), Some(plon))?;
                    ids.insert(name, id);
                    id
                }
            };
            tx.execute(
                "INSERT OR IGNORE INTO photo_places (photo_id, place_id) VALUES (?1, ?2)",
                params![photo, place],
            )?;
            placed += 1;
        }
    }
    tx.execute(
        "UPDATE photos SET place_auto = 1 WHERE lat IS NOT NULL AND place_auto = 0",
        [],
    )?;
    tx.commit()?;
    Ok(placed)
}

/// Every named place with how many photos are in it, most-photographed first.
///
/// Deliberately **not** the place facet. A facet is a description of the
/// current query, which is right in the filter rail and wrong in the Places
/// workspace: leaving Places by opening a trip in Browse sets a date filter,
/// and coming back then showed one place out of three, with a count of 1,
/// because the facet was faithfully describing a query the user had forgotten
/// making. This is a navigation list and answers about the whole library.
pub fn list_places(conn: &Connection) -> Result<Vec<Place>> {
    let mut stmt = conn.prepare(
        "SELECT pl.id, pl.name, pl.lat, pl.lon,
                (SELECT COUNT(*) FROM photo_places pp
                  JOIN photos p ON p.id = pp.photo_id
                 WHERE pp.place_id = pl.id) AS c
         FROM places pl
         ORDER BY c DESC, pl.name",
    )?;
    let rows = stmt.query_map([], |r| {
        let name: String = r.get(1)?;
        let lat: Option<f64> = r.get(2)?;
        let lon: Option<f64> = r.get(3)?;
        let country = crate::geo::split_country(&name)
            .map(|(_, country)| country)
            .or_else(|| {
                lat.zip(lon)
                    .and_then(|(la, lo)| crate::geo::country_at(la, lo))
            });
        Ok(Place {
            id: r.get(0)?,
            name,
            lat,
            lon,
            count: r.get::<_, i64>(4)? as i32,
            country,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Up to `n` thumbnails spread across a place's photos in capture order —
/// what the map shows when a pin is hovered, so a place can be recognised
/// without opening it.
pub fn place_thumbs(conn: &Connection, place_id: i32, n: usize) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT p.thumb_path FROM photo_places pp JOIN photos p ON p.id = pp.photo_id
          WHERE pp.place_id = ?1 AND p.thumb_path IS NOT NULL
          ORDER BY p.capture_date, p.id",
    )?;
    let all = stmt
        .query_map(params![place_id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(crate::trips::spread(&all, n))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::*;

    /// A photo with a position gets its town; one the user placed by
    /// hand keeps theirs; and a place taken off is not put back.
    #[test]
    fn gps_names_places_but_never_overrides_the_user() {
        let mut conn = open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at)
                 VALUES (1, '/a.arw', 'a.arw', 'h1', '2026-01-01'),
                        (2, '/b.arw', 'b.arw', 'h2', '2026-01-01'),
                        (3, '/c.arw', 'c.arw', 'h3', '2026-01-01');",
        )
        .unwrap();
        // Edinburgh, Edinburgh again, and somewhere the user already named.
        set_gps(&conn, 1, Some((55.9533, -3.1883))).unwrap();
        set_gps(&conn, 2, Some((55.9500, -3.2000))).unwrap();
        set_gps(&conn, 3, Some((55.9533, -3.1883))).unwrap();
        let skye = upsert_place(&conn, "Skye", None, None).unwrap();
        assign_place(&mut conn, &[3], skye).unwrap();

        assert_eq!(place_from_gps(&mut conn).unwrap(), 2);
        let place_of = |conn: &Connection, id: i32| -> Option<String> {
            conn.query_row(
                "SELECT pl.name FROM photo_places pp JOIN places pl ON pl.id = pp.place_id
                  WHERE pp.photo_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .ok()
        };
        assert_eq!(
            place_of(&conn, 1).as_deref(),
            Some("Edinburgh, United Kingdom")
        );
        assert_eq!(
            place_of(&conn, 2).as_deref(),
            Some("Edinburgh, United Kingdom")
        );
        assert_eq!(
            place_of(&conn, 3).as_deref(),
            Some("Skye"),
            "a hand-set place wins"
        );
        let edinburgh = list_places(&conn)
            .unwrap()
            .into_iter()
            .find(|p| p.name.starts_with("Edinburgh"))
            .unwrap();
        assert!(edinburgh.lat.is_some(), "the map needs the town's position");

        // Taken off by hand: a second pass leaves it off.
        conn.execute("DELETE FROM photo_places WHERE photo_id = 1", [])
            .unwrap();
        assert_eq!(place_from_gps(&mut conn).unwrap(), 0);
        assert_eq!(place_of(&conn, 1), None);
        assert!(photos_needing_gps(&conn).unwrap().is_empty());
    }

    #[test]
    fn places_can_be_renamed_merged_and_deleted() {
        let mut conn = open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at)
                 VALUES (1, '/a.arw', 'a.arw', 'h1', '2026-01-01'),
                        (2, '/b.arw', 'b.arw', 'h2', '2026-01-01'),
                        (3, '/c.arw', 'c.arw', 'h3', '2026-01-01');",
        )
        .unwrap();
        let skye = upsert_place(&conn, "Skye", None, None).unwrap();
        assign_place(&mut conn, &[1], skye).unwrap();
        // Typed with different case, it is the same place — not a second one.
        assert_eq!(upsert_place(&conn, "  skye ", None, None).unwrap(), skye);
        let isle = upsert_place(&conn, "Isle of Skye", Some(57.3), Some(-6.2)).unwrap();
        assign_place(&mut conn, &[2, 3], isle).unwrap();
        let find = |conn: &Connection, id: i32| {
            list_places(conn).unwrap().into_iter().find(|p| p.id == id)
        };

        // A plain rename, and a case-only rename of itself, both go through.
        assert_eq!(rename_place(&conn, skye, "Skye, Scotland").unwrap(), None);
        assert_eq!(rename_place(&conn, skye, "skye, scotland").unwrap(), None);
        assert_eq!(find(&conn, skye).unwrap().name, "skye, scotland");
        assert!(rename_place(&conn, skye, "  ").is_err());

        // A clash renames nothing and says who with.
        assert_eq!(
            rename_place(&conn, skye, "isle of skye").unwrap(),
            Some(isle)
        );
        assert_eq!(find(&conn, skye).unwrap().name, "skye, scotland");

        // Merging the pinned place into the unpinned one moves the photos and
        // the pin, and removes the old place.
        merge_places(&mut conn, &[isle], skye).unwrap();
        assert!(find(&conn, isle).is_none());
        let merged = find(&conn, skye).unwrap();
        assert_eq!(merged.count, 3);
        assert_eq!((merged.lat, merged.lon), (Some(57.3), Some(-6.2)));

        // Deleting keeps the photos; they just lose the place.
        delete_place(&conn, skye).unwrap();
        assert!(list_places(&conn).unwrap().is_empty());
        let photos: i64 = conn
            .query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0))
            .unwrap();
        let placed: i64 = conn
            .query_row("SELECT COUNT(*) FROM photo_places", [], |r| r.get(0))
            .unwrap();
        assert_eq!((photos, placed), (3, 0));
    }

    #[test]
    fn places_know_their_country_and_show_a_spread_of_photos() {
        let mut conn = open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at, capture_date, thumb_path)
                 VALUES (1, '/1.jpg', '1.jpg', 'h1', '2026-01-01', '2025-01-01 10:00:00', 't1'),
                        (2, '/2.jpg', '2.jpg', 'h2', '2026-01-01', '2025-01-02 10:00:00', 't2'),
                        (3, '/3.jpg', '3.jpg', 'h3', '2026-01-01', '2025-01-03 10:00:00', NULL),
                        (4, '/4.jpg', '4.jpg', 'h4', '2026-01-01', '2025-01-04 10:00:00', 't4');",
        )
        .unwrap();
        let edinburgh = upsert_place(&conn, "Edinburgh, United Kingdom", None, None).unwrap();
        let karoo = upsert_place(&conn, "Karoo", Some(-32.35), Some(22.58)).unwrap();
        let skye = upsert_place(&conn, "Skye", None, None).unwrap();
        assign_place(&mut conn, &[1, 2, 3, 4], edinburgh).unwrap();
        let country = |id: i32| {
            list_places(&conn)
                .unwrap()
                .into_iter()
                .find(|p| p.id == id)
                .unwrap()
                .country
        };
        assert_eq!(
            country(edinburgh).as_deref(),
            Some("United Kingdom"),
            "from the name"
        );
        assert_eq!(
            country(skye),
            None,
            "a typed name with no pin is nowhere yet"
        );
        // From the pin — the Karoo is in South Africa, if its pin is near a town.
        set_place_position(&conn, karoo, -32.35, 22.58).unwrap();
        assert_eq!(country(karoo).as_deref(), Some("South Africa"));

        // Capture order, photos with no thumbnail left out.
        assert_eq!(
            place_thumbs(&conn, edinburgh, 4).unwrap(),
            vec!["t1", "t2", "t4"]
        );
        assert_eq!(place_thumbs(&conn, edinburgh, 2).unwrap(), vec!["t1", "t4"]);
        assert!(place_thumbs(&conn, skye, 4).unwrap().is_empty());
    }

    #[test]
    fn a_deleted_gps_place_is_not_named_again() {
        let mut conn = open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at)
                 VALUES (1, '/a.jpg', 'a.jpg', 'h1', '2026-01-01');",
        )
        .unwrap();
        set_gps(&conn, 1, Some((55.9533, -3.1883))).unwrap();
        assert_eq!(place_from_gps(&mut conn).unwrap(), 1);
        let edinburgh = list_places(&conn).unwrap()[0].id;
        delete_place(&conn, edinburgh).unwrap();
        assert_eq!(place_from_gps(&mut conn).unwrap(), 0);
        assert!(list_places(&conn).unwrap().is_empty());
    }

    #[test]
    fn naming_a_trip_leaves_photos_placed_by_their_own_gps() {
        let mut conn = open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at)
                 VALUES (1, '/a.arw', 'a.arw', 'h1', '2026-01-01'),
                        (2, '/b.arw', 'b.arw', 'h2', '2026-01-01'),
                        (3, '/c.jpg', 'c.jpg', 'h3', '2026-01-01');",
        )
        .unwrap();
        // Two camera frames with no GPS, one phone shot placed by its own.
        set_gps(&conn, 3, Some((55.9533, -3.1883))).unwrap();
        place_from_gps(&mut conn).unwrap();
        let place_of = |conn: &Connection, id: i32| -> Option<String> {
            conn.query_row(
                "SELECT pl.name FROM photo_places pp JOIN places pl ON pl.id = pp.place_id
                  WHERE pp.photo_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .ok()
        };

        let named = name_trip(&mut conn, &[1, 2, 3], "Scotland").unwrap();
        assert_eq!(
            named,
            TripNamed {
                placed: 2,
                kept: 1,
                kept_placed: 0
            }
        );
        assert_eq!(place_of(&conn, 1).as_deref(), Some("Scotland"));
        assert_eq!(place_of(&conn, 2).as_deref(), Some("Scotland"));
        assert_eq!(
            place_of(&conn, 3).as_deref(),
            Some("Edinburgh, United Kingdom"),
            "the phone shot keeps the town its GPS gave it"
        );

        // Renaming the trip moves the camera frames, still not the phone's.
        let named = name_trip(&mut conn, &[1, 2, 3], "Highlands").unwrap();
        assert_eq!(
            named,
            TripNamed {
                placed: 2,
                kept: 1,
                kept_placed: 0
            }
        );
        assert_eq!(place_of(&conn, 1).as_deref(), Some("Highlands"));
        assert_eq!(
            place_of(&conn, 3).as_deref(),
            Some("Edinburgh, United Kingdom")
        );

        // A trip of nothing but GPS photos creates no empty place.
        let named = name_trip(&mut conn, &[3], "Nowhere").unwrap();
        assert_eq!(
            named,
            TripNamed {
                placed: 0,
                kept: 1,
                kept_placed: 0
            }
        );
        assert!(list_places(&conn)
            .unwrap()
            .iter()
            .all(|p| p.name != "Nowhere"));

        // Picking photos by hand is still an override.
        let skye = upsert_place(&conn, "Skye", None, None).unwrap();
        assign_place(&mut conn, &[3], skye).unwrap();
        assert_eq!(place_of(&conn, 3).as_deref(), Some("Skye"));
    }

    /// Naming a trip again renames its own place, and leaves the photos
    /// someone placed one by one where they put them.
    #[test]
    fn naming_a_trip_keeps_places_set_by_hand() {
        let mut conn = open_memory().unwrap();
        for id in 1..=4 {
            photo_at(&conn, id, &format!("/p/{id}.arw"));
        }
        let named = name_trip(&mut conn, &[1, 2, 3, 4], "Scotland").unwrap();
        assert_eq!(named.placed, 4);
        let skye = upsert_place(&conn, "Skye", None, None).unwrap();
        assign_place(&mut conn, &[4], skye).unwrap();

        let named = name_trip(&mut conn, &[1, 2, 3, 4], "Scotland 2025").unwrap();
        assert_eq!(
            named,
            TripNamed {
                placed: 3,
                kept: 0,
                kept_placed: 1
            }
        );
        let place_of = |id: i32| -> String {
            conn.query_row(
                "SELECT pl.name FROM photo_places pp JOIN places pl ON pl.id = pp.place_id
                  WHERE pp.photo_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(place_of(1), "Scotland 2025");
        assert_eq!(place_of(4), "Skye", "placed by hand, and kept");
    }

    #[test]
    fn a_named_place_can_be_pinned_to_the_map_by_hand() {
        let mut conn = open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at)
                 VALUES (1, '/a.arw', 'a.arw', 'h1', '2026-01-01');",
        )
        .unwrap();
        let karoo = upsert_place(&conn, "Karoo", None, None).unwrap();
        assign_place(&mut conn, &[1], karoo).unwrap();
        let find = |conn: &Connection| {
            list_places(conn)
                .unwrap()
                .into_iter()
                .find(|p| p.id == karoo)
                .unwrap()
        };
        assert_eq!(find(&conn).lat, None, "named from a trip, so nowhere yet");

        set_place_position(&conn, karoo, -32.3, 22.5).unwrap();
        let p = find(&conn);
        assert_eq!((p.lat, p.lon), (Some(-32.3), Some(22.5)));
        assert_eq!(p.count, 1, "pinning the place keeps its photos");

        // Moved, not stacked — and the photo's own position is untouched.
        set_place_position(&conn, karoo, -31.0, 23.0).unwrap();
        let p = find(&conn);
        assert_eq!((p.lat, p.lon), (Some(-31.0), Some(23.0)));
        let photo_lat: Option<f64> = conn
            .query_row("SELECT lat FROM photos WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(photo_lat, None);

        // Naming it again — or a GPS pass landing on the same name — keeps
        // the pin where the user put it.
        upsert_place(&conn, "Karoo", Some(-33.0), Some(20.0)).unwrap();
        let p = find(&conn);
        assert_eq!((p.lat, p.lon), (Some(-31.0), Some(23.0)));
    }

    /// A place comes off photos without taking the place with it.
    #[test]
    fn a_place_comes_off_photos_and_stays_itself() {
        let mut conn = gear_library();
        assert_eq!(
            clear_place(&mut conn, &[1, 2]).unwrap(),
            1,
            "only 1 had one"
        );
        assert_eq!(get_photo(&conn, 1).unwrap().unwrap().place, None);
        let places: Vec<String> = list_places(&conn)
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(places, vec!["Karoo".to_string()]);
    }

    // --- Copy and paste location and keywords ----------------------------

    fn place_of(conn: &Connection, id: i32) -> Option<String> {
        photo_of(conn, id).place
    }

    fn position_of(conn: &Connection, id: i32) -> Option<(f64, f64)> {
        conn.query_row(
            "SELECT lat, lon FROM photos WHERE id = ?1",
            params![id],
            |r| Ok(r.get::<_, Option<f64>>(0)?.zip(r.get::<_, Option<f64>>(1)?)),
        )
        .unwrap()
    }

    #[test]
    fn a_pasted_location_places_all_and_positions_only_the_unplaced() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 3);
        // Photo 1 has a position and a place; photo 3 its own GPS.
        set_gps(&conn, 1, Some((57.27, -6.21))).unwrap();
        let skye = upsert_place(&conn, "Skye", None, None).unwrap();
        assign_place(&mut conn, &[1], skye).unwrap();
        set_gps(&conn, 3, Some((55.95, -3.19))).unwrap();
        let out = paste_location(&mut conn, &[1, 2, 3], 1).unwrap();
        assert_eq!(
            out,
            PastedLocation {
                placed: 2,
                positioned: 1
            }
        );
        assert_eq!(place_of(&conn, 2).as_deref(), Some("Skye"));
        assert_eq!(place_of(&conn, 3).as_deref(), Some("Skye"));
        assert_eq!(position_of(&conn, 2), Some((57.27, -6.21)));
        assert_eq!(
            position_of(&conn, 3),
            Some((55.95, -3.19)),
            "its own GPS stays"
        );
        // A GPX track now leaves photo 2 alone: it has a position.
        assert!(photos_without_position(&conn)
            .unwrap()
            .iter()
            .all(|(id, _)| *id != 2));
    }

    #[test]
    fn a_place_alone_pastes_and_nothing_to_paste_is_said() {
        let mut conn = open_memory().unwrap();
        add_photos(&conn, 3);
        let karoo = upsert_place(&conn, "Karoo", None, None).unwrap();
        assign_place(&mut conn, &[1], karoo).unwrap();
        let out = paste_location(&mut conn, &[2], 1).unwrap();
        assert_eq!(
            out,
            PastedLocation {
                placed: 1,
                positioned: 0
            }
        );
        assert_eq!(place_of(&conn, 2).as_deref(), Some("Karoo"));
        assert!(paste_location(&mut conn, &[1], 3).is_err());
    }

    // --- Photos a GPX track placed -------------------------------------

    fn scan_with_gps(conn: &Connection, path: &str, date: &str, gps: Option<(f64, f64)>) {
        upsert_photo(
            conn,
            &crate::scan::IngestedPhoto {
                path: path.into(),
                filename: path.into(),
                file_hash: path.into(),
                capture_date: Some(date.into()),
                camera: None,
                lens: None,
                iso: None,
                aperture: None,
                shutter: None,
                focal_length: None,
                width: 6000,
                height: 4000,
                orientation: 1,
                thumb_path: None,
                preview_path: None,
                sharpness: None,
                brightness: None,
                gps,
                stamp: None,
            },
        )
        .unwrap();
    }

    const EDINBURGH: (f64, f64) = (55.9533, -3.1883);

    const LONDON: (f64, f64) = (51.5074, -0.1278);

    #[test]
    fn a_rescan_keeps_a_tracks_position_but_the_cameras_own_wins() {
        let conn = open_memory().unwrap();
        let date = "2026-05-03 14:00:00";
        scan_with_gps(&conn, "a.arw", date, None);
        let id: i32 = conn
            .query_row("SELECT id FROM photos", [], |r| r.get(0))
            .unwrap();
        set_track_gps(&conn, id, EDINBURGH, "walk.gpx", 60).unwrap();

        // The file still has no position: the track's stays, and still counts.
        scan_with_gps(&conn, "a.arw", date, None);
        assert_eq!(position_of(&conn, id), Some(EDINBURGH));
        assert_eq!(track_placed(&conn, &[id]).unwrap().len(), 1);

        // The file has one now: the camera's own wins, and no track moves it.
        scan_with_gps(&conn, "a.arw", date, Some(LONDON));
        assert_eq!(position_of(&conn, id), Some(LONDON));
        assert!(track_placed(&conn, &[id]).unwrap().is_empty());

        // Likewise reading the file's position.
        set_track_gps(&conn, id, EDINBURGH, "walk.gpx", 60).unwrap();
        set_gps(&conn, id, Some(LONDON)).unwrap();
        assert!(track_placed(&conn, &[id]).unwrap().is_empty());
    }

    #[test]
    fn a_moved_photo_takes_its_new_town_unless_it_was_set_by_hand() {
        let mut conn = open_memory().unwrap();
        dated(&conn, 1, "a.arw", Some("2026-05-03 14:00:00"));
        dated(&conn, 2, "b.arw", Some("2026-05-03 14:01:00"));
        dated(&conn, 3, "c.arw", Some("2026-05-03 14:02:00"));
        for id in [1, 2, 3] {
            set_track_gps(&conn, id, EDINBURGH, "walk.gpx", 60).unwrap();
        }
        place_from_gps(&mut conn).unwrap();
        let edinburgh = crate::geo::place_for(EDINBURGH.0, EDINBURGH.1).unwrap().0;
        let london = crate::geo::place_for(LONDON.0, LONDON.1).unwrap().0;
        assert_eq!(place_of(&conn, 1).as_deref(), Some(edinburgh.as_str()));
        let home = upsert_place(&conn, "Home", None, None).unwrap();
        assign_place(&mut conn, &[2], home).unwrap();

        move_along_track(&mut conn, 1, LONDON).unwrap();
        move_along_track(&mut conn, 2, LONDON).unwrap();
        assert_eq!(position_of(&conn, 1), Some(LONDON));
        assert_eq!(place_of(&conn, 1).as_deref(), Some(london.as_str()));
        assert_eq!(
            place_of(&conn, 2).as_deref(),
            Some("Home"),
            "set by hand: kept"
        );

        // Out to sea, where nothing is named: the old town comes off.
        move_along_track(&mut conn, 3, (45.0, -30.0)).unwrap();
        assert_eq!(place_of(&conn, 3), None);
    }

    #[test]
    fn a_track_is_recorded_by_its_files_in_any_order() {
        let a = vec![
            "b.gpx".to_string(),
            "a.gpx".to_string(),
            "b.gpx".to_string(),
        ];
        let key = track_key(&a);
        assert_eq!(key, track_key(&["a.gpx".into(), "b.gpx".into()]));
        assert_eq!(track_paths(&key), vec!["a.gpx", "b.gpx"]);
    }
}
