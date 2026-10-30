//! Exporting many photos in one go.
//!
//! Deliberately separate from [`crate::develop::DevelopState`], which holds a
//! single photo resident on the GPU. Routing a batch through that slot would
//! evict whatever the user has open and hand their session back changed; worse,
//! it would tie a background job to UI state. This walks its own list instead,
//! calling the *same* `Pipeline::render_to_pixels` the preview does — if a
//! batch ever grows its own render path, the preview has stopped telling the
//! truth about what gets written.

use std::path::{Path, PathBuf};

use crate::develop::EditStack;
use crate::export::{encode_pixels, ExportOptions};
use crate::gpu::pipeline::Pipeline;
use crate::gpu::Gpu;
use crate::raster::Source;
use crate::raw::decoder::RawDecoder;

/// One photo to export, with the edits it should be rendered through.
pub struct BatchItem {
    pub photo_id: i32,
    pub path: PathBuf,
    pub stack: EditStack,
    /// Appended to the output name, for a virtual copy.
    ///
    /// Copies share their master's path, so without this a folder holding an
    /// original and two copies of it exports `DSC01234.jpg`, `DSC01234-1.jpg`
    /// and `DSC01234-2.jpg` — three files nobody can tell apart. The collision
    /// suffix is the safety net, not the naming scheme.
    pub suffix: Option<String>,
    /// The texels of the Look LUT the stack names, loaded by whoever made
    /// the item: batch export and Copy image rendered without it, so
    /// only an export from Develop, whose session had it, came out with the
    /// look. `None` when the stack has no LUT, or its file has gone.
    pub lut: Option<std::sync::Arc<Vec<f32>>>,
    /// The fills the stack's erases name, read by whoever made the item. One
    /// missing fails the photo rather than export it with the object back.
    pub fills: std::collections::HashMap<String, std::sync::Arc<crate::develop::erase::Fill>>,
    /// What the file's EXIF says, read from the catalogue with everything else
    /// the batch needs up front. `None` writes no EXIF.
    pub metadata: Option<super::exif::ExportMetadata>,
}

/// What happened. Failures are collected rather than returned as an error,
/// because a batch is not all-or-nothing.
#[derive(Debug, Default, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BatchOutcome {
    pub written: u32,
    /// Sidecars that already said what would be written, left as they were.
    /// Always 0 for images.
    #[serde(default)]
    pub unchanged: u32,
    pub failed: u32,
    /// One line per failure, already formatted for display.
    pub errors: Vec<String>,
    /// True when the run stopped early because the user cancelled the queue.
    pub cancelled: bool,
    /// Which photos were written, for the catalogue to record. Not
    /// sent to the page: it only needs the counts.
    #[serde(skip)]
    pub exported: Vec<i32>,
    /// Every photo that failed and why: only the first error was ever
    /// named, so the other six of seven failures had to be found by diffing
    /// folders.
    pub failures: Vec<BatchFailure>,
    /// The folder the files went into, subfolder and all, for "Show in
    /// folder" and "Retry failed". `None` for sidecars.
    pub directory: Option<String>,
}

/// One photo a batch could not write, and why.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BatchFailure {
    pub photo_id: i32,
    pub error: String,
}

/// Where a source file's export should be written.
///
/// **Never returns a path that already exists.** An export dialog pointed at a
/// folder twice, or at the folder the raws live in, would otherwise destroy
/// files the user did not choose to replace — and a batch makes that a
/// hundred files rather than one. Collisions get a numeric suffix instead.
///
/// A separate function so the naming rule is testable without a GPU, a raw
/// file, or anything else that makes a test slow enough to skip.
/// A rename pattern with `{seq}` numbered: the item's place in the
/// batch from 1, zero-padded to at least three digits and to the batch's
/// size, so files sort in the order they were exported.
pub fn number_pattern(pattern: &str, index: usize, total: usize) -> String {
    let width = total.to_string().len().max(3);
    pattern.replace("{seq}", &format!("{:0width$}", index + 1))
}

pub fn output_path(
    directory: &Path,
    source: &Path,
    extension: &str,
    rename: Option<&str>,
    suffix: Option<&str>,
) -> PathBuf {
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        // A path with no stem is pathological rather than impossible: it can
        // reach here from a database row written by an older build.
        .unwrap_or_else(|| "export".into());

    // A rename pattern substitutes `{name}` with the source stem; anything else
    // is copied through. The result is sanitised so a pattern can only ever name
    // a file *in* the target folder, never escape it with a separator.
    let stem = match rename {
        Some(pat) if !pat.trim().is_empty() => sanitize_name(&pat.replace("{name}", &stem)),
        _ => stem,
    };
    let stem = if stem.is_empty() {
        "export".to_string()
    } else {
        stem
    };

    // A copy's own name, sanitised the same way a rename pattern is so it can
    // never introduce a separator.
    let stem = match suffix.map(sanitize_name).filter(|s| !s.is_empty()) {
        Some(s) => format!("{stem}-{s}"),
        None => stem,
    };

    let first = directory.join(format!("{stem}.{extension}"));
    if !first.exists() {
        return first;
    }
    // Bounded rather than a `loop`: something is badly wrong if a thousand
    // variants of one name exist, and spinning forever is worse than
    // overwriting the last one.
    for n in 1..1000 {
        let candidate = directory.join(format!("{stem}-{n}.{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    directory.join(format!("{stem}-{}.{extension}", uuid_ish()))
}

/// Strips anything that could turn a rename pattern into a path — separators,
/// parent hops, and control characters — so the export always lands in the
/// chosen folder. Leading/trailing dots and spaces go too (Windows rejects them).
fn sanitize_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    cleaned
        .trim_matches(|c: char| c == '.' || c == ' ')
        .to_string()
}

/// A short, collision-resistant suffix for the pathological case above.
fn uuid_ish() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
}

/// AI noise reduction for export: a linear RGBA half-float image of
/// the given size in, denoised out. Supplied by the app layer, which owns the
/// model; `Err` explains why it cannot run (not downloaded, say).
pub type Denoiser<'a> = dyn Fn(&[u16], u32, u32) -> anyhow::Result<Vec<u16>> + 'a;

/// Renders and writes every item, reporting progress as it goes.
///
/// `progress` is called before each item with `(done_so_far, total, name)`, so
/// a caller can drive a progress bar without knowing anything about rendering.
/// `denoise` runs for the items whose edit has an AI noise reduction Amount.
///
/// Four stages, a photo apart: the next raw decodes on a thread of its own
/// while this one renders on the GPU (on the calling thread, which owns it),
/// the ones before it encode on a few threads ([`encoders`]), and one thread
/// names and writes them in order. One after another, a full-size photo took
/// the sum — about 90 ms to decode, 200 to render and 165 to encode on an M4 —
/// so the GPU sat idle while the CPU encoded and the other way round.
/// Overlapped, a batch goes at the pace of its slowest stage, and with a
/// single encoder that was the encode wherever the CPU is slower than an M4's
/// or the export is resized first. The stages hand photos over
/// directly, with no queue between them, so no more than a photo per encoder
/// and one more is ever in memory however long the batch.
#[allow(clippy::too_many_arguments)]
pub fn export_batch(
    gpu: &Gpu,
    pipeline: &Pipeline,
    decoder: &dyn RawDecoder,
    items: &[BatchItem],
    directory: &Path,
    options: &ExportOptions,
    rename: Option<&str>,
    denoise: Option<&Denoiser>,
    should_cancel: impl Fn() -> bool + Sync,
    progress: impl FnMut(usize, usize, &str),
) -> BatchOutcome {
    export_batch_reporting(
        gpu,
        pipeline,
        decoder,
        items,
        directory,
        options,
        rename,
        denoise,
        should_cancel,
        progress,
        &|_, _| {},
    )
}

/// [`export_batch`], telling `finished` as each photo is written or fails —
/// its id, and the reason when it failed. Every row in the queue was
/// ticked once its render began, so a failed file got a green tick.
#[allow(clippy::too_many_arguments)]
pub fn export_batch_reporting(
    gpu: &Gpu,
    pipeline: &Pipeline,
    decoder: &dyn RawDecoder,
    items: &[BatchItem],
    directory: &Path,
    options: &ExportOptions,
    rename: Option<&str>,
    denoise: Option<&Denoiser>,
    should_cancel: impl Fn() -> bool + Sync,
    mut progress: impl FnMut(usize, usize, &str),
    finished: &(dyn Fn(i32, Option<&str>) + Sync),
) -> BatchOutcome {
    use std::sync::mpsc::sync_channel;

    let should_cancel = &should_cancel;
    let mut rendered_count = 0;
    let (rendered_tx, rendered_rx) = sync_channel::<(usize, anyhow::Result<Rendered>)>(0);
    // The encoders share the rendered photos, each taking the next one that's
    // ready. Outside the scope, so its threads can borrow it.
    let rendered_rx = std::sync::Mutex::new(rendered_rx);
    let rendered_rx = &rendered_rx;
    let mut outcome = std::thread::scope(|s| {
        let (decoded_tx, decoded_rx) = sync_channel::<(usize, anyhow::Result<Source>)>(0);
        let (encoded_tx, encoded_rx) = sync_channel::<(usize, anyhow::Result<Vec<u8>>)>(0);

        s.spawn(move || {
            for (i, item) in items.iter().enumerate() {
                if should_cancel() {
                    break;
                }
                let source = crate::raster::load(&item.path, decoder);
                // The render stage has stopped: cancelled.
                if decoded_tx.send((i, source)).is_err() {
                    break;
                }
            }
        });

        for _ in 0..encoders() {
            let encoded_tx = encoded_tx.clone();
            s.spawn(move || loop {
                // Held only to take a photo, not while encoding it.
                let Ok(Ok((i, rendered))) = rendered_rx.lock().map(|rx| rx.recv()) else {
                    break;
                };
                let metadata = items[i].metadata.as_ref();
                let encoded = rendered
                    .and_then(|r| encode_pixels(&r.pixels, r.width, r.height, options, metadata));
                if encoded_tx.send((i, encoded)).is_err() {
                    break;
                }
            });
        }
        // The writer's only senders are the encoders' copies, so it finishes
        // once they all have.
        drop(encoded_tx);

        let writer = s.spawn(move || {
            let mut outcome = BatchOutcome::default();
            // Encoders finish out of turn. Photos are still written in the
            // batch's order, so a name a collision moves on (`-1`) goes to the
            // same photo every time, and results are told in order.
            let mut waiting = std::collections::BTreeMap::new();
            let mut next = 0;
            for (i, encoded) in encoded_rx {
                waiting.insert(i, encoded);
                while let Some(encoded) = waiting.remove(&next) {
                    let item = &items[next];
                    // Each item is attempted independently. One unreadable file
                    // in a shoot of two hundred must not cost the other hundred
                    // and ninety-nine, so failures are recorded and the batch
                    // goes on.
                    let numbered = rename.map(|r| number_pattern(r, next, items.len()));
                    let written = encoded.and_then(|bytes| {
                        write_one(item, &bytes, directory, options, numbered.as_deref())
                    });
                    record(&mut outcome, item, written, finished);
                    next += 1;
                }
            }
            outcome
        });

        for (i, source) in decoded_rx {
            // Checked before each file, so Cancel stops the queue promptly
            // without interrupting the write already in flight.
            if should_cancel() {
                break;
            }
            progress(i, items.len(), &file_name(&items[i]));
            let rendered = source
                .and_then(|source| render_one(gpu, pipeline, &items[i], &source, options, denoise));
            rendered_count += 1;
            if rendered_tx.send((i, rendered)).is_err() {
                break;
            }
        }
        // No more photos: the writer finishes what it has and hands back.
        drop(rendered_tx);
        writer.join().expect("the export writer panicked")
    });

    outcome.cancelled = rendered_count < items.len() && should_cancel();
    progress(items.len(), items.len(), "");
    outcome
}

/// A photo rendered and read back: RGBA pixels, and their size.
struct Rendered {
    pixels: super::Pixels,
    width: u32,
    height: u32,
}

/// Counts one photo's result, and tells `finished` about it.
fn record(
    outcome: &mut BatchOutcome,
    item: &BatchItem,
    written: anyhow::Result<()>,
    finished: &(dyn Fn(i32, Option<&str>) + Sync),
) {
    match written {
        Ok(()) => {
            outcome.written += 1;
            outcome.exported.push(item.photo_id);
            finished(item.photo_id, None);
        }
        Err(e) => {
            outcome.failed += 1;
            outcome.errors.push(format!("{}: {e}", file_name(item)));
            let error = format!("{e:#}");
            finished(item.photo_id, Some(&error));
            outcome.failures.push(BatchFailure {
                photo_id: item.photo_id,
                error,
            });
        }
    }
}

/// How many photos encode at once. Encoding is one core's work (the resize
/// and the JPEG encoder are single-threaded), and each encoder holds a
/// full-size render (96 MB at 24 MP, twice that at sixteen bits). Twelve
/// full-size raws on 8 cores took 7.95 s with one, 4.45 with two, 3.52 with
/// three and 3.38 with four or six, when the decode and the render set the
/// pace instead. So half the cores, up to three.
fn encoders() -> usize {
    std::thread::available_parallelism()
        .map_or(1, |n| n.get() / 2)
        .clamp(1, 3)
}

fn file_name(item: &BatchItem) -> String {
    item.path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Writes an encoded photo under a name no file has yet.
///
/// Named here, just before the write and one photo at a time, because the name
/// depends on what is already on disk: two photos named ahead of their writes
/// could both take the same one.
fn write_one(
    item: &BatchItem,
    bytes: &[u8],
    directory: &Path,
    options: &ExportOptions,
    rename: Option<&str>,
) -> anyhow::Result<()> {
    // Never over a file that is there, enforced as the file is opened rather
    // than by looking first: a name taken in between — by a sync
    // client, or another export — moves on to the next free one.
    let mut target = output_path(
        directory,
        &item.path,
        options.format.extension(),
        rename,
        item.suffix.as_deref(),
    );
    for _ in 0..1000 {
        match crate::fsutil::write_new(&target, bytes) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                target = output_path(
                    directory,
                    &item.path,
                    options.format.extension(),
                    rename,
                    item.suffix.as_deref(),
                );
            }
            Err(e) => anyhow::bail!("cannot write {}: {e}", target.display()),
        }
    }
    anyhow::bail!("cannot find a free name for {}", target.display())
}

/// A decoded photo through the GPU: its edit applied, read back as pixels.
fn render_one(
    gpu: &Gpu,
    pipeline: &Pipeline,
    item: &BatchItem,
    source: &Source,
    options: &ExportOptions,
    denoise: Option<&Denoiser>,
) -> anyhow::Result<Rendered> {
    let mut session = pipeline.prepare_source(gpu, source)?;
    session.lut_texels = item.lut.as_deref().cloned();
    // AI noise reduction first, on the linear image, as Develop does, then
    // the erases laid into it. Refused rather than skipped when either cannot
    // be done: a photo exported noisier than the one on screen, or with an
    // erased object back, with nothing said, is worse than a failed item.
    let amount = item.stack.detail.ai_denoise;
    let erases = &item.stack.erases;
    if amount > 0.0 || !erases.is_empty() {
        let (w, h) = session.stored_size();
        let mut linear = pipeline.read_linear(gpu, &session)?;
        if amount > 0.0 {
            let denoise =
                denoise.ok_or_else(|| anyhow::anyhow!("AI noise reduction is not available"))?;
            let denoised = denoise(&linear, w, h)?;
            linear = crate::denoise::blend(&linear, &denoised, amount);
        }
        let missing = crate::develop::erase::bake(&mut linear, (w, h), erases, |name| {
            item.fills.get(name).cloned()
        });
        anyhow::ensure!(
            missing.is_empty(),
            "an erase on this photo can't be found, so the object would show"
        );
        pipeline.write_linear(gpu, &session, &linear)?;
    }
    let (width, height) = session.framed_size(&item.stack);

    let pixels = pipeline.render_for_export(
        gpu,
        &session,
        &item.stack,
        width,
        height,
        options.output(),
        options.sixteen_bit(),
    )?;
    Ok(Rendered {
        pixels,
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh, empty scratch directory, so a previous run cannot make a
    /// collision test pass or fail for the wrong reason.
    fn scratch(name: &str) -> crate::test_support::Scratch {
        crate::test_support::Scratch::new(&format!("export-{name}"))
    }

    /// The plain case: a raw's name with the export format's extension.
    #[test]
    fn an_export_takes_its_name_from_the_source() {
        let dir = scratch("naming");
        let got = output_path(&dir, Path::new("/photos/DSC00042.ARW"), "jpg", None, None);
        assert_eq!(got, dir.join("DSC00042.jpg"));
    }

    /// A rename pattern substitutes `{name}` and cannot escape the target folder.
    #[test]
    fn a_rename_pattern_names_the_file() {
        let dir = scratch("rename");
        let src = Path::new("/photos/DSC00042.ARW");

        let edited = output_path(&dir, src, "jpg", Some("{name}-edited"), None);
        assert_eq!(edited, dir.join("DSC00042-edited.jpg"));

        // A pattern with no token is a literal base name.
        let literal = output_path(&dir, src, "jpg", Some("final"), None);
        // A virtual copy carries its own name into the filename. Without it an
        // original and two copies export as `X.jpg`, `X-1.jpg`, `X-2.jpg` —
        // three files nobody can tell apart afterwards.
        let copy = output_path(&dir, src, "jpg", None, Some("Mono"));
        assert!(copy.ends_with("DSC00042-Mono.jpg"), "{copy:?}");
        // And a name that could escape the folder is sanitised like any other.
        let nasty = output_path(&dir, src, "jpg", None, Some("../oops"));
        assert_eq!(nasty.parent(), Some(dir.path()));
        assert_eq!(literal, dir.join("final.jpg"));

        // Separators in the pattern are neutralised — no traversal, no subfolders.
        // (The `/` becomes `_`, then the leading dots are trimmed.)
        let escaped = output_path(&dir, src, "jpg", Some("../{name}"), None);
        assert_eq!(escaped, dir.join("_DSC00042.jpg"));

        // An empty (or whitespace) pattern falls back to the source stem.
        let empty = output_path(&dir, src, "jpg", Some("   "), None);
        assert_eq!(empty, dir.join("DSC00042.jpg"));
    }

    /// **Never overwrite.** Exporting into a folder twice must produce a second
    /// file, not replace the first — across a batch this is the difference
    /// between a duplicate and losing a hundred delivered images.
    #[test]
    fn an_existing_file_is_never_overwritten() {
        let dir = scratch("collisions");
        let source = Path::new("/photos/DSC00042.ARW");

        let first = output_path(&dir, source, "jpg", None, None);
        std::fs::write(&first, b"original").unwrap();

        let second = output_path(&dir, source, "jpg", None, None);
        assert_ne!(first, second);
        assert_eq!(second, dir.join("DSC00042-1.jpg"));

        std::fs::write(&second, b"second").unwrap();
        let third = output_path(&dir, source, "jpg", None, None);
        assert_eq!(third, dir.join("DSC00042-2.jpg"));

        // And the original is still untouched.
        assert_eq!(std::fs::read(&first).unwrap(), b"original");
    }

    /// Exporting JPEGs into the folder the raws live in must not collide with
    /// the raws themselves, and must not collide with a camera-made JPEG
    /// sitting beside them either.
    #[test]
    fn exporting_beside_the_originals_is_safe() {
        let dir = scratch("beside-originals");
        let raw = dir.join("DSC00042.ARW");
        std::fs::write(&raw, b"raw bytes").unwrap();
        std::fs::write(dir.join("DSC00042.jpg"), b"camera jpeg").unwrap();

        let got = output_path(&dir, &raw, "jpg", None, None);
        assert_eq!(got, dir.join("DSC00042-1.jpg"));
        assert_eq!(std::fs::read(&raw).unwrap(), b"raw bytes");
    }

    /// A path with no usable stem still produces a writable name rather than
    /// an empty one, which would land on the directory itself.
    #[test]
    fn a_source_without_a_stem_still_names_something() {
        let dir = scratch("no-stem");
        let got = output_path(&dir, Path::new("/"), "png", None, None);
        assert_eq!(got, dir.join("export.png"));
    }

    /// `{seq}` numbers a batch in order, wide enough for all of it.
    #[test]
    fn a_sequence_number_keeps_files_in_order() {
        assert_eq!(number_pattern("{seq}-{name}", 0, 12), "001-{name}");
        assert_eq!(number_pattern("{seq}-{name}", 1233, 1500), "1234-{name}");
        assert_eq!(number_pattern("{name}", 4, 9), "{name}");
    }
}
