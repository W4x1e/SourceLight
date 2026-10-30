//! Batch export, end to end on real raw files.
//!
//! The unit tests in `export::batch` cover naming and collisions without a GPU.
//! This covers the part they cannot: that a list of real photos actually decodes,
//! renders and lands on disk as readable images, and that a failure part way
//! through does not take the rest of the batch with it.
//!
//! ```sh
//! SOURCELIGHT_TEST_RAW=/path/to/raws cargo test --test batch_export -- --nocapture
//! ```

#![cfg(feature = "libraw")]

use sourcelight_lib::develop::EditStack;
use sourcelight_lib::export::batch::{export_batch, export_batch_reporting, BatchItem};
use sourcelight_lib::export::exif::ExportMetadata;
use sourcelight_lib::export::{BitDepth, ExportFormat, ExportOptions, SharpenLevel};
use sourcelight_lib::gpu::pipeline::Pipeline;
use sourcelight_lib::raw::decoder::RawDecoder;
use sourcelight_lib::raw::libraw::LibRawDecoder;
use sourcelight_lib::test_support::{self, Needs};
use std::path::PathBuf;

/// Up to `limit` raw fixtures, sorted. Empty when none are configured, which the
/// caller reports with `test_support::skip` — a path that is set but missing
/// fails in `raw_dir` instead of quietly returning nothing.
fn raws(limit: usize) -> Vec<PathBuf> {
    let Some(dir) = test_support::raw_dir() else {
        return Vec::new();
    };
    let exts = LibRawDecoder::new().extensions();
    let mut files = test_support::files_in(&dir, |e| exts.contains(&e));
    files.truncate(limit);
    files
}

fn scratch(name: &str) -> test_support::Scratch {
    test_support::Scratch::new(name)
}

#[test]
fn a_batch_writes_every_photo_and_survives_a_bad_one() {
    let files = raws(3);
    if files.is_empty() {
        test_support::skip(
            Needs::Raw,
            "set SOURCELIGHT_TEST_RAW to a folder of raw files",
        );
        return;
    }
    let Some(gpu) = sourcelight_lib::gpu::test_support::hardware_gpu() else {
        return;
    };

    let dir = scratch("batch-export");
    let decoder = LibRawDecoder::new();
    let pipeline = Pipeline::new(gpu);

    // A file that is not a raw, placed in the middle. One unreadable frame in a
    // shoot must cost only itself — the whole point of collecting failures
    // rather than returning on the first error.
    let broken = dir.join("broken.arw");
    std::fs::write(&broken, b"not a raw file").unwrap();

    let mut items: Vec<BatchItem> = Vec::new();
    for (i, path) in files.iter().enumerate() {
        if i == 1 {
            items.push(BatchItem {
                photo_id: -1,
                path: broken.clone(),
                stack: EditStack::default(),
                suffix: None,
                metadata: None,
                lut: None,
                fills: Default::default(),
            });
        }
        items.push(BatchItem {
            photo_id: i as i32,
            path: path.clone(),
            // A visible edit, so the export is exercising the develop pass
            // rather than some shortcut around it.
            stack: EditStack {
                exposure: 0.5,
                ..Default::default()
            },
            suffix: None,
            lut: None,
            fills: Default::default(),
            // The photographer's lines have to survive the batch path, not
            // just the encoder it calls.
            metadata: Some(ExportMetadata {
                copyright: Some("Test Photographer".into()),
                ..Default::default()
            }),
        });
    }

    let options = ExportOptions {
        format: ExportFormat::Jpeg,
        quality: 88,
        max_edge: Some(800),
        sharpen: SharpenLevel::default(),
        watermark: None,
        text_watermark: None,
        color_space: Default::default(),
        intent: Default::default(),
        strip_metadata: false,
        metadata: sourcelight_lib::xmp::MetadataChoices::default(),
        bit_depth: Default::default(),
    };

    let mut seen = Vec::new();
    let finished = std::sync::Mutex::new(Vec::new());
    let outcome = export_batch_reporting(
        gpu,
        &pipeline,
        &decoder,
        &items,
        &dir,
        &options,
        None,
        None,
        || false,
        |done, total, name| seen.push((done, total, name.to_string())),
        &|id, error| {
            finished
                .lock()
                .unwrap()
                .push((id, error.map(str::to_string)))
        },
    );

    eprintln!("batch: {outcome:?}");
    assert_eq!(
        outcome.written as usize,
        files.len(),
        "every readable photo should have been written: {outcome:?}"
    );
    assert_eq!(
        outcome.failed, 1,
        "the broken file should have failed alone"
    );
    assert_eq!(outcome.errors.len(), 1);
    assert!(
        outcome.errors[0].starts_with("broken.arw"),
        "the error should name the file that failed: {:?}",
        outcome.errors
    );
    // Every failure by photo, with its reason, and each photo's result
    // told as it landed: the broken one failed, the rest written.
    assert_eq!(outcome.failures.len(), 1);
    assert_eq!(outcome.failures[0].photo_id, -1);
    assert!(!outcome.failures[0].error.is_empty());
    let finished = finished.into_inner().unwrap();
    assert_eq!(finished.len(), items.len(), "{finished:?}");
    for (id, error) in &finished {
        assert_eq!(error.is_some(), *id == -1, "{finished:?}");
    }

    // Progress must be reported once per item plus a final call, or a progress
    // bar either stalls short of the end or never starts.
    assert_eq!(seen.len(), items.len() + 1, "{seen:?}");
    assert_eq!(seen.first().unwrap().0, 0);
    assert_eq!(seen.last().unwrap().0, items.len());

    // Every written file has to be a real, decodable image at the requested
    // size — a zero-byte or truncated file would satisfy a mere existence check.
    let mut written: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jpg"))
        .collect();
    written.sort();
    assert_eq!(written.len(), files.len(), "{written:?}");

    for path in &written {
        let img = image::open(path).expect("exported file should decode");
        assert!(
            img.width().max(img.height()) == 800,
            "{} came out {}x{}, expected a longest edge of 800",
            path.display(),
            img.width(),
            img.height()
        );
        let exif = exif::Reader::new()
            .read_from_container(&mut std::io::BufReader::new(
                std::fs::File::open(path).unwrap(),
            ))
            .expect("exported JPEG should carry EXIF");
        let copyright = exif
            .get_field(exif::Tag::Copyright, exif::In::PRIMARY)
            .expect("the copyright line should be in the export");
        assert!(
            copyright
                .display_value()
                .to_string()
                .contains("Test Photographer"),
            "{}",
            copyright.display_value()
        );
    }

    // Running the same batch again must add files rather than replace them.
    // This is the guarantee that stops a second export destroying the first.
    let before: Vec<Vec<u8>> = written.iter().map(|p| std::fs::read(p).unwrap()).collect();
    let again = export_batch(
        gpu,
        &pipeline,
        &decoder,
        &items,
        &dir,
        &options,
        None,
        None,
        || false,
        |_, _, _| {},
    );
    assert_eq!(again.written as usize, files.len());
    for (path, bytes) in written.iter().zip(before) {
        assert_eq!(
            std::fs::read(path).unwrap(),
            bytes,
            "{} was overwritten by the second batch",
            path.display()
        );
    }
}

/// Cancel stops the batch even though its stages run side by side: the photo
/// already rendering finishes and is written, nothing after it starts, and
/// the outcome says it was cancelled.
#[test]
fn a_cancelled_batch_stops_and_says_so() {
    let files = raws(4);
    if files.len() < 4 {
        test_support::skip(
            Needs::Raw,
            "set SOURCELIGHT_TEST_RAW to a folder of at least four raws",
        );
        return;
    }
    let Some(gpu) = sourcelight_lib::gpu::test_support::hardware_gpu() else {
        return;
    };
    let dir = scratch("batch-export-cancel");
    let decoder = LibRawDecoder::new();
    let pipeline = Pipeline::new(gpu);
    let items: Vec<BatchItem> = files
        .iter()
        .enumerate()
        .map(|(i, path)| BatchItem {
            photo_id: i as i32,
            path: path.clone(),
            stack: EditStack::default(),
            suffix: None,
            metadata: None,
            lut: None,
            fills: Default::default(),
        })
        .collect();
    let options = ExportOptions {
        format: ExportFormat::Jpeg,
        quality: 80,
        max_edge: Some(400),
        sharpen: SharpenLevel::default(),
        watermark: None,
        text_watermark: None,
        color_space: Default::default(),
        intent: Default::default(),
        strip_metadata: false,
        metadata: sourcelight_lib::xmp::MetadataChoices::default(),
        bit_depth: Default::default(),
    };

    // Cancel as the second photo starts rendering.
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let mut last = None;
    let outcome = export_batch(
        gpu,
        &pipeline,
        &decoder,
        &items,
        &dir,
        &options,
        None,
        None,
        || cancel.load(std::sync::atomic::Ordering::Relaxed),
        |done, total, _| {
            if done == 1 {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            last = Some((done, total));
        },
    );
    assert!(outcome.cancelled, "{outcome:?}");
    assert_eq!(
        outcome.written, 2,
        "the two photos started finish: {outcome:?}"
    );
    assert_eq!(outcome.failed, 0);
    // The progress bar still reaches its end.
    assert_eq!(last, Some((items.len(), items.len())));
}

/// A sixteen-bit TIFF, end to end: a real raw rendered at sixteen bits and
/// written as one, which taken to eight bits is the photo the eight-bit
/// export makes, and between those levels holds the tones eight bits cannot.
#[test]
fn a_sixteen_bit_export_is_the_eight_bit_one_finer() {
    let files = raws(1);
    if files.is_empty() {
        test_support::skip(
            Needs::Raw,
            "set SOURCELIGHT_TEST_RAW to a folder of raw files",
        );
        return;
    }
    let Some(gpu) = sourcelight_lib::gpu::test_support::hardware_gpu() else {
        return;
    };
    let decoder = LibRawDecoder::new();
    let pipeline = Pipeline::new(gpu);
    let items = vec![BatchItem {
        photo_id: 1,
        path: files[0].clone(),
        stack: EditStack::default(),
        suffix: None,
        metadata: None,
        lut: None,
        fills: Default::default(),
    }];
    let export = |bit_depth, name: &str| {
        let dir = scratch(name);
        let options = ExportOptions {
            format: ExportFormat::Tiff,
            max_edge: Some(600),
            bit_depth,
            ..Default::default()
        };
        let outcome = export_batch(
            gpu,
            &pipeline,
            &decoder,
            &items,
            &dir,
            &options,
            None,
            None,
            || false,
            |_, _, _| {},
        );
        assert_eq!(outcome.written, 1, "{outcome:?}");
        let file = std::fs::read_dir(&*dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "tif"))
            .expect("a TIFF");
        (image::open(&file).expect("the TIFF decodes"), dir)
    };
    let (eight, _eight_dir) = export(BitDepth::Eight, "export-8bit");
    let (sixteen, _sixteen_dir) = export(BitDepth::Sixteen, "export-16bit");
    assert_eq!(eight.color(), image::ColorType::Rgb8);
    assert_eq!(sixteen.color(), image::ColorType::Rgb16);
    let (eight, sixteen) = (eight.to_rgb8(), sixteen.to_rgb16());
    assert_eq!(eight.dimensions(), sixteen.dimensions());
    let mut between = 0usize;
    for (a, b) in eight.pixels().zip(sixteen.pixels()) {
        for c in 0..3 {
            let back = b[c] as f32 / 257.0;
            assert!(
                (a[c] as f32 - back).abs() <= 1.5,
                "{a:?} at eight bits, {b:?} at sixteen"
            );
            between += usize::from(b[c] % 257 != 0);
        }
    }
    // Most values fall between the eight-bit levels: the extra bits are real.
    let total = eight.pixels().len() * 3;
    assert!(between > total / 2, "{between} of {total} between levels");
}

/// A batch export, and so Copy image, applies the photo's Look LUT, as an
/// export from Develop does: it rendered without it, so the two paths
/// disagreed about what the photo looks like.
#[test]
fn a_batch_export_applies_the_looks_lut() {
    let files = raws(1);
    if files.is_empty() {
        test_support::skip(
            Needs::Raw,
            "set SOURCELIGHT_TEST_RAW to a folder of raw files",
        );
        return;
    }
    let Some(gpu) = sourcelight_lib::gpu::test_support::hardware_gpu() else {
        return;
    };
    let decoder = LibRawDecoder::new();
    let pipeline = Pipeline::new(gpu);
    let dim = sourcelight_lib::develop::lut::LUT_3D_DIM;
    let red: Vec<f32> = std::iter::repeat_n([1.0f32, 0.0, 0.0, 1.0], dim * dim * dim)
        .flatten()
        .collect();
    let mut stack = EditStack::default();
    stack.lut.name = "red.cube".into();
    stack.lut.intensity = 1.0;
    let export = |lut: Option<std::sync::Arc<Vec<f32>>>, name: &str| {
        let dir = scratch(name);
        let items = vec![BatchItem {
            photo_id: 1,
            path: files[0].clone(),
            stack: stack.clone(),
            suffix: None,
            metadata: None,
            lut,
            fills: Default::default(),
        }];
        let options = ExportOptions {
            format: ExportFormat::Png,
            max_edge: Some(200),
            strip_metadata: true,
            ..Default::default()
        };
        let outcome = export_batch(
            gpu,
            &pipeline,
            &decoder,
            &items,
            &dir,
            &options,
            None,
            None,
            || false,
            |_, _, _| {},
        );
        assert_eq!(outcome.written, 1, "{outcome:?}");
        let file = std::fs::read_dir(&*dir)
            .unwrap()
            .filter_map(Result::ok)
            .find(|e| e.path().extension().is_some_and(|x| x == "png"))
            .unwrap()
            .path();
        image::open(file).unwrap().to_rgb8()
    };
    let plain = export(None, "lut-none");
    let looked = export(Some(std::sync::Arc::new(red)), "lut-red");
    let mean = |img: &image::RgbImage, c: usize| {
        img.pixels().map(|p| f64::from(p[c])).sum::<f64>() / img.pixels().len() as f64
    };
    assert!(
        mean(&looked, 1) < mean(&plain, 1) * 0.5,
        "the all-red LUT should drain the green: {} vs {}",
        mean(&looked, 1),
        mean(&plain, 1)
    );
}
