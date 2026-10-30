//! Whether one warmth means one thing on every camera, measured on real ones.
//!
//! ```sh
//! SOURCELIGHT_TEST_RAW=/path/to/raws cargo test --test white_balance -- --nocapture
//! ```

#![cfg(feature = "libraw")]

use sourcelight_lib::develop::white_balance::{adaptation, placement};
use sourcelight_lib::develop::EditStack;
use sourcelight_lib::gpu::color::{self, Mat3};
use sourcelight_lib::raw::decoder::{invert3, CfaPattern, RawDecoder};
use sourcelight_lib::raw::libraw::LibRawDecoder;
use sourcelight_lib::test_support::{self, Needs};

/// Where a grey that was neutral as shot lands in the working space, in
/// mireds and Δuv from where it was: what the photographer sees the slider
/// do to it.
fn moved(neutral: [f32; 3], now: [f32; 3]) -> (f32, f32) {
    let to_xyz = invert3(color::XYZ_TO_REC2020).unwrap();
    let (m0, d0) = placement(color::apply(to_xyz, neutral));
    let (m, d) = placement(color::apply(to_xyz, now));
    (m - m0, d - d0)
}

fn spread(v: &[f32]) -> f32 {
    v.iter().cloned().fold(f32::MIN, f32::max) - v.iter().cloned().fold(f32::MAX, f32::min)
}

/// Warmth 0.5 on each fixture camera, balanced the old way (scaling the
/// camera's red and blue before its matrix) and the new (adapting from a
/// bluer light after it), as `DevelopSession::white_balance` does each.
///
/// The old way's warm grey landed wherever that sensor's red and blue happen
/// to point: 48 mireds apart across the fixtures, and pushed off the locus
/// towards green or magenta by up to 0.006 Δuv as well as warmer. The new
/// way's is one grey on every camera.
#[test]
fn one_warmth_is_one_warmth_on_every_camera() {
    let Some(dir) = test_support::raw_dir() else {
        test_support::skip(
            Needs::Raw,
            "set SOURCELIGHT_TEST_RAW to a folder of raw files",
        );
        return;
    };
    let decoder = LibRawDecoder::new();
    let exts = decoder.extensions();
    let old = EditStack {
        version: 2,
        warmth: 0.5,
        ..EditStack::default()
    };
    let new = adaptation(0.5, 0.0);
    let mut old_moves: Vec<(f32, f32)> = Vec::new();
    let mut new_moves: Vec<(f32, f32)> = Vec::new();
    eprintln!(
        "{:<24} {:>18} {:>18}",
        "warmth 0.5", "old: mireds, Δuv", "new: mireds, Δuv"
    );
    for path in test_support::files_in(&dir, |e| exts.contains(&e)) {
        let Ok(raw) = decoder.decode(&path) else {
            continue;
        };
        // A monochrome sensor has no colour to balance.
        if raw.cfa_pattern == CfaPattern::Bayer([1; 4]) {
            continue;
        }
        let Some(camera): Option<Mat3> = color::camera_to_working(raw.cam_to_xyz) else {
            continue;
        };
        let neutral = color::apply(camera, [1.0; 3]);
        let (om, od) = moved(neutral, color::apply(camera, old.wb_scale()));
        let (nm, nd) = moved(neutral, color::apply(new, neutral));
        eprintln!(
            "{:<24} {om:>9.1} {od:>+8.4} {nm:>9.1} {nd:>+8.4}",
            path.file_name().unwrap().to_string_lossy()
        );
        old_moves.push((om, od));
        new_moves.push((nm, nd));
    }
    if new_moves.is_empty() {
        return;
    }
    let mireds = |v: &[(f32, f32)]| spread(&v.iter().map(|m| m.0).collect::<Vec<_>>());
    let duv = |v: &[(f32, f32)]| spread(&v.iter().map(|m| m.1).collect::<Vec<_>>());
    eprintln!(
        "\nacross cameras: old lands {:.1} mireds and {:.4} Δuv apart, new {:.1} and {:.4}",
        mireds(&old_moves),
        duv(&old_moves),
        mireds(&new_moves),
        duv(&new_moves)
    );
    assert!(
        mireds(&new_moves) < 1.0 && duv(&new_moves) < 5e-4,
        "one warmth lands {:.1} mireds and {:.4} Δuv apart from camera to camera",
        mireds(&new_moves),
        duv(&new_moves)
    );
    // And it warms as far as the old sliders did on a typical body.
    let typical = {
        let mut m: Vec<f32> = old_moves.iter().map(|m| m.0).collect();
        m.sort_by(f32::total_cmp);
        m[m.len() / 2]
    };
    assert!(
        (new_moves[0].0 - typical).abs() < 0.25 * typical,
        "warmth 0.5 moves the grey {:.1} mireds where the old sliders moved it {typical:.1}",
        new_moves[0].0
    );
}
