//! Correcting capture times, and taking the times sidecars offer.

use super::*;

const CAPTURE_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// A capture time as the catalogue keeps it, or as a date-and-time field sends
/// it (`2025-06-14T10:30`, with or without seconds).
pub(super) fn parse_capture(s: &str) -> Option<chrono::NaiveDateTime> {
    let s = s.trim();
    chrono::NaiveDateTime::parse_from_str(s.get(..19).unwrap_or(s), CAPTURE_FORMAT)
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M"))
        .ok()
}

/// Every row that is one of these photos' files: the photos themselves and
/// all their virtual copies, since a correction to when a file was shot is
/// true of every copy of it.
fn same_files(conn: &Connection, ids: &[i32]) -> Result<Vec<(i32, String, Option<String>)>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT id, path, capture_date FROM photos
          WHERE path IN (SELECT path FROM photos WHERE id IN ({}))",
        placeholders(ids.len())
    ))?;
    let rows = stmt.query_map(rusqlite::params_from_iter(ids), |r| {
        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Changes these photos' capture times — see [`CaptureTimeChange`].
/// The file's own time is kept beside the new one the first time, so
/// [`revert_capture_time`] can go back to it, and a rescan keeps the new one.
/// Photos are counted by file: a virtual copy moves with its original.
pub fn set_capture_time(
    conn: &mut Connection,
    ids: &[i32],
    change: &CaptureTimeChange,
) -> Result<CaptureTimeOutcome> {
    let tx = conn.transaction()?;
    let out = set_capture_time_in(&tx, ids, change)?;
    tx.commit()?;
    Ok(out)
}

/// [`set_capture_time`] inside the caller's transaction.
fn set_capture_time_in(
    tx: &Connection,
    ids: &[i32],
    change: &CaptureTimeChange,
) -> Result<CaptureTimeOutcome> {
    let rows = same_files(tx, ids)?;
    let when = |s: &Option<String>| s.as_deref().and_then(parse_capture);
    let seconds = match change {
        CaptureTimeChange::Shift { seconds } => Some(i64::from(*seconds)),
        CaptureTimeChange::FirstTo { time } => {
            let target = parse_capture(time)
                .ok_or_else(|| anyhow::anyhow!("“{time}” isn't a date and time"))?;
            rows.iter()
                .filter_map(|(_, _, d)| when(d))
                .min()
                .map(|first| (target - first).num_seconds())
        }
        CaptureTimeChange::AllTo { .. } => None,
    };
    let all_to = match change {
        CaptureTimeChange::AllTo { time } => Some(
            parse_capture(time).ok_or_else(|| anyhow::anyhow!("“{time}” isn't a date and time"))?,
        ),
        _ => None,
    };
    let mut changed = std::collections::HashSet::new();
    let mut undated = std::collections::HashSet::new();
    {
        let mut set = tx.prepare(
            "UPDATE photos SET
                 original_capture_date = CASE WHEN capture_edited = 1 THEN original_capture_date
                                              ELSE capture_date END,
                 capture_edited = 1,
                 capture_date = ?2
              WHERE id = ?1",
        )?;
        for (id, path, date) in &rows {
            let new = match (all_to, seconds, when(date)) {
                (Some(t), _, _) => t,
                (None, Some(s), Some(d)) => d + chrono::Duration::seconds(s),
                _ => {
                    undated.insert(path.clone());
                    continue;
                }
            };
            set.execute(params![id, new.format(CAPTURE_FORMAT).to_string()])?;
            changed.insert(path.clone());
        }
    }
    Ok(CaptureTimeOutcome {
        changed: changed.len() as u32,
        undated: undated.len() as u32,
    })
}

/// Puts these photos' capture times back to their files' own; how
/// many photos that changed, counted by file.
pub fn revert_capture_time(conn: &Connection, ids: &[i32]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let files: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(DISTINCT path) FROM photos WHERE capture_edited = 1
                AND path IN (SELECT path FROM photos WHERE id IN ({}))",
            placeholders(ids.len())
        ),
        rusqlite::params_from_iter(ids),
        |r| r.get(0),
    )?;
    conn.execute(
        &format!(
            "UPDATE photos SET capture_date = original_capture_date, original_capture_date = NULL,
                               capture_edited = 0
              WHERE capture_edited = 1
                AND path IN (SELECT path FROM photos WHERE id IN ({}))",
            placeholders(ids.len())
        ),
        rusqlite::params_from_iter(ids),
    )?;
    Ok(files as usize)
}

/// The capture time a sidecar gives this photo, when it is one to offer:
/// a whole time someone else wrote, that is neither the catalogue's
/// time nor the file's own. The camera's time, which other editors copy into
/// every sidecar, is no correction and is not offered.
pub fn sidecar_time_offer(
    conn: &Connection,
    photo: &Photo,
    sidecar: &crate::xmp::Sidecar,
) -> Result<Option<SidecarTime>> {
    let Some(theirs) = sidecar.meta.captured.filter(|_| !sidecar.captured_is_ours) else {
        return Ok(None);
    };
    let original: Option<String> = conn
        .query_row(
            "SELECT original_capture_date FROM photos WHERE id = ?1",
            params![photo.id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let known = [photo.capture_date.as_deref(), original.as_deref()];
    if known
        .iter()
        .flatten()
        .any(|d| parse_capture(d) == Some(theirs))
    {
        return Ok(None);
    }
    Ok(Some(SidecarTime {
        photo_id: photo.id,
        time: theirs.format(CAPTURE_FORMAT).to_string(),
    }))
}

/// Takes the capture times sidecars offered, each for its own photo and
/// that photo's copies. Each is a correction like any other: the file's own
/// time is kept beside it, and reverting goes back to that.
pub fn use_sidecar_times(
    conn: &mut Connection,
    times: &[SidecarTime],
) -> Result<CaptureTimeOutcome> {
    // One transaction for them all: one each made accepting ten
    // thousand offered times ten thousand commits.
    let tx = conn.transaction()?;
    let mut out = CaptureTimeOutcome::default();
    for t in times {
        let change = CaptureTimeChange::AllTo {
            time: t.time.clone(),
        };
        let one = set_capture_time_in(&tx, &[t.photo_id], &change)?;
        out.changed += one.changed;
        out.undated += one.undated;
    }
    tx.commit()?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::testing::*;

    fn time_of(conn: &Connection, id: i32) -> Option<String> {
        photo_of(conn, id).capture_date
    }

    fn scan_again(conn: &Connection, path: &str, date: Option<&str>) {
        upsert_photo(
            conn,
            &crate::scan::IngestedPhoto {
                path: path.into(),
                filename: path.into(),
                file_hash: path.into(),
                capture_date: date.map(String::from),
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
                gps: None,
                stamp: None,
            },
        )
        .unwrap();
    }

    #[test]
    fn a_shift_moves_every_dated_photo_and_keeps_the_files_time() {
        let mut conn = open_memory().unwrap();
        dated(&conn, 1, "a", Some("2025-06-14 10:00:00"));
        dated(&conn, 2, "b", Some("2025-01-31 23:30:00"));
        dated(&conn, 3, "c", None);
        let out = set_capture_time(
            &mut conn,
            &[1, 2, 3],
            &CaptureTimeChange::Shift { seconds: 3600 },
        )
        .unwrap();
        assert_eq!(
            out,
            CaptureTimeOutcome {
                changed: 2,
                undated: 1
            }
        );
        assert_eq!(time_of(&conn, 1).as_deref(), Some("2025-06-14 11:00:00"));
        // Across the end of a month.
        assert_eq!(time_of(&conn, 2).as_deref(), Some("2025-02-01 00:30:00"));
        assert_eq!(time_of(&conn, 3), None);
        assert!(photo_of(&conn, 1).capture_edited && !photo_of(&conn, 3).capture_edited);
        // Twice, and back: to the file's own time, not the first shift's.
        set_capture_time(
            &mut conn,
            &[1],
            &CaptureTimeChange::Shift { seconds: -7200 },
        )
        .unwrap();
        assert_eq!(time_of(&conn, 1).as_deref(), Some("2025-06-14 09:00:00"));
        assert_eq!(revert_capture_time(&conn, &[1, 3]).unwrap(), 1);
        assert_eq!(time_of(&conn, 1).as_deref(), Some("2025-06-14 10:00:00"));
        assert!(!photo_of(&conn, 1).capture_edited);
    }

    /// Offered times are taken together, each to its own photo, and a
    /// bad one leaves none of them applied.
    #[test]
    fn sidecar_times_are_taken_together() {
        let mut conn = open_memory().unwrap();
        dated(&conn, 1, "a", Some("2025-06-14 10:00:00"));
        dated(&conn, 2, "b", Some("2025-06-14 11:00:00"));
        let offered = |id: i32, time: &str| crate::models::SidecarTime {
            photo_id: id,
            time: time.into(),
        };
        let out = use_sidecar_times(
            &mut conn,
            &[
                offered(1, "2025-06-14 09:00:00"),
                offered(2, "2025-06-14 09:30:00"),
            ],
        )
        .unwrap();
        assert_eq!(out.changed, 2);
        assert_eq!(time_of(&conn, 1).as_deref(), Some("2025-06-14 09:00:00"));
        assert_eq!(time_of(&conn, 2).as_deref(), Some("2025-06-14 09:30:00"));

        assert!(use_sidecar_times(
            &mut conn,
            &[offered(1, "2025-06-14 08:00:00"), offered(2, "soon")],
        )
        .is_err());
        assert_eq!(
            time_of(&conn, 1).as_deref(),
            Some("2025-06-14 09:00:00"),
            "the good one was rolled back with the bad"
        );
    }

    #[test]
    fn setting_the_first_keeps_the_rest_spaced_as_they_were() {
        let mut conn = open_memory().unwrap();
        dated(&conn, 1, "a", Some("2025-06-14 10:00:00"));
        dated(&conn, 2, "b", Some("2025-06-14 10:05:30"));
        let change = CaptureTimeChange::FirstTo {
            time: "2025-06-15T08:00".into(),
        };
        set_capture_time(&mut conn, &[2, 1], &change).unwrap();
        assert_eq!(time_of(&conn, 1).as_deref(), Some("2025-06-15 08:00:00"));
        assert_eq!(time_of(&conn, 2).as_deref(), Some("2025-06-15 08:05:30"));
        let bad = CaptureTimeChange::FirstTo {
            time: "soon".into(),
        };
        assert!(set_capture_time(&mut conn, &[1], &bad).is_err());
    }

    /// Scans have no time of their own: "set all" gives them one, and going
    /// back takes it away again.
    #[test]
    fn setting_all_dates_a_scan_and_revert_undates_it() {
        let mut conn = open_memory().unwrap();
        dated(&conn, 1, "scan1", None);
        dated(&conn, 2, "scan2", None);
        let change = CaptureTimeChange::AllTo {
            time: "1987-07-04 12:00:00".into(),
        };
        assert_eq!(
            set_capture_time(&mut conn, &[1, 2], &change)
                .unwrap()
                .changed,
            2
        );
        assert_eq!(time_of(&conn, 2).as_deref(), Some("1987-07-04 12:00:00"));
        // A rescan of a file with no time keeps the one given it.
        scan_again(&conn, "scan1", None);
        assert_eq!(time_of(&conn, 1).as_deref(), Some("1987-07-04 12:00:00"));
        revert_capture_time(&conn, &[1]).unwrap();
        assert_eq!(time_of(&conn, 1), None);
    }

    #[test]
    fn a_rescan_keeps_a_corrected_time_and_notes_the_files() {
        let mut conn = open_memory().unwrap();
        scan_again(&conn, "a.arw", Some("2025-06-14 10:00:00"));
        let id: i32 = conn
            .query_row("SELECT id FROM photos", [], |r| r.get(0))
            .unwrap();
        set_capture_time(
            &mut conn,
            &[id],
            &CaptureTimeChange::Shift { seconds: -3600 },
        )
        .unwrap();
        // The file was edited elsewhere, and now says 10:00:05.
        scan_again(&conn, "a.arw", Some("2025-06-14 10:00:05"));
        assert_eq!(time_of(&conn, id).as_deref(), Some("2025-06-14 09:00:00"));
        revert_capture_time(&conn, &[id]).unwrap();
        assert_eq!(time_of(&conn, id).as_deref(), Some("2025-06-14 10:00:05"));
        // Uncorrected, a rescan takes the file's time as it always has.
        scan_again(&conn, "a.arw", Some("2025-06-14 10:00:09"));
        assert_eq!(time_of(&conn, id).as_deref(), Some("2025-06-14 10:00:09"));
    }

    #[test]
    fn a_virtual_copy_moves_with_its_original() {
        let mut conn = open_memory().unwrap();
        dated(&conn, 1, "a", Some("2025-06-14 10:00:00"));
        let copy = create_variant(&mut conn, 1, None).unwrap();
        let out = set_capture_time(
            &mut conn,
            &[copy],
            &CaptureTimeChange::Shift { seconds: 60 },
        )
        .unwrap();
        assert_eq!(out.changed, 1, "one file");
        assert_eq!(time_of(&conn, 1).as_deref(), Some("2025-06-14 10:01:00"));
        assert_eq!(time_of(&conn, copy).as_deref(), Some("2025-06-14 10:01:00"));
        // A copy made after the correction carries it, and reverts with it.
        let later = create_variant(&mut conn, 1, None).unwrap();
        assert!(photo_of(&conn, later).capture_edited);
        revert_capture_time(&conn, &[1]).unwrap();
        assert_eq!(
            time_of(&conn, later).as_deref(),
            Some("2025-06-14 10:00:00")
        );
    }

    // --- Corrected capture times in sidecars --------------------------

    fn sidecar_dated(time: &str, ours: bool) -> crate::xmp::Sidecar {
        crate::xmp::Sidecar {
            meta: crate::xmp::XmpMeta {
                captured: parse_capture(time),
                ..Default::default()
            },
            description_is_ours: false,
            text_from_elsewhere: Default::default(),
            captured_is_ours: ours,
            keywords_ours: Vec::new(),
        }
    }

    #[test]
    fn only_a_corrected_photo_gives_its_sidecar_a_time() {
        let mut conn = open_memory().unwrap();
        dated(&conn, 1, "a.arw", Some("2025-06-14 10:00:00"));
        dated(&conn, 2, "b.arw", Some("2025-06-14 10:05:00"));
        set_capture_time(&mut conn, &[1], &CaptureTimeChange::Shift { seconds: 3600 }).unwrap();

        let meta = xmp_meta(&conn, &photo_of(&conn, 1)).unwrap();
        assert_eq!(meta.captured, parse_capture("2025-06-14 11:00:00"));
        assert_eq!(meta.captured_original, parse_capture("2025-06-14 10:00:00"));
        let plain = xmp_meta(&conn, &photo_of(&conn, 2)).unwrap();
        assert_eq!((plain.captured, plain.captured_original), (None, None));

        revert_capture_time(&conn, &[1]).unwrap();
        assert_eq!(xmp_meta(&conn, &photo_of(&conn, 1)).unwrap().captured, None);
    }

    #[test]
    fn only_a_time_set_elsewhere_is_offered() {
        let mut conn = open_memory().unwrap();
        dated(&conn, 1, "a.arw", Some("2025-06-14 10:00:00"));
        let offer = |conn: &Connection, s: &crate::xmp::Sidecar| {
            sidecar_time_offer(conn, &photo_of(conn, 1), s).unwrap()
        };

        // Another editor's clock fix: offered, as the catalogue keeps times.
        assert_eq!(
            offer(&conn, &sidecar_dated("2025-06-14T09:30:00", false)),
            Some(SidecarTime {
                photo_id: 1,
                time: "2025-06-14 09:30:00".into()
            })
        );
        // The time it already has, and one this app wrote: nothing to offer.
        assert_eq!(
            offer(&conn, &sidecar_dated("2025-06-14T10:00:00", false)),
            None
        );
        assert_eq!(
            offer(&conn, &sidecar_dated("2025-06-14T09:30:00", true)),
            None
        );
        assert_eq!(
            offer(
                &conn,
                &crate::xmp::Sidecar {
                    meta: Default::default(),
                    description_is_ours: false,
                    text_from_elsewhere: Default::default(),
                    captured_is_ours: false,
                    keywords_ours: Vec::new(),
                }
            ),
            None
        );

        // Corrected here, with the camera's time still in the other editor's sidecar:
        // that's no correction, so nothing is offered.
        set_capture_time(&mut conn, &[1], &CaptureTimeChange::Shift { seconds: 3600 }).unwrap();
        assert_eq!(
            offer(&conn, &sidecar_dated("2025-06-14T10:00:00", false)),
            None
        );
    }

    #[test]
    fn a_sidecar_time_taken_is_a_correction_like_any_other() {
        let mut conn = open_memory().unwrap();
        dated(&conn, 1, "a.arw", Some("2025-06-14 10:00:00"));
        dated(&conn, 2, "b.arw", Some("2025-06-14 10:05:00"));
        let copy = create_variant(&mut conn, 1, None).unwrap();

        let out = use_sidecar_times(
            &mut conn,
            &[
                SidecarTime {
                    photo_id: 1,
                    time: "2025-06-14 09:30:00".into(),
                },
                SidecarTime {
                    photo_id: 2,
                    time: "2025-06-14 09:35:00".into(),
                },
            ],
        )
        .unwrap();
        assert_eq!(
            out,
            CaptureTimeOutcome {
                changed: 2,
                undated: 0
            }
        );
        assert_eq!(time_of(&conn, 1).as_deref(), Some("2025-06-14 09:30:00"));
        assert_eq!(time_of(&conn, copy).as_deref(), Some("2025-06-14 09:30:00"));
        assert_eq!(time_of(&conn, 2).as_deref(), Some("2025-06-14 09:35:00"));

        revert_capture_time(&conn, &[1, 2]).unwrap();
        assert_eq!(time_of(&conn, 1).as_deref(), Some("2025-06-14 10:00:00"));
        assert_eq!(time_of(&conn, 2).as_deref(), Some("2025-06-14 10:05:00"));
    }
}
