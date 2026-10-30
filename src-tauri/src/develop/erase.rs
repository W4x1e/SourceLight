//! Magic erase: an object taken out of the photo, the gap filled by LaMa
//! (Suvorov et al. 2021, Apache-2.0), and the fill baked into the linear
//! image so every later edit treats it like the rest of the photo.
//!
//! The spike (`docs/AI-MODEL-SPIKES.md`) settled the shape of it:
//!
//! - **LaMa over MI-GAN.** MI-GAN is an eighth of the download and ten
//!   times faster, but smudges a gap a person leaves and invents objects in
//!   a large one. LaMa continues tyre tracks, paths and water through the gap.
//!   Its fixed 512 px input takes 2.2–2.8 s on an M4's CPU; CoreML cannot
//!   compile it (ONNX Runtime 1.24.4), so on a Mac it goes straight to the
//!   CPU rather than spend 7 s finding that out on every load.
//! - **A window around the object, squeezed into 512 px.** Square, 1.6 times
//!   the object's long side plus a margin, so the model sees enough of what
//!   surrounds it ([`plan`]). How much it is squeezed (`f`, sensor pixels per
//!   model pixel) is how soft the fill comes out: up to 2 it is clean at
//!   100%, past 4 it is soft. Separate objects selected together each get
//!   their own window, unless one sharp window holds them all.
//! - **Tiles for a long, thin object.** A flag pole 120 px wide and 2,000
//!   tall would get a window squeezed six times. Tiles of 1,024 px (f = 2)
//!   along it each mask *all* of the object they can see — with only their
//!   own share masked, LaMa redrew the pole from the pieces above and below —
//!   and keep only the share inside their margin. A bulky object, which would
//!   leave a tile little to go on, gets the one window and a soft fill.
//! - **The fill is stored, not the model's input.** Each window's fill is
//!   decoded back to linear and saved, with how much of it to use, as a
//!   16-bit PNG named by its SHA-256 ([`FillStore`]); the edit stack holds
//!   only the names and where they go ([`Erase`]). History, virtual copies
//!   and the catalogue stay small, and a fill is never worked out again.
//! - **Matched grain.** A fill is smoother than the sensor noise around it,
//!   and looks pasted on. Noise of the same strength, measured from the
//!   window's untouched pixels when the fill is laid in ([`composite`]), hides
//!   it. Seeded by position, so laying a fill in again gives the same pixels.
//! - **Through the linear image, like AI noise reduction.** The same sRGB
//!   transfer on the way in and out (`crate::denoise::encode`), scaled so a
//!   window's real highlights fit. LaMa returns the pixels it was not asked to
//!   fill unchanged, so a fill meets its surroundings with no step in colour.
//!
//! The pure parts are here and tested; the model sits behind `--features ai`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use half::f16;
pub use sl_develop::develop::erase::{dirty_regions, fill_names, Erase, Piece, Rect};

/// The model's fixed input edge.
pub const MODEL_EDGE: u32 = 512;
/// The model file, under the models folder.
pub const MODEL_FILE: &str = "erase/lama_fp32.onnx";
/// The most sensor pixels a window may squeeze into one model pixel before
/// a long object is tiled instead.
const MAX_SQUEEZE: f32 = 2.0;
/// The tile edges tried, smallest (least squeezed) first: the first is
/// [`MAX_SQUEEZE`] model inputs. A wide object too thick for small tiles,
/// a boat's hull, still squeezes less in big ones than in one window.
const TILE_EDGES: [u32; 4] = [1024, 1536, 2048, 3072];
/// A tile more than this much hole has too little left to go on.
const TILE_HOLE_LIMIT: f32 = 0.4;
/// Past this squeeze a fill is visibly soft at 100%, which the tool says.
pub const SOFT_SQUEEZE: f32 = 4.0;
/// The long edge a selection from the masking tools is rasterised at: the
/// size SAM 2 and BiRefNet give theirs (`ai::coverage_size`).
pub const SELECTION_EDGE: u32 = 1024;
/// How far past the selection's edge an erase reaches, at least, in sensor
/// pixels: a selection stops a pixel or two short of a soft edge, and what
/// is left there is drawn back in by the model.
const REACH: u32 = 16;

/// One window's fill: linear RGB as the stored image holds it, and in alpha
/// how much of it to use, at [`MODEL_EDGE`]² — the detail the model gave.
#[derive(Debug, Clone, PartialEq)]
pub struct Fill {
    pub size: u32,
    /// RGBA half floats (their bits).
    pub rgba: Vec<u16>,
}

impl Fill {
    /// As a 16-bit RGBA PNG holding the half floats' bits: lossless, and
    /// what the colour is where nothing is used is zeroed, so it packs small.
    pub fn to_png(&self) -> Result<Vec<u8>> {
        let img = image::ImageBuffer::<image::Rgba<u16>, _>::from_raw(
            self.size,
            self.size,
            self.rgba.clone(),
        )
        .context("a fill of the wrong size")?;
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba16(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .context("encoding a fill")?;
        Ok(out.into_inner())
    }

    pub fn from_png(bytes: &[u8]) -> Result<Self> {
        let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
            .context("reading a fill")?;
        anyhow::ensure!(
            matches!(img, image::DynamicImage::ImageRgba16(_)),
            "a fill is a 16-bit RGBA PNG"
        );
        let img = img.into_rgba16();
        anyhow::ensure!(img.width() == img.height(), "a fill is square");
        Ok(Self {
            size: img.width(),
            rgba: img.into_raw(),
        })
    }
}

/// A fill's name: the SHA-256 of its PNG, in hex.
pub fn fill_name(png: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(png)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Whether `name` could be a fill's name. Edit stacks reach the backend from
/// the webview and from the catalogue, so a name is checked before it becomes
/// part of a path.
pub fn is_fill_name(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The folder fills are kept in (`<app data>/patches`), one file each, named
/// by content: saving one twice writes it once.
#[derive(Debug, Clone)]
pub struct FillStore {
    dir: PathBuf,
}

impl FillStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, name: &str) -> Result<PathBuf> {
        anyhow::ensure!(is_fill_name(name), "not a fill: {name:?}");
        Ok(self.dir.join(format!("{name}.png")))
    }

    /// Saves a fill's PNG, returning its name.
    pub fn save(&self, png: &[u8]) -> Result<String> {
        let name = fill_name(png);
        let path = self.path(&name)?;
        if !path.exists() {
            std::fs::create_dir_all(&self.dir)
                .with_context(|| format!("creating {}", self.dir.display()))?;
            crate::fsutil::write_atomic(&path, png)
                .with_context(|| format!("saving {}", path.display()))?;
        }
        Ok(name)
    }

    pub fn load(&self, name: &str) -> Result<Fill> {
        let path = self.path(name)?;
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        anyhow::ensure!(fill_name(&bytes) == name, "{} is damaged", path.display());
        Fill::from_png(&bytes)
    }
}

/// Deletes the fills in `dir` that no name in `in_use` names and that are
/// older than `min_age`, and temp files a save left behind. How many went.
/// The age keeps a fill whose edit is not saved yet.
pub fn sweep(
    dir: &Path,
    in_use: &std::collections::HashSet<String>,
    min_age: std::time::Duration,
) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let now = std::time::SystemTime::now();
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let used = match name.strip_suffix(".png") {
            Some(fill) if is_fill_name(fill) => in_use.contains(fill),
            // A save that never finished.
            _ if name.ends_with(".sourcelight-tmp") => false,
            // Not a fill: not ours to delete.
            _ => continue,
        };
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|at| now.duration_since(at).ok())
            .is_some_and(|age| age >= min_age);
        if !used && old && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

// ------------------------------------------------------------------------
// The selection, on the sensor.
// ------------------------------------------------------------------------

/// What `mask` covers, as a `width` x `height` coverage over the framed
/// photo: AI selections, brushing and the shapes, each inverted and combined
/// as the mask does. A brightness or colour range selects by what pixels
/// look like, not where an object is, so it is refused.
pub fn selection_coverage(
    mask: &crate::develop::mask::Mask,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    use crate::develop::mask::{self, MaskMode, MaskShape};
    let n = (width * height) as usize;
    let mut acc = vec![0.0f32; n];
    for (i, comp) in mask.components.iter().enumerate() {
        let raster: Vec<f32> = match &comp.shape {
            MaskShape::Ai {
                width: aw,
                height: ah,
                rle,
                ..
            } => {
                let data = mask::decode_ai_sized(rle, *aw, *ah);
                mask::resize_coverage(&data, *aw, *ah, width, height)
                    .iter()
                    .map(|&v| v as f32 / 255.0)
                    .collect()
            }
            MaskShape::Brush { strokes } => mask::rasterize(strokes, width, height)
                .iter()
                .map(|&v| v as f32 / 255.0)
                .collect(),
            MaskShape::Radial { .. } | MaskShape::Linear { .. } => (0..n)
                .map(|k| {
                    let u = ((k as u32 % width) as f32 + 0.5) / width as f32;
                    let v = ((k as u32 / width) as f32 + 0.5) / height as f32;
                    comp.shape.coverage(u, v)
                })
                .collect(),
            MaskShape::Luminance { .. } | MaskShape::ColorRange { .. } => anyhow::bail!(
                "a brightness or colour range can't be erased — select the object instead"
            ),
        };
        for (a, c) in acc.iter_mut().zip(raster) {
            let c = if comp.inverted { 1.0 - c } else { c };
            *a = if i == 0 {
                c
            } else {
                match comp.mode {
                    MaskMode::Add => a.max(c),
                    MaskMode::Subtract => *a * (1.0 - c),
                    MaskMode::Intersect => a.min(c),
                }
            };
        }
    }
    Ok(acc
        .iter()
        .map(|a| mask::round_to_byte(a.clamp(0.0, 1.0) * 255.0))
        .collect())
}

/// A selection's coverage over the framed photo, 0..=255, as the AI
/// selections and the brush give it.
pub struct Coverage<'a> {
    pub data: &'a [u8],
    pub width: u32,
    pub height: u32,
}

impl Coverage<'_> {
    /// Bilinear at framed UV, 0..=1; nothing outside the frame.
    fn at(&self, (u, v): (f32, f32)) -> f32 {
        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
            return 0.0;
        }
        let x = (u * self.width as f32 - 0.5).clamp(0.0, (self.width - 1) as f32);
        let y = (v * self.height as f32 - 0.5).clamp(0.0, (self.height - 1) as f32);
        bilinear(
            |xx, yy| self.data[yy * self.width as usize + xx] as f32 / 255.0,
            self.width as usize,
            self.height as usize,
            x,
            y,
        )
    }
}

/// A selection on the stored image, on a grid of square cells about as
/// big as one texel of the coverage it came from, grown by the erase's reach.
#[derive(Debug, Clone)]
pub struct Grid {
    /// Where cell (0, 0) starts, in stored pixels.
    x: i32,
    y: i32,
    /// Stored pixels a cell.
    cell: u32,
    w: u32,
    h: u32,
    on: Vec<bool>,
}

impl Grid {
    /// The selection `coverage` (framed UV) carried onto the stored image of
    /// `size` pixels. `to_stored` and `to_framed` map UV between the two, as
    /// the develop pass does (`DevelopSession::sample_uv` and `framed_uv`).
    /// `None` when nothing is selected.
    pub fn from_coverage(
        coverage: &Coverage,
        size: (u32, u32),
        to_stored: impl Fn((f32, f32)) -> (f32, f32),
        to_framed: impl Fn((f32, f32)) -> (f32, f32),
    ) -> Option<Grid> {
        let (sw, sh) = (size.0 as f32, size.1 as f32);
        // Where the selected texels land, and how far apart.
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        let (cw, ch) = (coverage.width, coverage.height);
        let mut spacing: f32 = 1.0;
        for ty in 0..ch {
            for tx in 0..cw {
                if coverage.data[(ty * cw + tx) as usize] < 128 {
                    continue;
                }
                let framed = ((tx as f32 + 0.5) / cw as f32, (ty as f32 + 0.5) / ch as f32);
                let (u, v) = to_stored(framed);
                let (px, py) = (u * sw, v * sh);
                if x0 == f32::MAX {
                    let (u1, v1) = to_stored((framed.0 + 1.0 / cw as f32, framed.1));
                    spacing = ((u1 - u) * sw).hypot((v1 - v) * sh).max(1.0);
                }
                (x0, y0, x1, y1) = (x0.min(px), y0.min(py), x1.max(px), y1.max(py));
            }
        }
        if x0 == f32::MAX {
            return None;
        }
        let cell = spacing.round().max(1.0) as u32;
        let reach = REACH.max(3 * cell);
        let pad = (reach + 2 * cell) as f32 + spacing;
        let bounds = Rect::from_edges(
            (x0 - pad).floor() as i32,
            (y0 - pad).floor() as i32,
            (x1 + pad).ceil() as i32,
            (y1 + pad).ceil() as i32,
        )?
        .within(size)?;
        let (w, h) = (bounds.w.div_ceil(cell), bounds.h.div_ceil(cell));
        let mut on = vec![false; (w * h) as usize];
        for gy in 0..h {
            for gx in 0..w {
                let px = bounds.x as f32 + (gx as f32 + 0.5) * cell as f32;
                let py = bounds.y as f32 + (gy as f32 + 0.5) * cell as f32;
                on[(gy * w + gx) as usize] = coverage.at(to_framed((px / sw, py / sh))) >= 0.5;
            }
        }
        let mut grid = Grid {
            x: bounds.x,
            y: bounds.y,
            cell,
            w,
            h,
            on,
        };
        grid.on = grid.grow(&grid.on, reach.div_ceil(cell));
        grid.bounds(&grid.on).map(|_| grid)
    }

    fn cells(&self) -> usize {
        (self.w * self.h) as usize
    }

    /// The cell a stored pixel's centre falls in, if any.
    fn cell_at(&self, px: f32, py: f32) -> Option<usize> {
        let gx = ((px - self.x as f32) / self.cell as f32).floor();
        let gy = ((py - self.y as f32) / self.cell as f32).floor();
        (gx >= 0.0 && gy >= 0.0 && gx < self.w as f32 && gy < self.h as f32)
            .then(|| gy as usize * self.w as usize + gx as usize)
    }

    /// Whether the stored pixel centred at `(px, py)` is in `set`.
    fn has(&self, set: &[bool], px: f32, py: f32) -> bool {
        self.cell_at(px, py).is_some_and(|i| set[i])
    }

    /// The centre of cell `i`, in stored pixels.
    fn centre(&self, i: usize) -> (f32, f32) {
        let (gx, gy) = (i as u32 % self.w, i as u32 / self.w);
        (
            self.x as f32 + (gx as f32 + 0.5) * self.cell as f32,
            self.y as f32 + (gy as f32 + 0.5) * self.cell as f32,
        )
    }

    /// The stored pixels `set` covers.
    fn bounds(&self, set: &[bool]) -> Option<Rect> {
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        for (i, _) in set.iter().enumerate().filter(|(_, on)| **on) {
            let (gx, gy) = (i as u32 % self.w, i as u32 / self.w);
            (x0, y0, x1, y1) = (x0.min(gx), y0.min(gy), x1.max(gx + 1), y1.max(gy + 1));
        }
        let c = self.cell as i32;
        Rect::from_edges(
            self.x + x0 as i32 * c,
            self.y + y0 as i32 * c,
            self.x + x1 as i32 * c,
            self.y + y1 as i32 * c,
        )
    }

    /// `set` grown by `r` cells in every direction (a square reach).
    fn grow(&self, set: &[bool], r: u32) -> Vec<bool> {
        let (w, h, r) = (self.w as usize, self.h as usize, r as usize);
        let mut rows = vec![false; set.len()];
        for y in 0..h {
            for x in (0..w).filter(|&x| set[y * w + x]) {
                rows[y * w + x.saturating_sub(r)..y * w + (x + r + 1).min(w)].fill(true);
            }
        }
        let mut out = vec![false; set.len()];
        for y in 0..h {
            for x in (0..w).filter(|&x| rows[y * w + x]) {
                for yy in y.saturating_sub(r)..(y + r + 1).min(h) {
                    out[yy * w + x] = true;
                }
            }
        }
        out
    }
}

// ------------------------------------------------------------------------
// Where the model looks.
// ------------------------------------------------------------------------

/// One run of the model.
#[derive(Debug, Clone)]
pub struct Window {
    /// Square, in stored pixels; can reach past the image on its short side.
    pub rect: Rect,
    /// The grid cells the model fills: everything still selected that it
    /// can see.
    seen: Vec<bool>,
    /// The cells whose fill is kept from this run.
    keep: Vec<bool>,
}

impl Window {
    /// Sensor pixels a model pixel.
    pub fn squeeze(&self) -> f32 {
        self.rect.w as f32 / MODEL_EDGE as f32
    }
}

/// How an erase is filled.
#[derive(Debug, Clone)]
pub struct Plan {
    pub grid: Grid,
    pub windows: Vec<Window>,
}

impl Plan {
    /// Whether the fill will come out soft, squeezed past [`SOFT_SQUEEZE`].
    pub fn soft(&self) -> bool {
        self.windows.iter().any(|w| w.squeeze() > SOFT_SQUEEZE)
    }

    /// Every window, within the image: the part of it to read.
    pub fn reach(&self, size: (u32, u32)) -> Option<Rect> {
        self.windows
            .iter()
            .filter_map(|w| w.rect.within(size))
            .reduce(|a, b| a.union(&b))
    }
}

/// The windows, or tiles, to fill `grid` with, in an image of `size`. Each
/// object gets its own, so several people taken out at once fill as sharply
/// as one; each window masks every part of the selection still unfilled that
/// it can see, but keeps only its own object's fill.
pub fn plan(grid: Grid, size: (u32, u32)) -> Option<Plan> {
    let mut left = grid.on.clone();
    let mut windows = Vec::new();
    for part in objects(&grid, size) {
        let bounds = grid.bounds(&part)?;
        let single = Window {
            rect: around(bounds, size),
            seen: left.clone(),
            keep: part.clone(),
        };
        if single.squeeze() <= MAX_SQUEEZE {
            windows.push(single);
        } else {
            // The smallest tiles that leave each enough to go on, while they
            // squeeze clearly less than the one window would.
            windows.extend(
                TILE_EDGES
                    .into_iter()
                    .take_while(|&edge| (edge as f32) < 0.8 * single.rect.w as f32)
                    .find_map(|edge| tiles(&grid, &part, &left, bounds, size, edge))
                    .unwrap_or_else(|| vec![single]),
            );
        }
        for (l, p) in left.iter_mut().zip(&part) {
            *l &= !p;
        }
    }
    (!windows.is_empty()).then_some(Plan { grid, windows })
}

/// The selection split into separate objects: what lies within a reach of
/// itself stays together, and objects one sharp window can hold share it.
fn objects(grid: &Grid, size: (u32, u32)) -> Vec<Vec<bool>> {
    let near = grid.grow(&grid.on, REACH.div_ceil(grid.cell));
    let (w, h) = (grid.w as usize, grid.h as usize);
    let mut seen = vec![false; grid.cells()];
    let mut parts: Vec<(Vec<bool>, Rect)> = Vec::new();
    for start in 0..grid.cells() {
        if !near[start] || seen[start] {
            continue;
        }
        let mut part = vec![false; grid.cells()];
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(i) = stack.pop() {
            part[i] = grid.on[i];
            let (x, y) = (i % w, i / w);
            let next = [
                (x > 0).then(|| i - 1),
                (x + 1 < w).then(|| i + 1),
                (y > 0).then(|| i - w),
                (y + 1 < h).then(|| i + w),
            ];
            for j in next.into_iter().flatten() {
                if near[j] && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            }
        }
        // Every reach-grown piece holds some of the selection.
        if let Some(b) = grid.bounds(&part) {
            parts.push((part, b));
        }
    }
    let sharp = |b: Rect| around(b, size).w as f32 / MODEL_EDGE as f32 <= MAX_SQUEEZE;
    'merge: loop {
        for i in 0..parts.len() {
            for j in i + 1..parts.len() {
                let both = parts[i].1.union(&parts[j].1);
                if sharp(both) {
                    let (other, _) = parts.remove(j);
                    for (a, b) in parts[i].0.iter_mut().zip(other) {
                        *a |= b;
                    }
                    parts[i].1 = both;
                    continue 'merge;
                }
            }
        }
        return parts.into_iter().map(|(p, _)| p).collect();
    }
}

/// The square window around `object`: 1.6 times its long side and a
/// margin, at least the model's own edge, at most the image's long side.
/// Placed inside the image where it fits, centred on it where it does not.
fn around(object: Rect, (sw, sh): (u32, u32)) -> Rect {
    let long = object.w.max(object.h) as f32;
    let side = ((1.6 * long + 64.0).max(MODEL_EDGE as f32) as u32).min(sw.max(sh));
    let place = |start: i32, len: u32, dim: u32| -> i32 {
        if side <= dim {
            (start + len as i32 / 2 - side as i32 / 2).clamp(0, (dim - side) as i32)
        } else {
            (dim as i32 - side as i32) / 2
        }
    };
    Rect {
        x: place(object.x, object.w, sw),
        y: place(object.y, object.h, sh),
        w: side,
        h: side,
    }
}

/// Tiles of `edge` along `part`, a long object of `bounds`, masking all of
/// `left` (the selection still unfilled) they see; `None` when one of them
/// would be too much hole, which an object thick for the size makes.
fn tiles(
    grid: &Grid,
    part: &[bool],
    left: &[bool],
    bounds: Rect,
    (sw, sh): (u32, u32),
    edge: u32,
) -> Option<Vec<Window>> {
    let edge = edge.min(sw).min(sh);
    let margin = edge / 6;
    let inner = edge - 2 * margin;
    let starts = |from: i32, len: u32, dim: u32| -> Vec<i32> {
        let last = dim.saturating_sub(edge) as i32;
        let mut out = Vec::new();
        let mut at = from - margin as i32;
        loop {
            out.push(at.clamp(0, last));
            if at + (margin + inner) as i32 >= from + len as i32 {
                return out;
            }
            at += inner as i32;
        }
    };
    let mut left = left.to_vec();
    let mut windows = Vec::new();
    for y in starts(bounds.y, bounds.h, sh) {
        for x in starts(bounds.x, bounds.w, sw) {
            let rect = Rect {
                x,
                y,
                w: edge,
                h: edge,
            };
            let (m, e) = (margin as f32, edge as f32);
            // A tile at the image's edge keeps its margin there too: no
            // other tile reaches it.
            let lo = |at: i32| if at == 0 { 0.0 } else { m };
            let hi = |at: i32, dim: u32| {
                if at + edge as i32 >= dim as i32 {
                    e
                } else {
                    e - m
                }
            };
            let (kx0, ky0, kx1, ky1) = (lo(x), lo(y), hi(x, sw), hi(y, sh));
            let mut seen = vec![false; grid.cells()];
            let mut keep = vec![false; grid.cells()];
            let mut holes = 0;
            for i in (0..grid.cells()).filter(|&i| left[i]) {
                let (cx, cy) = grid.centre(i);
                let (lx, ly) = (cx - x as f32, cy - y as f32);
                if !(0.0..e).contains(&lx) || !(0.0..e).contains(&ly) {
                    continue;
                }
                seen[i] = true;
                holes += 1;
                if part[i] && (kx0..kx1).contains(&lx) && (ky0..ky1).contains(&ly) {
                    keep[i] = true;
                }
            }
            let hole = holes as f32 * (grid.cell * grid.cell) as f32 / (e * e);
            if hole > TILE_HOLE_LIMIT {
                return None;
            }
            if keep.iter().any(|&k| k) {
                for (l, k) in left.iter_mut().zip(&keep) {
                    *l &= !k;
                }
                windows.push(Window { rect, seen, keep });
            }
        }
    }
    (!left.iter().zip(part).any(|(&l, &p)| l && p)).then_some(windows)
}

// ------------------------------------------------------------------------
// Into the model and back.
// ------------------------------------------------------------------------

/// Part of the linear image, as RGBA half floats (their bits).
#[derive(Debug, Clone)]
pub struct Region {
    /// Where it lies, inside the image.
    pub rect: Rect,
    pub rgba: Vec<u16>,
}

impl Region {
    /// Channel `c` at stored pixel `(x, y)`, clamped into the region.
    fn at(&self, x: i32, y: i32, c: usize) -> f32 {
        let x = (x - self.rect.x).clamp(0, self.rect.w as i32 - 1) as usize;
        let y = (y - self.rect.y).clamp(0, self.rect.h as i32 - 1) as usize;
        f16::from_bits(self.rgba[(y * self.rect.w as usize + x) * 4 + c]).to_f32()
    }
}

/// One window, ready for the model: RGB interleaved and encoded 0..1, and
/// the hole (1) to fill, both [`MODEL_EDGE`]².
pub struct ModelInput {
    pub rgb: Vec<f32>,
    pub hole: Vec<f32>,
    /// What the linear values were divided by before encoding.
    pub scale: f32,
}

/// `window` of `region`, averaged down to the model's size.
pub fn model_input(region: &Region, grid: &Grid, window: &Window) -> ModelInput {
    let m = MODEL_EDGE as usize;
    let f = window.squeeze();
    // Enough samples a model pixel to average every sensor pixel under it.
    let n = f.ceil().max(1.0) as usize;
    let (wx, wy) = (window.rect.x as f32, window.rect.y as f32);
    let mut rgb = vec![0.0f32; m * m * 3];
    let mut hole = vec![0.0f32; m * m];
    for my in 0..m {
        for mx in 0..m {
            let mut sum = [0.0f32; 3];
            for j in 0..n {
                for i in 0..n {
                    let sx = (wx + (mx as f32 + (i as f32 + 0.5) / n as f32) * f).floor() as i32;
                    let sy = (wy + (my as f32 + (j as f32 + 0.5) / n as f32) * f).floor() as i32;
                    for (c, s) in sum.iter_mut().enumerate() {
                        *s += region.at(sx, sy, c);
                    }
                }
            }
            let k = my * m + mx;
            for c in 0..3 {
                rgb[k * 3 + c] = sum[c] / (n * n) as f32;
            }
            let (cx, cy) = (wx + (mx as f32 + 0.5) * f, wy + (my as f32 + 0.5) * f);
            if grid.has(&window.seen, cx, cy) {
                hole[k] = 1.0;
            }
        }
    }
    // The 99.9th percentile of what the model is shown, never below 1.
    let mut known: Vec<f32> = (0..m * m)
        .filter(|&k| hole[k] == 0.0)
        .flat_map(|k| [rgb[k * 3], rgb[k * 3 + 1], rgb[k * 3 + 2]])
        .filter(|v| v.is_finite())
        .collect();
    let scale = if known.is_empty() {
        1.0
    } else {
        let at = ((known.len() - 1) as f32 * 0.999) as usize;
        known
            .select_nth_unstable_by(at, |a, b| a.total_cmp(b))
            .1
            .max(1.0)
    };
    for v in rgb.iter_mut() {
        *v = crate::denoise::encode(*v, scale);
    }
    ModelInput { rgb, hole, scale }
}

/// The model's answer for `window` (interleaved RGB, encoded 0..1) as the
/// fill to keep: back to linear, with the share it is kept for, feathered.
pub fn fill_from(output: &[f32], scale: f32, grid: &Grid, window: &Window) -> Fill {
    let m = MODEL_EDGE as usize;
    let f = window.squeeze();
    let (wx, wy) = (window.rect.x as f32, window.rect.y as f32);
    let mut keep = vec![0.0f32; m * m];
    for (k, a) in keep.iter_mut().enumerate() {
        let (cx, cy) = (
            wx + ((k % m) as f32 + 0.5) * f,
            wy + ((k / m) as f32 + 0.5) * f,
        );
        if grid.has(&window.keep, cx, cy) {
            *a = 1.0;
        }
    }
    // Soften the edge outwards: what was kept stays wholly the fill, and it
    // fades into the photo over the next couple of model pixels.
    let alpha: Vec<f32> = box_blur(&keep, m, m, 1, 2)
        .iter()
        .map(|a| (a * 2.0).min(1.0))
        .collect();
    // The colour is kept wherever a bilinear read of a used pixel can reach.
    let used: Vec<bool> = alpha.iter().map(|&a| a > 0.0).collect();
    let near = |k: usize| {
        let (x, y) = ((k % m) as i32, (k / m) as i32);
        (-2..=2).any(|dy| {
            (-2..=2).any(|dx| {
                let (xx, yy) = (x + dx, y + dy);
                xx >= 0
                    && yy >= 0
                    && (xx as usize) < m
                    && (yy as usize) < m
                    && used[yy as usize * m + xx as usize]
            })
        })
    };
    let mut rgba = vec![0u16; m * m * 4];
    for k in 0..m * m {
        if near(k) {
            for c in 0..3 {
                let v = crate::denoise::decode(output[k * 3 + c], scale);
                rgba[k * 4 + c] = f16::from_f32(v).to_bits();
            }
        }
        rgba[k * 4 + 3] = f16::from_f32(alpha[k]).to_bits();
    }
    Fill {
        size: MODEL_EDGE,
        rgba,
    }
}

/// `passes` box blurs of radius `r` over a `w` x `h` plane.
fn box_blur(plane: &[f32], w: usize, h: usize, r: usize, passes: usize) -> Vec<f32> {
    let mut a = plane.to_vec();
    for _ in 0..passes {
        for horizontal in [true, false] {
            let src = a.clone();
            for y in 0..h {
                for x in 0..w {
                    let (mut s, mut n) = (0.0, 0.0);
                    for d in -(r as isize)..=(r as isize) {
                        let (xx, yy) = if horizontal {
                            (x as isize + d, y as isize)
                        } else {
                            (x as isize, y as isize + d)
                        };
                        if xx >= 0 && yy >= 0 && (xx as usize) < w && (yy as usize) < h {
                            s += src[yy as usize * w + xx as usize];
                            n += 1.0;
                        }
                    }
                    a[y * w + x] = s / n;
                }
            }
        }
    }
    a
}

fn bilinear(at: impl Fn(usize, usize) -> f32, w: usize, h: usize, x: f32, y: f32) -> f32 {
    let x = x.clamp(0.0, (w - 1) as f32);
    let y = y.clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
    let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
    top * (1.0 - fy) + bottom * fy
}

/// A standard normal value, the same every time for the same `seed`.
fn gauss(seed: u64) -> f32 {
    let mix = |mut x: u64| {
        x ^= x >> 30;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 27;
        x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^ (x >> 31)
    };
    let a = mix(seed.wrapping_add(0x9E37_79B9_7F4A_7C15));
    let b = mix(a);
    let u1 = ((a >> 11) as f64 / (1u64 << 53) as f64).max(1e-12);
    let u2 = (b >> 11) as f64 / (1u64 << 53) as f64;
    ((-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()) as f32
}

/// How strong the sensor noise is in `rect` of `region`, per channel: the
/// spread of a 3x3 high pass over the pixels `fill` leaves alone, measured
/// robustly (the median absolute deviation) so edges do not count as noise.
fn noise(region: &Region, rect: Rect, alpha: impl Fn(i32, i32) -> f32) -> [f32; 3] {
    let Some(r) = rect.intersect(&region.rect) else {
        return [0.0; 3];
    };
    let mut sigma = [0.0f32; 3];
    for (c, s) in sigma.iter_mut().enumerate() {
        let mut highs = Vec::new();
        for y in (r.y + 1..r.bottom() - 1).step_by(3) {
            for x in (r.x + 1..r.right() - 1).step_by(3) {
                if alpha(x, y) > 0.0 {
                    continue;
                }
                let mut sum = 0.0;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        sum += region.at(x + dx, y + dy, c);
                    }
                }
                highs.push((region.at(x, y, c) - sum / 9.0).abs());
            }
        }
        if highs.len() < 64 {
            continue;
        }
        let mid = highs.len() / 2;
        let mad = *highs.select_nth_unstable_by(mid, |a, b| a.total_cmp(b)).1;
        // A 3x3 high pass keeps 8/9 of white noise's variance.
        *s = 1.4826 * mad / (8.0f32 / 9.0).sqrt();
    }
    sigma
}

/// Lays `piece`'s fill into `region`, with grain to match the noise around
/// it. The region should hold the piece's whole window inside the image
/// ([`dirty_regions`]), so the grain is measured on the same pixels however
/// the fill comes to be laid in.
pub fn composite(region: &mut Region, piece: &Piece, fill: &Fill) {
    use rayon::prelude::*;
    let rect = piece.rect();
    let Some(r) = rect.intersect(&region.rect) else {
        return;
    };
    let m = fill.size as usize;
    let f = rect.w as f32 / fill.size as f32;
    let plane: Vec<f32> = fill
        .rgba
        .iter()
        .map(|&b| f16::from_bits(b).to_f32())
        .collect();
    let sample = |x: i32, y: i32, c: usize| {
        let u = (x - rect.x) as f32 + 0.5;
        let v = (y - rect.y) as f32 + 0.5;
        bilinear(
            |xx, yy| plane[(yy * m + xx) * 4 + c],
            m,
            m,
            u / f - 0.5,
            v / f - 0.5,
        )
    };
    let sigma = noise(region, rect, |x, y| sample(x, y, 3));
    let seed = piece.seed();
    let (rw, rx, ry) = (region.rect.w as usize, region.rect.x, region.rect.y);
    region
        .rgba
        .par_chunks_mut(rw * 4)
        .enumerate()
        .filter(|(row, _)| (r.y..r.bottom()).contains(&(ry + *row as i32)))
        .for_each(|(row, line)| {
            let y = ry + row as i32;
            for x in r.x..r.right() {
                let a = sample(x, y, 3);
                if a <= 0.0 {
                    continue;
                }
                let i = (x - rx) as usize * 4;
                // Mostly shared between the channels, as demosaiced sensor
                // noise is: grain drawn for each alone reads as colour
                // speckle. 0.84² + 0.55² ≈ 1, so each keeps its strength.
                let at = seed ^ ((x as u64) << 34) ^ ((y as u64) << 4);
                let shared = gauss(at ^ 3);
                for c in 0..3 {
                    let grain = sigma[c] * (0.84 * shared + 0.55 * gauss(at ^ c as u64));
                    let fill = sample(x, y, c) + grain;
                    let now = f16::from_bits(line[i + c]).to_f32();
                    line[i + c] = f16::from_f32(now + (fill - now) * a.min(1.0)).to_bits();
                }
            }
        });
}

/// Lays every erase's fills into the whole linear image (`size` pixels), in
/// order. A fill `fills` cannot give is left out; the names left out come
/// back.
pub fn bake(
    rgba: &mut Vec<u16>,
    size: (u32, u32),
    erases: &[Erase],
    fills: impl Fn(&str) -> Option<Arc<Fill>>,
) -> Vec<String> {
    let mut region = Region {
        rect: Rect {
            x: 0,
            y: 0,
            w: size.0,
            h: size.1,
        },
        rgba: std::mem::take(rgba),
    };
    let missing = lay_in(&mut region, erases, fills);
    *rgba = region.rgba;
    missing
}

/// Lays every fill whose window touches `region` into it, in order: what
/// [`bake`] does to the whole image, for part of it.
pub fn lay_in(
    region: &mut Region,
    erases: &[Erase],
    fills: impl Fn(&str) -> Option<Arc<Fill>>,
) -> Vec<String> {
    let mut missing = Vec::new();
    for piece in erases.iter().flat_map(|e| &e.pieces) {
        if piece.rect().intersect(&region.rect).is_none() {
            continue;
        }
        match fills(&piece.fill) {
            Some(fill) => composite(region, piece, &fill),
            None => missing.push(piece.fill.clone()),
        }
    }
    missing
}

/// One fill worked out by [`run`]: where it goes, and the fill itself.
pub struct Made {
    pub piece: Piece,
    pub fill: Fill,
    pub png: Vec<u8>,
}

/// Fills every window of `plan` in turn, laying each fill into `region` (the
/// plan's [`Plan::reach`]) before the next window is read, so a tile sees
/// the ones before it filled. `model` takes a [`ModelInput`] and gives back
/// interleaved RGB, encoded 0..1.
pub fn run(
    region: &mut Region,
    plan: &Plan,
    mut model: impl FnMut(&ModelInput) -> Result<Vec<f32>>,
    mut progress: impl FnMut(usize, usize),
) -> Result<Vec<Made>> {
    let mut made = Vec::with_capacity(plan.windows.len());
    for (i, window) in plan.windows.iter().enumerate() {
        progress(i, plan.windows.len());
        let input = model_input(region, &plan.grid, window);
        let output = model(&input)?;
        anyhow::ensure!(
            output.len() == input.rgb.len(),
            "the model gave {} values for {}",
            output.len(),
            input.rgb.len()
        );
        let fill = fill_from(&output, input.scale, &plan.grid, window);
        let png = fill.to_png()?;
        let piece = Piece {
            fill: fill_name(&png),
            x: window.rect.x,
            y: window.rect.y,
            size: window.rect.w,
        };
        composite(region, &piece, &fill);
        made.push(Made { piece, fill, png });
    }
    progress(plan.windows.len(), plan.windows.len());
    Ok(made)
}

#[cfg(feature = "ai")]
pub use imp::Inpainter;

#[cfg(feature = "ai")]
mod imp {
    use super::*;

    /// LaMa, loaded once.
    pub struct Inpainter {
        session: std::sync::Mutex<ort::session::Session>,
    }

    impl Inpainter {
        pub fn load(models_dir: &Path) -> Result<Self> {
            crate::develop::ai::ensure_ort_dylib();
            let path = models_dir.join(MODEL_FILE);
            anyhow::ensure!(
                path.exists(),
                "Magic erase isn't downloaded yet: get it in Settings → On-device models"
            );
            // On the CPU everywhere. CoreML cannot compile LaMa, and finding
            // that out takes seconds on every load. DirectML accepts the
            // graph and then fails its first pass, at a MatMul inside the
            // Fourier units ("The parameter is incorrect"), on an RTX 5070 Ti
            // and on integrated Radeon graphics alike: on a Windows PC with
            // any GPU, magic erase never got past its first window.
            let session = crate::develop::ai::commit_on_cpu(&path)?;
            Ok(Self {
                session: std::sync::Mutex::new(session),
            })
        }

        /// Runs the model once on nothing in particular: the first pass after
        /// loading takes a second longer than the rest, so it is better spent
        /// before anyone is waiting.
        pub fn warm(&self) -> Result<()> {
            let plane = (MODEL_EDGE * MODEL_EDGE) as usize;
            let mut hole = vec![0.0f32; plane];
            hole[plane / 2] = 1.0;
            self.fill(&ModelInput {
                rgb: vec![0.5; plane * 3],
                hole,
                scale: 1.0,
            })
            .map(|_| ())
        }

        /// One window through the model: interleaved RGB in and out,
        /// encoded 0..1.
        pub fn fill(&self, input: &ModelInput) -> Result<Vec<f32>> {
            let plane = (MODEL_EDGE * MODEL_EDGE) as usize;
            let mut planar = vec![0.0f32; 3 * plane];
            for k in 0..plane {
                for c in 0..3 {
                    planar[c * plane + k] = input.rgb[k * 3 + c];
                }
            }
            let e = MODEL_EDGE as i64;
            let image = ort::value::Value::from_array(([1_i64, 3, e, e], planar))?;
            let mask = ort::value::Value::from_array(([1_i64, 1, e, e], input.hole.clone()))?;
            let mut session = crate::sync::lock_recover(&self.session);
            let outputs = session.run(ort::inputs!["image" => image, "mask" => mask])?;
            let (_shape, out) = outputs[0].try_extract_tensor::<f32>()?;
            anyhow::ensure!(out.len() == 3 * plane, "LaMa gave {} values", out.len());
            // This export answers in 0..255.
            let mut rgb = vec![0.0f32; 3 * plane];
            for k in 0..plane {
                for c in 0..3 {
                    rgb[k * 3 + c] = (out[c * plane + k] / 255.0).clamp(0.0, 1.0);
                }
            }
            Ok(rgb)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(fill: &str, x: i32, y: i32, size: u32) -> Piece {
        Piece {
            fill: fill.repeat(64 / fill.len()),
            x,
            y,
            size,
        }
    }

    fn erase(pieces: Vec<Piece>) -> Erase {
        Erase {
            label: "Person".into(),
            pieces,
        }
    }

    /// A grey image with a little noise, as half floats.
    fn noisy(w: u32, h: u32, level: f32) -> Vec<u16> {
        (0..w * h)
            .flat_map(|i| {
                let n = |c: u64| level + 0.01 * gauss(i as u64 * 4 + c);
                [n(0), n(1), n(2), 1.0].map(|v| f16::from_f32(v).to_bits())
            })
            .collect()
    }

    /// A selection that is an upright bar, `bw` x `bh` texels at `(bx, by)`
    /// of a `cw` x `ch` coverage.
    fn bar(cw: u32, ch: u32, bx: u32, by: u32, bw: u32, bh: u32) -> Vec<u8> {
        (0..cw * ch)
            .map(|i| {
                let (x, y) = (i % cw, i / cw);
                if (bx..bx + bw).contains(&x) && (by..by + bh).contains(&y) {
                    255
                } else {
                    0
                }
            })
            .collect()
    }

    fn grid_of(data: &[u8], cw: u32, ch: u32, size: (u32, u32)) -> Grid {
        Grid::from_coverage(
            &Coverage {
                data,
                width: cw,
                height: ch,
            },
            size,
            |uv| uv,
            |uv| uv,
        )
        .expect("something selected")
    }

    #[test]
    fn rectangles_meet_and_join() {
        let a = Rect {
            x: 0,
            y: 0,
            w: 10,
            h: 10,
        };
        let b = Rect {
            x: 5,
            y: -5,
            w: 10,
            h: 10,
        };
        assert_eq!(
            a.intersect(&b),
            Some(Rect {
                x: 5,
                y: 0,
                w: 5,
                h: 5
            })
        );
        assert_eq!(
            a.union(&b),
            Rect {
                x: 0,
                y: -5,
                w: 15,
                h: 15
            }
        );
        assert_eq!(
            a.intersect(&Rect {
                x: 10,
                y: 0,
                w: 4,
                h: 4
            }),
            None
        );
        assert_eq!(
            b.within((12, 12)),
            Some(Rect {
                x: 5,
                y: 0,
                w: 7,
                h: 5
            })
        );
    }

    /// Fill names are what the files are called, so only a SHA-256 in hex
    /// may become one: a stack from the webview cannot name `../x`.
    #[test]
    fn only_a_sha256_names_a_fill() {
        let name = fill_name(b"fill");
        assert!(is_fill_name(&name));
        assert!(!is_fill_name("../../etc/passwd"));
        assert!(!is_fill_name(&name.to_uppercase()));
        assert!(!is_fill_name(&name[1..]));
        assert!(FillStore::new(PathBuf::from("/tmp")).path("..").is_err());
    }

    /// A fill survives the trip through its file exactly, and the file is
    /// named by its content, so saving it again changes nothing.
    #[test]
    fn a_fill_is_kept_exactly_under_its_own_name() {
        let dir = crate::test_support::Scratch::new("erase-fills");
        let store = FillStore::new(dir.join("patches"));
        let fill = Fill {
            size: 8,
            rgba: (0..8 * 8 * 4)
                .map(|i| f16::from_f32(i as f32 * 0.37 - 20.0).to_bits())
                .collect(),
        };
        let png = fill.to_png().unwrap();
        let name = store.save(&png).unwrap();
        assert_eq!(name, fill_name(&png));
        assert_eq!(store.save(&png).unwrap(), name);
        assert_eq!(store.load(&name).unwrap(), fill);
        // A file that no longer matches its name is refused.
        std::fs::write(store.dir().join(format!("{name}.png")), b"junk").unwrap();
        assert!(store.load(&name).is_err());
        assert!(store.load(&fill_name(b"never saved")).is_err());
    }

    /// The selection lands on the stored image through the mapping it is
    /// given, grown by the erase's reach.
    #[test]
    fn a_selection_is_carried_onto_the_stored_image() {
        // 100 x 50 texels over a 2000 x 1000 image: 20 px a texel.
        let data = bar(100, 50, 40, 10, 5, 20);
        let grid = grid_of(&data, 100, 50, (2000, 1000));
        assert_eq!(grid.cell, 20);
        let b = grid.bounds(&grid.on).unwrap();
        // 100 px wide, 400 tall, grown by three cells (60 px) a side, give
        // or take the cell a texel's edge falls in.
        let near = |got: i32, want: i32| (got - want).abs() <= 20;
        assert!(near(b.x, 800 - 60) && near(b.w as i32, 100 + 120), "{b:?}");
        assert!(near(b.y, 200 - 60) && near(b.h as i32, 400 + 120), "{b:?}");
        assert!(grid.has(&grid.on, 850.0, 400.0));
        assert!(!grid.has(&grid.on, 500.0, 400.0));

        // Mirrored by the mapping, it lands mirrored.
        let flipped = Grid::from_coverage(
            &Coverage {
                data: &data,
                width: 100,
                height: 50,
            },
            (2000, 1000),
            |(u, v)| (1.0 - u, v),
            |(u, v)| (1.0 - u, v),
        )
        .unwrap();
        let fb = flipped.bounds(&flipped.on).unwrap();
        assert!(
            near(fb.x, 2000 - (800 + 100 + 60)) && near(fb.w as i32, 100 + 120),
            "{fb:?}"
        );

        let none = vec![0u8; 100 * 50];
        assert!(Grid::from_coverage(
            &Coverage {
                data: &none,
                width: 100,
                height: 50
            },
            (2000, 1000),
            |uv| uv,
            |uv| uv
        )
        .is_none());
    }

    /// A small object gets one window around it, inside the image.
    #[test]
    fn a_small_object_is_filled_in_one_window() {
        let data = bar(600, 400, 300, 200, 10, 20);
        let plan = plan(grid_of(&data, 600, 400, (6000, 4000)), (6000, 4000)).unwrap();
        assert_eq!(plan.windows.len(), 1);
        let w = &plan.windows[0];
        assert!(w.squeeze() <= MAX_SQUEEZE, "{}", w.squeeze());
        assert!(w.rect.within((6000, 4000)) == Some(w.rect));
        assert!(!plan.soft());
        // The object is inside its window.
        let b = plan.grid.bounds(&plan.grid.on).unwrap();
        assert_eq!(b.intersect(&w.rect), Some(b));
    }

    /// Two people far apart, taken out at once, each get their own sharp
    /// window rather than one squeezed window over both; two close together
    /// share one.
    #[test]
    fn objects_far_apart_get_a_window_each() {
        let size = (6000, 4000);
        let both =
            |a: Vec<u8>, b: Vec<u8>| -> Vec<u8> { a.iter().zip(&b).map(|(x, y)| x | y).collect() };
        let data = both(
            bar(600, 400, 50, 50, 10, 20),
            bar(600, 400, 500, 300, 10, 20),
        );
        let plan = plan(grid_of(&data, 600, 400, size), size).unwrap();
        assert_eq!(plan.windows.len(), 2);
        let mut kept = vec![0; plan.grid.cells()];
        for w in &plan.windows {
            assert!(w.squeeze() <= MAX_SQUEEZE, "{}", w.squeeze());
            // Its own window around it, not a tile of the span of both.
            assert!(w.rect.w < TILE_EDGES[0], "{:?}", w.rect);
            let b = plan.grid.bounds(&w.keep).unwrap();
            assert_eq!(b.intersect(&w.rect), Some(b));
            for (i, k) in w.keep.iter().enumerate() {
                kept[i] += *k as i32;
            }
        }
        for (i, on) in plan.grid.on.iter().enumerate() {
            assert_eq!(kept[i], *on as i32, "cell {i}");
        }

        let data = both(bar(600, 400, 50, 50, 10, 20), bar(600, 400, 80, 50, 10, 20));
        let close = super::plan(grid_of(&data, 600, 400, size), size).unwrap();
        assert_eq!(close.windows.len(), 1);
    }

    /// A long, thin object is tiled at a squeeze of two, and every part of
    /// it is kept from exactly one tile.
    #[test]
    fn a_pole_is_tiled() {
        // 12 x 200 texels at 10 px: a 120 x 2000 px pole.
        let data = bar(600, 400, 300, 100, 12, 200);
        let plan = plan(grid_of(&data, 600, 400, (6000, 4000)), (6000, 4000)).unwrap();
        assert!(plan.windows.len() >= 2, "{}", plan.windows.len());
        let mut kept = vec![0; plan.grid.cells()];
        for w in &plan.windows {
            assert_eq!(w.rect.w, TILE_EDGES[0]);
            assert!(w.squeeze() <= MAX_SQUEEZE);
            for (i, k) in w.keep.iter().enumerate() {
                if *k {
                    kept[i] += 1;
                    assert!(w.seen[i], "a tile keeps only what it filled");
                }
            }
        }
        for (i, on) in plan.grid.on.iter().enumerate() {
            assert_eq!(kept[i], *on as i32, "cell {i}");
        }
        // Each tile fills all of the pole it can see, not just its share.
        let first = &plan.windows[0];
        assert!(
            first.seen.iter().filter(|s| **s).count() > first.keep.iter().filter(|k| **k).count()
        );
    }

    /// A wide object too thick for small tiles, a boat's hull, takes tiles
    /// big enough to see around it, squeezed less than one window would be.
    #[test]
    fn a_hull_takes_big_tiles() {
        // 400 x 90 texels at 10 px: a 4000 x 900 px hull.
        let size = (6000, 4000);
        let data = bar(600, 400, 100, 250, 400, 90);
        let plan = plan(grid_of(&data, 600, 400, size), size).unwrap();
        assert!(plan.windows.len() >= 2, "{}", plan.windows.len());
        let edge = plan.windows[0].rect.w;
        assert!(edge > TILE_EDGES[0] && edge < 4000, "{edge}");
        assert!(plan.windows.iter().all(|w| w.rect.w == edge));
        assert!(plan.windows[0].squeeze() < 6000.0 / MODEL_EDGE as f32);
    }

    /// A bulky object would leave a tile mostly hole: one window, soft.
    #[test]
    fn a_bulky_object_takes_one_soft_window() {
        let data = bar(600, 400, 150, 100, 300, 200);
        let plan = plan(grid_of(&data, 600, 400, (6000, 4000)), (6000, 4000)).unwrap();
        assert_eq!(plan.windows.len(), 1);
        assert!(plan.soft());
        // Wider than the image is tall: the window reaches past the top and
        // bottom, centred, and still holds the object.
        let w = plan.windows[0].rect;
        assert!(w.w > 4000 && w.y < 0 && w.bottom() > 4000, "{w:?}");
        let b = plan.grid.bounds(&plan.grid.on).unwrap();
        assert_eq!(b.intersect(&w), Some(b));
    }

    /// The model sees the window averaged down, encoded, with the selection
    /// as its hole; what comes back is laid in where the selection was and
    /// nowhere else.
    #[test]
    fn a_fill_replaces_the_selection_and_nothing_else() {
        let size = (1200, 800);
        let data = bar(120, 80, 50, 30, 10, 10);
        let plan = plan(grid_of(&data, 120, 80, size), size).unwrap();
        let reach = plan.reach(size).unwrap();
        let mut rgba = noisy(size.0, size.1, 0.2);
        let original = rgba.clone();
        let mut region = Region {
            rect: reach,
            rgba: Vec::new(),
        };
        for y in reach.y..reach.bottom() {
            let at = (y as usize * size.0 as usize + reach.x as usize) * 4;
            region
                .rgba
                .extend_from_slice(&rgba[at..at + reach.w as usize * 4]);
        }
        // A model that answers a flat 0.5 (encoded) everywhere.
        let made = run(
            &mut region,
            &plan,
            |input| {
                assert_eq!(input.rgb.len(), (MODEL_EDGE * MODEL_EDGE * 3) as usize);
                let holes = input.hole.iter().filter(|h| **h > 0.0).count();
                assert!(holes > 0 && holes < input.hole.len() / 2, "{holes}");
                assert_eq!(input.scale, 1.0);
                // 0.2 linear, encoded.
                let k = input.hole.iter().position(|h| *h == 0.0).unwrap();
                assert!((input.rgb[k * 3] - crate::denoise::encode(0.2, 1.0)).abs() < 0.06);
                Ok(vec![0.5; input.rgb.len()])
            },
            |_, _| {},
        )
        .unwrap();
        assert_eq!(made.len(), 1);
        let erases = vec![erase(made.iter().map(|m| m.piece.clone()).collect())];
        let fills: std::collections::HashMap<String, Arc<Fill>> = made
            .into_iter()
            .map(|m| (m.piece.fill.clone(), Arc::new(m.fill)))
            .collect();
        let missing = bake(&mut rgba, size, &erases, |n| fills.get(n).cloned());
        assert!(missing.is_empty());

        let at = |px: &[u16], x: u32, y: u32| {
            f16::from_bits(px[((y * size.0 + x) * 4) as usize]).to_f32()
        };
        let want = crate::denoise::decode(0.5, 1.0);
        // The middle of the selection is the fill, give or take the grain.
        assert!(
            (at(&rgba, 550, 350) - want).abs() < 0.05,
            "{}",
            at(&rgba, 550, 350)
        );
        // Well away from it, the photo, untouched.
        for (x, y) in [(100, 100), (1100, 700), (550, 50)] {
            assert_eq!(at(&rgba, x, y), at(&original, x, y), "({x}, {y})");
        }
        // Laid in the same way twice, it gives the same pixels: the grain is
        // seeded by where it is.
        let mut again = original.clone();
        bake(&mut again, size, &erases, |n| fills.get(n).cloned());
        assert_eq!(again, rgba);
        // The same as the run laid into the region it worked on.
        for (x, y) in [(550u32, 350u32), (500, 300), (445, 245)] {
            let row = (y - reach.y as u32) as usize;
            let col = (x - reach.x as u32) as usize;
            assert_eq!(
                region.rgba[(row * reach.w as usize + col) * 4],
                rgba[((y * size.0 + x) * 4) as usize],
                "({x}, {y})"
            );
        }
        // A fill that cannot be found is said so, and the rest is laid in.
        let mut gone = original.clone();
        assert_eq!(bake(&mut gone, size, &erases, |_| None).len(), 1);
        assert_eq!(gone, original);
    }

    /// The grain laid over a fill is as strong as the noise around it.
    #[test]
    fn the_grain_matches_the_noise_around_the_fill() {
        let size = (600, 600);
        let mut rgba = noisy(size.0, size.1, 0.3);
        let mut alpha = vec![0u16; 64 * 64 * 4];
        for y in 16..48 {
            for x in 16..48 {
                let k = (y * 64 + x) * 4;
                for c in 0..3 {
                    alpha[k + c] = f16::from_f32(0.3).to_bits();
                }
                alpha[k + 3] = f16::from_f32(1.0).to_bits();
            }
        }
        let fill = Fill {
            size: 64,
            rgba: alpha,
        };
        let p = piece("ab", 0, 0, 600);
        let mut region = Region {
            rect: Rect {
                x: 0,
                y: 0,
                w: 600,
                h: 600,
            },
            rgba: std::mem::take(&mut rgba),
        };
        composite(&mut region, &p, &fill);
        let spread = |x0: i32, y0: i32| {
            let v: Vec<f32> = (y0..y0 + 60)
                .flat_map(|y| (x0..x0 + 60).map(move |x| (x, y)))
                .map(|(x, y)| region.at(x, y, 1))
                .collect();
            let mean = v.iter().sum::<f32>() / v.len() as f32;
            (v.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / v.len() as f32).sqrt()
        };
        let (inside, outside) = (spread(270, 270), spread(20, 20));
        assert!(
            (inside / outside - 1.0).abs() < 0.25,
            "grain {inside} against noise {outside}"
        );
    }

    /// Only what changed is laid in again, grown to whole windows that touch
    /// it, and windows far apart stay apart.
    #[test]
    fn only_the_changed_windows_are_laid_in_again() {
        let size = (4000, 3000);
        let a = erase(vec![piece("a1", 0, 0, 600)]);
        let b = erase(vec![piece("b2", 400, 400, 600)]);
        let c = erase(vec![piece("c3", 3000, 2000, 600)]);
        let before = vec![a.clone(), b.clone()];
        // Nothing changed: nothing to do.
        assert!(dirty_regions(&before, &before, size).is_empty());
        // Added far away: just its window.
        let after = vec![a.clone(), b.clone(), c.clone()];
        assert_eq!(
            dirty_regions(&before, &after, size),
            vec![Rect {
                x: 3000,
                y: 2000,
                w: 600,
                h: 600
            }]
        );
        // b undone: its window, grown to take in a, which overlaps it.
        let undone = dirty_regions(&before, std::slice::from_ref(&a), size);
        assert_eq!(
            undone,
            vec![Rect {
                x: 0,
                y: 0,
                w: 1000,
                h: 1000
            }]
        );
        // a undone with b and c still there: a and b together, c alone.
        let mut regions = dirty_regions(&after, &[b.clone(), c.clone()], size);
        regions.sort_by_key(|r| r.x);
        assert_eq!(
            regions,
            vec![
                Rect {
                    x: 0,
                    y: 0,
                    w: 1000,
                    h: 1000
                },
                Rect {
                    x: 3000,
                    y: 2000,
                    w: 600,
                    h: 600
                }
            ]
        );
        // A window past the image's edge is taken only as far as the edge.
        let edge = erase(vec![piece("d4", 3800, -100, 600)]);
        assert_eq!(
            dirty_regions(&[], &[edge], size),
            vec![Rect {
                x: 3800,
                y: 0,
                w: 200,
                h: 500
            }]
        );
    }

    /// Only fills nothing names, old enough not to belong to an edit being
    /// saved, are swept; anything else in the folder is left alone.
    #[test]
    fn the_sweep_takes_only_old_unused_fills() {
        let dir = crate::test_support::Scratch::new("erase-sweep");
        let (kept, unused, fresh) = (
            fill_name(b"kept"),
            fill_name(b"unused"),
            fill_name(b"fresh"),
        );
        for name in [&kept, &unused, &fresh] {
            std::fs::write(dir.join(format!("{name}.png")), b"x").unwrap();
        }
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        std::fs::write(dir.join(format!("{unused}.png.sourcelight-tmp")), b"x").unwrap();
        let in_use: std::collections::HashSet<String> = [kept.clone()].into();
        // Nothing is old enough yet.
        assert_eq!(
            sweep(&dir, &in_use, std::time::Duration::from_secs(3600)),
            0
        );
        assert_eq!(sweep(&dir, &in_use, std::time::Duration::ZERO), 3);
        assert!(dir.join(format!("{kept}.png")).exists());
        assert!(dir.join("notes.txt").exists());
        assert!(!dir.join(format!("{fresh}.png")).exists());
        assert_eq!(
            sweep(&dir.join("absent"), &in_use, std::time::Duration::ZERO),
            0
        );
    }

    /// A selection's components fold together as the mask's do: an AI
    /// selection with a brushed hole carved out of it.
    #[test]
    fn a_selection_becomes_one_coverage() {
        use crate::develop::mask::{encode_ai, Mask, MaskComponent, MaskMode, MaskShape, Stroke};
        let block = bar(40, 20, 10, 5, 20, 10);
        let mut mask = Mask::single(MaskShape::Ai {
            label: "Person".into(),
            width: 40,
            height: 20,
            rle: encode_ai(&block),
        });
        let cov = selection_coverage(&mask, 80, 40).unwrap();
        assert_eq!(cov.len(), 80 * 40);
        assert_eq!(cov[20 * 80 + 40], 255);
        assert_eq!(cov[2 * 80 + 2], 0);
        mask.components.push(MaskComponent {
            shape: MaskShape::Brush {
                strokes: vec![Stroke {
                    x: 0.5,
                    y: 0.5,
                    radius: 0.05,
                    hardness: 1.0,
                    erase: false,
                }],
            },
            inverted: false,
            mode: MaskMode::Subtract,
        });
        let carved = selection_coverage(&mask, 80, 40).unwrap();
        assert!(carved[20 * 80 + 40] < 30, "{}", carved[20 * 80 + 40]);
        assert_eq!(carved[12 * 80 + 24], 255);
        // A range of brightness is not a place.
        let range = Mask::single(MaskShape::Luminance {
            low: 0.2,
            high: 0.8,
            feather: 0.1,
        });
        assert!(selection_coverage(&range, 80, 40).is_err());
    }

    /// An edit saved before erases existed loads with none; one with erases
    /// round-trips.
    #[test]
    fn erases_ride_in_the_edit_stack() {
        let stack =
            crate::develop::EditStack::from_json(r#"{"version":4,"exposure":0.5}"#).unwrap();
        assert!(stack.erases.is_empty());
        let mut with = stack.clone();
        with.erases.push(erase(vec![piece("ef", -20, 10, 700)]));
        let back = crate::develop::EditStack::from_json(&with.to_json().unwrap()).unwrap();
        assert_eq!(back, with);
        assert!(!with.is_default());
        assert_eq!(fill_names(&with.erases).len(), 1);
    }
}
