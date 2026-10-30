//! Where a photo was taken, as an exported file or a sidecar says it.
//!
//! Location never used to leave the app: a photo placed by its own GPS, or an
//! a6400 frame whose trip was named and pinned on the map, exported with no
//! position and no town, so other editors, photo services and the stock sites all
//! saw a photo from nowhere.
//!
//! Two sources, and the file says which it got:
//!
//! - **The photo's own GPS** — what its file said. Written as it is.
//! - **Its place's pin** — for a camera with no GPS, where the user put the
//!   place on the map. That is a place, not a fix, so it goes out with a
//!   horizontal error of [`PIN_ERROR_M`]: a map still shows the photo there,
//!   and a tool that cares can tell it is approximate.
//!
//! Text rides alongside: the place's name as the IPTC location, and a town
//! and country when they can be told apart — see [`crate::geo::split_country`].

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

/// How far a pin may be from where a photo was actually taken, in metres.
/// Pins name a town or a region; a few kilometres says so without pretending
/// to know which.
pub const PIN_ERROR_M: u32 = 5_000;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PhotoLocation {
    /// Signed decimal degrees, (latitude, longitude).
    pub position: Option<(f64, f64)>,
    /// The position is the place's pin, not the photo's own GPS.
    pub approximate: bool,
    /// The place's name, as the user or GPS naming gave it.
    pub name: Option<String>,
    pub city: Option<String>,
    pub country: Option<String>,
}

impl PhotoLocation {
    /// Works out what to say from the photo's own position and its place
    /// (name, pin). `None` when there is nothing to say.
    pub fn resolve(
        own: Option<(f64, f64)>,
        place: Option<(String, Option<(f64, f64)>)>,
    ) -> Option<Self> {
        let (name, pin) = match place {
            Some((name, pin)) => (Some(name.trim().to_string()).filter(|n| !n.is_empty()), pin),
            None => (None, None),
        };
        let (position, approximate) = match (own, pin) {
            (Some(p), _) => (Some(p), false),
            (None, Some(p)) => (Some(p), true),
            (None, None) => (None, false),
        };
        let (city, country) = match name.as_deref().and_then(crate::geo::split_country) {
            Some((city, country)) => (Some(city), Some(country)),
            None => (
                None,
                position.and_then(|(lat, lon)| crate::geo::country_at(lat, lon)),
            ),
        };
        let out = Self {
            position,
            approximate,
            name,
            city,
            country,
        };
        (out != Self::default()).then_some(out)
    }

    /// Reads the photo's own position and its place from the catalogue.
    pub fn load(conn: &Connection, photo_id: i32) -> Result<Option<Self>> {
        let row = conn
            .query_row(
                "SELECT p.lat, p.lon, pl.name, pl.lat, pl.lon
                   FROM photos p
                   LEFT JOIN photo_places pp ON pp.photo_id = p.id
                   LEFT JOIN places pl ON pl.id = pp.place_id
                  WHERE p.id = ?1",
                params![photo_id],
                |r| {
                    Ok((
                        r.get::<_, Option<f64>>(0)?,
                        r.get::<_, Option<f64>>(1)?,
                        r.get::<_, Option<String>>(2)?,
                        r.get::<_, Option<f64>>(3)?,
                        r.get::<_, Option<f64>>(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((lat, lon, name, plat, plon)) = row else {
            return Ok(None);
        };
        let own = lat.zip(lon);
        let place = name.map(|n| (n, plat.zip(plon)));
        Ok(Self::resolve(own, place))
    }
}

/// The XMP namespaces [`PhotoLocation::xmp_fields`] uses, as (prefix, URI).
pub const XMP_NAMESPACES: [(&str, &str); 3] = [
    ("exif", "http://ns.adobe.com/exif/1.0/"),
    ("photoshop", "http://ns.adobe.com/photoshop/1.0/"),
    (
        "Iptc4xmpCore",
        "http://iptc.org/std/Iptc4xmpCore/1.0/xmlns/",
    ),
];

/// Every XMP property a location can write, so a sidecar being updated can
/// clear the ones a photo no longer has.
pub const XMP_PROPERTIES: [&str; 7] = [
    "exif:GPSVersionID",
    "exif:GPSLatitude",
    "exif:GPSLongitude",
    "exif:GPSHPositioningError",
    "photoshop:City",
    "photoshop:Country",
    "Iptc4xmpCore:Location",
];

impl PhotoLocation {
    /// The location as simple XMP properties, (qualified name, unescaped
    /// value) — the fields editors' GPS and IPTC panels, file browsers and
    /// the stock-site uploaders read.
    pub fn xmp_fields(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        if let Some((lat, lon)) = self.position {
            out.push(("exif:GPSVersionID", "2.3.0.0".to_string()));
            out.push(("exif:GPSLatitude", xmp_coordinate(lat, 'N', 'S')));
            out.push(("exif:GPSLongitude", xmp_coordinate(lon, 'E', 'W')));
            if self.approximate {
                out.push(("exif:GPSHPositioningError", format!("{PIN_ERROR_M}/1")));
            }
        }
        if let Some(city) = &self.city {
            out.push(("photoshop:City", city.clone()));
        }
        if let Some(country) = &self.country {
            out.push(("photoshop:Country", country.clone()));
        }
        if let Some(name) = &self.name {
            out.push(("Iptc4xmpCore:Location", name.clone()));
        }
        out
    }
}

/// A coordinate as EXIF stores it: a hemisphere letter and three rationals —
/// whole degrees, whole minutes, seconds to a hundredth.
pub fn exif_dms(value: f64, positive: char, negative: char) -> (char, [(u32, u32); 3]) {
    let hemisphere = if value < 0.0 { negative } else { positive };
    let v = value.abs();
    let deg = v.floor();
    let min = ((v - deg) * 60.0).floor();
    let sec = (v - deg - min / 60.0) * 3600.0;
    (
        hemisphere,
        [
            (deg as u32, 1),
            (min as u32, 1),
            ((sec * 100.0).round() as u32, 100),
        ],
    )
}

/// A coordinate as XMP's `exif:GPSLatitude` wants it: `DDD,MM.mmmmmmK`.
pub fn xmp_coordinate(value: f64, positive: char, negative: char) -> String {
    let hemisphere = if value < 0.0 { negative } else { positive };
    let v = value.abs();
    let deg = v.floor();
    let min = (v - deg) * 60.0;
    format!("{},{:.6}{hemisphere}", deg as u32, min)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_photos_own_gps_is_exact_and_a_pin_is_not() {
        let own = PhotoLocation::resolve(
            Some((55.95, -3.19)),
            Some(("Edinburgh, United Kingdom".into(), Some((55.9533, -3.1883)))),
        )
        .unwrap();
        assert_eq!(own.position, Some((55.95, -3.19)));
        assert!(!own.approximate);
        assert_eq!(own.city.as_deref(), Some("Edinburgh"));
        assert_eq!(own.country.as_deref(), Some("United Kingdom"));

        let pinned =
            PhotoLocation::resolve(None, Some(("Karoo".into(), Some((-32.3, 22.5))))).unwrap();
        assert_eq!(pinned.position, Some((-32.3, 22.5)));
        assert!(pinned.approximate);
        assert_eq!(pinned.name.as_deref(), Some("Karoo"));
        assert_eq!(
            pinned.city, None,
            "a typed name is not guessed to be a town"
        );
    }

    #[test]
    fn a_name_without_a_pin_still_travels_and_nothing_means_nothing() {
        let named = PhotoLocation::resolve(None, Some(("Skye".into(), None))).unwrap();
        assert_eq!(named.position, None);
        assert_eq!(named.name.as_deref(), Some("Skye"));
        assert_eq!(PhotoLocation::resolve(None, None), None);
    }

    #[test]
    fn coordinates_in_exif_and_xmp_form() {
        // 55.9533° N = 55° 57' 11.88"
        let (h, dms) = exif_dms(55.9533, 'N', 'S');
        assert_eq!(h, 'N');
        assert_eq!(dms, [(55, 1), (57, 1), (1188, 100)]);
        assert_eq!(exif_dms(-3.1883, 'E', 'W').0, 'W');
        assert_eq!(xmp_coordinate(55.9533, 'N', 'S'), "55,57.198000N");
        assert_eq!(xmp_coordinate(-3.1883, 'E', 'W'), "3,11.298000W");
    }
}
