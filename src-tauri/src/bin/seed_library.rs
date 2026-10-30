//! Fills a catalogue with a synthetic library, so the app can be used at a size
//! nobody has a spare 480 GB of raw files to reproduce.
//!
//! `tests/scale.rs` measures the *catalogue* at scale. It cannot measure the
//! part that decides whether the app feels usable — the grid scrolling, the
//! filter rail redrawing, the Lightbox filmstrip, the cull survey — because that
//! lives in a webview. This puts fifty thousand rows in front of the real UI so
//! those can be judged by looking.
//!
//! ```text
//! cargo run --manifest-path src-tauri/Cargo.toml --features dev-tools --bin seed_library -- \
//!     --photos 50000 --app --yes
//! ```
//!
//! `--app` writes into the catalogue the installed app opens. That is somebody's
//! real library, so it **takes a backup first, every time, with no way to turn
//! that off** — and afterwards Settings → Catalogue backups restores it in two
//! clicks. Without `--yes` it prints what it would do and stops.
//!
//! The photos it invents point at whatever derivatives you give it. Without
//! `--derivatives` they point at paths that do not exist, which is fine for
//! everything except looking at the pictures: the grid will render its
//! broken-thumbnail state fifty thousand times, which is itself worth seeing
//! once, but it is not the layout test. Point it at the existing thumbnail cache
//! to get a grid full of real images:
//!
//! ```text
//!     --derivatives "%LOCALAPPDATA%/app.sourcelight/thumbs"
//! ```
//!
//! **Without touching the real library:** set `SOURCELIGHT_DATA_DIR` to an
//! empty folder, and `--app` writes there instead. A development build (debug,
//! or built with `--features dev-tools`) launched with the same variable opens
//! that folder as its whole data directory. The app only serves images from
//! inside its data directory, so copy the derivatives into it first:
//!
//! ```text
//! export SOURCELIGHT_DATA_DIR=/tmp/big-library
//! mkdir -p $SOURCELIGHT_DATA_DIR/thumbs
//! cp "$LOCALAPPDATA/app.sourcelight/thumbs/"* $SOURCELIGHT_DATA_DIR/thumbs/
//! cargo run ... --bin seed_library -- --photos 50000 --app --yes \
//!     --derivatives $SOURCELIGHT_DATA_DIR/thumbs
//! ```

use std::path::{Path, PathBuf};

use sourcelight_lib::seed::{seed, Shape};
use sourcelight_lib::{backup, db};

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

struct Args {
    photos: usize,
    out: Option<PathBuf>,
    app: bool,
    derivatives: Option<PathBuf>,
    yes: bool,
    embeddings: bool,
}

fn run() -> anyhow::Result<()> {
    let args = parse()?;

    let path = match (&args.out, args.app) {
        (Some(p), _) => p.clone(),
        (None, true) => app_catalogue()?,
        (None, false) => {
            anyhow::bail!("give me somewhere to write: --out <path> or --app")
        }
    };

    let derivatives = match &args.derivatives {
        Some(dir) => collect_derivatives(dir)?,
        None => Vec::new(),
    };

    let shape = Shape {
        embeddings: args.embeddings,
        derivatives,
        ..Shape::library(args.photos)
    };

    println!("catalogue:   {}", path.display());
    println!("photos:      {}", shape.photos);
    println!(
        "derivatives: {}",
        if shape.derivatives.is_empty() {
            "none — thumbnails will be missing".to_string()
        } else {
            format!("{} real files, cycled", shape.derivatives.len())
        }
    );
    println!(
        "embeddings:  {}",
        if shape.embeddings {
            format!("yes — about {} MB of blobs", shape.photos * 2 / 1024)
        } else {
            "no".into()
        }
    );

    if !args.yes {
        println!("\nnothing written. Re-run with --yes.");
        return Ok(());
    }

    // Always, and before anything is opened for writing. The whole point of the
    // backup work was that the catalogue is the only copy of every
    // rating, tag and edit the user has ever made; a developer tool that dumps
    // fifty thousand rows into it is exactly the case that was written for.
    if path.exists() {
        let conn = db::connect(&path)?;
        let existing: i64 = conn.query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0))?;
        let at = backup::create(&conn, &path, "before-seed")?;
        println!("\nbacked up {existing} existing photos to {}", at.display());
        println!("restore from Settings -> Catalogue backups when you are done.");
    }

    let mut conn = db::open(&path)?;
    let t = std::time::Instant::now();
    let stats = seed(&mut conn, &shape)?;
    // The seeded photos live under `D:/Photos`, so that is their library
    // folder: it files every row, so the folder facet is counted at
    // scale too. The folder need not exist — the app lists it as "not found"
    // and the launch scan skips it rather than marking 50,000 photos missing.
    db::add_library_folder(&mut conn, "D:/Photos")?;
    println!("library folder: D:/Photos (filed every seeded photo)");
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

    println!(
        "\nseeded {} photos in {:.1}s\n  {} tag links, {} placed, {} faces over {} people, \
         {} captions, {} burst frames, {} virtual copies\n  catalogue is now {:.0} MB",
        stats.photos,
        t.elapsed().as_secs_f64(),
        stats.tag_links,
        stats.placed,
        stats.faces,
        stats.people,
        stats.captions,
        stats.bursts,
        stats.variants,
        bytes as f64 / 1_048_576.0,
    );
    Ok(())
}

fn parse() -> anyhow::Result<Args> {
    let mut args = Args {
        photos: 50_000,
        out: None,
        app: false,
        derivatives: None,
        yes: false,
        embeddings: true,
    };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut value = || {
            it.next()
                .ok_or_else(|| anyhow::anyhow!("{a} needs a value"))
        };
        match a.as_str() {
            "--photos" => args.photos = value()?.parse()?,
            "--out" => args.out = Some(PathBuf::from(value()?)),
            "--derivatives" => args.derivatives = Some(PathBuf::from(value()?)),
            "--app" => args.app = true,
            "--yes" => args.yes = true,
            "--no-embeddings" => args.embeddings = false,
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument {other}\n\n{USAGE}"),
        }
    }
    Ok(args)
}

const USAGE: &str = "\
seed_library — fill a catalogue with a synthetic library

  --photos N        how many (default 50000)
  --app             write to the installed app's catalogue (backs it up first)
  --out PATH        write to a specific file instead
  --derivatives DIR point thumb/preview paths at the images in DIR, cycled
  --no-embeddings   skip CLIP vectors (~2 KB each) — faster, smaller, but the
                    search timings then measure nothing
  --yes             actually do it";

/// Where the app keeps its catalogue — or, when `SOURCELIGHT_DATA_DIR` is
/// set, where a development build pointed at that folder will look.
///
/// Resolved by hand rather than through Tauri's `app_local_data_dir`, which
/// needs a running `App`. Kept in step with `lib.rs`'s `setup` and with the
/// `identifier` in tauri.conf.json — if that identifier ever changes,
/// this changes with it.
fn app_catalogue() -> anyhow::Result<PathBuf> {
    if let Some(dir) = std::env::var_os(sourcelight_lib::DATA_DIR_ENV).filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir).join("library.db"));
    }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share"))
    };
    let base =
        base.ok_or_else(|| anyhow::anyhow!("could not find the local app data directory"))?;
    Ok(base.join("app.sourcelight").join("library.db"))
}

/// Every image directly in `dir`, as absolute paths.
///
/// Not recursive on purpose: the thumbnail cache is flat, and walking a photo
/// library by accident because someone pointed this at `D:/Photos` would take a
/// very long time to do something nobody wanted.
fn collect_derivatives(dir: &Path) -> anyhow::Result<Vec<String>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let p = entry?.path();
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        if matches!(ext.as_deref(), Some("jpg" | "jpeg" | "png" | "webp")) {
            out.push(p.to_string_lossy().replace('\\', "/"));
        }
    }
    if out.is_empty() {
        anyhow::bail!("no images in {}", dir.display());
    }
    out.sort();
    Ok(out)
}
