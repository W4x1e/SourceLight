//! Parsing `.cube` 3D LUTs — the creative-"look" stage.
//!
//! A `.cube` file (the Adobe/Iridas format, the one DaVinci Resolve, Premiere,
//! Photoshop and every film-emulation pack speak) is a lookup table: a cube of
//! output colours indexed by input RGB. Unlike a preset (which is *our* sliders),
//! a LUT is pipeline-independent — "this input colour becomes that output colour"
//! — so it ports across tools and lets a user bring any LUT they own.
//!
//! We apply it as the final creative stage, on the display-encoded image, which
//! is the space the vast majority of look LUTs are authored for.
//!
//! This module is the pure parser plus a CPU reference sampler (used in tests and
//! as the ground truth the GPU's hardware trilinear must match). It has no GPU or
//! Tauri dependency, so every parsing and interpolation decision is a unit test.

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};

/// An `EditStack`'s reference to a creative LUT: which one, and how strongly.
/// Part of the edit, so it is saved with the photo and copied by presets like any
/// other setting. An empty `name` means no LUT.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LutRef {
    /// Filename of the `.cube` in the app's LUTs directory. Empty = none. Stored
    /// by name (not the LUT data) so the edit stays small and the same look is
    /// shared across photos; a missing file degrades gracefully to no LUT.
    pub name: String,
    /// 0..1 blend between the un-LUT'd image and the LUT.
    pub intensity: f32,
}

impl Default for LutRef {
    fn default() -> Self {
        // Intensity defaults to full so that *setting a name* is enough to see the
        // look; a fresh stack has an empty name and so no LUT regardless.
        LutRef {
            name: String::new(),
            intensity: 1.0,
        }
    }
}

/// The largest 3D LUT we accept. 33 is the common high-quality size (Resolve's
/// default); 64 is the practical ceiling. Rejecting anything larger bounds memory
/// (a 64³ RGBA32F texture is ~4 MB) and guards against a malformed header.
pub const MAX_SIZE: usize = 64;

/// The fixed grid every imported LUT is resampled onto, so the shader assumes one
/// 3D-texture size. 33 is the high-quality standard (Resolve's default) — plenty
/// for a creative look, and a 33³ RGBA32F texture is only ~575 KB.
pub const LUT_3D_DIM: usize = 33;

/// A parsed 3D LUT: an `size³` grid of output colours, red index varying fastest
/// (the `.cube` ordering), plus the input domain the grid spans.
#[derive(Debug, Clone, PartialEq)]
pub struct CubeLut {
    /// Entries per axis (`LUT_3D_SIZE`).
    pub size: usize,
    /// `size³` output triples, ordered `index = r + g*size + b*size*size` — the
    /// order `.cube` writes them and, conveniently, the order a GPU 3D texture
    /// wants them (x fastest, then y, then z-slice).
    pub data: Vec<[f32; 3]>,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
}

impl CubeLut {
    /// Parses `.cube` text. Tolerates comments (`#`), a `TITLE`, blank lines and
    /// arbitrary whitespace; understands `LUT_3D_SIZE`, `DOMAIN_MIN`/`DOMAIN_MAX`.
    /// A 1D LUT (`LUT_1D_SIZE`) is rejected with a clear message rather than
    /// silently mishandled — creative looks are effectively always 3D.
    pub fn parse(text: &str) -> Result<CubeLut> {
        let mut size: Option<usize> = None;
        let mut domain_min = [0.0f32; 3];
        let mut domain_max = [1.0f32; 3];
        let mut data: Vec<[f32; 3]> = Vec::new();

        for (n, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut toks = line.split_whitespace();
            // Unreachable after the trim above, but this is an untrusted file:
            // no panics on the parse path, however sure the reasoning.
            let Some(key) = toks.next() else {
                continue;
            };
            match key {
                "TITLE" => {} // decorative, ignored
                "LUT_1D_SIZE" => {
                    bail!("1D LUTs are not supported — this stage expects a 3D (LUT_3D_SIZE) .cube")
                }
                "LUT_3D_SIZE" => {
                    let s: usize = toks
                        .next()
                        .and_then(|t| t.parse().ok())
                        .with_context(|| format!("line {}: bad LUT_3D_SIZE", n + 1))?;
                    ensure!(
                        (2..=MAX_SIZE).contains(&s),
                        "LUT_3D_SIZE {s} out of range (2..={MAX_SIZE})"
                    );
                    size = Some(s);
                }
                "DOMAIN_MIN" => domain_min = parse_triple(&mut toks, n)?,
                "DOMAIN_MAX" => domain_max = parse_triple(&mut toks, n)?,
                _ => {
                    // A data row: three floats. `key` is the first of them.
                    let r: f32 = key
                        .parse()
                        .with_context(|| format!("line {}: unrecognised token '{key}'", n + 1))?;
                    let g: f32 = toks
                        .next()
                        .and_then(|t| t.parse().ok())
                        .with_context(|| format!("line {}: expected 3 values", n + 1))?;
                    let b: f32 = toks
                        .next()
                        .and_then(|t| t.parse().ok())
                        .with_context(|| format!("line {}: expected 3 values", n + 1))?;
                    // Refuse the row that overflows, rather than collecting
                    // every row of a pathological file and counting after:
                    // the declared size bounds it, or the largest size we
                    // accept if no header has been seen yet.
                    let cap = size.unwrap_or(MAX_SIZE).pow(3);
                    ensure!(
                        data.len() < cap,
                        "line {}: more than {cap} entries — too many for the LUT's size",
                        n + 1
                    );
                    data.push([r, g, b]);
                }
            }
        }

        let size = size.context("no LUT_3D_SIZE found — not a 3D .cube")?;
        let expected = size * size * size;
        ensure!(
            data.len() == expected,
            "expected {expected} entries for a {size}³ LUT, found {}",
            data.len()
        );
        ensure!(
            (0..3).all(|i| domain_max[i] > domain_min[i]),
            "DOMAIN_MAX must exceed DOMAIN_MIN on every channel"
        );

        Ok(CubeLut {
            size,
            data,
            domain_min,
            domain_max,
        })
    }

    /// The grid value at integer coordinates (clamped), red fastest.
    fn at(&self, r: usize, g: usize, b: usize) -> [f32; 3] {
        let n = self.size;
        let c = |v: usize| v.min(n - 1);
        self.data[c(r) + c(g) * n + c(b) * n * n]
    }

    /// Trilinearly samples the LUT at an input colour — the CPU reference the
    /// GPU's hardware trilinear filtering reproduces. Input is mapped through the
    /// domain and clamped to the cube.
    pub fn sample(&self, rgb: [f32; 3]) -> [f32; 3] {
        let n = self.size;
        let last = (n - 1) as f32;
        // Normalise into [0, last] grid coordinates.
        let coord = |i: usize| {
            let t = (rgb[i] - self.domain_min[i]) / (self.domain_max[i] - self.domain_min[i]);
            (t.clamp(0.0, 1.0) * last).clamp(0.0, last)
        };
        let (x, y, z) = (coord(0), coord(1), coord(2));
        let (x0, y0, z0) = (x.floor() as usize, y.floor() as usize, z.floor() as usize);
        let (x1, y1, z1) = (x0 + 1, y0 + 1, z0 + 1);
        let (fx, fy, fz) = (x - x0 as f32, y - y0 as f32, z - z0 as f32);

        let lerp = |a: [f32; 3], b: [f32; 3], t: f32| {
            [
                a[0] + (b[0] - a[0]) * t,
                a[1] + (b[1] - a[1]) * t,
                a[2] + (b[2] - a[2]) * t,
            ]
        };
        // Interpolate along x, then y, then z.
        let c00 = lerp(self.at(x0, y0, z0), self.at(x1, y0, z0), fx);
        let c10 = lerp(self.at(x0, y1, z0), self.at(x1, y1, z0), fx);
        let c01 = lerp(self.at(x0, y0, z1), self.at(x1, y0, z1), fx);
        let c11 = lerp(self.at(x0, y1, z1), self.at(x1, y1, z1), fx);
        let c0 = lerp(c00, c10, fy);
        let c1 = lerp(c01, c11, fy);
        lerp(c0, c1, fz)
    }

    /// The grid flattened to RGBA32F texels (`[r, g, b, 1]`), ready to upload as a
    /// 3D texture. The entry order already matches a GPU 3D texture's memory
    /// layout, so no reshuffle is needed.
    pub fn rgba_f32(&self) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.data.len() * 4);
        for c in &self.data {
            out.extend_from_slice(&[c[0], c[1], c[2], 1.0]);
        }
        out
    }

    /// Resamples this LUT onto a fixed `dim³` grid, returning RGBA32F texels. Every
    /// imported LUT, whatever its native size, is normalised to one grid the
    /// shader can assume — so there is a single 3D-texture size and no per-render
    /// texture reallocation. `dim` is small (33) and the trilinear sample cheap.
    /// Texel order is red-fastest, matching a GPU 3D texture's memory layout.
    pub fn resample_to(&self, dim: usize) -> Vec<f32> {
        let last = (dim - 1) as f32;
        let mut out = Vec::with_capacity(dim * dim * dim * 4);
        for b in 0..dim {
            for g in 0..dim {
                for r in 0..dim {
                    let s = self.sample([r as f32 / last, g as f32 / last, b as f32 / last]);
                    out.extend_from_slice(&[s[0], s[1], s[2], 1.0]);
                }
            }
        }
        out
    }
}

/// RGBA32F texels for an identity `dim³` LUT — the default contents of the 3D
/// texture, so it is always a valid bind even before any LUT is chosen (and a
/// no-op when intensity is 0).
/// The texels of the `.cube` named `name` in `dir`, resampled for the GPU, or
/// `None` when there is no such LUT. Only a bare file name in `dir` is read:
/// an edit's LUT name comes from a catalogue or a preset, and is never a path.
pub fn load_texels(dir: &std::path::Path, name: &str) -> Option<Vec<f32>> {
    if !is_lut_name(name) {
        return None;
    }
    let text = std::fs::read_to_string(dir.join(name)).ok()?;
    let cube = CubeLut::parse(&text).ok()?;
    Some(cube.resample_to(LUT_3D_DIM))
}

/// Whether `name` can only mean a file in the LUTs folder: one plain path
/// part ending `.cube`. Checked wherever a name is joined to the folder.
/// `C:x.cube` is a path on drive C, and `Path::join` lets it replace the
/// folder; separators and `:` are refused on every platform, so a catalogue
/// means the same on each.
pub fn is_lut_name(name: &str) -> bool {
    use std::path::{Component, Path};
    let mut parts = Path::new(name).components();
    matches!(
        (parts.next(), parts.next()),
        (Some(Component::Normal(_)), None)
    ) && !name.contains(['/', '\\', ':'])
        && name.to_ascii_lowercase().ends_with(".cube")
}

pub fn identity_texels(dim: usize) -> Vec<f32> {
    let last = (dim - 1) as f32;
    let mut out = Vec::with_capacity(dim * dim * dim * 4);
    for b in 0..dim {
        for g in 0..dim {
            for r in 0..dim {
                out.extend_from_slice(&[r as f32 / last, g as f32 / last, b as f32 / last, 1.0]);
            }
        }
    }
    out
}

fn parse_triple<'a>(toks: &mut impl Iterator<Item = &'a str>, line: usize) -> Result<[f32; 3]> {
    let mut v = [0.0f32; 3];
    for (i, slot) in v.iter_mut().enumerate() {
        *slot = toks
            .next()
            .and_then(|t| t.parse().ok())
            .with_context(|| format!("line {}: expected 3 numbers (value {})", line + 1, i + 1))?;
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An identity 2³ LUT: the eight cube corners map to themselves.
    const IDENTITY_2: &str = "\
TITLE \"identity\"
LUT_3D_SIZE 2
0.0 0.0 0.0
1.0 0.0 0.0
0.0 1.0 0.0
1.0 1.0 0.0
0.0 0.0 1.0
1.0 0.0 1.0
0.0 1.0 1.0
1.0 1.0 1.0
";

    /// A LUT name never reaches outside the LUTs folder, however it is spelt.
    #[test]
    fn only_a_plain_cube_file_name_is_a_lut_name() {
        for good in ["Film.cube", "Portra 400 (2).CUBE", "my..look.cube"] {
            assert!(is_lut_name(good), "{good}");
        }
        for bad in [
            "",
            "C:x.cube",
            r"C:\x.cube",
            r"..\x.cube",
            "../x.cube",
            "a/b.cube",
            r"a\b.cube",
            "/x.cube",
            r"\\host\share\x.cube",
            "..",
            ".",
            "film.txt",
        ] {
            assert!(!is_lut_name(bad), "{bad}");
        }
        let dir = std::env::temp_dir();
        assert!(load_texels(&dir, "C:x.cube").is_none());
        assert!(load_texels(&dir, r"..\x.cube").is_none());
    }

    /// A file that keeps going past its declared size is refused at the first
    /// row too many, not after every row has been collected.
    #[test]
    fn rows_past_the_declared_size_are_refused_as_they_arrive() {
        let mut text = IDENTITY_2.to_string();
        text.push_str("0.5 0.5 0.5\n");
        let err = CubeLut::parse(&text).unwrap_err().to_string();
        assert!(err.contains("more than 8 entries"), "{err}");
        assert!(err.contains("line 11"), "{err}");
    }

    /// Before any header, the cap is the largest LUT accepted at all.
    #[test]
    fn rows_before_a_header_are_capped_at_the_largest_size() {
        let row = "0.1 0.2 0.3\n";
        let text = row.repeat(MAX_SIZE.pow(3) + 1);
        let err = CubeLut::parse(&text).unwrap_err().to_string();
        assert!(
            err.contains(&format!("more than {}", MAX_SIZE.pow(3))),
            "{err}"
        );
    }

    fn approx(a: [f32; 3], b: [f32; 3]) {
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() < 1e-5, "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn parses_a_well_formed_cube() {
        let lut = CubeLut::parse(IDENTITY_2).unwrap();
        assert_eq!(lut.size, 2);
        assert_eq!(lut.data.len(), 8);
        assert_eq!(lut.domain_min, [0.0, 0.0, 0.0]);
        assert_eq!(lut.domain_max, [1.0, 1.0, 1.0]);
        // Red varies fastest: index 1 is the +red corner.
        assert_eq!(lut.data[1], [1.0, 0.0, 0.0]);
    }

    #[test]
    fn tolerates_comments_blanks_and_whitespace() {
        let text = "# a comment\n\n  LUT_3D_SIZE   2  \n# data:\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        let lut = CubeLut::parse(text).unwrap();
        assert_eq!(lut.size, 2);
        assert_eq!(lut.data.len(), 8);
    }

    #[test]
    fn identity_lut_returns_the_input() {
        let lut = CubeLut::parse(IDENTITY_2).unwrap();
        approx(lut.sample([0.0, 0.0, 0.0]), [0.0, 0.0, 0.0]);
        approx(lut.sample([1.0, 1.0, 1.0]), [1.0, 1.0, 1.0]);
        // A mid value trilinearly interpolates back to itself on an identity cube.
        approx(lut.sample([0.3, 0.6, 0.9]), [0.3, 0.6, 0.9]);
    }

    #[test]
    fn a_swap_lut_maps_predictably() {
        // 2³ LUT that swaps red and blue at the corners; interpolation follows.
        let text = "LUT_3D_SIZE 2\n0 0 0\n0 0 1\n0 1 0\n0 1 1\n1 0 0\n1 0 1\n1 1 0\n1 1 1\n";
        let lut = CubeLut::parse(text).unwrap();
        // Pure red input -> the +red corner entry, which is [0,0,1] (blue out).
        approx(lut.sample([1.0, 0.0, 0.0]), [0.0, 0.0, 1.0]);
        approx(lut.sample([0.0, 0.0, 1.0]), [1.0, 0.0, 0.0]);
    }

    #[test]
    fn sampling_clamps_outside_the_domain() {
        let lut = CubeLut::parse(IDENTITY_2).unwrap();
        approx(lut.sample([-0.5, 2.0, 0.5]), [0.0, 1.0, 0.5]);
    }

    #[test]
    fn a_custom_domain_is_honoured() {
        let text = "LUT_3D_SIZE 2\nDOMAIN_MIN 0 0 0\nDOMAIN_MAX 2 2 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        let lut = CubeLut::parse(text).unwrap();
        // Input 2.0 maps to the top of the domain -> the +red corner = 1.0.
        approx(lut.sample([2.0, 0.0, 0.0]), [1.0, 0.0, 0.0]);
        // Input 1.0 is the domain midpoint -> half of red = 0.5.
        approx(lut.sample([1.0, 0.0, 0.0]), [0.5, 0.0, 0.0]);
    }

    #[test]
    fn rgba_f32_pads_alpha_and_keeps_order() {
        let lut = CubeLut::parse(IDENTITY_2).unwrap();
        let tex = lut.rgba_f32();
        assert_eq!(tex.len(), 8 * 4);
        assert_eq!(&tex[0..4], &[0.0, 0.0, 0.0, 1.0]); // first entry
        assert_eq!(&tex[4..8], &[1.0, 0.0, 0.0, 1.0]); // second entry (+red)
    }

    #[test]
    fn resampling_an_identity_lut_gives_the_identity_grid() {
        let lut = CubeLut::parse(IDENTITY_2).unwrap();
        // An identity 2³ resampled to 5³ must equal a freshly-built 5³ identity.
        let resampled = lut.resample_to(5);
        let identity = identity_texels(5);
        assert_eq!(resampled.len(), identity.len());
        for (a, b) in resampled.iter().zip(&identity) {
            assert!((a - b).abs() < 1e-5, "{a} vs {b}");
        }
    }

    #[test]
    fn rejects_a_wrong_entry_count() {
        let text = "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n"; // needs 8, has 2
        assert!(CubeLut::parse(text).is_err());
    }

    #[test]
    fn rejects_a_1d_lut_and_a_missing_size() {
        assert!(CubeLut::parse("LUT_1D_SIZE 16\n").is_err());
        assert!(CubeLut::parse("0 0 0\n1 1 1\n").is_err());
    }

    #[test]
    fn rejects_an_absurd_size() {
        assert!(CubeLut::parse("LUT_3D_SIZE 200\n").is_err());
        assert!(CubeLut::parse("LUT_3D_SIZE 1\n").is_err());
    }
}
