//! Where a photo was taken: GPS coordinates to a place name, offline.
//!
//! Phones, drones and most current bodies paired with a phone app write GPS
//! into EXIF, and for those photos Places was empty work the camera had
//! already done. This names the place without a network: the nearest town or
//! city from GeoNames' list of the ~34,000 with at least 15,000 people
//! (`resources/geo/cities.tsv`, CC BY 4.0, credited in NOTICE), with its
//! country — "Edinburgh, United Kingdom".
//!
//! The list is embedded in the binary (about 1.1 MB) and indexed on first use
//! into one-degree cells, so a lookup checks a few dozen cities, not all of
//! them.

use std::collections::HashMap;
use std::sync::OnceLock;

const CITIES: &str = include_str!("../resources/geo/cities.tsv");
const COUNTRIES: &str = include_str!("../resources/geo/countries.tsv");

/// Within this distance, a bigger town beats a nearer suburb: a photo on the
/// edge of a city is "in" the city, not the district whose centre is closest.
const PREFER_BIGGER_KM: f64 = 20.0;
/// Beyond this from every listed town, no name is given — open sea, deep
/// wilderness — rather than one that would mislead.
const MAX_KM: f64 = 60.0;

struct City {
    lat: f64,
    lon: f64,
    pop: u64,
    cc: &'static str,
    name: &'static str,
}

struct Index {
    cities: Vec<City>,
    cells: HashMap<(i32, i32), Vec<usize>>,
    countries: HashMap<&'static str, &'static str>,
}

fn cell(lat: f64, lon: f64) -> (i32, i32) {
    (lat.floor() as i32, lon.floor() as i32)
}

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut cities = Vec::new();
        for line in CITIES.lines().filter(|l| !l.starts_with('#')) {
            let mut f = line.split('\t');
            let (Some(lat), Some(lon), Some(pop), Some(cc), Some(name)) =
                (f.next(), f.next(), f.next(), f.next(), f.next())
            else {
                continue;
            };
            let (Ok(lat), Ok(lon)) = (lat.parse(), lon.parse()) else {
                continue;
            };
            cities.push(City {
                lat,
                lon,
                pop: pop.parse().unwrap_or(0),
                cc,
                name,
            });
        }
        let mut cells: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (i, c) in cities.iter().enumerate() {
            cells.entry(cell(c.lat, c.lon)).or_default().push(i);
        }
        let countries = COUNTRIES
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| l.split_once('\t'))
            .collect();
        Index {
            cities,
            cells,
            countries,
        }
    })
}

/// Great-circle distance in kilometres.
fn km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = p2 - p1;
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    6371.0 * 2.0 * a.sqrt().asin()
}

/// A named place for a coordinate: "Town, Country", or `None` when nothing
/// listed is near enough. Also the place's own coordinates, for the map.
pub fn place_for(lat: f64, lon: f64) -> Option<(String, f64, f64)> {
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    let idx = index();
    let (clat, clon) = cell(lat, lon);
    let mut near: Vec<(f64, &City)> = Vec::new();
    for dlat in -1..=1 {
        for dlon in -1..=1 {
            // Longitude wraps at the antimeridian.
            let lon_cell = (clon + dlon + 180).rem_euclid(360) - 180;
            if let Some(ids) = idx.cells.get(&(clat + dlat, lon_cell)) {
                for &i in ids {
                    let c = &idx.cities[i];
                    let d = km(lat, lon, c.lat, c.lon);
                    if d <= MAX_KM {
                        near.push((d, c));
                    }
                }
            }
        }
    }
    let nearest = near.iter().map(|(d, _)| *d).fold(f64::INFINITY, f64::min);
    if !nearest.is_finite() {
        return None;
    }
    let reach = nearest.max(PREFER_BIGGER_KM);
    let (_, city) = near
        .iter()
        .filter(|(d, _)| *d <= reach)
        .max_by_key(|(_, c)| c.pop)?;
    let country = idx.countries.get(city.cc).copied().unwrap_or(city.cc);
    Some((format!("{}, {}", city.name, country), city.lat, city.lon))
}

/// Splits a place name into town and country when it ends in a country's
/// name — "Edinburgh, United Kingdom" is Edinburgh in the United Kingdom, as
/// [`place_for`] writes them. `None` for anything else ("Karoo", "Skye,
/// Scotland"): a name someone typed is not guessed at.
pub fn split_country(name: &str) -> Option<(String, String)> {
    let (town, country) = name.rsplit_once(", ")?;
    let (town, country) = (town.trim(), country.trim());
    if town.is_empty() {
        return None;
    }
    let known = index()
        .countries
        .values()
        .find(|c| c.eq_ignore_ascii_case(country))?;
    Some((town.to_string(), known.to_string()))
}

/// The country a coordinate is in, going by the nearest listed town — `None`
/// out at sea or in the wilderness, like [`place_for`].
pub fn country_at(lat: f64, lon: f64) -> Option<String> {
    let (name, _, _) = place_for(lat, lon)?;
    split_country(&name).map(|(_, country)| country)
}

pub use sl_develop::exif_read::{from_dms, plausible};

#[cfg(test)]
mod tests {
    use super::*;

    /// The point a GPS block in `exif_read`'s test decodes to.
    #[test]
    fn edinburgh_reads_as_edinburgh() {
        let (name, _, _) = place_for(55.9533, -3.1883).unwrap();
        assert_eq!(name, "Edinburgh, United Kingdom");
    }

    #[test]
    fn a_city_centre_is_named_after_its_city() {
        let (name, _, _) = place_for(55.9533, -3.1883).unwrap();
        assert_eq!(name, "Edinburgh, United Kingdom");
        let (name, _, _) = place_for(-33.9249, 18.4241).unwrap();
        assert_eq!(name, "Cape Town, South Africa");
    }

    #[test]
    fn open_sea_has_no_name() {
        // Mid-Atlantic.
        assert!(place_for(30.0, -40.0).is_none());
    }

    #[test]
    fn a_town_and_country_split_only_when_the_country_is_real() {
        assert_eq!(
            split_country("Edinburgh, United Kingdom"),
            Some(("Edinburgh".into(), "United Kingdom".into()))
        );
        // The country comes back as the list spells it.
        assert_eq!(
            split_country("cape town, south africa").map(|(_, c)| c),
            Some("South Africa".into())
        );
        // Typed names are not guessed at.
        assert_eq!(split_country("Skye, Scotland"), None);
        assert_eq!(split_country("Karoo"), None);
        assert_eq!(split_country(", France"), None);
        assert_eq!(
            country_at(55.9533, -3.1883).as_deref(),
            Some("United Kingdom")
        );
    }

    #[test]
    fn dms_and_hemispheres() {
        let lat = from_dms(51.0, 30.0, 0.0, false);
        assert!((lat - 51.5).abs() < 1e-9);
        assert!((from_dms(0.0, 7.0, 39.0, true) + 0.1275).abs() < 1e-9);
        assert!(!plausible(0.0, 0.0));
        assert!(plausible(51.5, -0.12));
        assert!(!plausible(91.0, 0.0));
    }

    #[test]
    fn the_antimeridian_wraps() {
        // Fiji straddles 180°; a point just east of it still finds towns.
        assert!(place_for(-16.8, 179.9).is_some() || place_for(-18.14, 178.44).is_some());
    }
}
