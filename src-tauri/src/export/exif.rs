//! The EXIF block an exported file carries.
//!
//! Every export used to strip the photographer's name and copyright, the camera
//! and lens, the exposure, and the capture date — silently, so nobody found out
//! until an agency rejected the files or a client's library sorted them into
//! the wrong year.
//!
//! **Built from the catalogue, not copied from the raw.** A raw's own EXIF
//! carries a maker note that can run to tens of kilobytes, an embedded
//! thumbnail of the unedited frame, raw-only tags that mean nothing in a JPEG —
//! and its **Orientation**. The export's pixels are already rotated upright, so
//! copying that tag would make every viewer rotate the photo a second time. So
//! this writes the fields a delivered photo should have, with Orientation 1,
//! from the values the catalogue already read out of the raw.
//!
//! Hand-written rather than a dependency: a little-endian TIFF structure with
//! two directories is a hundred lines, and it is read back in the tests by the
//! same EXIF reader the scanner uses, which is what keeps it honest.

/// What an export's EXIF says. Every field is optional; absent ones are left
/// out rather than written empty.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExportMetadata {
    /// The camera as the catalogue names it — "Sony ILCE-6400". Written as
    /// Model, with its first word as Make.
    pub camera: Option<String>,
    pub lens: Option<String>,
    pub iso: Option<i32>,
    pub aperture: Option<f64>,
    /// As EXIF presents it: "1/250", "0.5", "2".
    pub shutter: Option<String>,
    pub focal_length: Option<f64>,
    /// `YYYY-MM-DD HH:MM:SS`, as the catalogue stores it.
    pub capture_date: Option<String>,
    /// The photographer: the photo's own, else Settings'.
    pub creator: Option<String>,
    /// The copyright line: the photo's own, else Settings'.
    pub copyright: Option<String>,
    /// Its title, as `dc:title`. XMP only.
    pub title: Option<String>,
    /// The credit line, as `photoshop:Credit`. XMP only.
    pub credit: Option<String>,
    /// How it may be used, as `xmpRights:UsageTerms`. XMP only.
    pub usage_terms: Option<String>,
    /// Where it was taken — see [`super::location`].
    pub location: Option<super::location::PhotoLocation>,
    /// Keywords — subjects, genre and conditions — as its sidecar has them.
    /// XMP only: EXIF has no keyword field.
    pub keywords: Vec<String>,
    /// The caption, as `dc:description`: one someone wrote, else
    /// the model's. XMP only.
    pub caption: Option<String>,
    /// Who is in it, by name. XMP only.
    pub people: Vec<String>,
    /// Which of `keywords` only the tagger gave.
    pub ai_keywords: Vec<String>,
    /// Whether `caption` is the vision model's rather than the user's:
    /// marked as such in the file when it goes out at all.
    pub caption_is_ai: bool,
}

/// Settings keys for the two lines the catalogue cannot know.
pub const CREATOR_KEY: &str = "export_creator";
pub const COPYRIGHT_KEY: &str = "export_copyright";

impl ExportMetadata {
    /// Takes out what `choices` leave out, as
    /// [`crate::xmp::MetadataChoices::apply`] does for a sidecar.
    pub fn apply(&mut self, choices: &crate::xmp::MetadataChoices) {
        if !choices.names {
            self.people.clear();
        }
        if !choices.location {
            self.location = None;
        }
        if self.caption_is_ai && !(choices.ai_caption && choices.location) {
            self.caption = None;
            self.caption_is_ai = false;
        }
        if !choices.ai_keywords {
            let ai = std::mem::take(&mut self.ai_keywords);
            self.keywords.retain(|k| !ai.contains(k));
        }
    }

    /// A photo's catalogue fields plus the photographer's own lines.
    pub fn from_photo(
        photo: &crate::models::Photo,
        creator: Option<String>,
        copyright: Option<String>,
    ) -> Self {
        Self {
            camera: photo.camera.clone(),
            lens: photo.lens.clone(),
            iso: photo.iso,
            aperture: photo.aperture,
            shutter: photo.shutter.clone(),
            focal_length: photo.focal_length,
            capture_date: photo.capture_date.clone(),
            creator,
            copyright,
            title: None,
            credit: None,
            usage_terms: None,
            location: None,
            keywords: Vec::new(),
            caption: None,
            people: Vec::new(),
            ai_keywords: Vec::new(),
            caption_is_ai: false,
        }
    }

    /// [`Self::from_photo`] plus what needs the catalogue: where it was taken,
    /// the keywords, caption and people its sidecar carries, and its own title
    /// and rights, which win over the Settings lines.
    pub fn for_photo(
        conn: &rusqlite::Connection,
        photo: &crate::models::Photo,
        creator: Option<String>,
        copyright: Option<String>,
    ) -> anyhow::Result<Self> {
        let side = crate::db::xmp_meta(conn, photo)?;
        let text = side.text;
        Ok(Self {
            location: side.location,
            keywords: side.keywords,
            caption_is_ai: side.description.is_some() && !side.description_is_own,
            caption: side.description,
            people: side.people,
            ai_keywords: side.ai_keywords,
            title: text.title,
            credit: text.credit,
            usage_terms: text.usage_terms,
            ..Self::from_photo(photo, text.creator.or(creator), text.rights.or(copyright))
        })
    }

    /// Reads the photo and the Settings lines. `None` when the photo is gone,
    /// which exports without EXIF rather than failing the export.
    pub fn load(conn: &rusqlite::Connection, photo_id: i32) -> anyhow::Result<Option<Self>> {
        let Some(photo) = crate::db::get_photo(conn, photo_id)? else {
            return Ok(None);
        };
        let creator = crate::db::get_setting(conn, CREATOR_KEY)?;
        let copyright = crate::db::get_setting(conn, COPYRIGHT_KEY)?;
        Ok(Some(Self::for_photo(conn, &photo, creator, copyright)?))
    }
}

enum Value {
    /// EXIF's text type. Nominally 7-bit ASCII, but written as UTF-8: a
    /// copyright line nearly always starts with ©, and UTF-8 is what ExifTool
    /// reads EXIF text as by default and what other editors write. Stripping it to
    /// "(c)" would be safer on paper and wrong in every tool that matters.
    Ascii(String),
    Short(u16),
    Long(u32),
    Rational(u32, u32),
    Rationals(Vec<(u32, u32)>),
    Bytes(Vec<u8>),
    Undefined(Vec<u8>),
}

impl Value {
    /// (TIFF field type, count, bytes).
    fn encode(&self) -> (u16, u32, Vec<u8>) {
        match self {
            Value::Ascii(s) => {
                let mut b = s.as_bytes().to_vec();
                b.push(0);
                (2, b.len() as u32, b)
            }
            Value::Short(v) => (3, 1, v.to_le_bytes().to_vec()),
            Value::Long(v) => (4, 1, v.to_le_bytes().to_vec()),
            Value::Rational(n, d) => {
                let mut b = n.to_le_bytes().to_vec();
                b.extend_from_slice(&d.to_le_bytes());
                (5, 1, b)
            }
            Value::Rationals(v) => {
                let b = v
                    .iter()
                    .flat_map(|(n, d)| n.to_le_bytes().into_iter().chain(d.to_le_bytes()))
                    .collect();
                (5, v.len() as u32, b)
            }
            Value::Bytes(b) => (1, b.len() as u32, b.clone()),
            Value::Undefined(b) => (7, b.len() as u32, b.clone()),
        }
    }
}

/// Appends one image file directory at `out.len()` — its entries, sorted by
/// tag as the format requires, then the values too big to sit inline. Offsets
/// are from the start of the TIFF header, which is the start of `out`.
fn write_ifd(out: &mut Vec<u8>, mut entries: Vec<(u16, Value)>) {
    entries.sort_by_key(|(tag, _)| *tag);
    let start = out.len() as u32;
    let ifd_len = 2 + 12 * entries.len() as u32 + 4;
    let mut data: Vec<u8> = Vec::new();

    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (tag, value) in &entries {
        let (kind, count, bytes) = value.encode();
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        if bytes.len() <= 4 {
            let mut inline = bytes;
            inline.resize(4, 0);
            out.extend_from_slice(&inline);
        } else {
            let offset = start + ifd_len + data.len() as u32;
            out.extend_from_slice(&offset.to_le_bytes());
            data.extend_from_slice(&bytes);
            // Values start on a word boundary.
            if data.len() % 2 == 1 {
                data.push(0);
            }
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes()); // no next directory
    out.extend_from_slice(&data);
}

/// The size `write_ifd` will produce for `entries`, so a directory's position
/// can be known before it is written.
fn ifd_size(entries: &[(u16, Value)]) -> u32 {
    let data: u32 = entries
        .iter()
        .map(|(_, v)| {
            let n = v.encode().2.len() as u32;
            if n <= 4 {
                0
            } else {
                n + n % 2
            }
        })
        .sum();
    2 + 12 * entries.len() as u32 + 4 + data
}

/// "1/250" → 1/250, "0.5" → 1/2, "2" → 2/1. `None` for anything else.
fn shutter_rational(text: &str) -> Option<(u32, u32)> {
    let text = text.trim();
    if let Some((n, d)) = text.split_once('/') {
        let (n, d) = (n.trim().parse::<u32>().ok()?, d.trim().parse::<u32>().ok()?);
        return (d > 0).then_some((n, d));
    }
    let secs: f64 = text.parse().ok()?;
    if !(secs.is_finite() && secs > 0.0) {
        return None;
    }
    if secs >= 1.0 {
        Some(((secs * 10.0).round() as u32, 10))
    } else {
        Some((1, (1.0 / secs).round() as u32))
    }
}

/// A decimal like 2.8 or 35.0 as a rational with two places.
fn decimal_rational(v: f64) -> Option<(u32, u32)> {
    (v.is_finite() && v > 0.0).then(|| ((v * 100.0).round() as u32, 100))
}

/// Builds the EXIF block for [`image`]'s `set_exif_metadata` — a bare
/// little-endian TIFF structure; each encoder adds its own container framing.
pub fn build(meta: &ExportMetadata) -> Vec<u8> {
    // Each text field is capped: the whole block has to fit one JPEG
    // APP1 segment, whose length is 16 bits, and the encoder would silently
    // wrap a longer one into a corrupt file. Five fields of this size leave
    // room to spare; no real lens, camera or credit is near it.
    const TEXT_MAX: usize = 2048;
    let text = |s: &Option<String>| {
        s.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                let mut end = s.len().min(TEXT_MAX);
                while !s.is_char_boundary(end) {
                    end -= 1;
                }
                s[..end].to_string()
            })
    };

    let mut ifd0: Vec<(u16, Value)> = vec![
        // The pixels are already upright — see the module notes.
        (0x0112, Value::Short(1)),
        (0x0131, Value::Ascii("SourceLight".into())),
    ];
    if let Some(camera) = text(&meta.camera) {
        let make = camera.split_whitespace().next().unwrap_or("").to_string();
        ifd0.push((0x010F, Value::Ascii(make)));
        ifd0.push((0x0110, Value::Ascii(camera)));
    }
    if let Some(creator) = text(&meta.creator) {
        ifd0.push((0x013B, Value::Ascii(creator)));
    }
    if let Some(copyright) = text(&meta.copyright) {
        ifd0.push((0x8298, Value::Ascii(copyright)));
    }

    let mut exif: Vec<(u16, Value)> = vec![(0x9000, Value::Undefined(b"0232".to_vec()))];
    if let Some((n, d)) = meta.shutter.as_deref().and_then(shutter_rational) {
        exif.push((0x829A, Value::Rational(n, d)));
    }
    if let Some((n, d)) = meta.aperture.and_then(decimal_rational) {
        exif.push((0x829D, Value::Rational(n, d)));
    }
    if let Some(iso) = meta.iso.filter(|&i| i > 0) {
        exif.push((0x8827, Value::Short(iso.min(u16::MAX as i32) as u16)));
    }
    if let Some(date) = text(&meta.capture_date) {
        // The catalogue's `2024-06-01 14:30:00` is EXIF's `2024:06:01 14:30:00`.
        let (day, time) = date.split_once(' ').unwrap_or((&date, "00:00:00"));
        exif.push((
            0x9003,
            Value::Ascii(format!("{} {time}", day.replace('-', ":"))),
        ));
    }
    if let Some((n, d)) = meta.focal_length.and_then(decimal_rational) {
        exif.push((0x920A, Value::Rational(n, d)));
    }
    if let Some(lens) = text(&meta.lens) {
        exif.push((0xA434, Value::Ascii(lens)));
    }

    let gps = meta
        .location
        .as_ref()
        .and_then(|l| l.position.map(|p| (p, l.approximate)))
        .map(|((lat, lon), approximate)| gps_ifd(lat, lon, approximate));

    // IFD0 points at the EXIF directory, which follows it and its data, and
    // at the GPS directory after that. Both pointers are counted before
    // either offset is known, since each adds an entry to IFD0.
    let pointers = 1 + u32::from(gps.is_some());
    let exif_at = 8 + ifd_size(&ifd0) + 12 * pointers;
    ifd0.push((0x8769, Value::Long(exif_at)));
    let gps_at = exif_at + ifd_size(&exif);
    if gps.is_some() {
        ifd0.push((0x8825, Value::Long(gps_at)));
    }

    let mut out = b"II*\0".to_vec();
    out.extend_from_slice(&8u32.to_le_bytes()); // IFD0 straight after the header
    write_ifd(&mut out, ifd0);
    debug_assert_eq!(out.len() as u32, exif_at);
    write_ifd(&mut out, exif);
    if let Some(gps) = gps {
        debug_assert_eq!(out.len() as u32, gps_at);
        write_ifd(&mut out, gps);
    }
    out
}

/// The GPS directory: version, both coordinates, and — for a place's pin
/// rather than the photo's own fix — how far out it may be.
fn gps_ifd(lat: f64, lon: f64, approximate: bool) -> Vec<(u16, Value)> {
    use super::location::{exif_dms, PIN_ERROR_M};
    let (lat_ref, lat_dms) = exif_dms(lat, 'N', 'S');
    let (lon_ref, lon_dms) = exif_dms(lon, 'E', 'W');
    let mut gps = vec![
        (0x0000, Value::Bytes(vec![2, 3, 0, 0])),
        (0x0001, Value::Ascii(lat_ref.to_string())),
        (0x0002, Value::Rationals(lat_dms.to_vec())),
        (0x0003, Value::Ascii(lon_ref.to_string())),
        (0x0004, Value::Rationals(lon_dms.to_vec())),
    ];
    if approximate {
        gps.push((0x001F, Value::Rational(PIN_ERROR_M, 1)));
    }
    gps
}

#[cfg(test)]
mod tests {
    use super::*;
    use exif::{In, Reader, Tag};

    fn full() -> ExportMetadata {
        ExportMetadata {
            camera: Some("Sony ILCE-6400".into()),
            lens: Some("E 18-135mm F3.5-5.6 OSS".into()),
            iso: Some(400),
            aperture: Some(5.6),
            shutter: Some("1/250".into()),
            focal_length: Some(35.0),
            capture_date: Some("2024-06-01 14:30:05".into()),
            creator: Some("Ada Lovelace".into()),
            copyright: Some("© 2024 Ada Lovelace".into()),
            ..Default::default()
        }
    }

    /// Read back with the same EXIF reader the scanner uses: if this parses
    /// it, so will everything else that matters.
    fn read(blob: &[u8]) -> exif::Exif {
        Reader::new()
            .read_raw(blob.to_vec())
            .expect("the block is valid EXIF")
    }

    fn text(e: &exif::Exif, tag: Tag) -> String {
        e.get_field(tag, In::PRIMARY)
            .unwrap_or_else(|| panic!("{tag} missing"))
            .display_value()
            .to_string()
            .trim_matches('"')
            .to_string()
    }

    #[test]
    fn every_field_reads_back_as_written() {
        let e = read(&build(&full()));
        assert_eq!(text(&e, Tag::Make), "Sony");
        assert_eq!(text(&e, Tag::Model), "Sony ILCE-6400");
        assert_eq!(text(&e, Tag::LensModel), "E 18-135mm F3.5-5.6 OSS");
        assert_eq!(text(&e, Tag::PhotographicSensitivity), "400");
        assert_eq!(text(&e, Tag::FNumber), "5.6");
        assert_eq!(text(&e, Tag::ExposureTime), "1/250");
        assert_eq!(text(&e, Tag::FocalLength), "35");
        assert_eq!(text(&e, Tag::DateTimeOriginal), "2024-06-01 14:30:05");
        assert_eq!(text(&e, Tag::Artist), "Ada Lovelace");
        assert_eq!(text(&e, Tag::Software), "SourceLight");

        // Compared as bytes: EXIF's text type is nominally 7-bit, so this reader
        // escapes the ©. It is written as UTF-8 on purpose — see `Value::Ascii`.
        let copyright = e.get_field(Tag::Copyright, In::PRIMARY).unwrap();
        let exif::Value::Ascii(parts) = &copyright.value else {
            panic!("copyright is not text");
        };
        assert_eq!(parts[0], "© 2024 Ada Lovelace".as_bytes());
    }

    fn located(position: (f64, f64), approximate: bool) -> ExportMetadata {
        ExportMetadata {
            location: Some(super::super::location::PhotoLocation {
                position: Some(position),
                approximate,
                ..Default::default()
            }),
            ..full()
        }
    }

    /// Signed degrees from a GPS coordinate and its hemisphere reference.
    fn degrees(e: &exif::Exif, tag: Tag, reference: Tag) -> f64 {
        let f = e
            .get_field(tag, In::PRIMARY)
            .unwrap_or_else(|| panic!("{tag} missing"));
        let exif::Value::Rational(r) = &f.value else {
            panic!("{tag} is not rational");
        };
        let v = r[0].to_f64() + r[1].to_f64() / 60.0 + r[2].to_f64() / 3600.0;
        let sign = text(e, reference);
        if sign == "S" || sign == "W" {
            -v
        } else {
            v
        }
    }

    #[test]
    fn a_position_reads_back_as_gps() {
        // Somewhere south and west, so both hemisphere letters are exercised.
        let e = read(&build(&located((-33.9249, -18.4241), false)));
        let lat = degrees(&e, Tag::GPSLatitude, Tag::GPSLatitudeRef);
        let lon = degrees(&e, Tag::GPSLongitude, Tag::GPSLongitudeRef);
        assert!((lat + 33.9249).abs() < 1e-5, "{lat}");
        assert!((lon + 18.4241).abs() < 1e-5, "{lon}");
        assert!(
            e.get_field(Tag::GPSHPositioningError, In::PRIMARY)
                .is_none(),
            "the photo's own fix is not marked approximate"
        );
        // The EXIF directory is still found alongside it.
        assert_eq!(text(&e, Tag::LensModel), "E 18-135mm F3.5-5.6 OSS");
    }

    #[test]
    fn a_pin_is_marked_approximate() {
        let e = read(&build(&located((57.3, -6.2), true)));
        assert!((degrees(&e, Tag::GPSLatitude, Tag::GPSLatitudeRef) - 57.3).abs() < 1e-5);
        assert_eq!(text(&e, Tag::GPSHPositioningError), "5000"); // metres
    }

    #[test]
    fn no_position_means_no_gps_directory() {
        let e = read(&build(&full()));
        assert!(e.get_field(Tag::GPSLatitude, In::PRIMARY).is_none());
        assert!(e.get_field(Tag::GPSVersionID, In::PRIMARY).is_none());
    }

    /// The pixels an export writes are already upright. Copying the raw's
    /// orientation would have every viewer turn a portrait on its side again.
    #[test]
    fn orientation_is_always_upright() {
        let e = read(&build(&full()));
        assert_eq!(
            e.get_field(Tag::Orientation, In::PRIMARY)
                .unwrap()
                .value
                .get_uint(0),
            Some(1)
        );
    }

    #[test]
    fn absent_fields_are_left_out_not_written_empty() {
        let e = read(&build(&ExportMetadata::default()));
        for tag in [
            Tag::Make,
            Tag::Artist,
            Tag::Copyright,
            Tag::LensModel,
            Tag::DateTimeOriginal,
        ] {
            assert!(e.get_field(tag, In::PRIMARY).is_none(), "{tag} was written");
        }
        let blank = ExportMetadata {
            creator: Some("   ".into()),
            ..Default::default()
        };
        assert!(read(&build(&blank))
            .get_field(Tag::Artist, In::PRIMARY)
            .is_none());
    }

    #[test]
    fn shutter_speeds_become_rationals() {
        assert_eq!(shutter_rational("1/250"), Some((1, 250)));
        assert_eq!(shutter_rational("0.5"), Some((1, 2)));
        assert_eq!(shutter_rational("2"), Some((20, 10)));
        assert_eq!(shutter_rational("1/0"), None);
        assert_eq!(shutter_rational("fast"), None);
    }

    /// However long the lens, camera or credit a file or sidecar brought, the
    /// block fits one JPEG APP1 segment, whose length is 16 bits.
    #[test]
    fn the_block_always_fits_a_jpeg_segment() {
        let huge = Some("L".repeat(70_000));
        let meta = ExportMetadata {
            camera: huge.clone(),
            lens: huge.clone(),
            creator: huge.clone(),
            copyright: huge,
            ..Default::default()
        };
        let block = build(&meta);
        assert!(block.len() <= 65_527, "{} bytes", block.len());
    }
}
