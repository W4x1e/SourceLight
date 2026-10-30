//! Places: positions read from the files, GPX tracks, and the places and
//! trips they make.

#[cfg(feature = "libraw")]
use std::path::Path;
use tauri::{AppHandle, Emitter, State};

use super::common::{beyond_undo, count, err};
use crate::models::{Place, ScanProgress, TripCluster, TripNamed};
#[cfg(feature = "libraw")]
use crate::raw;
use crate::{db, exif_read, gpx, trips, AppState};

/// How many photos have not yet been checked for a GPS position.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn locations_status(state: State<'_, AppState>) -> Result<u32, String> {
    let conn = state.conn()?;
    Ok(db::photos_needing_gps(&conn).map_err(err)?.len() as u32)
}

/// What reading locations found.
#[derive(serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocationsOutcome {
    /// Photos checked.
    read: u32,
    /// Of those, how many had a position.
    found: u32,
    /// Photos given a place from their position.
    placed: u32,
}

/// Reads GPS positions from photos catalogued before positions were read,
/// then names places from them. A cancellable `locations` job, progress
/// on `locations-progress`.
///
/// Reads only the first 512 KB of each file — where a TIFF-based raw keeps its
/// GPS block — and asks LibRaw only when there is no EXIF there to read (CR3,
/// RAF), so a library does not pay a full rescan to learn most photos have no
/// position.
#[tauri::command]
#[specta::specta]
pub(crate) async fn read_locations(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<LocationsOutcome, String> {
    let db_path = state.db_path.clone();
    let cancel = state.jobs.start("locations");
    let emitter = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<LocationsOutcome, String> {
        let mut conn = db::connect(&db_path).map_err(err)?;
        let todo = db::photos_needing_gps(&conn).map_err(err)?;
        let total = todo.len();
        let (mut read, mut found) = (0u32, 0u32);
        let progress = |done: usize, phase: &str| {
            let _ = emitter.emit(
                "locations-progress",
                ScanProgress {
                    phase: phase.into(),
                    current: count(done),
                    total: count(total),
                    message: "Reading locations".into(),
                },
            );
        };
        for (i, (id, path)) in todo.iter().enumerate() {
            if cancel.is_cancelled() {
                break;
            }
            let mut head = Vec::with_capacity(512 * 1024);
            let opened = std::fs::File::open(path).and_then(|f| {
                use std::io::Read;
                f.take(512 * 1024).read_to_end(&mut head)
            });
            if opened.is_err() {
                continue; // Gone since the last scan; it stays unread.
            }
            let gps = exif_read::read_gps_head(head);
            // No EXIF to read (CR3, RAF): LibRaw may still find a position.
            #[cfg(feature = "libraw")]
            let gps = gps.or_else(|| {
                Some(
                    raw::libraw::LibRawDecoder::new()
                        .metadata(Path::new(path))
                        .ok()
                        .and_then(|m| m.gps),
                )
            });
            let gps = gps.flatten();
            db::set_gps(&conn, *id, gps).map_err(err)?;
            read += 1;
            found += gps.is_some() as u32;
            if i % 25 == 0 {
                progress(i + 1, "read");
            }
        }
        let placed = db::place_from_gps(&mut conn).map_err(err)? as u32;
        progress(total, "done");
        Ok(LocationsOutcome {
            read,
            found,
            placed,
        })
    })
    .await
    .map_err(err)?
}

/// The points of every GPX file given, as one sorted track.
fn read_tracks(paths: &[String]) -> Result<Vec<gpx::Point>, String> {
    let mut points = Vec::new();
    for path in paths {
        let xml = std::fs::read_to_string(path)
            .map_err(|e| err(anyhow::Error::new(e).context(format!("Couldn't read {path}"))))?;
        points.extend(gpx::parse(&xml)?);
    }
    points.sort_by_key(|p| p.0);
    Ok(points)
}

/// One day's worth of photos a track would place.
#[derive(serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
struct GpxDay {
    /// `YYYY-MM-DD`, on the camera's clock.
    date: String,
    count: u32,
}

/// What matching photos against a GPX track would do, before anything is
/// written.
#[derive(serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GpxPreview {
    /// Timed points across the files.
    points: u32,
    /// Photos that could be placed at all — a capture time and no position.
    candidates: u32,
    /// The camera's offset from UTC used: the one asked for, or the best
    /// guess when none was.
    offset_minutes: i32,
    /// Photos the track places at that offset.
    matched: u32,
    /// Those photos by day, earliest first.
    days: Vec<GpxDay>,
}

/// Photos a track places, with the position it gives each.
type TrackMatches = Vec<(i32, (f64, f64))>;

/// The photos a track places at `offset`, plus the preview.
fn match_track(
    conn: &rusqlite::Connection,
    paths: &[String],
    offset_minutes: Option<i32>,
) -> Result<(TrackMatches, GpxPreview), String> {
    let points = read_tracks(paths)?;
    let photos = db::photos_without_position(conn).map_err(err)?;
    let as_utc: Vec<(i32, &str, i64)> = photos
        .iter()
        .filter_map(|(id, c)| Some((*id, c.as_str(), gpx::capture_to_utc(c, 0)?)))
        .collect();
    let offset = offset_minutes.unwrap_or_else(|| {
        let captures: Vec<i64> = as_utc.iter().map(|p| p.2).collect();
        gpx::best_offset(&points, &captures).unwrap_or(0)
    });
    let shift = offset as i64 * 60;
    let mut matched = Vec::new();
    let mut days: std::collections::BTreeMap<String, u32> = Default::default();
    for (id, capture, t) in &as_utc {
        if let Some(pos) = gpx::position_at(&points, t - shift) {
            matched.push((*id, pos));
            *days.entry(capture[..10].to_string()).or_default() += 1;
        }
    }
    let preview = GpxPreview {
        points: count(points.len()),
        candidates: count(as_utc.len()),
        offset_minutes: offset,
        matched: count(matched.len()),
        days: days
            .into_iter()
            .map(|(date, count)| GpxDay { date, count })
            .collect(),
    };
    Ok((matched, preview))
}

/// What placing photos from GPX files would do, without writing
/// anything. With no offset, guesses the camera's from the photos
/// themselves.
#[tauri::command]
#[specta::specta]
pub(crate) async fn gpx_preview(
    state: State<'_, AppState>,
    paths: Vec<String>,
    offset_minutes: Option<i32>,
) -> Result<GpxPreview, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let conn = db::connect(&db_path).map_err(err)?;
        match_track(&conn, &paths, offset_minutes).map(|(_, p)| p)
    })
    .await
    .map_err(err)?
}

/// What placing from a track did.
#[derive(serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GpxOutcome {
    /// Photos given a position.
    matched: u32,
    /// Of those, photos given a place — ones placed by hand keep theirs.
    placed: u32,
}

/// Gives the photos a GPX track places their positions, then names their
/// places, as reading locations does.
#[tauri::command]
#[specta::specta]
pub(crate) async fn gpx_apply(
    state: State<'_, AppState>,
    paths: Vec<String>,
    offset_minutes: i32,
) -> Result<GpxOutcome, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = db::connect(&db_path).map_err(err)?;
        let (matched, _) = match_track(&conn, &paths, Some(offset_minutes))?;
        // Recorded with each photo, so a later clock fix can move it.
        let track = db::track_key(&paths);
        let tx = conn.transaction().map_err(err)?;
        for (id, pos) in &matched {
            db::set_track_gps(&tx, *id, *pos, &track, offset_minutes).map_err(err)?;
        }
        tx.commit().map_err(err)?;
        let placed = db::place_from_gps(&mut conn).map_err(err)?;
        Ok(GpxOutcome {
            matched: count(matched.len()),
            placed: count(placed),
        })
    })
    .await
    .map_err(err)?
}

/// What moving photos along their tracks did.
#[derive(serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GpxReplaced {
    /// Photos moved to where their track puts them now.
    moved: u32,
    /// Photos whose new time falls outside their track: left where they were.
    outside: u32,
    /// Photos whose track's files have gone: left where they were.
    missing: u32,
}

/// How many of these photos a GPX track placed — the ones a change to
/// their capture times has moved off it.
#[tauri::command]
#[specta::specta]
pub(crate) fn count_track_placed(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
) -> Result<u32, String> {
    let conn = state.conn()?;
    db::track_placed(&conn, &photo_ids)
        .map(|p| count(p.len()))
        .map_err(err)
}

/// Moves the photos among these that a GPX track placed to where that track
/// puts them at their capture times now, at the offset it was applied
/// with. A position from the camera or set by hand is never moved.
#[tauri::command]
#[specta::specta]
pub(crate) async fn gpx_replace(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
) -> Result<GpxReplaced, String> {
    let db_path = state.db_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = db::connect(&db_path).map_err(err)?;
        replace_along_tracks(&mut conn, &photo_ids)
    })
    .await
    .map_err(err)?
}

/// [`gpx_replace`]'s work: each track read once, and every photo it placed
/// matched again at its time now.
fn replace_along_tracks(
    conn: &mut rusqlite::Connection,
    photo_ids: &[i32],
) -> Result<GpxReplaced, String> {
    let placed = db::track_placed(conn, photo_ids).map_err(err)?;
    let mut out = GpxReplaced {
        moved: 0,
        outside: 0,
        missing: 0,
    };
    let mut tracks: std::collections::HashMap<String, Option<Vec<gpx::Point>>> =
        std::collections::HashMap::new();
    for p in &placed {
        let points = tracks
            .entry(p.track.clone())
            .or_insert_with(|| read_tracks(&db::track_paths(&p.track)).ok());
        let Some(points) = points else {
            out.missing += 1;
            continue;
        };
        let at = gpx::capture_to_utc(&p.capture_date, 0)
            .and_then(|t| gpx::position_at(points, t - i64::from(p.offset_minutes) * 60));
        match at {
            Some(pos) => {
                db::move_along_track(conn, p.id, pos).map_err(err)?;
                out.moved += 1;
            }
            None => out.outside += 1,
        }
    }
    Ok(out)
}

#[tauri::command(async)]
#[specta::specta]
pub(crate) fn list_places(state: State<'_, AppState>) -> Result<Vec<Place>, String> {
    let conn = state.conn()?;
    db::list_places(&conn).map_err(err)
}

/// Takes the place off these photos; the place itself stays.
#[tauri::command]
#[specta::specta]
pub(crate) fn clear_place(state: State<'_, AppState>, photo_ids: Vec<i32>) -> Result<u32, String> {
    let mut conn = state.conn()?;
    let done = db::clear_place(&mut conn, &photo_ids)
        .map(|n| n as u32)
        .map_err(err);
    beyond_undo(&state, "Place removed", photo_ids.len(), done)
}

#[tauri::command]
#[specta::specta]
pub(crate) fn assign_place(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
    name: String,
    lat: Option<f64>,
    lon: Option<f64>,
) -> Result<i32, String> {
    let mut conn = state.conn()?;
    let place_id = db::upsert_place(&conn, &name, lat, lon).map_err(err)?;
    let done = db::assign_place(&mut conn, &photo_ids, place_id)
        .map_err(err)
        .map(|_| place_id);
    beyond_undo(&state, "Place", photo_ids.len(), done)
}

/// Names a trip. Only its photos with no position of their own are placed;
/// see [`db::name_trip`].
#[tauri::command]
#[specta::specta]
pub(crate) fn name_trip(
    state: State<'_, AppState>,
    photo_ids: Vec<i32>,
    name: String,
) -> Result<TripNamed, String> {
    if name.trim().is_empty() {
        return Err("a place needs a name".into());
    }
    let mut conn = state.conn()?;
    db::name_trip(&mut conn, &photo_ids, &name).map_err(err)
}

/// Renames a place, or — when another place already has the name — renames
/// nothing and returns that place's id so the user can be asked to merge.
#[tauri::command]
#[specta::specta]
pub(crate) fn rename_place(
    state: State<'_, AppState>,
    place_id: i32,
    name: String,
) -> Result<Option<i32>, String> {
    let conn = state.conn()?;
    db::rename_place(&conn, place_id, &name).map_err(err)
}

/// Folds places into one; see [`db::merge_places`].
#[tauri::command]
#[specta::specta]
pub(crate) fn merge_places(
    state: State<'_, AppState>,
    from_ids: Vec<i32>,
    into_id: i32,
) -> Result<(), String> {
    let mut conn = state.conn()?;
    db::merge_places(&mut conn, &from_ids, into_id).map_err(err)
}

/// A few thumbnails from a place, for the map's hover card.
#[tauri::command(async)]
#[specta::specta]
pub(crate) fn place_thumbs(
    state: State<'_, AppState>,
    place_id: i32,
) -> Result<Vec<String>, String> {
    let conn = state.conn()?;
    db::place_thumbs(&conn, place_id, 4).map_err(err)
}

/// Deletes a place; its photos are kept.
#[tauri::command]
#[specta::specta]
pub(crate) fn delete_place(state: State<'_, AppState>, place_id: i32) -> Result<(), String> {
    let conn = state.conn()?;
    db::delete_place(&conn, place_id).map_err(err)
}

/// Pins a place to the map where the user clicked, or moves its pin.
#[tauri::command]
#[specta::specta]
pub(crate) fn set_place_position(
    state: State<'_, AppState>,
    place_id: i32,
    lat: f64,
    lon: f64,
) -> Result<(), String> {
    let conn = state.conn()?;
    db::set_place_position(&conn, place_id, lat, lon).map_err(err)
}

#[tauri::command(async)]
#[specta::specta]
pub(crate) fn get_trip_clusters(
    state: State<'_, AppState>,
    gap_hours: Option<i32>,
) -> Result<Vec<TripCluster>, String> {
    let conn = state.conn()?;
    trips::cluster(&conn, i64::from(gap_hours.unwrap_or(12))).map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models;

    /// A GPX track places the photos taken while it was recording, at the
    /// camera offset it guesses, and says so by day before writing.
    #[test]
    fn a_gpx_track_places_photos_taken_along_it() {
        let dir = crate::test_support::Scratch::new("gpx");
        let track = dir.join("walk.gpx");
        std::fs::write(
            &track,
            r#"<gpx><trk><trkseg>
                <trkpt lat="55.9500" lon="-3.2000"><time>2026-05-03T13:00:00Z</time></trkpt>
                <trkpt lat="55.9533" lon="-3.1883"><time>2026-05-03T13:05:00Z</time></trkpt>
            </trkseg></trk></gpx>"#,
        )
        .unwrap();
        let paths = vec![track.to_string_lossy().into_owned()];

        let mut conn = db::open_memory().unwrap();
        // On a camera set to BST: 14:02 local is 13:02 UTC. The third photo
        // was taken the next day, off the track.
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at, capture_date)
                 VALUES (1, '/a.arw', 'a.arw', 'h1', 'x', '2026-05-03 14:02:00'),
                        (2, '/b.arw', 'b.arw', 'h2', 'x', '2026-05-03 14:04:10'),
                        (3, '/c.arw', 'c.arw', 'h3', 'x', '2026-05-04 09:00:00');",
        )
        .unwrap();

        let (matched, preview) = match_track(&conn, &paths, None).unwrap();
        assert_eq!(preview.offset_minutes, 60);
        assert_eq!(
            (preview.points, preview.candidates, preview.matched),
            (2, 3, 2)
        );
        assert_eq!(preview.days.len(), 1);
        assert_eq!(preview.days[0].date, "2026-05-03");
        let (_, nothing) = match_track(&conn, &paths, Some(0)).unwrap();
        assert_eq!(nothing.matched, 0, "on the wrong zone nothing lines up");

        for (id, pos) in &matched {
            db::set_gps(&conn, *id, Some(*pos)).unwrap();
        }
        assert_eq!(db::place_from_gps(&mut conn).unwrap(), 2);
        assert_eq!(db::photos_without_position(&conn).unwrap().len(), 1);
    }

    /// Photos a track placed move along it when their times change:
    /// to the right point, left alone when the new time is off the track or
    /// the track has gone, and a camera's own position never moves.
    #[test]
    fn a_clock_fix_moves_track_placed_photos_along_their_track() {
        let dir = crate::test_support::Scratch::new("gpx-replace");
        let track = dir.join("walk.gpx");
        std::fs::write(
            &track,
            r#"<gpx><trk><trkseg>
                <trkpt lat="55.9500" lon="-3.2000"><time>2026-05-03T13:00:00Z</time></trkpt>
                <trkpt lat="55.9533" lon="-3.1883"><time>2026-05-03T13:05:00Z</time></trkpt>
            </trkseg></trk></gpx>"#,
        )
        .unwrap();
        let paths = vec![track.to_string_lossy().into_owned()];

        let mut conn = db::open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at, capture_date)
                 VALUES (1, '/a.arw', 'a.arw', 'h1', 'x', '2026-05-03 14:00:00'),
                        (2, '/b.arw', 'b.arw', 'h2', 'x', '2026-05-03 14:01:00'),
                        (3, '/c.arw', 'c.arw', 'h3', 'x', '2026-05-03 14:02:00');",
        )
        .unwrap();
        let copy = db::create_variant(&mut conn, 1, None).unwrap();
        // Photo 3 has the camera's own position; the track places 1 and 2.
        db::set_gps(&conn, 3, Some((51.5, -0.12))).unwrap();
        let (matched, _) = match_track(&conn, &paths, Some(60)).unwrap();
        assert_eq!(matched.len(), 2);
        let key = db::track_key(&paths);
        for (id, pos) in &matched {
            db::set_track_gps(&conn, *id, *pos, &key, 60).unwrap();
        }
        let at = |conn: &rusqlite::Connection, id: i32| -> (f64, f64) {
            conn.query_row("SELECT lat, lon FROM photos WHERE id = ?1", [id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap()
        };
        assert_eq!(at(&conn, 1), (55.95, -3.2));

        // The camera was five minutes slow: now 14:05, the track's last point.
        let everyone = [1, 2, 3];
        db::set_capture_time(
            &mut conn,
            &everyone,
            &models::CaptureTimeChange::Shift { seconds: 300 },
        )
        .unwrap();
        // And photo 2 is two hours past the end of the walk.
        let before = at(&conn, 2);
        db::set_capture_time(
            &mut conn,
            &[2],
            &models::CaptureTimeChange::Shift { seconds: 7200 },
        )
        .unwrap();
        assert_eq!(db::track_placed(&conn, &everyone).unwrap().len(), 2);
        let out = replace_along_tracks(&mut conn, &[copy, 2, 3]).unwrap();
        assert_eq!((out.moved, out.outside, out.missing), (1, 1, 0));
        assert_eq!(at(&conn, 1), (55.9533, -3.1883));
        assert_eq!(at(&conn, copy), (55.9533, -3.1883), "the copy moves too");
        assert_eq!(at(&conn, 2), before, "off the track: left alone");
        assert_eq!(at(&conn, 3), (51.5, -0.12), "the camera's own never moves");

        // A track that has gone moves nothing, and says so.
        std::fs::remove_file(&track).unwrap();
        let out = replace_along_tracks(&mut conn, &everyone).unwrap();
        assert_eq!((out.moved, out.outside, out.missing), (0, 0, 2));
        assert_eq!(at(&conn, 1), (55.9533, -3.1883));
    }
}
