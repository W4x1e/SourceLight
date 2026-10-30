//! Shoot tracks on the phone (docs/MOBILE.md, "Shoot track"): where a
//! recording is written, and turning a finished one into the GPX the desktop
//! places photos from.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sl_link::protocol::track::{parse_recording, to_gpx};

/// Ask for a position this often. The desktop bridges up to ten minutes
/// between two points and matches within five of one (`gpx.rs`), so ten
/// seconds is generous for placing photos and easy on the battery.
pub const INTERVAL_MS: u64 = 10_000;

fn folder(dir: &Path) -> PathBuf {
    dir.join("tracks")
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Where a track file noted down earlier is now: in this app's tracks folder,
/// by its name. iOS moves an app's data folder when the app is updated or
/// reinstalled, so a whole path noted before then points nowhere; a track
/// read from it was never sent.
pub fn here(dir: &Path, noted: &str) -> PathBuf {
    folder(dir).join(Path::new(noted).file_name().unwrap_or_default())
}

/// A new file for the recording service to append fixes to.
pub fn new_recording(dir: &Path) -> PathBuf {
    let folder = folder(dir);
    let _ = std::fs::create_dir_all(&folder);
    folder.join(format!("recording-{}.csv", now_ms()))
}

/// A finished track, written as GPX.
#[derive(Debug, Clone, PartialEq)]
pub struct Finished {
    pub gpx: PathBuf,
    pub started_ms: i64,
    pub ended_ms: i64,
    pub points: u32,
}

/// Turns a recording into a GPX file beside it and removes the recording.
/// `None` when it holds no fixes (stopped before the first one): then nothing
/// is kept.
pub fn finish(recording: &Path, name: &str) -> std::io::Result<Option<Finished>> {
    let csv = match std::fs::read_to_string(recording) {
        Ok(csv) => csv,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let points = parse_recording(&csv);
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        let _ = std::fs::remove_file(recording);
        return Ok(None);
    };
    let gpx = recording.with_file_name(format!("track-{}.gpx", first.time_ms));
    std::fs::write(&gpx, to_gpx(&points, name))?;
    std::fs::remove_file(recording)?;
    Ok(Some(Finished {
        gpx,
        started_ms: first.time_ms,
        ended_ms: last.time_ms,
        points: points.len().min(u32::MAX as usize) as u32,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sl-track-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_recording_becomes_gpx() {
        let d = dir("finish");
        let rec = new_recording(&d);
        std::fs::write(
            &rec,
            "1791019815000,46.5,11.9,4.0,1200.5\n1791019825000,46.51,11.91,4.0,\n",
        )
        .unwrap();
        let done = finish(&rec, "Dolomites").unwrap().unwrap();
        assert_eq!(
            (done.started_ms, done.ended_ms, done.points),
            (1_791_019_815_000, 1_791_019_825_000, 2)
        );
        assert!(
            !rec.exists(),
            "the recording is removed once written as GPX"
        );
        let gpx = std::fs::read_to_string(&done.gpx).unwrap();
        assert!(gpx.contains("<name>Dolomites</name>"));
        assert_eq!(gpx.matches("<trkpt").count(), 2);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn an_empty_recording_leaves_nothing() {
        let d = dir("empty");
        let rec = new_recording(&d);
        std::fs::write(&rec, "").unwrap();
        assert_eq!(finish(&rec, "x").unwrap(), None);
        assert!(!rec.exists());
        assert_eq!(finish(&d.join("missing.csv"), "x").unwrap(), None);
        let _ = std::fs::remove_dir_all(d);
    }

    /// A path noted before iOS moved the app's folder (an update, a
    /// reinstall) finds the file in today's tracks folder, as a bare name does.
    #[test]
    fn a_track_is_found_after_the_app_folder_moves() {
        let d = dir("moved");
        let today = d.join("tracks").join("track-1.gpx");
        let before =
            "/private/var/mobile/Containers/Data/Application/OLD/Library/tracks/track-1.gpx";
        assert_eq!(here(&d, before), today);
        assert_eq!(here(&d, "track-1.gpx"), today);
        let _ = std::fs::remove_dir_all(d);
    }
}
