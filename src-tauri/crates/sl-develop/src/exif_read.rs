//! EXIF extraction. ARW is a TIFF container, so the standard reader handles it
//! directly — no exiftool dependency needed.

use exif::{In, Tag};

#[derive(Debug, Default, Clone)]
pub struct ExifData {
    pub capture_date: Option<String>,
    pub camera: Option<String>,
    pub lens: Option<String>,
    pub iso: Option<i64>,
    pub aperture: Option<f64>,
    pub shutter: Option<String>,
    pub focal_length: Option<f64>,
    pub orientation: u32,
    /// True sensor dimensions. The embedded preview is much smaller (Sony ships
    /// a 1616x1080 preview), so these must come from EXIF, not the decoded JPEG.
    pub width: Option<i64>,
    pub height: Option<i64>,
    /// Where it was taken, in signed decimal degrees.
    pub gps: Option<(f64, f64)>,
    /// The colour space the camera was set to, for pixels that carry no ICC
    /// profile to say so: a camera's JPEG, and the preview inside its raw.
    pub colour_space: Option<ColourSpace>,
}

/// The two colour spaces a camera offers for its JPEGs and raw previews.
///
/// Cameras do not embed a profile. They say "Adobe RGB" the way the DCF
/// standard tells them to — `InteropIndex` "R03" with `ColorSpace`
/// "uncalibrated" — or in their own maker notes, and a reader that takes the
/// missing profile to mean sRGB shows every such photo washed out: greens,
/// cyans and reds all visibly flatter than the camera recorded them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColourSpace {
    Srgb,
    AdobeRgb,
}

/// What the EXIF says the pixels are encoded in, if anything.
///
/// In order: DCF's "R03" interoperability index, the only standard way to
/// say Adobe RGB; `ColorSpace` 2, which some cameras write instead; Nikon's
/// maker note, the only place a NEF records it; and `ColorSpace`
/// "uncalibrated" from a camera, which means Adobe RGB because it is the one
/// other choice a camera offers — Canon writes exactly that, with an "R98"
/// index alongside, for its Adobe RGB files. A phone's wide-gamut photo is
/// also "uncalibrated", but it carries an ICC profile, which is read first.
fn colour_space(e: &exif::Exif) -> Option<ColourSpace> {
    let interop = e
        .fields()
        .find(|f| f.tag == Tag::InteroperabilityIndex)
        .and_then(|f| match &f.value {
            exif::Value::Ascii(parts) => parts.first().cloned(),
            exif::Value::Undefined(bytes, _) => Some(bytes.clone()),
            _ => None,
        });
    let declared = field_i64(e, Tag::ColorSpace);
    if interop.as_deref().is_some_and(|i| i.starts_with(b"R03")) || declared == Some(2) {
        return Some(ColourSpace::AdobeRgb);
    }
    if let Some(nikon) = nikon_colour_space(e) {
        return Some(nikon);
    }
    let from_camera = field_string(e, Tag::Make).is_some();
    match declared {
        Some(0xFFFF) if from_camera => Some(ColourSpace::AdobeRgb),
        Some(1) => Some(ColourSpace::Srgb),
        _ => interop
            .is_some_and(|i| i.starts_with(b"R98"))
            .then_some(ColourSpace::Srgb),
    }
}

/// The colour space the start of a file states: its EXIF read as far as
/// `head` goes, which for a raw is usually short of the whole IFD tree, so
/// partial results count — as in [`read_gps_head`].
pub fn read_colour_space_head(head: &[u8]) -> Option<ColourSpace> {
    let mut reader = exif::Reader::new();
    reader.continue_on_error(true);
    let read = if head.starts_with(&[0xFF, 0xD8]) {
        reader.read_from_container(&mut std::io::Cursor::new(head))
    } else {
        reader.read_raw(head.to_vec())
    };
    let e = match read {
        Ok(e) => e,
        Err(err) => err.distill_partial_result(|_| {}).ok()?,
    };
    colour_space(&e)
}

/// The colour space an embedded JPEG's own EXIF states, for the first one
/// found in `bytes` — the start of a raw that keeps its preview near the
/// front with EXIF of its own, as Fujifilm and Panasonic do.
pub fn embedded_colour_space(bytes: &[u8]) -> Option<ColourSpace> {
    let mut from = 0;
    while let Some(at) = bytes[from..]
        .windows(10)
        .position(|w| w[..2] == [0xFF, 0xE1] && &w[4..] == b"Exif\0\0")
    {
        let start = from + at;
        let len = u16::from_be_bytes([bytes[start + 2], bytes[start + 3]]) as usize;
        let tiff = bytes.get(start + 10..start + 2 + len.max(8));
        let mut reader = exif::Reader::new();
        reader.continue_on_error(true);
        let read = tiff.map(|t| reader.read_raw(t.to_vec()));
        let found = match read {
            Some(Ok(e)) => colour_space(&e),
            Some(Err(err)) => err
                .distill_partial_result(|_| {})
                .ok()
                .and_then(|e| colour_space(&e)),
            None => None,
        };
        if found.is_some() {
            return found;
        }
        from = start + 10;
    }
    None
}

/// The colour space in a Nikon maker note (tag 0x001E: 1 sRGB, 2 Adobe RGB).
///
/// A NEF says nothing about it in its EXIF proper. The maker note is a TIFF
/// structure of its own after a ten-byte "Nikon" header, with offsets from
/// that inner header.
fn nikon_colour_space(e: &exif::Exif) -> Option<ColourSpace> {
    let note = e.get_field(Tag::MakerNote, In::PRIMARY)?;
    let exif::Value::Undefined(bytes, _) = &note.value else {
        return None;
    };
    let tiff = bytes.strip_prefix(b"Nikon\0")?.get(4..)?;
    let le = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |o: usize| -> Option<u16> {
        let b: [u8; 2] = tiff.get(o..o + 2)?.try_into().ok()?;
        Some(if le {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    };
    let u32_at = |o: usize| -> Option<u32> {
        let b: [u8; 4] = tiff.get(o..o + 4)?.try_into().ok()?;
        Some(if le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    };
    let ifd = u32_at(4)? as usize;
    let count = u16_at(ifd)? as usize;
    (0..count.min(512)).find_map(|i| {
        let entry = ifd + 2 + i * 12;
        (u16_at(entry)? == 0x001E && u16_at(entry + 2)? == 3).then_some(())?;
        match u16_at(entry + 8)? {
            1 => Some(ColourSpace::Srgb),
            2 => Some(ColourSpace::AdobeRgb),
            _ => None,
        }
    })
}

/// EXIF stores dates as `YYYY:MM:DD HH:MM:SS`. We rewrite the date part with
/// dashes so the string sorts and range-compares correctly in SQLite.
fn normalise_datetime(raw: &str) -> Option<String> {
    let raw = raw.trim().trim_matches('"');
    // Bytes, not characters, are counted and split below: a stray non-ASCII
    // byte made byte 10 fall inside a character, and the split panicked the
    // scan, leaving the rest of the folder unindexed on every launch.
    if raw.len() < 19 || !raw.is_ascii() {
        return None;
    }
    let (date, time) = raw.split_at(10);
    let date = date.replace(':', "-");
    let time = time.trim();
    if date.starts_with("0000") {
        return None;
    }
    Some(format!("{date} {time}"))
}

/// A text field, taking the typed value rather than its display form.
///
/// `display_value()` renders an ASCII field as a comma-separated list when the
/// tag holds several strings, and some cameras write one real value followed by
/// a run of empty ones — Nikon's `LensModel` on a Z 6 comes back as
/// `NIKKOR Z 24-70mm f/4 S", "", "", ""...` for forty more entries. Trimming
/// the outer quotes leaves the whole mess in the middle, which is how that
/// landed in the catalogue as a lens name.
///
/// Reading `Value::Ascii` directly sidesteps the formatting entirely: take the
/// first component that actually says something.
/// One readable camera name from a maker and a model.
///
/// Makers repeat themselves, and inconsistently. Nikon's model already carries
/// the brand ("NIKON Z 6") while its make is "NIKON CORPORATION", so joining
/// gives "NIKON CORPORATION NIKON Z 6" — which is what the camera facet used to
/// show. Sony's model does not carry the brand, so dropping the make loses it.
///
/// Comparing against the **first word** of the make settles both: it is the
/// brand, and the rest ("CORPORATION", "IMAGING CORP") is legal boilerplate no
/// photographer wants in a filter.
///
/// Public because the LibRaw fallback in `scan` needs exactly the same answer
/// for the formats EXIF cannot read; two copies would drift and put the same
/// camera in the facet list twice under different names.
pub fn camera_name(make: String, model: String) -> Option<String> {
    let (make, model) = (make.trim().to_string(), model.trim().to_string());
    if model.is_empty() {
        return (!make.is_empty()).then_some(make);
    }
    let brand = make.split_whitespace().next().unwrap_or("");
    if brand.is_empty() || model.to_uppercase().starts_with(&brand.to_uppercase()) {
        Some(model)
    } else {
        Some(format!("{brand} {model}"))
    }
}

fn field_string(e: &exif::Exif, tag: Tag) -> Option<String> {
    let f = e.get_field(tag, In::PRIMARY)?;
    let text = match &f.value {
        exif::Value::Ascii(parts) => parts
            .iter()
            .map(|p| String::from_utf8_lossy(p).trim().to_string())
            .find(|p| !p.is_empty())?,
        // Anything else keeps the old path: it is the right answer for the
        // single-valued rational and integer tags that also come through here.
        _ => f.display_value().to_string(),
    };
    let text = text.trim().trim_matches('"').trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn field_f64(e: &exif::Exif, tag: Tag) -> Option<f64> {
    let f = e.get_field(tag, In::PRIMARY)?;
    match &f.value {
        exif::Value::Rational(v) => v.first().map(|r| r.to_f64()),
        exif::Value::SRational(v) => v.first().map(|r| r.to_f64()),
        exif::Value::Short(v) => v.first().map(|x| *x as f64),
        exif::Value::Long(v) => v.first().map(|x| *x as f64),
        _ => None,
    }
}

fn field_i64(e: &exif::Exif, tag: Tag) -> Option<i64> {
    let f = e.get_field(tag, In::PRIMARY)?;
    match &f.value {
        exif::Value::Short(v) => v.first().map(|x| *x as i64),
        exif::Value::Long(v) => v.first().map(|x| *x as i64),
        _ => None,
    }
}

/// The three rationals of a GPS coordinate (degrees, minutes, seconds).
fn dms(e: &exif::Exif, tag: Tag) -> Option<(f64, f64, f64)> {
    let f = e.get_field(tag, In::PRIMARY)?;
    match &f.value {
        exif::Value::Rational(v) if v.len() >= 3 => {
            Some((v[0].to_f64(), v[1].to_f64(), v[2].to_f64()))
        }
        _ => None,
    }
}

/// The GPS position, if the EXIF carries one worth keeping.
fn gps(e: &exif::Exif) -> Option<(f64, f64)> {
    let (la, lam, las) = dms(e, Tag::GPSLatitude)?;
    let (lo, lom, los) = dms(e, Tag::GPSLongitude)?;
    let south = field_string(e, Tag::GPSLatitudeRef).is_some_and(|r| r.trim().starts_with('S'));
    let west = field_string(e, Tag::GPSLongitudeRef).is_some_and(|r| r.trim().starts_with('W'));
    let lat = from_dms(la, lam, las, south);
    let lon = from_dms(lo, lom, los, west);
    plausible(lat, lon).then_some((lat, lon))
}

/// What the start of a file says about where it was taken — `Some(None)` when
/// its EXIF parses and has no position, `None` when there is no EXIF to read
/// (CR3, RAF), so the caller can ask LibRaw instead. Reads only `head`: in a
/// TIFF-based raw the GPS block sits well inside the first few hundred KB,
/// and reading a whole 25 MB file to learn it has none would make the
/// backfill over an existing library cost as much as rescanning it.
///
/// A JPEG keeps its EXIF in an APP1 segment near the start rather
/// than being a TIFF, so it is read as a container.
pub fn read_gps_head(head: Vec<u8>) -> Option<Option<(f64, f64)>> {
    let mut reader = exif::Reader::new();
    reader.continue_on_error(true);
    let read = if head.starts_with(&[0xFF, 0xD8]) {
        reader.read_from_container(&mut std::io::Cursor::new(&head))
    } else {
        reader.read_raw(head)
    };
    let e = match read {
        Ok(e) => e,
        Err(err) => err.distill_partial_result(|_| {}).ok()?,
    };
    Some(gps(&e))
}

pub fn read(data: &[u8]) -> ExifData {
    let mut out = ExifData {
        orientation: 1,
        ..Default::default()
    };

    let mut cursor = std::io::Cursor::new(data);
    let Ok(e) = exif::Reader::new().read_from_container(&mut cursor) else {
        return out;
    };

    out.capture_date = field_string(&e, Tag::DateTimeOriginal)
        .or_else(|| field_string(&e, Tag::DateTime))
        .and_then(|s| normalise_datetime(&s));

    out.camera = camera_name(
        field_string(&e, Tag::Make).unwrap_or_default(),
        field_string(&e, Tag::Model).unwrap_or_default(),
    );

    out.lens = field_string(&e, Tag::LensModel);
    out.iso = field_i64(&e, Tag::PhotographicSensitivity)
        .or_else(|| field_i64(&e, Tag::StandardOutputSensitivity));
    out.aperture = field_f64(&e, Tag::FNumber);
    out.focal_length = field_f64(&e, Tag::FocalLength);
    out.shutter = field_string(&e, Tag::ExposureTime);
    out.orientation = field_i64(&e, Tag::Orientation).unwrap_or(1) as u32;

    // PixelXDimension lives in the Exif SubIFD and holds the real capture size;
    // ImageWidth in IFD0 often describes the preview instead.
    out.width = field_i64(&e, Tag::PixelXDimension).or_else(|| field_i64(&e, Tag::ImageWidth));
    out.height = field_i64(&e, Tag::PixelYDimension).or_else(|| field_i64(&e, Tag::ImageLength));
    out.gps = gps(&e);
    out.colour_space = colour_space(&e);

    out
}

/// Degrees, minutes, seconds (as EXIF and LibRaw store them) to signed
/// decimal degrees. `negative` for south or west.
pub fn from_dms(d: f64, m: f64, s: f64, negative: bool) -> f64 {
    let v = d + m / 60.0 + s / 3600.0;
    if negative {
        -v
    } else {
        v
    }
}

/// A coordinate worth keeping. (0, 0) is what an unset GPS block reads as —
/// a point in the Gulf of Guinea nobody photographed.
pub fn plausible(lat: f64, lon: f64) -> bool {
    lat.is_finite()
        && lon.is_finite()
        && (-90.0..=90.0).contains(&lat)
        && (-180.0..=180.0).contains(&lon)
        && !(lat.abs() < 1e-6 && lon.abs() < 1e-6)
}

#[cfg(test)]
mod gps_tests {
    use super::*;
    use exif::{experimental::Writer, Field, Rational, Value};

    /// A minimal TIFF carrying a GPS block, as a phone or a GPS-linked camera
    /// writes one: 55° 57' 12" N, 3° 11' 18" W — Edinburgh.
    fn tiff_with_gps() -> Vec<u8> {
        let r = |n: u32, d: u32| Rational::from((n, d));
        let fields = [
            Field {
                tag: Tag::GPSLatitudeRef,
                ifd_num: In::PRIMARY,
                value: Value::Ascii(vec![b"N".to_vec()]),
            },
            Field {
                tag: Tag::GPSLatitude,
                ifd_num: In::PRIMARY,
                value: Value::Rational(vec![r(55, 1), r(57, 1), r(12, 1)]),
            },
            Field {
                tag: Tag::GPSLongitudeRef,
                ifd_num: In::PRIMARY,
                value: Value::Ascii(vec![b"W".to_vec()]),
            },
            Field {
                tag: Tag::GPSLongitude,
                ifd_num: In::PRIMARY,
                value: Value::Rational(vec![r(3, 1), r(11, 1), r(18, 1)]),
            },
        ];
        let mut w = Writer::new();
        for f in &fields {
            w.push_field(f);
        }
        let mut out = std::io::Cursor::new(Vec::new());
        w.write(&mut out, false).unwrap();
        out.into_inner()
    }

    /// Edinburgh. That it names its town is the desktop's `geo` to test.
    #[test]
    fn a_gps_block_reads_as_signed_degrees() {
        let (lat, lon) = read_gps_head(tiff_with_gps()).unwrap().unwrap();
        assert!((lat - 55.9533).abs() < 1e-3, "{lat}");
        assert!((lon + 3.1883).abs() < 1e-3, "{lon}");
    }

    /// A phone's JPEG: the same block inside an APP1 segment, read by
    /// both the scan and the backfill.
    #[test]
    fn a_jpegs_gps_reads_from_its_app1_segment() {
        let tiff = tiff_with_gps();
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE1];
        jpeg.extend_from_slice(&((2 + 6 + tiff.len()) as u16).to_be_bytes());
        jpeg.extend_from_slice(b"Exif\0\0");
        jpeg.extend_from_slice(&tiff);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);

        let (lat, lon) = read(&jpeg).gps.expect("the scan finds it");
        assert!((lat - 55.9533).abs() < 1e-3 && (lon + 3.1883).abs() < 1e-3);
        assert_eq!(
            read_gps_head(jpeg).unwrap(),
            Some((lat, lon)),
            "and so does the backfill"
        );
    }

    #[test]
    fn no_exif_is_told_apart_from_no_position() {
        // Not a TIFF at all (a CR3 box, say): ask LibRaw instead.
        assert_eq!(read_gps_head(b"ftypcrx not a tiff".to_vec()), None);
    }
}

#[cfg(test)]
mod colour_space_tests {
    use super::*;
    use exif::{experimental::Writer, Field, Value};

    /// A TIFF with `ColorSpace`, an interoperability index and a make, each
    /// only when given — the combinations cameras actually write.
    fn tiff(space: Option<u16>, interop: Option<&[u8]>, make: Option<&str>) -> Vec<u8> {
        let mut fields = Vec::new();
        if let Some(s) = space {
            fields.push(Field {
                tag: Tag::ColorSpace,
                ifd_num: In::PRIMARY,
                value: Value::Short(vec![s]),
            });
        }
        if let Some(i) = interop {
            fields.push(Field {
                tag: Tag::InteroperabilityIndex,
                ifd_num: In::PRIMARY,
                value: Value::Ascii(vec![i.to_vec()]),
            });
        }
        if let Some(m) = make {
            fields.push(Field {
                tag: Tag::Make,
                ifd_num: In::PRIMARY,
                value: Value::Ascii(vec![m.as_bytes().to_vec()]),
            });
        }
        let mut w = Writer::new();
        for f in &fields {
            w.push_field(f);
        }
        let mut out = std::io::Cursor::new(Vec::new());
        w.write(&mut out, false).unwrap();
        out.into_inner()
    }

    /// The DCF way: "R03" means Adobe RGB, whatever `ColorSpace` says.
    #[test]
    fn the_dcf_adobe_rgb_index_is_read() {
        let t = tiff(Some(0xFFFF), Some(b"R03"), Some("SONY"));
        assert_eq!(read(&t).colour_space, Some(ColourSpace::AdobeRgb));
    }

    #[test]
    fn an_srgb_camera_file_is_srgb() {
        let t = tiff(Some(1), Some(b"R98"), Some("SONY"));
        assert_eq!(read(&t).colour_space, Some(ColourSpace::Srgb));
    }

    /// Canon's Adobe RGB files say "uncalibrated" with an "R98" index; a
    /// camera has no third choice, so uncalibrated is Adobe RGB.
    #[test]
    fn canons_uncalibrated_is_adobe_rgb() {
        let t = tiff(Some(0xFFFF), Some(b"R98"), Some("Canon"));
        assert_eq!(read(&t).colour_space, Some(ColourSpace::AdobeRgb));
    }

    /// Without a make it is not a camera's file, and "uncalibrated" says
    /// nothing about which space it is.
    #[test]
    fn uncalibrated_software_output_is_not_guessed() {
        let t = tiff(Some(0xFFFF), None, None);
        assert_eq!(read(&t).colour_space, None);
        assert_eq!(read(&tiff(None, None, Some("SONY"))).colour_space, None);
    }

    /// A Nikon maker note: "Nikon\0", version, padding, then its own TIFF with
    /// tag 0x001E, ColorSpace.
    fn nikon_note(space: u16) -> Vec<u8> {
        let mut note = b"Nikon\0\x02\x10\0\0".to_vec();
        note.extend_from_slice(b"II*\0");
        note.extend_from_slice(&8u32.to_le_bytes());
        note.extend_from_slice(&1u16.to_le_bytes());
        note.extend_from_slice(&0x001Eu16.to_le_bytes());
        note.extend_from_slice(&3u16.to_le_bytes());
        note.extend_from_slice(&1u32.to_le_bytes());
        note.extend_from_slice(&space.to_le_bytes());
        note.extend_from_slice(&[0, 0]);
        note.extend_from_slice(&0u32.to_le_bytes());
        note
    }

    /// A RAF or RW2 keeps a JPEG near its start with EXIF of its own; the
    /// colour space is found there without extracting the preview.
    #[test]
    fn a_preview_near_the_start_of_a_raw_is_read() {
        let exif = tiff(Some(0xFFFF), Some(b"R03"), Some("FUJIFILM"));
        let mut raw = b"FUJIFILMCCD-RAW 0201".to_vec();
        raw.extend_from_slice(&[0u8; 200]);
        raw.extend_from_slice(&[0xFF, 0xD8, 0xFF, 0xE1]);
        raw.extend_from_slice(&((2 + 6 + exif.len()) as u16).to_be_bytes());
        raw.extend_from_slice(b"Exif\0\0");
        raw.extend_from_slice(&exif);
        raw.extend_from_slice(&[0u8; 64]);
        assert_eq!(embedded_colour_space(&raw), Some(ColourSpace::AdobeRgb));
        assert_eq!(embedded_colour_space(&raw[..100]), None);
    }

    /// A NEF says it only in its maker note.
    #[test]
    fn a_nikon_maker_note_is_read() {
        for (space, want) in [(1, ColourSpace::Srgb), (2, ColourSpace::AdobeRgb)] {
            let mut w = Writer::new();
            let make = Field {
                tag: Tag::Make,
                ifd_num: In::PRIMARY,
                value: Value::Ascii(vec![b"NIKON CORPORATION".to_vec()]),
            };
            let note = Field {
                tag: Tag::MakerNote,
                ifd_num: In::PRIMARY,
                value: Value::Undefined(nikon_note(space), 0),
            };
            w.push_field(&make);
            w.push_field(&note);
            let mut out = std::io::Cursor::new(Vec::new());
            w.write(&mut out, false).unwrap();
            assert_eq!(read(&out.into_inner()).colour_space, Some(want));
        }
    }

    /// Whatever bytes a date field holds, reading it never panics: a
    /// corrupted or crafted one is simply no date.
    #[test]
    fn a_date_of_any_bytes_never_panics() {
        assert_eq!(
            normalise_datetime("2024:05:01 12:00:00").as_deref(),
            Some("2024-05-01 12:00:00")
        );
        assert_eq!(normalise_datetime("2024:05:0\u{FFFD} 12:00:00"), None);
        assert_eq!(normalise_datetime("2024:05:0é 12:00:00"), None);
        // Every two-byte window of a date, each byte replaced: as lossy UTF-8
        // decodes them, which is how the field reaches here.
        let good = b"2024:05:01 12:00:00".to_vec();
        for i in 0..good.len() {
            for b in [0x80u8, 0xC3, 0xE2, 0xFF, 0x00] {
                let mut bad = good.clone();
                bad[i] = b;
                let _ = normalise_datetime(&String::from_utf8_lossy(&bad));
            }
        }
    }
}
