//! DNG opcodes that correct the sensor's shading: gain maps and radial
//! vignetting.
//!
//! A phone's lens darkens and tints the corners of every frame, and its DNG
//! carries the correction as data rather than applying it: a `GainMap` per
//! colour channel (a Pixel writes four, one per Bayer site), sometimes a
//! `FixVignetteRadial` polynomial (DJI). Without them a phone or drone DNG
//! opens with dark, colour-shifted corners the camera's own JPEG never had.
//! LibRaw parses the opcode lists but applies neither.
//!
//! Applied to the mosaic as decoded, before white balance and demosaic. For
//! `OpcodeList2` that is exactly where the DNG specification puts them. The
//! corrections in `OpcodeList3` are defined on the demosaiced image, but a
//! gain map and a vignette are smooth across the frame, so applying each to
//! the photosites of the colour it names gives the same result to within the
//! demosaic's own interpolation.
//!
//! Semantics follow the DNG SDK (`dng_gain_map.cpp`, `dng_lens_correction.cpp`):
//! map positions are fractions of the whole image (the active area), sampled
//! at pixel centres, interpolated bilinearly and held at the map's edges; a
//! gain never lifts a value past white. Other opcodes — geometry, bad pixels,
//! tables — are skipped: this module is about colour and tone only.

use rayon::prelude::*;

/// A rectangle of the image and the rows, columns and planes an opcode acts
/// on: `dng_area_spec`.
#[derive(Debug, Clone, PartialEq)]
pub struct Area {
    pub top: u32,
    pub left: u32,
    pub bottom: u32,
    pub right: u32,
    pub plane: u32,
    pub planes: u32,
    pub row_pitch: u32,
    pub col_pitch: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GainMap {
    pub area: Area,
    pub points: (u32, u32),
    pub spacing: (f64, f64),
    pub origin: (f64, f64),
    pub map_planes: u32,
    /// Row-major, planes innermost: `(row * points_h + col) * map_planes + plane`.
    pub gains: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Opcode {
    GainMap(GainMap),
    /// Gain `1 + k0 r² + k1 r⁴ + … + k4 r¹⁰`, `r` the distance from the optical
    /// centre as a fraction of the farthest corner's.
    FixVignetteRadial {
        k: [f64; 5],
        centre: (f64, f64),
    },
}

const GAIN_MAP: u32 = 9;
const FIX_VIGNETTE_RADIAL: u32 = 3;

/// Reads a DNG opcode list (always big-endian), keeping the opcodes this
/// module applies and skipping the rest. A malformed entry ends the list
/// rather than failing the photo: a correction left out is a darker corner,
/// a misread one could be anything.
pub fn parse(bytes: &[u8]) -> Vec<Opcode> {
    let r = Reader(bytes);
    let mut out = Vec::new();
    let Some(count) = r.u32(0) else {
        return out;
    };
    let mut at = 4usize;
    for _ in 0..count.min(1024) {
        let (Some(id), Some(size)) = (r.u32(at), r.u32(at + 12)) else {
            break;
        };
        let params = at + 16;
        let end = params.saturating_add(size as usize);
        if end > bytes.len() {
            break;
        }
        let parsed = match id {
            GAIN_MAP => r.gain_map(params),
            FIX_VIGNETTE_RADIAL => r.vignette(params),
            _ => None,
        };
        out.extend(parsed);
        at = end;
    }
    out
}

/// Big-endian reads from an opcode list, `None` past its end.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn u32(&self, o: usize) -> Option<u32> {
        Some(u32::from_be_bytes(self.0.get(o..o + 4)?.try_into().ok()?))
    }

    fn f32(&self, o: usize) -> Option<f32> {
        Some(f32::from_be_bytes(self.0.get(o..o + 4)?.try_into().ok()?))
    }

    fn f64(&self, o: usize) -> Option<f64> {
        Some(f64::from_be_bytes(self.0.get(o..o + 8)?.try_into().ok()?))
    }

    fn gain_map(&self, at: usize) -> Option<Opcode> {
        let area = Area {
            top: self.u32(at)?,
            left: self.u32(at + 4)?,
            bottom: self.u32(at + 8)?,
            right: self.u32(at + 12)?,
            plane: self.u32(at + 16)?,
            planes: self.u32(at + 20)?,
            row_pitch: self.u32(at + 24)?.max(1),
            col_pitch: self.u32(at + 28)?.max(1),
        };
        let points = (self.u32(at + 32)?, self.u32(at + 36)?);
        let spacing = (self.f64(at + 40)?, self.f64(at + 48)?);
        let origin = (self.f64(at + 56)?, self.f64(at + 64)?);
        let map_planes = self.u32(at + 72)?;
        let n = (points.0 as usize)
            .checked_mul(points.1 as usize)?
            .checked_mul(map_planes as usize)?;
        if n == 0 || n > 1 << 22 || !(spacing.0 > 0.0 && spacing.1 > 0.0) {
            return None;
        }
        let gains = (0..n)
            .map(|i| self.f32(at + 76 + 4 * i))
            .collect::<Option<Vec<f32>>>()?;
        Some(Opcode::GainMap(GainMap {
            area,
            points,
            spacing,
            origin,
            map_planes,
            gains,
        }))
    }

    fn vignette(&self, at: usize) -> Option<Opcode> {
        let mut k = [0.0; 5];
        for (i, v) in k.iter_mut().enumerate() {
            *v = self.f64(at + 8 * i)?;
        }
        let centre = (self.f64(at + 40)?, self.f64(at + 48)?);
        Some(Opcode::FixVignetteRadial { k, centre })
    }
}

impl GainMap {
    fn entry(&self, row: usize, col: usize, plane: usize) -> f32 {
        self.gains[(row * self.points.1 as usize + col) * self.map_planes as usize + plane]
    }

    /// The gain at image position `(row, col)` in an image of `size`
    /// (height, width), for map plane `plane`.
    pub fn gain(&self, row: u32, col: u32, size: (u32, u32), plane: u32) -> f32 {
        let plane = plane.min(self.map_planes - 1) as usize;
        // Index along one axis: the pixel centre as a fraction of the image,
        // in map steps from the origin, held at the ends.
        let axis = |pos: u32, extent: u32, origin: f64, spacing: f64, points: u32| {
            let f = (((pos as f64 + 0.5) / extent.max(1) as f64) - origin) / spacing;
            let last = points.saturating_sub(1) as usize;
            if f <= 0.0 {
                (0, 0, 0.0)
            } else if f as usize >= last {
                (last, last, 0.0)
            } else {
                (f as usize, f as usize + 1, (f - f.floor()) as f32)
            }
        };
        let (r0, r1, rf) = axis(row, size.0, self.origin.0, self.spacing.0, self.points.0);
        let (c0, c1, cf) = axis(col, size.1, self.origin.1, self.spacing.1, self.points.1);
        let top = self.entry(r0, c0, plane) * (1.0 - cf) + self.entry(r0, c1, plane) * cf;
        let bottom = self.entry(r1, c0, plane) * (1.0 - cf) + self.entry(r1, c1, plane) * cf;
        top * (1.0 - rf) + bottom * rf
    }
}

/// The radial vignette's gain at `(row, col)` in an image of `size`.
fn vignette_gain(k: &[f64; 5], centre: (f64, f64), row: u32, col: u32, size: (u32, u32)) -> f32 {
    let (h, w) = (size.0 as f64, size.1 as f64);
    // `centre` is (horizontal, vertical), as the opcode stores it.
    let (cx, cy) = (centre.0 * w, centre.1 * h);
    let max_r2 = cx.max(w - cx).powi(2) + cy.max(h - cy).powi(2);
    let (dx, dy) = (col as f64 + 0.5 - cx, row as f64 + 0.5 - cy);
    let x = ((dx * dx + dy * dy) / max_r2.max(1e-9)).min(1.0);
    let mut sum = 0.0;
    for term in k.iter().rev() {
        sum = x * (term + sum);
    }
    (1.0 + sum) as f32
}

/// Where the opcodes apply: the mosaic's values (or a linear image's
/// interleaved samples) inside the active area, with each value's colour.
pub struct Target<'a> {
    pub data: &'a mut [u16],
    /// Values per row of `data`: the full sensor width times the samples per
    /// pixel.
    pub stride: usize,
    /// The active area's origin, in pixels, and its size (height, width).
    pub origin: (u32, u32),
    pub size: (u32, u32),
    /// Values per pixel: 1 for a mosaic, 3 for a linear image.
    pub samples: u32,
    /// The colour (0 red, 1 green, 2 blue) of the value at `(row, col, sample)`
    /// of the active area.
    pub colour: &'a (dyn Fn(u32, u32, u32) -> u32 + Sync),
    pub black: [u16; 4],
    pub white: [u16; 4],
}

/// Which list an opcode came from, which decides what its planes mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// `OpcodeList2`: the image as stored, one plane per sample.
    Raw,
    /// `OpcodeList3`: the demosaiced image, planes red, green, blue.
    Demosaiced,
}

/// Applies `ops` to `target` in place. A clipped value is left alone: the
/// mosaic pass recognises blown highlights by their sitting at white, and a
/// gain below one would move them off it.
pub fn apply(ops: &[Opcode], stage: Stage, target: &mut Target<'_>) {
    if ops.is_empty() {
        return;
    }
    let (origin, size, samples, stride) =
        (target.origin, target.size, target.samples, target.stride);
    let (black, white) = (target.black, target.white);
    let colour = target.colour;
    let start = origin.0 as usize * stride;
    let rows = &mut target.data[start..start + size.0 as usize * stride];
    rows.par_chunks_mut(stride)
        .enumerate()
        .for_each(|(row, line)| {
            let row = row as u32;
            let line =
                &mut line[(origin.1 * samples) as usize..((origin.1 + size.1) * samples) as usize];
            for op in ops {
                match op {
                    Opcode::GainMap(map) => {
                        let a = &map.area;
                        if row < a.top
                            || row >= a.bottom
                            || !(row - a.top).is_multiple_of(a.row_pitch)
                        {
                            continue;
                        }
                        let right = a.right.min(size.1);
                        let mut col = a.left;
                        while col < right {
                            for s in 0..samples {
                                let c = colour(row, col, s);
                                // Which plane of the image this value belongs to.
                                let plane = match stage {
                                    Stage::Raw => s,
                                    Stage::Demosaiced => c,
                                };
                                // Subtracted, not added: both come from the file.
                                if plane < a.plane || plane - a.plane >= a.planes {
                                    continue;
                                }
                                let gain = map.gain(row, col, size, plane - a.plane);
                                scale(
                                    &mut line[(col * samples + s) as usize],
                                    gain,
                                    black[c as usize],
                                    white[c as usize],
                                );
                            }
                            col = col.saturating_add(a.col_pitch);
                        }
                    }
                    Opcode::FixVignetteRadial { k, centre } => {
                        for col in 0..size.1 {
                            let gain = vignette_gain(k, *centre, row, col, size);
                            for s in 0..samples {
                                let c = colour(row, col, s) as usize;
                                scale(
                                    &mut line[(col * samples + s) as usize],
                                    gain,
                                    black[c],
                                    white[c],
                                );
                            }
                        }
                    }
                }
            }
        });
}

fn scale(value: &mut u16, gain: f32, black: u16, white: u16) {
    if *value >= white {
        return;
    }
    let above = *value as f32 - black as f32;
    *value = (black as f32 + above * gain)
        .round()
        .clamp(0.0, white as f32) as u16;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An opcode list as a DNG writer lays it out.
    fn list(ops: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut out = (ops.len() as u32).to_be_bytes().to_vec();
        for (id, params) in ops {
            out.extend_from_slice(&id.to_be_bytes());
            out.extend_from_slice(&0x0103_0000u32.to_be_bytes()); // DNG 1.3
            out.extend_from_slice(&0u32.to_be_bytes()); // flags
            out.extend_from_slice(&(params.len() as u32).to_be_bytes());
            out.extend_from_slice(params);
        }
        out
    }

    fn gain_map_params(
        area: [u32; 8],
        points: (u32, u32),
        spacing: (f64, f64),
        origin: (f64, f64),
        planes: u32,
        gains: &[f32],
    ) -> Vec<u8> {
        let mut p = Vec::new();
        for v in area {
            p.extend_from_slice(&v.to_be_bytes());
        }
        p.extend_from_slice(&points.0.to_be_bytes());
        p.extend_from_slice(&points.1.to_be_bytes());
        for v in [spacing.0, spacing.1, origin.0, origin.1] {
            p.extend_from_slice(&v.to_be_bytes());
        }
        p.extend_from_slice(&planes.to_be_bytes());
        for g in gains {
            p.extend_from_slice(&g.to_be_bytes());
        }
        p
    }

    #[test]
    fn a_gain_map_and_a_vignette_parse_and_the_rest_are_skipped() {
        let gm = gain_map_params(
            [0, 0, 10, 10, 0, 1, 2, 2],
            (2, 2),
            (1.0, 1.0),
            (0.0, 0.0),
            1,
            &[1.0, 2.0, 3.0, 4.0],
        );
        let mut vig = Vec::new();
        for v in [0.1f64, 0.0, 0.0, 0.0, 0.0, 0.5, 0.5] {
            vig.extend_from_slice(&v.to_be_bytes());
        }
        let bytes = list(&[(1, vec![0; 68]), (9, gm), (3, vig)]);
        let ops = parse(&bytes);
        assert_eq!(ops.len(), 2, "{ops:?}");
        let Opcode::GainMap(m) = &ops[0] else {
            panic!()
        };
        assert_eq!(m.points, (2, 2));
        assert_eq!(m.area.row_pitch, 2);
        assert_eq!(m.gains, vec![1.0, 2.0, 3.0, 4.0]);
        assert!(matches!(
            ops[1],
            Opcode::FixVignetteRadial {
                centre: (0.5, 0.5),
                ..
            }
        ));
        // Truncated: nothing, rather than a panic or a misread.
        assert!(parse(&bytes[..30]).is_empty());
    }

    /// Positions are fractions of the image at pixel centres; the map is held
    /// at its edges and interpolated between points.
    #[test]
    fn gains_interpolate_across_the_image_and_hold_at_the_edges() {
        // A 2x2 map spanning the whole image: 1 at the top-left, 2 top-right,
        // 3 bottom-left, 4 bottom-right.
        let m = GainMap {
            area: Area {
                top: 0,
                left: 0,
                bottom: 100,
                right: 100,
                plane: 0,
                planes: 1,
                row_pitch: 1,
                col_pitch: 1,
            },
            points: (2, 2),
            spacing: (1.0, 1.0),
            origin: (0.0, 0.0),
            map_planes: 1,
            gains: vec![1.0, 2.0, 3.0, 4.0],
        };
        let size = (100, 100);
        assert!(
            (m.gain(0, 0, size, 0) - 1.015).abs() < 1e-3,
            "{}",
            m.gain(0, 0, size, 0)
        );
        assert!((m.gain(99, 99, size, 0) - 3.985).abs() < 1e-3);
        // The middle: all four equally.
        let mid = m.gain(49, 49, size, 0) + m.gain(50, 50, size, 0);
        assert!((mid / 2.0 - 2.5).abs() < 1e-3, "{mid}");
        // A map that starts a quarter in holds its first value before that.
        let late = GainMap {
            origin: (0.25, 0.25),
            spacing: (0.5, 0.5),
            ..m
        };
        assert_eq!(late.gain(0, 0, size, 0), 1.0);
        assert_eq!(late.gain(99, 99, size, 0), 4.0);
    }

    /// A Pixel-style map: four maps, one per Bayer site, picked out by the
    /// area's top/left and a pitch of two. Each touches only its own sites;
    /// clipped values stay at white; values below black scale around black.
    #[test]
    fn a_per_site_map_touches_only_its_own_photosites() {
        let map = |top: u32, left: u32, g: f32| {
            Opcode::GainMap(GainMap {
                area: Area {
                    top,
                    left,
                    bottom: 4,
                    right: 4,
                    plane: 0,
                    planes: 1,
                    row_pitch: 2,
                    col_pitch: 2,
                },
                points: (1, 1),
                spacing: (1.0, 1.0),
                origin: (0.0, 0.0),
                map_planes: 1,
                gains: vec![g],
            })
        };
        let ops = [map(0, 0, 2.0), map(1, 1, 0.5)];
        let mut data = vec![600u16; 16];
        data[5] = 1000; // a blue site (1, 1) at white
        let tile = [0u32, 1, 1, 2];
        let colour = move |r: u32, c: u32, _s: u32| tile[((r % 2) * 2 + c % 2) as usize];
        apply(
            &ops,
            Stage::Raw,
            &mut Target {
                data: &mut data,
                stride: 4,
                origin: (0, 0),
                size: (4, 4),
                samples: 1,
                colour: &colour,
                black: [100; 4],
                white: [1000; 4],
            },
        );
        assert_eq!(
            data[0], 1000,
            "red (0,0) doubled above black and capped at white"
        );
        assert_eq!(data[1], 600, "green untouched");
        assert_eq!(data[5], 1000, "clipped blue left at white");
        assert_eq!(data[15], 350, "blue (3,3) halved above black");
    }

    /// A map's planes and pitches are the file's numbers, and the largest of
    /// them must not overflow: planes reaching past `u32::MAX` still cover
    /// the ones they start from, and a step past the row's end ends it.
    #[test]
    fn the_largest_planes_and_pitches_do_not_overflow() {
        let map = |left: u32, plane: u32, planes: u32, col_pitch: u32| {
            Opcode::GainMap(GainMap {
                area: Area {
                    top: 0,
                    left,
                    bottom: 4,
                    right: 4,
                    plane,
                    planes,
                    row_pitch: 1,
                    col_pitch,
                },
                points: (1, 1),
                spacing: (1.0, 1.0),
                origin: (0.0, 0.0),
                map_planes: 1,
                gains: vec![2.0],
            })
        };
        let green = |_: u32, _: u32, _: u32| 1;
        let run = |op: Opcode, stage: Stage| {
            let mut data = vec![200u16; 16];
            apply(
                &[op],
                stage,
                &mut Target {
                    data: &mut data,
                    stride: 4,
                    origin: (0, 0),
                    size: (4, 4),
                    samples: 1,
                    colour: &green,
                    black: [100; 4],
                    white: [1000; 4],
                },
            );
            data
        };
        let all = run(map(0, 1, u32::MAX, 1), Stage::Demosaiced);
        assert_eq!(all, vec![300; 16]);
        let one_column = run(map(1, 0, 1, u32::MAX), Stage::Raw);
        for (i, v) in one_column.iter().enumerate() {
            assert_eq!(*v, if i % 4 == 1 { 300 } else { 200 }, "value {i}");
        }
    }

    #[test]
    fn the_radial_vignette_brightens_corners_and_not_the_centre() {
        let k = [0.5, 0.0, 0.0, 0.0, 0.0];
        let size = (1000, 1500);
        let centre = vignette_gain(&k, (0.5, 0.5), 500, 750, size);
        let corner = vignette_gain(&k, (0.5, 0.5), 0, 0, size);
        assert!((centre - 1.0).abs() < 1e-3, "{centre}");
        assert!((corner - 1.5).abs() < 1e-2, "{corner}");
    }
}
