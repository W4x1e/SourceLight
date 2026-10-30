//! Full sensor decode against real raw files.
//!
//! Raw files are far too large to commit, so this is opt-in. Point
//! `SOURCELIGHT_TEST_RAW` at a folder of raw files — any mix of formats:
//!
//! ```sh
//! SOURCELIGHT_TEST_RAW=/path/to/raws cargo test --test raw_decode -- --nocapture
//! ```
//!
//! Multi-camera support that is never exercised is not support, so this walks
//! whatever it is given rather than assuming Sony.

#![cfg(feature = "libraw")]

use sourcelight_lib::raw::decoder::{CfaPattern, RawDecoder};
use sourcelight_lib::raw::libraw::LibRawDecoder;
use sourcelight_lib::test_support::{self, Needs};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The raw-fixture folder. A path that is set but missing fails here rather than
/// reading as "no fixtures" — see `test_support`.
fn sample_dir() -> Option<PathBuf> {
    test_support::raw_dir()
}

fn raw_files(dir: &PathBuf, decoder: &LibRawDecoder) -> Vec<PathBuf> {
    let exts = decoder.extensions();
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if exts.contains(&ext.to_ascii_lowercase().as_str()) {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// Decodes every raw in the sample folder. `RawImage::validate` does the heavy
/// lifting — it rejects short buffers, impossible black/white levels and active
/// areas that fall outside the sensor, all of which otherwise surface much
/// later as a black or torn frame in the GPU pipeline.
#[test]
fn decodes_every_sample_raw() {
    let Some(dir) = sample_dir() else {
        test_support::skip(
            Needs::Raw,
            "set SOURCELIGHT_TEST_RAW to a folder of raw files",
        );
        return;
    };

    let decoder = LibRawDecoder::new();
    eprintln!("{}", decoder.describe());

    let files = raw_files(&dir, &decoder);
    assert!(!files.is_empty(), "no raw files found in {}", dir.display());

    let mut by_model: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures = Vec::new();

    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        match decoder.decode(path) {
            Ok(img) => {
                // validate() already ran inside decode(); assert the things a
                // caller actually depends on being sane.
                assert!(
                    img.width >= 1000 && img.height >= 1000,
                    "{name}: implausible sensor size {}x{}",
                    img.width,
                    img.height
                );
                assert!(
                    img.white.iter().all(|&w| w > 255),
                    "{name}: white level {:?} looks like an 8-bit value",
                    img.white
                );
                // All four, including the second green — a zero there would
                // multiply half the green photosites to black.
                assert!(
                    img.wb_coeffs.iter().all(|c| *c > 0.0),
                    "{name}: white balance {:?} has a zero channel",
                    img.wb_coeffs
                );
                // A camera->XYZ matrix of all zeros means LibRaw had no colour
                // data for this body; the pipeline would render grey mush.
                let matrix_sum: f32 = img.cam_to_xyz.iter().flatten().map(|v| v.abs()).sum();
                assert!(matrix_sum > 0.0, "{name}: cam_to_xyz is all zeros");

                *by_model
                    .entry(format!("{} {}", img.make.trim(), img.model.trim()))
                    .or_default() += 1;

                let tile = match &img.cfa_pattern {
                    CfaPattern::Bayer(_) => "Bayer",
                    CfaPattern::XTrans(_) => "X-Trans",
                    CfaPattern::Linear => "linear",
                };
                eprintln!(
                    "{name}: {}x{} {tile} black={:?} white={:?} wb={:.2?} orient={}",
                    img.width, img.height, img.black, img.white, img.wb_coeffs, img.orientation
                );
            }
            Err(e) => failures.push(format!("{name}: {e}")),
        }
    }

    eprintln!("\ndecoded {} files", files.len() - failures.len());
    for (model, n) in &by_model {
        eprintln!("  {model}: {n}");
    }

    assert!(
        failures.is_empty(),
        "{} of {} files failed to decode:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
}

/// The sensor data must actually resemble the picture the camera made.
///
/// A decoder can return a perfectly well-formed frame that is nonetheless
/// wrong — wrong stride, wrong bit shift, wrong plane — and structural checks
/// will not notice. Comparing a crude CPU demosaic against the camera's own
/// embedded preview catches that: the two are produced by completely different
/// code paths, so agreement is meaningful.
#[test]
fn decoded_pixels_track_the_camera_preview() {
    let Some(dir) = sample_dir() else {
        test_support::skip(
            Needs::Raw,
            "set SOURCELIGHT_TEST_RAW to a folder of raw files",
        );
        return;
    };

    let decoder = LibRawDecoder::new();
    let files = raw_files(&dir, &decoder);
    assert!(!files.is_empty(), "no raw files found in {}", dir.display());

    // A few files is plenty — this is about correctness of the mapping, not coverage.
    for path in files.iter().take(3) {
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        let img = decoder.decode(path).expect("decode");
        let preview_bytes = decoder.embedded_preview(path).expect("preview");
        let preview = image::load_from_memory(&preview_bytes)
            .expect("preview decodes")
            .to_luma8();

        // Average the green photosites inside the active area, normalised over
        // the black-to-white range. Green carries most of the luminance and is
        // half the Bayer tile, so it is the cheapest honest proxy for brightness.
        let a = img.active_area;
        let (mut sum, mut count) = (0.0f64, 0u64);
        let black = img.black[1] as f32;
        let range = (img.white[1] as f32 - black).max(1.0);
        // Sample whole mosaic tiles on a coarse grid, then take the green
        // photosites within each. Stepping by a multiple of the period instead
        // would only ever land on one phase of the mosaic — on an RGGB sensor
        // that is the red photosite, and the green count comes out zero.
        let period = img.cfa_pattern.period();
        let stride = (period * 4) as usize;
        for ty in (0..a.height.saturating_sub(period)).step_by(stride) {
            for tx in (0..a.width.saturating_sub(period)).step_by(stride) {
                for dy in 0..period {
                    for dx in 0..period {
                        let (x, y) = (tx + dx, ty + dy);
                        if img.cfa_pattern.color_at(x, y) != 1 {
                            continue;
                        }
                        let Some(v) = img.at(a.x + x, a.y + y) else {
                            continue;
                        };
                        sum += (((v as f32) - black) / range).clamp(0.0, 1.0) as f64;
                        count += 1;
                    }
                }
            }
        }
        assert!(count > 100, "{name}: sampled only {count} green photosites");
        let raw_mean = sum / count as f64;

        // The preview is gamma-encoded and the raw is linear, so undo roughly
        // sRGB's 2.2 to get them into comparable territory.
        let preview_mean = preview.pixels().map(|p| p.0[0] as f64 / 255.0).sum::<f64>()
            / preview.pixels().len() as f64;
        let raw_gamma = raw_mean.powf(1.0 / 2.2);

        eprintln!(
            "{name}: raw(linear)={raw_mean:.4} raw(gamma)={raw_gamma:.4} preview={preview_mean:.4}"
        );

        // Deliberately loose. The camera applies its own tone curve, white
        // balance and picture profile, so these will never match closely — but
        // a decode that is genuinely broken lands orders of magnitude out, and
        // that is what this catches.
        assert!(
            (raw_gamma - preview_mean).abs() < 0.35,
            "{name}: decoded brightness {raw_gamma:.4} is nowhere near the camera preview {preview_mean:.4} — \
             suspect a wrong stride, bit depth or plane"
        );
    }
}

/// Every advertised extension should be one LibRaw actually knows, and the list
/// must stay lowercase and dot-free since callers compare against it directly.
#[test]
fn advertised_extensions_are_well_formed() {
    let decoder = LibRawDecoder::new();
    let exts = decoder.extensions();
    assert!(!exts.is_empty());
    for ext in exts {
        assert!(!ext.starts_with('.'), "{ext} should not include the dot");
        assert_eq!(*ext, ext.to_ascii_lowercase(), "{ext} should be lowercase");
    }
    assert!(exts.contains(&"arw"), "Sony ARW must be supported");
    assert!(exts.contains(&"dng"), "DNG must be supported");
}

/// Metadata, not just pixels — the half of ingest that decides whether a photo
/// is findable at all.
///
/// `exif_read` is built on a TIFF reader, and its own doc says so. CR3 is
/// ISO-BMFF and RAF, ORF and RW2 are not plain TIFF either, so for four of the
/// six sample formats it returns nothing — and ingest used to have no second
/// source, which meant those photos arrived with no capture date, fell out of
/// every date filter, and never appeared in a trip cluster.
///
/// So this asserts on `scan`'s own path rather than on `exif_read` directly:
/// what matters is not which reader answered but whether the photo ends up
/// findable.
#[test]
fn every_sample_imports_with_a_date_a_camera_and_real_dimensions() {
    let Some(dir) = sample_dir() else {
        test_support::skip(
            Needs::Raw,
            "set SOURCELIGHT_TEST_RAW to a folder of raw files",
        );
        return;
    };
    let decoder = LibRawDecoder::new();

    let mut bad = Vec::new();
    for path in raw_files(&dir, &decoder) {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let meta = sourcelight_lib::scan::metadata_for(&path);
        eprintln!(
            "{name}: date={:?} camera={:?} lens={:?} iso={:?} {}x{}",
            meta.capture_date,
            meta.camera,
            meta.lens,
            meta.iso,
            meta.width.unwrap_or(0),
            meta.height.unwrap_or(0),
        );

        if meta.capture_date.is_none() {
            bad.push(format!("{name}: no capture date"));
        }
        if meta.camera.is_none() {
            bad.push(format!("{name}: no camera"));
        }
        // The trap this is really guarding: EXIF can hand back the embedded
        // thumbnail's size, which is a wrong number that looks like a right
        // one. 160x120 passed an `is_some()` check for months.
        match (meta.width, meta.height) {
            (Some(w), Some(h)) if w >= 512 && h >= 512 => {}
            (w, h) => bad.push(format!("{name}: implausible size {w:?}x{h:?}")),
        }
    }

    assert!(
        bad.is_empty(),
        "metadata gaps:
  {}",
        bad.join(
            "
  "
        )
    );
}

/// Each fixture is read in the colour space it was shot in, and each maker
/// says so somewhere different — Fujifilm and Panasonic in their preview's own
/// EXIF, Sony in the raw's, Nikon only in its maker note, Canon and Olympus
/// where LibRaw reads them. Taking an sRGB one for Adobe RGB would
/// oversaturate every thumbnail from that camera, and the reverse flatten it,
/// so the scan's reading and the backfill's must both be right.
///
/// Most were shot in sRGB. The Pentax K-3 III Monochrome was set to Adobe RGB
/// (its maker note says so, in the DNG and the PEF). The phones' previews are
/// Display P3 and embed the profile that says so, which is honoured, and
/// which the backfill must not mistake their "uncalibrated" EXIF for Adobe
/// RGB over.
#[test]
fn every_sample_is_read_in_the_colour_space_it_was_shot_in() {
    use sourcelight_lib::exif_read::ColourSpace;
    let Some(dir) = sample_dir() else {
        test_support::skip(
            Needs::Raw,
            "set SOURCELIGHT_TEST_RAW to a folder of raw files",
        );
        return;
    };
    const ADOBE_RGB: &[&str] = &["pentax_k3iii_mono"];
    let decoder = LibRawDecoder::new();
    for path in raw_files(&dir, &decoder) {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let adobe = ADOBE_RGB.iter().any(|a| name.starts_with(a));
        let meta = sourcelight_lib::scan::metadata_for(&path);
        let preview = sourcelight_lib::scan::preview_of(&path).expect("preview");
        let profile = sourcelight_lib::raster::preview_profile(&preview, meta.colour_space);
        let stale = sourcelight_lib::scan::stale_colours(&path).expect("readable");
        let own_profile = {
            use image::ImageDecoder;
            image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(&preview))
                .ok()
                .and_then(|mut d| d.icc_profile().ok().flatten())
                .is_some()
        };
        eprintln!(
            "{name}: raw says {:?}, preview converted: {}{}",
            meta.colour_space,
            profile.is_some(),
            if own_profile {
                " (by its own profile)"
            } else {
                ""
            }
        );
        assert_eq!(
            meta.colour_space == Some(ColourSpace::AdobeRgb),
            adobe,
            "{name}: read as {:?}",
            meta.colour_space
        );
        assert_eq!(
            profile.is_some(),
            adobe || own_profile,
            "{name}: its preview {} be converted",
            if profile.is_some() {
                "would"
            } else {
                "would not"
            }
        );
        // The backfill remakes previews made before the colour space was
        // read — so the Adobe RGB ones, and only those.
        assert_eq!(
            stale,
            adobe.then_some(ColourSpace::AdobeRgb),
            "{name}: the backfill has it wrong"
        );
    }
}
