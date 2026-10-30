//! A shoot's GPS track as the phone records it and the desktop reads it
//! (docs/MOBILE.md, "Shoot track").
//!
//! The phone writes GPX 1.1, the format the desktop's track matching already
//! reads (`gpx.rs`), so a phone's track is placed exactly like a file from any
//! other logger. It lives here, shared, so the two cannot drift apart: the
//! desktop's tests read what this writes.

/// One recorded position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackPoint {
    /// UTC, milliseconds since 1970.
    pub time_ms: i64,
    pub lat: f64,
    pub lon: f64,
    /// Metres above sea level, when the phone had one.
    pub ele: Option<f64>,
}

/// Reads the phone's recording file: one `epochMillis,lat,lon,accuracy,altitude`
/// line per fix. Lines that do not parse (a write cut off by the phone dying)
/// are skipped.
pub fn parse_recording(csv: &str) -> Vec<TrackPoint> {
    let mut points: Vec<TrackPoint> = csv
        .lines()
        .filter_map(|line| {
            let mut f = line.split(',');
            let time_ms = f.next()?.trim().parse().ok()?;
            let lat: f64 = f.next()?.trim().parse().ok()?;
            let lon: f64 = f.next()?.trim().parse().ok()?;
            let _accuracy = f.next();
            let ele = f.next().and_then(|e| e.trim().parse().ok());
            let valid = (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon);
            valid.then_some(TrackPoint {
                time_ms,
                lat,
                lon,
                ele,
            })
        })
        .collect();
    points.sort_by_key(|p| p.time_ms);
    points
}

/// The track as GPX 1.1, one segment.
pub fn to_gpx(points: &[TrackPoint], name: &str) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <gpx version=\"1.1\" creator=\"SourceLight\" xmlns=\"http://www.topografix.com/GPX/1/1\">\n\
         <trk><name>",
    );
    out.push_str(&escape(name));
    out.push_str("</name><trkseg>\n");
    for p in points {
        out.push_str(&format!(
            "<trkpt lat=\"{:.7}\" lon=\"{:.7}\">",
            p.lat, p.lon
        ));
        if let Some(ele) = p.ele {
            out.push_str(&format!("<ele>{ele:.1}</ele>"));
        }
        out.push_str(&format!("<time>{}</time></trkpt>\n", utc(p.time_ms)));
    }
    out.push_str("</trkseg></trk>\n</gpx>\n");
    out
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `YYYY-MM-DDTHH:MM:SSZ` for a UTC time in milliseconds: the civil date
/// from a day count (Howard Hinnant's algorithm), so this crate needs no date
/// library.
pub fn utc(time_ms: i64) -> String {
    let secs = time_ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_times_match_known_dates() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        // 2026-10-03 09:30:15 UTC.
        assert_eq!(utc(1_791_019_815_000), "2026-10-03T09:30:15Z");
        // A leap day, and a time before 1970.
        assert_eq!(utc(951_782_400_000), "2000-02-29T00:00:00Z");
        assert_eq!(utc(-1_000), "1969-12-31T23:59:59Z");
    }

    #[test]
    fn a_recording_reads_and_skips_damaged_lines() {
        let csv = "1791019815000,46.5,11.9,4.0,1200.5\n\
                   oops\n\
                   1791019805000,46.4,11.8,5.0,\n\
                   1791019825000,91.0,11.9,4.0,1\n\
                   1791019835000,46.6";
        let points = parse_recording(csv);
        assert_eq!(points.len(), 2, "{points:?}");
        // Sorted by time; a missing altitude is None.
        assert_eq!(points[0].time_ms, 1_791_019_805_000);
        assert_eq!(points[0].ele, None);
        assert_eq!(points[1].ele, Some(1200.5));
    }

    #[test]
    fn gpx_has_every_point_with_its_time() {
        let points =
            parse_recording("1791019815000,46.5,11.9,4.0,1200.5\n1791019825000,46.51,11.91,4.0,\n");
        let gpx = to_gpx(&points, "Dolomites & <friends>");
        assert!(gpx.contains("<name>Dolomites &amp; &lt;friends&gt;</name>"));
        assert_eq!(gpx.matches("<trkpt ").count(), 2);
        assert!(gpx.contains(
            "<trkpt lat=\"46.5000000\" lon=\"11.9000000\"><ele>1200.5</ele><time>2026-10-03T09:30:15Z</time></trkpt>"
        ));
    }
}
