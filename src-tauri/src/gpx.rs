//! Placing photos from a GPX track.
//!
//! A camera with no GPS — the a6400 — still records when each frame was
//! taken, and a phone or watch in a pocket records where it was, and when. A
//! GPX track pairs the two: each photo's capture time, corrected for the
//! camera's clock, is looked up on the track and given the position there.
//!
//! The correction matters more than anything else here. EXIF capture times
//! are the camera's local clock with no zone; GPX times are UTC. So the user
//! gives the camera's offset from UTC (BST is +60), and `best_offset` guesses
//! it for them by trying every quarter hour and keeping the one that lands
//! most photos on the track — a wrong zone matches nothing, the right one
//! matches a whole day.
//!
//! A photo is given a position only when the track vouches for it: between
//! two points recorded close together (interpolated), or within a few minutes
//! of one. A gap in the track — the phone died, the recording was paused —
//! is not bridged, because a straight line across it is a guess.

use chrono::{DateTime, NaiveDateTime};
use quick_xml::events::Event;
use quick_xml::Reader;

/// Two points this close are treated as a line the photo sits on.
const MAX_GAP_S: i64 = 10 * 60;
/// Otherwise a photo this close to one point takes that point's position.
const TOLERANCE_S: i64 = 5 * 60;

/// One recorded position: UTC seconds, latitude, longitude.
pub type Point = (i64, f64, f64);

fn local_name(qname: &[u8]) -> &[u8] {
    match qname.iter().position(|&b| b == b':') {
        Some(i) => &qname[i + 1..],
        None => qname,
    }
}

/// A GPX timestamp to UTC seconds. GPX says ISO 8601 in UTC; some loggers
/// leave off the `Z`, which is read as UTC too.
fn parse_time(s: &str) -> Option<i64> {
    let s = s.trim();
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.timestamp())
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(s.trim_end_matches('Z'), "%Y-%m-%dT%H:%M:%S%.f")
                .ok()
                .map(|t| t.and_utc().timestamp())
        })
}

/// The timed points of a GPX file — track points, and route or way points
/// that carry a time — sorted by time. Points without a time are useless for
/// matching and are skipped.
pub fn parse(xml: &str) -> Result<Vec<Point>, String> {
    let mut reader = Reader::from_str(xml);
    let mut points = Vec::new();
    // The point being read: its position, and its time once seen.
    let mut current: Option<(f64, f64)> = None;
    let mut time: Option<i64> = None;
    let mut in_time = false;
    let mut text = String::new();
    loop {
        match reader
            .read_event()
            .map_err(|e| format!("Not a readable GPX file: {e}"))?
        {
            Event::Eof => break,
            Event::Start(e) => match local_name(e.name().as_ref()) {
                b"trkpt" | b"rtept" | b"wpt" => {
                    let (mut lat, mut lon) = (None, None);
                    for a in e.attributes().flatten() {
                        let v = String::from_utf8_lossy(&a.value);
                        match local_name(a.key.as_ref()) {
                            b"lat" => lat = v.trim().parse::<f64>().ok(),
                            b"lon" => lon = v.trim().parse::<f64>().ok(),
                            _ => {}
                        }
                    }
                    current = lat
                        .zip(lon)
                        .filter(|&(la, lo)| crate::geo::plausible(la, lo));
                    time = None;
                }
                b"time" if current.is_some() => {
                    in_time = true;
                    text.clear();
                }
                _ => {}
            },
            Event::Text(e) if in_time => {
                if let Ok(t) = e.decode() {
                    text.push_str(&t);
                }
            }
            Event::End(e) => match local_name(e.name().as_ref()) {
                b"time" if in_time => {
                    in_time = false;
                    time = parse_time(&text);
                }
                b"trkpt" | b"rtept" | b"wpt" => {
                    if let (Some((lat, lon)), Some(t)) = (current.take(), time.take()) {
                        points.push((t, lat, lon));
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    points.sort_by_key(|p| p.0);
    Ok(points)
}

/// Where the track says it was at `t` (UTC seconds), if it can vouch for it.
pub fn position_at(points: &[Point], t: i64) -> Option<(f64, f64)> {
    let i = points.partition_point(|p| p.0 < t);
    let after = points.get(i);
    let before = i.checked_sub(1).and_then(|j| points.get(j));
    if let Some(a) = after.filter(|a| a.0 == t) {
        return Some((a.1, a.2));
    }
    if let (Some(b), Some(a)) = (before, after) {
        if a.0 - b.0 <= MAX_GAP_S {
            let f = (t - b.0) as f64 / (a.0 - b.0) as f64;
            return Some((b.1 + (a.1 - b.1) * f, b.2 + (a.2 - b.2) * f));
        }
    }
    [before, after]
        .into_iter()
        .flatten()
        .filter(|p| (p.0 - t).abs() <= TOLERANCE_S)
        .min_by_key(|p| (p.0 - t).abs())
        .map(|p| (p.1, p.2))
}

/// A capture date as the catalogue stores it ("2026-05-03 14:22:07", the
/// camera's own clock) to UTC seconds, given the camera's offset from UTC.
pub fn capture_to_utc(capture: &str, offset_minutes: i32) -> Option<i64> {
    let t = NaiveDateTime::parse_from_str(capture.get(..19)?, "%Y-%m-%d %H:%M:%S").ok()?;
    Some(t.and_utc().timestamp() - offset_minutes as i64 * 60)
}

/// How many of `captures` land on the track at this offset.
pub fn count_matches(points: &[Point], captures: &[i64], offset_minutes: i32) -> usize {
    let shift = offset_minutes as i64 * 60;
    captures
        .iter()
        .filter(|&&c| position_at(points, c - shift).is_some())
        .count()
}

/// The camera offset, in whole quarter hours from −12 h to +14 h, that puts
/// the most photos on the track. `captures` are capture dates read as if the
/// camera were on UTC. Ties go to the smallest correction, so a camera set
/// to UTC stays at 0. None when no offset matches anything.
pub fn best_offset(points: &[Point], captures: &[i64]) -> Option<i32> {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return None;
    };
    // Only photos within a day of the track can match at any offset.
    let near: Vec<i64> = captures
        .iter()
        .copied()
        .filter(|&c| c >= first.0 - 13 * 3600 && c <= last.0 + 15 * 3600)
        .collect();
    (-48..=56)
        .map(|q| q * 15)
        .map(|m| (count_matches(points, &near, m), m))
        .filter(|&(n, _)| n > 0)
        .max_by_key(|&(n, m)| (n, -m.abs()))
        .map(|(_, m)| m)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACK: &str = r#"<?xml version="1.0"?>
<gpx version="1.1" creator="test" xmlns="http://www.topografix.com/GPX/1/1">
  <trk><trkseg>
    <trkpt lat="55.9500" lon="-3.2000"><ele>60</ele><time>2026-05-03T13:00:00Z</time></trkpt>
    <trkpt lat="55.9600" lon="-3.1800"><time>2026-05-03T13:05:00Z</time></trkpt>
    <trkpt lat="56.0000" lon="-3.0000"><time>2026-05-03T15:00:00Z</time></trkpt>
    <trkpt lat="1" lon="2"></trkpt>
  </trkseg></trk>
</gpx>"#;

    #[test]
    fn reads_timed_points_in_order() {
        let p = parse(TRACK).unwrap();
        assert_eq!(p.len(), 3, "the untimed point is skipped");
        assert!(p.windows(2).all(|w| w[0].0 <= w[1].0));
        assert!((p[0].1 - 55.95).abs() < 1e-9 && (p[0].2 + 3.2).abs() < 1e-9);
    }

    #[test]
    fn interpolates_short_gaps_but_not_long_ones() {
        let p = parse(TRACK).unwrap();
        let t0 = p[0].0;
        // Halfway along a five-minute leg.
        let (lat, lon) = position_at(&p, t0 + 150).unwrap();
        assert!((lat - 55.955).abs() < 1e-6 && (lon + 3.19).abs() < 1e-6);
        // Three minutes after the last point before a two-hour gap: nearest.
        assert_eq!(position_at(&p, t0 + 480), Some((55.96, -3.18)));
        // An hour into the gap: nothing vouches for it.
        assert_eq!(position_at(&p, t0 + 3600), None);
        // Before the track starts, beyond tolerance.
        assert_eq!(position_at(&p, t0 - 600), None);
    }

    #[test]
    fn finds_the_camera_clock_offset() {
        let p = parse(TRACK).unwrap();
        // The camera was on BST: 14:02 on its clock was 13:02 UTC.
        let captures: Vec<i64> = [
            "2026-05-03 14:02:00",
            "2026-05-03 14:03:30",
            "2026-05-03 15:59:00",
        ]
        .iter()
        .map(|c| capture_to_utc(c, 0).unwrap())
        .collect();
        assert_eq!(best_offset(&p, &captures), Some(60));
        assert_eq!(count_matches(&p, &captures, 60), 3);
        assert_eq!(count_matches(&p, &captures, 0), 0);
        assert_eq!(best_offset(&p, &[]), None);
    }

    #[test]
    fn junk_yields_nothing_rather_than_a_panic() {
        // Truncated mid-tag: an error or an empty track, either is fine.
        assert!(parse("<gpx><trk><trkpt lat=\"1\"").map_or(true, |p| p.is_empty()));
        assert!(parse("").unwrap().is_empty());
        assert!(parse("not xml at all").map_or(true, |p| p.is_empty()));
    }
}
