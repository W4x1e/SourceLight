//! Trip clustering.
//!
//! The a6400 has no GPS, so location has to be entered by hand. Grouping photos
//! into date-gap clusters turns that from "tag 800 photos" into "name 6 trips".
//!
//! A cluster carries enough to *recognise* the trip, not just count it: a
//! spread of thumbnails across its whole span, and the camera bodies it was
//! shot on. Naming a trip means remembering which one it was, and one 56px
//! thumbnail of its first frame is not enough to do that from.

use anyhow::Result;
use chrono::NaiveDateTime;
use rusqlite::Connection;

use crate::models::{TripCluster, TripPlace};

/// How many thumbnails a cluster carries for its contact sheet. Enough to fill
/// a wide pane with fixed-width tiles, few enough that a 400-photo trip doesn't
/// ship 400 paths over IPC for a strip that shows a dozen.
pub const CONTACT_SHEET: usize = 12;

fn parse(dt: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(dt, "%Y-%m-%d %H:%M:%S").ok()
}

/// Every thumbnail collected for a cluster while it was being built, before it
/// is thinned to a spread.
struct Building {
    cluster: TripCluster,
    /// Every thumbnail seen, in capture order.
    thumbs: Vec<String>,
    /// Camera bodies and how many frames each took.
    cameras: Vec<(String, usize)>,
    /// Places the photos are in: (name, photos, of those with no position of
    /// their own). The last is what the trip was *named*; the rest is GPS.
    places: Vec<(String, usize, usize)>,
}

impl Building {
    fn note_camera(&mut self, camera: Option<String>) {
        let Some(name) = camera.filter(|c| !c.trim().is_empty()) else {
            return;
        };
        match self.cameras.iter_mut().find(|(c, _)| *c == name) {
            Some((_, n)) => *n += 1,
            None => self.cameras.push((name, 1)),
        }
    }

    fn note_place(&mut self, place: Option<String>, located: bool) {
        if located {
            self.cluster.located += 1;
        }
        let Some(name) = place else {
            return;
        };
        let by_hand = usize::from(!located);
        match self.places.iter_mut().find(|(p, _, _)| *p == name) {
            Some((_, n, h)) => {
                *n += 1;
                *h += by_hand;
            }
            None => self.places.push((name, 1, by_hand)),
        }
    }

    /// Thins the thumbnails to an evenly-spread [`CONTACT_SHEET`] and orders the
    /// cameras by use, then hands back the finished cluster.
    fn finish(mut self) -> TripCluster {
        self.cluster.thumb_paths = spread(&self.thumbs, CONTACT_SHEET);
        self.cameras.sort_by_key(|c| std::cmp::Reverse(c.1));
        self.cluster.cameras = self.cameras.into_iter().map(|(c, _)| c).collect();
        // Stable sorts, so a tie keeps the order the trip reached them in.
        self.places.sort_by_key(|p| std::cmp::Reverse(p.1));
        self.cluster.existing_place = self
            .places
            .iter()
            .filter(|p| p.2 > 0)
            .min_by_key(|p| std::cmp::Reverse(p.2)) // the first of the most
            .map(|p| p.0.clone());
        self.cluster.places = self
            .places
            .into_iter()
            .map(|(name, count, _)| TripPlace {
                name,
                count: count as i32,
            })
            .collect();
        self.cluster
    }
}

/// Picks at most `n` items spread evenly across `items`, keeping the first and
/// (when there is room) the last.
///
/// A contact sheet of the first eight frames would show eight variations of one
/// moment; the point is to show the shape of the trip, so it samples across it.
pub(crate) fn spread<T: Clone>(items: &[T], n: usize) -> Vec<T> {
    if items.len() <= n {
        return items.to_vec();
    }
    if n == 0 {
        return Vec::new();
    }
    // Divide by `n - 1` so the last index lands on the final item rather than
    // short of it; `n == 1` would divide by zero, so take the first and stop.
    if n == 1 {
        return vec![items[0].clone()];
    }
    (0..n)
        .map(|i| items[i * (items.len() - 1) / (n - 1)].clone())
        .collect()
}

/// Splits the library into runs of photos separated by more than `gap_hours`.
pub fn cluster(conn: &Connection, gap_hours: i64) -> Result<Vec<TripCluster>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.capture_date, p.thumb_path, p.camera,
                (SELECT pl.name FROM photo_places pp JOIN places pl ON pl.id = pp.place_id
                  WHERE pp.photo_id = p.id) AS place,
                p.lat IS NOT NULL AS located
         FROM photos p
         WHERE p.capture_date IS NOT NULL
         ORDER BY p.capture_date ASC",
    )?;

    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i32>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, bool>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut clusters: Vec<TripCluster> = Vec::new();
    let mut current: Option<Building> = None;
    let mut last: Option<NaiveDateTime> = None;

    for (id, date, thumb, camera, place, located) in rows {
        let parsed = parse(&date);
        let new_cluster = match (last, parsed) {
            (Some(prev), Some(now)) => (now - prev).num_hours().abs() > gap_hours,
            _ => current.is_none(),
        };

        if new_cluster {
            if let Some(c) = current.take() {
                clusters.push(c.finish());
            }
            let mut building = Building {
                cluster: TripCluster {
                    photo_ids: vec![id],
                    start_date: date.clone(),
                    end_date: date.clone(),
                    count: 1,
                    existing_place: None,
                    places: Vec::new(),
                    located: 0,
                    thumb_path: thumb.clone(),
                    thumb_paths: Vec::new(),
                    cameras: Vec::new(),
                },
                thumbs: thumb.clone().into_iter().collect(),
                cameras: Vec::new(),
                places: Vec::new(),
            };
            building.note_camera(camera);
            building.note_place(place, located);
            current = Some(building);
        } else if let Some(c) = current.as_mut() {
            c.cluster.photo_ids.push(id);
            c.cluster.end_date = date.clone();
            c.cluster.count += 1;
            if c.cluster.thumb_path.is_none() {
                c.cluster.thumb_path = thumb.clone();
            }
            if let Some(t) = thumb {
                c.thumbs.push(t);
            }
            c.note_camera(camera);
            c.note_place(place, located);
        }

        if parsed.is_some() {
            last = parsed;
        }
    }

    if let Some(c) = current.take() {
        clusters.push(c.finish());
    }

    // Most recent trips first — that's what people are usually organising.
    clusters.reverse();
    Ok(clusters)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trip_reports_every_place_and_is_named_by_its_camera_frames() {
        let mut conn = crate::db::open_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO photos (id, path, filename, file_hash, indexed_at, capture_date)
                 VALUES (1, '/1.jpg', '1.jpg', 'h1', '2026-01-01', '2025-06-12 09:00:00'),
                        (2, '/2.jpg', '2.jpg', 'h2', '2026-01-01', '2025-06-12 12:00:00'),
                        (3, '/3.arw', '3.arw', 'h3', '2026-01-01', '2025-06-13 10:00:00'),
                        (4, '/4.arw', '4.arw', 'h4', '2026-01-01', '2025-06-14 10:00:00'),
                        (5, '/5.arw', '5.arw', 'h5', '2026-01-01', '2025-06-14 11:00:00');",
        )
        .unwrap();
        // Two phone shots in Edinburgh — the first photos of the trip — and
        // three camera frames the user named Skye.
        crate::db::set_gps(&conn, 1, Some((55.9533, -3.1883))).unwrap();
        crate::db::set_gps(&conn, 2, Some((55.9533, -3.1883))).unwrap();
        crate::db::place_from_gps(&mut conn).unwrap();
        crate::db::name_trip(&mut conn, &[1, 2, 3, 4, 5], "Skye").unwrap();

        let trips = cluster(&conn, 24).unwrap();
        assert_eq!(trips.len(), 1);
        let t = &trips[0];
        assert_eq!(t.located, 2);
        assert_eq!(
            t.existing_place.as_deref(),
            Some("Skye"),
            "named by what the user called it, not the first GPS town"
        );
        let places: Vec<(&str, i32)> = t
            .places
            .iter()
            .map(|p| (p.name.as_str(), p.count))
            .collect();
        assert_eq!(places, vec![("Skye", 3), ("Edinburgh, United Kingdom", 2)]);
    }

    #[test]
    fn spread_keeps_everything_when_it_already_fits() {
        assert_eq!(spread(&[1, 2, 3], 8), vec![1, 2, 3]);
        assert_eq!(spread::<i32>(&[], 8), Vec::<i32>::new());
    }

    #[test]
    fn spread_samples_across_the_run_not_just_the_front() {
        // The first eight frames of a 100-photo trip are eight versions of one
        // moment. A contact sheet has to cover the span to be worth showing.
        let items: Vec<i32> = (0..100).collect();
        let picked = spread(&items, 5);
        assert_eq!(picked, vec![0, 24, 49, 74, 99]);
    }

    #[test]
    fn spread_keeps_both_ends() {
        let items: Vec<i32> = (0..10).collect();
        let picked = spread(&items, 4);
        assert_eq!(picked.first(), Some(&0));
        assert_eq!(picked.last(), Some(&9));
        assert_eq!(picked.len(), 4);
    }

    #[test]
    fn spread_handles_degenerate_counts_without_dividing_by_zero() {
        let items: Vec<i32> = (0..10).collect();
        assert_eq!(spread(&items, 1), vec![0]);
        assert_eq!(spread(&items, 0), Vec::<i32>::new());
    }
}
