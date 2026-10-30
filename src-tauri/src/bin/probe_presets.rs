//! Preset probe: renders the built-in presets on real raws as contact sheets,
//! so they can be tuned by eye.
//!
//! ```sh
//! cargo run --release --features dev-tools --bin probe_presets -- \
//!     <photos.txt> <out dir> [group or preset id …] [--edge N] [--amount A]
//!     [--presets <copy of presets.json>] [--rows N]
//! ```
//!
//! `photos.txt` lists raw files, one per line. Each photo is developed with
//! Auto (tone and white balance), as a photo usually is before a preset goes
//! on, then rendered once bare and once per preset. One sheet per group,
//! `<out dir>/<group>.jpg`: a row per photo, the bare render first, then the
//! group's presets in order. Naming presets instead of groups puts them on one
//! sheet, `<out dir>/picked.jpg`. The column order is printed. `--rows N`
//! splits each sheet into files of N photos (`<group>-1.jpg`, …), small enough
//! to judge. `--presets` renders an edited copy of the pack instead of the
//! built-in one, so values can be tuned without a rebuild.

use image::{imageops, Rgba, RgbaImage};
use sourcelight_lib::develop::auto::{self, AutoTone, AutoWb};
use sourcelight_lib::develop::{presets::Pack, EditStack};
use sourcelight_lib::gpu::pipeline::Pipeline;
use sourcelight_lib::gpu::Gpu;
use sourcelight_lib::raw::decoder::RawDecoder;
use sourcelight_lib::raw::libraw::LibRawDecoder;
use std::path::PathBuf;

/// The gap between tiles, and its colour.
const GAP: u32 = 6;
const BACKGROUND: Rgba<u8> = Rgba([24, 24, 24, 255]);

fn main() -> anyhow::Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut edge = 300u32;
    let mut amount = 1.0f32;
    let mut pack_file: Option<PathBuf> = None;
    let mut rows_per_file = usize::MAX;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--edge" => {
                edge = args[i + 1].parse()?;
                args.drain(i..i + 2);
            }
            "--amount" => {
                amount = args[i + 1].parse()?;
                args.drain(i..i + 2);
            }
            "--presets" => {
                pack_file = Some(PathBuf::from(&args[i + 1]));
                args.drain(i..i + 2);
            }
            "--rows" => {
                rows_per_file = args[i + 1].parse()?;
                args.drain(i..i + 2);
            }
            _ => i += 1,
        }
    }
    anyhow::ensure!(
        args.len() >= 2,
        "usage: probe_presets <photos.txt> <out dir> [group or preset id …] [--edge N] [--amount A]"
    );
    let photos: Vec<PathBuf> = std::fs::read_to_string(&args[0])?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(PathBuf::from)
        .collect();
    let out = PathBuf::from(&args[1]);
    std::fs::create_dir_all(&out)?;
    let wanted: Vec<&str> = args[2..].iter().map(String::as_str).collect();

    let pack = match &pack_file {
        Some(f) => Pack::parse(&std::fs::read_to_string(f)?)?,
        None => Pack::parse(include_str!(
            "../../crates/sl-develop/src/develop/presets.json"
        ))?,
    };
    // The sheets: (file stem, preset ids).
    let catalogue = pack.catalogue();
    let mut sheets: Vec<(String, Vec<String>)> = Vec::new();
    let mut picked = Vec::new();
    for g in &catalogue {
        if wanted.is_empty() || wanted.contains(&g.id.as_str()) {
            sheets.push((
                g.id.clone(),
                g.presets.iter().map(|p| p.id.clone()).collect(),
            ));
        }
        for p in &g.presets {
            if wanted.contains(&p.id.as_str()) {
                picked.push(p.id.clone());
            }
        }
    }
    if !picked.is_empty() {
        sheets.push(("picked".into(), picked));
    }
    anyhow::ensure!(!sheets.is_empty(), "no group or preset called {wanted:?}");

    let gpu = Gpu::new()?;
    let pipeline = Pipeline::new(&gpu);
    let decoder = LibRawDecoder::new();

    // tiles[sheet][photo][column]
    let mut tiles: Vec<Vec<Vec<RgbaImage>>> = vec![Vec::new(); sheets.len()];
    for path in &photos {
        let raw = match decoder.decode(path) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("skip {}: {e:#}", path.display());
                continue;
            }
        };
        let session = pipeline.prepare(&gpu, &raw)?;
        let render = |stack: &EditStack| -> anyhow::Result<RgbaImage> {
            let (w, h) = session.framed_proxy(stack, edge);
            let px = pipeline.render_to_pixels(&gpu, &session, stack, w, h)?;
            RgbaImage::from_raw(w, h, px).ok_or_else(|| anyhow::anyhow!("short frame"))
        };

        // Auto first, the way most photos are before a preset goes on.
        let measurement =
            AutoWb::neutral_measurement(&AutoTone::neutral_measurement(&EditStack::default()));
        let (w, h) = session.framed_proxy(&measurement, 320);
        let px = pipeline.render_to_pixels(&gpu, &session, &measurement, w, h)?;
        let mut base = EditStack::default();
        auto::auto_tone(&auto::stats_from_pixels(&px)).apply_to(&mut base);
        auto::auto_wb(&auto::wb_stats_from_pixels(&px)).apply_to(&mut base);

        let bare = render(&base)?;
        for (sheet, (_, ids)) in sheets.iter().enumerate() {
            let mut row = vec![bare.clone()];
            for id in ids {
                let stack = pack.apply(id, amount, &base).expect("listed preset");
                row.push(render(&stack)?);
            }
            tiles[sheet].push(row);
        }
        eprintln!("rendered {}", path.display());
    }

    for (sheet, (stem, ids)) in sheets.iter().enumerate() {
        let chunks: Vec<&[Vec<RgbaImage>]> = tiles[sheet].chunks(rows_per_file.max(1)).collect();
        for (part, rows) in chunks.iter().enumerate() {
            let name = if chunks.len() > 1 {
                format!("{stem}-{}", part + 1)
            } else {
                stem.clone()
            };
            let cols = ids.len() as u32 + 1;
            let cell_w = rows
                .iter()
                .flatten()
                .map(|t| t.width())
                .max()
                .unwrap_or(edge);
            let heights: Vec<u32> = rows
                .iter()
                .map(|r| r.iter().map(|t| t.height()).max().unwrap_or(0))
                .collect();
            let width = GAP + cols * (cell_w + GAP);
            let height = GAP + heights.iter().map(|h| h + GAP).sum::<u32>();
            let mut canvas = RgbaImage::from_pixel(width, height, BACKGROUND);
            let mut y = GAP;
            for (row, h) in rows.iter().zip(&heights) {
                for (c, tile) in row.iter().enumerate() {
                    let x = GAP + c as u32 * (cell_w + GAP) + (cell_w - tile.width()) / 2;
                    imageops::overlay(
                        &mut canvas,
                        tile,
                        x as i64,
                        (y + (h - tile.height()) / 2) as i64,
                    );
                }
                y += h + GAP;
            }
            let file = out.join(format!("{name}.jpg"));
            image::DynamicImage::ImageRgba8(canvas)
                .to_rgb8()
                .save(&file)?;
            println!("{}: bare, {}", file.display(), ids.join(", "));
        }
    }
    Ok(())
}
