//! Where a sensor actually clips.
//!
//! The white level a raw's metadata gives is the *format's* ceiling, which is
//! not always where the sensor saturates. A Canon EOS R6 writes 14-bit data,
//! so LibRaw reports 16383, but the sensor saturates at 14008 at base ISO —
//! Canon records that separately, as its specular white level. Normalising by
//! 16383 puts a blown sky at 0.85 of full scale instead of 1.0, and two things
//! go wrong at once: the whole frame renders about a quarter of a stop dark,
//! and the highlight handling in `mosaic.wgsl`, which recognises a clipped
//! photosite by green reaching full scale, never fires — so white balance
//! pulls the blown channels apart and the sky comes out pink.
//!
//! LibRaw's own pipeline hides this by lowering the white level to the
//! brightest value in the frame (`adjust_maximum`), which this app never calls
//! and which misfires on any frame without a clipped highlight. So two sources
//! are used instead: the camera's own statement where it makes one (Canon's
//! specular white level, applied by the decoder), and the frame's data, where
//! clipping leaves an unmistakable signature — a pile of photosites on one
//! value at the very top of the range, far more than on any value near it.

use rayon::prelude::*;

use super::decoder::{CfaPattern, Rect};

/// How far below the stated level to look for a pile-up, as a fraction of the
/// range above black. Canon's newer bodies saturate as much as a fifth of the
/// way below their 14-bit ceiling (13036 on an R7), so the window has to
/// reach past that.
const SEARCH: f32 = 0.30;
/// A pile-up needs at least this many photosites. Fewer is a handful of hot
/// or specular pixels, which says nothing about the level.
const MIN_PILE: u32 = 32;
/// And it has to stand this far above every other value near the top. A
/// bright but unclipped area spreads over many neighbouring values with
/// similar counts; clipping stacks them on one.
const DOMINANCE: u32 = 8;

/// The active area of a mosaic, read row by row with each photosite's colour.
struct Mosaic<'a> {
    cfa: &'a [u16],
    width: u32,
    active: Rect,
    /// Values per pixel: three, interleaved, for an image with no mosaic.
    samples: u32,
    /// The colour of every value in a row, for each row of the tile.
    colours: Vec<Vec<usize>>,
}

impl<'a> Mosaic<'a> {
    fn new(cfa: &'a [u16], width: u32, active: Rect, pattern: &CfaPattern) -> Self {
        let samples = pattern.samples();
        let colours = match pattern {
            CfaPattern::Linear => vec![(0..active.width * 3).map(|i| (i % 3) as usize).collect()],
            _ => (0..pattern.period())
                .map(|y| {
                    (0..active.width)
                        .map(|x| pattern.color_at(x, y) as usize)
                        .collect()
                })
                .collect(),
        };
        Mosaic {
            cfa,
            width,
            active,
            samples,
            colours,
        }
    }

    /// Row `y` of the active area, and its values' colours.
    fn row(&self, y: u32) -> (&[u16], &[usize]) {
        let start = (((self.active.y + y) * self.width + self.active.x) * self.samples) as usize;
        (
            &self.cfa[start..start + (self.active.width * self.samples) as usize],
            &self.colours[y as usize % self.colours.len()],
        )
    }

    /// How many photosites of each colour read each value from `lo[c]` to
    /// `hi[c]`, inclusive.
    fn histogram(&self, lo: [u16; 4], hi: [u16; 4]) -> [Vec<u32>; 4] {
        let empty = || -> [Vec<u32>; 4] {
            std::array::from_fn(|c| vec![0u32; hi[c].saturating_sub(lo[c]) as usize + 1])
        };
        (0..self.active.height)
            .into_par_iter()
            .fold(empty, |mut counts, y| {
                let (row, colours) = self.row(y);
                for (&v, &c) in row.iter().zip(colours) {
                    if v >= lo[c] && v <= hi[c] {
                        counts[c][(v - lo[c]) as usize] += 1;
                    }
                }
                counts
            })
            .reduce(empty, |mut a, b| {
                for (x, y) in a.iter_mut().zip(&b) {
                    for (p, q) in x.iter_mut().zip(y) {
                        *p += q;
                    }
                }
                a
            })
    }
}

/// Where the counts show clipping: the value the pile-up starts at, or `None`.
///
/// `counts[i]` is how many photosites read `lo + i`. The pile-up is the
/// fullest value, widened by `tolerance` either side (a few cameras scatter a
/// clipped photosite over neighbouring values); it has to beat every value
/// outside that window by [`DOMINANCE`], and have little above it.
fn pile_up(counts: &[u32], lo: u16, tolerance: usize) -> Option<u16> {
    let (peak, &peak_count) = counts.iter().enumerate().max_by_key(|&(i, c)| (*c, i))?;
    if peak_count == 0 {
        return None;
    }
    let from = peak.saturating_sub(tolerance);
    let to = (peak + tolerance).min(counts.len() - 1);
    let pile: u32 = counts[from..=to].iter().sum();
    let rival = counts
        .iter()
        .enumerate()
        .filter(|(i, _)| *i < from || *i > to)
        .map(|(_, c)| *c)
        .max()
        .unwrap_or(0);
    let above: u32 = counts[to + 1..].iter().sum();
    let first = (from..=to).find(|&i| counts[i] > 0)?;
    (pile >= MIN_PILE && pile >= DOMINANCE * rival.max(1) && above * 4 <= pile)
        .then(|| lo + first as u16)
}

/// The level each colour channel saturates at, found from the frame itself.
///
/// Starts from `nominal` (the metadata's white level, or the camera's own
/// statement of it) and moves only when the data shows clipping below it, as
/// on a camera whose format ceiling is above its sensor's. A frame without a
/// clipped highlight has no pile-up and keeps `nominal`, which is why this
/// cannot darken an ordinary photo the way lowering the level to the frame's
/// brightest value does.
///
/// **Never above `nominal`.** A Leica M10 DNG states a white level of 15000
/// and still writes values up to 16383, piled up at the top. Past the stated
/// level is clipped by definition; following the pile-up there moved green
/// and blue to 16376 while red, with too few photosites to show one, stayed
/// at 15000 — the three channels normalised 9% apart, a cast on every frame.
pub fn clip_levels(
    cfa: &[u16],
    width: u32,
    active: Rect,
    pattern: &CfaPattern,
    black: [u16; 4],
    nominal: [u16; 4],
) -> [u16; 4] {
    let lo: [u16; 4] = std::array::from_fn(|c| {
        let range = nominal[c].saturating_sub(black[c]) as f32;
        nominal[c].saturating_sub((range * SEARCH) as u16)
    });
    let mosaic = Mosaic::new(cfa, width, active, pattern);
    let counts = mosaic.histogram(lo, nominal);
    std::array::from_fn(|c| {
        // A clipped photosite lands on one value on every camera measured;
        // allow 0.05% of the range for any that scatter it.
        let range = nominal[c].saturating_sub(black[c]) as usize;
        let tolerance = (range / 2000).max(1);
        match pile_up(&counts[c], lo[c], tolerance) {
            // Clipping in the lower half would be a scene, not a sensor.
            Some(level) if level > black[c] + (nominal[c].saturating_sub(black[c])) / 2 => level,
            _ => nominal[c],
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAYER: [u8; 4] = [0, 1, 3, 2];

    /// A 64x64 RGGB frame: a smooth ramp from black to `peak`, with `clipped`
    /// photosites of every colour forced to `clip`.
    fn frame(peak: u16, clip: u16, clipped: usize) -> Vec<u16> {
        let mut cfa: Vec<u16> = (0..64 * 64)
            .map(|i| 512 + ((i as f32 / 4096.0) * (peak - 512) as f32) as u16)
            .collect();
        // Clip whole 2x2 tiles, so every colour gets its share.
        for t in 0..clipped {
            let (tx, ty) = ((t % 32) * 2, (t / 32) * 2);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                cfa[(ty + dy) * 64 + tx + dx] = clip;
            }
        }
        cfa
    }

    fn levels(cfa: &[u16], nominal: u16) -> [u16; 4] {
        let area = Rect {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        };
        clip_levels(
            cfa,
            64,
            area,
            &CfaPattern::Bayer(BAYER),
            [512; 4],
            [nominal; 4],
        )
    }

    /// The Canon case: a 14-bit ceiling, a sensor that saturates well below
    /// it. The pile-up is found and becomes the level.
    #[test]
    fn clipping_below_the_ceiling_is_found() {
        let cfa = frame(13_500, 14_008, 200);
        assert_eq!(levels(&cfa, 16_383), [14_008; 4]);
    }

    /// The Sony case: clipped values a few counts below the stated maximum.
    #[test]
    fn clipping_just_under_the_maximum_is_found() {
        let cfa = frame(16_000, 16_372, 100);
        assert_eq!(levels(&cfa, 16_380), [16_372; 4]);
    }

    /// No clipped highlight, no change — however bright the frame gets. This
    /// is what LibRaw's `adjust_maximum` gets wrong.
    #[test]
    fn an_unclipped_frame_keeps_the_metadata_level() {
        let cfa = frame(15_000, 0, 0);
        assert_eq!(levels(&cfa, 16_383), [16_383; 4]);
    }

    /// A few hot pixels are not a clip level.
    #[test]
    fn a_handful_of_pixels_is_not_a_pile_up() {
        let cfa = frame(12_000, 14_000, 3);
        assert_eq!(levels(&cfa, 16_383), [16_383; 4]);
    }

    /// Data piled up above the stated level is clipped by definition and leaves
    /// the level where it is, for every channel alike (the Leica M10 case).
    #[test]
    fn clipping_above_the_stated_level_does_not_raise_it() {
        let cfa = frame(13_000, 16_383, 150);
        assert_eq!(levels(&cfa, 15_000), [15_000; 4]);
    }

    /// A pile-up in the lower half of the range is scene content, not the
    /// sensor's ceiling.
    #[test]
    fn a_pile_up_far_down_the_range_is_ignored() {
        let pile = pile_up(&[0, 0, 500, 0, 3], 100, 1);
        assert_eq!(pile, Some(102), "found where it is");
        let cfa = frame(4_000, 6_000, 400);
        assert_eq!(levels(&cfa, 16_383), [16_383; 4]);
    }

    #[test]
    fn a_spread_pile_up_starts_at_its_lowest_value() {
        // Clipped photosites scattered over three values.
        let mut counts = vec![2u32; 40];
        counts[30] = 300;
        counts[31] = 900;
        counts[32] = 250;
        assert_eq!(pile_up(&counts, 1000, 2), Some(1029));
    }
}
