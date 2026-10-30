//! DNG colour, end to end through LibRaw, on DNGs written here.
//!
//! No sample DNG is needed, and none is shipped: each test writes a small
//! uncompressed DNG whose camera behaves exactly as its own calibration says
//! — raw values are the file's colour matrix applied to known XYZ — so the
//! colour the pipeline recovers has a right answer. That checks LibRaw's
//! parsing of the tags as well as `raw::colour`'s use of them, which a unit
//! test of the arithmetic alone cannot.

#![cfg(feature = "libraw")]

use sourcelight_lib::gpu::color::{self, Mat3};
use sourcelight_lib::raw::colour::{bradford, D65};
use sourcelight_lib::raw::decoder::{invert3, CfaPattern, RawDecoder, RawImage};
use sourcelight_lib::raw::libraw::LibRawDecoder;
use sourcelight_lib::test_support::Scratch;

/// A daylight calibration for the synthetic camera (the a6400's published
/// matrix) and a tungsten one that differs the way a StdA matrix does.
const CM_D65: Mat3 = [
    [0.7657, -0.2847, -0.0607],
    [-0.4083, 1.1966, 0.2389],
    [-0.0684, 0.1418, 0.5844],
];
const CM_A: Mat3 = [
    [0.8100, -0.3500, -0.0400],
    [-0.3900, 1.1600, 0.2700],
    [-0.0300, 0.1100, 0.7200],
];

const XY_D65: [f32; 2] = [0.3127, 0.3290];
const XY_A: [f32; 2] = [0.4476, 0.4074];

fn xy_to_xyz(xy: [f32; 2]) -> [f32; 3] {
    [xy[0] / xy[1], 1.0, (1.0 - xy[0] - xy[1]) / xy[1]]
}

/// Sky, foliage, skin and a mid grey, as XYZ under D65 (from sRGB).
fn patches() -> [[f32; 3]; 4] {
    let srgb_to_xyz = sourcelight_lib::raster::SRGB_TO_XYZ;
    let lin = |v: f32| sourcelight_lib::raster::srgb_to_linear(v);
    [
        [0.28, 0.45, 0.85],
        [0.10, 0.35, 0.08],
        [0.75, 0.45, 0.35],
        [0.46, 0.46, 0.46],
    ]
    .map(|c| color::apply(srgb_to_xyz, c.map(lin)))
}

#[derive(Clone)]
struct Dng {
    linear: bool,
    /// (illuminant, colour matrix, forward matrix)
    calibrations: Vec<(u16, Mat3, Option<Mat3>)>,
    as_shot: [f32; 3],
    black: [u16; 3],
    white: u16,
    baseline: Option<f32>,
    /// Per pixel, the patch it shows, as raw camera RGB above black, 0..1 of
    /// the range.
    patch_rgb: [[f32; 3]; 4],
}

const SIZE: u32 = 32;

/// Which of the four quadrant patches a pixel shows.
fn patch_at(x: u32, y: u32) -> usize {
    ((y >= SIZE / 2) as usize) * 2 + (x >= SIZE / 2) as usize
}

fn entry(out: &mut Vec<(u16, u16, u32, Vec<u8>)>, tag: u16, kind: u16, count: u32, data: Vec<u8>) {
    out.push((tag, kind, count, data));
}

fn shorts(v: &[u16]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}
fn longs(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}
fn srationals(v: &[f32]) -> Vec<u8> {
    v.iter()
        .flat_map(|x| {
            let mut b = ((x * 10000.0).round() as i32).to_le_bytes().to_vec();
            b.extend_from_slice(&10000i32.to_le_bytes());
            b
        })
        .collect()
}
fn rationals(v: &[f32]) -> Vec<u8> {
    v.iter()
        .flat_map(|x| {
            let mut b = ((x * 1_000_000.0).round() as u32).to_le_bytes().to_vec();
            b.extend_from_slice(&1_000_000u32.to_le_bytes());
            b
        })
        .collect()
}
fn ascii(s: &str) -> Vec<u8> {
    let mut b = s.as_bytes().to_vec();
    b.push(0);
    b
}
fn flat(m: &Mat3) -> Vec<f32> {
    m.iter().flatten().copied().collect()
}

/// An uncompressed little-endian DNG of `SIZE` x `SIZE`: RGGB mosaic, or
/// three samples a pixel when linear.
fn write(dng: &Dng) -> Vec<u8> {
    let spp: u32 = if dng.linear { 3 } else { 1 };
    let mut pixels = Vec::with_capacity((SIZE * SIZE * spp * 2) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let rgb = dng.patch_rgb[patch_at(x, y)];
            let value = |c: usize| {
                let range = (dng.white - dng.black[c]) as f32;
                (dng.black[c] as f32 + rgb[c] * range)
                    .round()
                    .clamp(0.0, dng.white as f32) as u16
            };
            if dng.linear {
                for c in 0..3 {
                    pixels.extend_from_slice(&value(c).to_le_bytes());
                }
            } else {
                let c = [[0, 1], [1, 2]][(y % 2) as usize][(x % 2) as usize];
                pixels.extend_from_slice(&value(c).to_le_bytes());
            }
        }
    }

    let mut e = Vec::new();
    entry(&mut e, 254, 4, 1, longs(&[0]));
    entry(&mut e, 256, 4, 1, longs(&[SIZE]));
    entry(&mut e, 257, 4, 1, longs(&[SIZE]));
    entry(&mut e, 258, 3, spp, shorts(&vec![16; spp as usize]));
    entry(&mut e, 259, 3, 1, shorts(&[1]));
    entry(
        &mut e,
        262,
        3,
        1,
        shorts(&[if dng.linear { 34892 } else { 32803 }]),
    );
    entry(&mut e, 271, 2, 11, ascii("SourceLight"));
    entry(&mut e, 272, 2, 10, ascii("Synthetic"));
    entry(&mut e, 273, 4, 1, longs(&[0])); // patched below
    entry(&mut e, 274, 3, 1, shorts(&[1]));
    entry(&mut e, 277, 3, 1, shorts(&[spp as u16]));
    entry(&mut e, 278, 4, 1, longs(&[SIZE]));
    entry(&mut e, 279, 4, 1, longs(&[pixels.len() as u32]));
    entry(&mut e, 284, 3, 1, shorts(&[1]));
    if !dng.linear {
        entry(&mut e, 33421, 3, 2, shorts(&[2, 2]));
        entry(&mut e, 33422, 1, 4, vec![0, 1, 1, 2]);
    }
    entry(&mut e, 50706, 1, 4, vec![1, 4, 0, 0]);
    entry(&mut e, 50707, 1, 4, vec![1, 1, 0, 0]);
    entry(&mut e, 50708, 2, 21, ascii("SourceLight Synthetic"));
    if dng.linear {
        entry(&mut e, 50714, 3, 3, shorts(&dng.black));
        entry(&mut e, 50717, 3, 3, shorts(&[dng.white; 3]));
    } else {
        entry(&mut e, 50714, 3, 1, shorts(&[dng.black[1]]));
        entry(&mut e, 50717, 3, 1, shorts(&[dng.white]));
    }
    let tags = [(50721u16, 50778u16, 50964u16), (50722, 50779, 50965)];
    for (i, (ill, cm, _)) in dng.calibrations.iter().enumerate() {
        entry(&mut e, tags[i].0, 10, 9, srationals(&flat(cm)));
        let _ = ill;
    }
    entry(&mut e, 50728, 5, 3, rationals(&dng.as_shot));
    if let Some(b) = dng.baseline {
        entry(&mut e, 50730, 10, 1, srationals(&[b]));
    }
    for (i, (ill, _, _)) in dng.calibrations.iter().enumerate() {
        entry(&mut e, tags[i].1, 3, 1, shorts(&[*ill]));
    }
    for (i, (_, _, fm)) in dng.calibrations.iter().enumerate() {
        if let Some(fm) = fm {
            entry(&mut e, tags[i].2, 10, 9, srationals(&flat(fm)));
        }
    }
    e.sort_by_key(|(tag, ..)| *tag);

    // Header, IFD, then the values that do not fit in an entry, then pixels.
    let ifd_len = 2 + e.len() * 12 + 4;
    let mut extra = Vec::new();
    let extra_at = 8 + ifd_len;
    let mut ifd = (e.len() as u16).to_le_bytes().to_vec();
    let mut strip_entry = 0;
    for (i, (tag, kind, count, data)) in e.iter().enumerate() {
        ifd.extend_from_slice(&tag.to_le_bytes());
        ifd.extend_from_slice(&kind.to_le_bytes());
        ifd.extend_from_slice(&count.to_le_bytes());
        if *tag == 273 {
            strip_entry = 8 + 2 + i * 12 + 8;
        }
        if data.len() <= 4 {
            let mut v = data.clone();
            v.resize(4, 0);
            ifd.extend_from_slice(&v);
        } else {
            ifd.extend_from_slice(&((extra_at + extra.len()) as u32).to_le_bytes());
            extra.extend_from_slice(data);
            if extra.len() % 2 == 1 {
                extra.push(0);
            }
        }
    }
    ifd.extend_from_slice(&0u32.to_le_bytes());
    let mut out = b"II*\0".to_vec();
    out.extend_from_slice(&8u32.to_le_bytes());
    out.extend_from_slice(&ifd);
    out.extend_from_slice(&extra);
    let pixels_at = out.len() as u32;
    out[strip_entry..strip_entry + 4].copy_from_slice(&pixels_at.to_le_bytes());
    out.extend_from_slice(&pixels);
    out
}

/// The DNG model's camera: raw values are `XYZ → camera` at the scene's
/// illuminant, applied to each patch's XYZ under that illuminant (Bradford
/// from D65, standing in for the surfaces' own reflectance). Returned with the
/// camera neutral, green = 1, and scaled so the brightest channel of the
/// brightest patch sits at 60% of the range.
fn camera_of(xyz_to_camera: Mat3, white_xy: [f32; 2]) -> ([[f32; 3]; 4], [f32; 3]) {
    let white = xy_to_xyz(white_xy);
    let to_scene = bradford(D65, white);
    let raw = patches().map(|p| color::apply(xyz_to_camera, color::apply(to_scene, p)));
    let neutral = color::apply(xyz_to_camera, white);
    let peak = raw.iter().flatten().cloned().fold(0.0f32, f32::max);
    (
        raw.map(|p| p.map(|v| v * 0.6 / peak)),
        [neutral[0] / neutral[1], 1.0, neutral[2] / neutral[1]],
    )
}

fn decode(dng: &Dng, name: &str) -> RawImage {
    let dir = Scratch::new("dng_colour");
    let path = dir.join(name);
    std::fs::write(&path, write(dng)).unwrap();
    LibRawDecoder::new()
        .decode(&path)
        .unwrap_or_else(|e| panic!("{name}: {e:#}"))
}

/// Each patch as the pipeline's working space would hold it, from the decoded
/// frame alone: levels, the as-shot balance normalised to green (as the mosaic
/// pass does), then the camera matrix.
fn working_patches(raw: &RawImage) -> [[f32; 3]; 4] {
    let m = color::camera_to_working(raw.cam_to_xyz).expect("matrix inverts");
    let samples = raw.cfa_pattern.samples() as usize;
    let a = raw.active_area;
    std::array::from_fn(|p| {
        // The middle of the patch's quadrant, averaged over one mosaic tile.
        let (qx, qy) = (
            (p % 2) as u32 * SIZE / 2 + SIZE / 4,
            (p / 2) as u32 * SIZE / 2 + SIZE / 4,
        );
        let mut sum = [0f32; 3];
        let mut count = [0f32; 3];
        for dy in 0..2 {
            for dx in 0..2 {
                let (x, y) = (a.x + qx + dx, a.y + qy + dy);
                let at = ((y * raw.width + x) as usize) * samples;
                let colours: Vec<(usize, usize)> = match raw.cfa_pattern {
                    CfaPattern::Linear => (0..3).map(|c| (c, at + c)).collect(),
                    _ => {
                        let c = raw.cfa_pattern.color_at(qx + dx, qy + dy) as usize;
                        vec![(if c == 3 { 1 } else { c }, at)]
                    }
                };
                for (c, i) in colours {
                    let v = (raw.cfa[i] as f32 - raw.black[c] as f32)
                        / (raw.white[c] as f32 - raw.black[c] as f32);
                    sum[c] += v * raw.wb_coeffs[c] / raw.wb_coeffs[1];
                    count[c] += 1.0;
                }
            }
        }
        let camera = std::array::from_fn(|c| sum[c] / count[c]);
        color::apply(m, camera)
    })
}

/// The working-space colours the scene should come out as: each patch under
/// D65, relative to the grey patch so exposure drops out.
fn expected() -> [[f32; 3]; 4] {
    patches().map(|p| color::apply(color::XYZ_TO_REC2020, p))
}

/// Both sets relative to their grey patch, so exposure drops out.
fn relative(got: [[f32; 3]; 4]) -> ([[f32; 3]; 4], [[f32; 3]; 4]) {
    let scale = |set: [[f32; 3]; 4]| {
        let grey = set[3][1];
        set.map(|p| p.map(|v| v / grey))
    };
    (scale(got), scale(expected()))
}

/// The largest error in any channel of any patch.
fn worst_error(got: [[f32; 3]; 4]) -> f32 {
    let (got, want) = relative(got);
    (0..4)
        .flat_map(|p| (0..3).map(move |c| (p, c)))
        .map(|(p, c)| (got[p][c] - want[p][c]).abs())
        .fold(0.0, f32::max)
}

fn assert_matches(got: [[f32; 3]; 4], label: &str, tolerance: f32) {
    let error = worst_error(got);
    let (got, want) = relative(got);
    assert!(
        error < tolerance,
        "{label}: off by {error:.4}; came out {got:?}, expected {want:?}"
    );
}

fn daylight_dng() -> Dng {
    let (patch_rgb, neutral) = camera_of(CM_D65, XY_D65);
    Dng {
        linear: false,
        calibrations: vec![(21, CM_D65, None)],
        as_shot: neutral,
        black: [256; 3],
        white: 16383,
        baseline: None,
        patch_rgb,
    }
}

#[test]
fn a_daylight_dng_renders_its_colours() {
    let raw = decode(&daylight_dng(), "daylight.dng");
    assert!(matches!(raw.cfa_pattern, CfaPattern::Bayer(_)));
    assert_eq!(raw.black, [256; 4]);
    assert_eq!(raw.baseline_exposure, 0.0);
    assert_matches(working_patches(&raw), "daylight", 0.01);
}

/// Shot under tungsten, with a tungsten and a daylight calibration. The file
/// says which matrix fits the scene; using the daylight one alone, as the
/// decoder used to, misrenders every colour that is not grey.
#[test]
fn a_tungsten_dng_uses_its_tungsten_calibration() {
    let (patch_rgb, neutral) = camera_of(CM_A, XY_A);
    let dng = Dng {
        calibrations: vec![(17, CM_A, None), (21, CM_D65, None)],
        as_shot: neutral,
        patch_rgb,
        ..daylight_dng()
    };
    let mut raw = decode(&dng, "tungsten.dng");
    assert_matches(working_patches(&raw), "tungsten", 0.01);

    // What the decoder used to do — the second matrix, "D65 by convention",
    // and nothing else — is visibly off on the same frame, so the check
    // above is one the old behaviour would have failed.
    raw.cam_to_xyz = invert3(CM_D65).unwrap();
    let old = worst_error(working_patches(&raw));
    assert!(old > 0.1, "the daylight matrix alone was off by only {old}");
}

/// A DNG with forward matrices is rendered through them. Built consistent
/// with its colour matrix, so the right answer is still the scene's colour.
#[test]
fn a_dng_with_a_forward_matrix_renders_through_it() {
    use sourcelight_lib::raw::colour::D50;
    let (patch_rgb, neutral) = camera_of(CM_D65, XY_D65);
    // FM = Bradford(D65 -> D50) * inverse(CM) * diag(neutral): balanced
    // camera white to D50, as the specification requires.
    let n = color::apply(CM_D65, xy_to_xyz(XY_D65));
    let fm = color::multiply(
        bradford(xy_to_xyz(XY_D65), D50),
        color::multiply(
            invert3(CM_D65).unwrap(),
            [[n[0], 0.0, 0.0], [0.0, n[1], 0.0], [0.0, 0.0, n[2]]],
        ),
    );
    let dng = Dng {
        calibrations: vec![(21, CM_D65, Some(fm))],
        as_shot: neutral,
        patch_rgb,
        ..daylight_dng()
    };
    let raw = decode(&dng, "forward.dng");
    assert_matches(working_patches(&raw), "forward matrix", 0.01);
}

/// A phone's DNG says how much brighter it is meant to be shown.
#[test]
fn baseline_exposure_is_read() {
    let dng = Dng {
        baseline: Some(1.25),
        ..daylight_dng()
    };
    assert!((decode(&dng, "baseline.dng").baseline_exposure - 1.25).abs() < 1e-3);
}

/// Through the GPU: the linear frame skips the mosaic pass, and has to come
/// out as the mosaic frame of the same scene does.
#[test]
fn a_linear_dng_renders_as_the_mosaic_does() {
    use sourcelight_lib::develop::EditStack;
    use sourcelight_lib::gpu::pipeline::Pipeline;
    let Some(gpu) = sourcelight_lib::gpu::test_support::hardware_gpu() else {
        return;
    };
    let pipeline = Pipeline::new(gpu);
    let render = |raw: &RawImage| {
        let session = pipeline.prepare(gpu, raw).expect("prepare");
        pipeline
            .render_to_pixels(gpu, &session, &EditStack::default(), SIZE, SIZE)
            .expect("render")
    };
    let mosaic = render(&decode(&daylight_dng(), "mosaic.dng"));
    let linear = render(&decode(
        &Dng {
            linear: true,
            ..daylight_dng()
        },
        "linear-render.dng",
    ));
    for p in 0..4 {
        let (x, y) = (
            (p % 2) as u32 * SIZE / 2 + SIZE / 4,
            (p / 2) as u32 * SIZE / 2 + SIZE / 4,
        );
        let i = ((y * SIZE + x) * 4) as usize;
        for c in 0..3 {
            assert!(
                (mosaic[i + c] as i32 - linear[i + c] as i32).abs() <= 2,
                "patch {p}: mosaic {:?}, linear {:?}",
                &mosaic[i..i + 3],
                &linear[i..i + 3]
            );
        }
    }
}

/// A linear DNG — three samples a pixel, no mosaic — opens, with its own
/// black level per sample, and renders the same colours as a mosaic one.
#[test]
fn a_linear_dng_opens_in_its_colours() {
    let dng = Dng {
        linear: true,
        black: [240, 256, 272],
        ..daylight_dng()
    };
    let raw = decode(&dng, "linear.dng");
    assert!(matches!(raw.cfa_pattern, CfaPattern::Linear));
    assert_eq!(&raw.black[..3], &[240, 256, 272]);
    assert_matches(working_patches(&raw), "linear", 0.01);
}
