//! Handing files to the operating system's shell: show one in the file manager,
//! and move rejects to the Recycle Bin.
//!
//! SourceLight otherwise never writes to a photo, so both are narrow on purpose.
//! Revealing is read-only. Binning moves files to the OS's own Recycle Bin (the
//! Trash on macOS), where they stay recoverable, and only ever for photos the
//! user has already rejected and then asked, behind a confirmation, to remove.
//! Nothing here deletes a file outright: on a drive with no Recycle Bin,
//! Windows itself asks before destroying anything (the `trash` crate passes
//! `FOF_WANTNUKEWARNING`).
//!
//! **Platform code.** The file manager is a per-OS program, so `reveal` carries
//! a `#[cfg]` — one of the few allowed outside `present/`, like `hardware.rs`
//! and `sidecar.rs`, and for the same reason: it is the OS integration itself.
//! Binning is portable through `trash`.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// Opens the file manager with `path` selected — Explorer, Finder, or (on
/// other Unixes) the containing folder.
pub fn reveal(path: &Path) -> Result<()> {
    if !path.exists() {
        bail!("{} isn't there any more", path.display());
    }
    #[cfg(windows)]
    let spawned = {
        use std::os::windows::process::CommandExt;
        // Explorer parses `/select,` itself and wants the path quoted inside
        // the one argument, which the default argument quoting does not do.
        std::process::Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn()
    };
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    let spawned = std::process::Command::new("xdg-open")
        .arg(path.parent().unwrap_or(path))
        .spawn();
    spawned
        .map(|_| ())
        .with_context(|| format!("opening the file manager at {}", path.display()))
}

/// The files that belong with a raw and should travel with it: XMP sidecars
/// (`NAME.xmp`, and the `NAME.ARW.xmp` some editors write) and the JPEG or
/// HEIF a camera writes beside it in RAW+JPEG (Fujifilm and Canon offer
/// RAW+HEIF too). Only those that exist, each once —
/// on a case-insensitive disk `NAME.xmp` and `NAME.XMP` are one file — and
/// never the photo itself: a JPEG photo matches its own `NAME.jpg`.
pub fn companions(raw: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(stem), Some(name)) = (raw.parent(), raw.file_stem(), raw.file_name())
    else {
        return Vec::new();
    };
    let stem = stem.to_string_lossy();
    let name = name.to_string_lossy();
    let candidates = [
        format!("{name}.xmp"),
        format!("{name}.XMP"),
        format!("{stem}.xmp"),
        format!("{stem}.XMP"),
        format!("{stem}.jpg"),
        format!("{stem}.JPG"),
        format!("{stem}.jpeg"),
        format!("{stem}.JPEG"),
        format!("{stem}.hif"),
        format!("{stem}.HIF"),
        format!("{stem}.heic"),
        format!("{stem}.HEIC"),
    ];
    let mut seen = std::collections::HashSet::new();
    seen.insert(std::fs::canonicalize(raw).unwrap_or_else(|_| raw.to_path_buf()));
    candidates
        .iter()
        .map(|c| dir.join(c))
        .filter(|p| p.is_file())
        .filter(|p| {
            let key = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
            seen.insert(key)
        })
        .collect()
}

/// Which photos share a name stem with another photo beside them:
/// `scan.tif` and `scan.jpg`, `IMG.HEIC` and `IMG.JPG`, `foo.CR2` and the
/// `foo.dng` a converter wrote next to it. Those are separate
/// photos, so neither is the other's companion, and a `{stem}.xmp` sidecar
/// cannot be both of theirs.
///
/// A JPEG or HEIF beside a raw is not a photo of its own but the raw's
/// companion (`scan::without_raw_partners`), so RAW+JPEG shares nothing.
/// Each folder is listed once, however many of its photos are asked about.
#[derive(Default)]
pub struct StemIndex {
    folders: std::collections::HashMap<PathBuf, std::collections::HashMap<String, Vec<PathBuf>>>,
}

impl StemIndex {
    /// Whether another photo in `photo`'s folder has its stem.
    pub fn shares_stem(&mut self, photo: &Path) -> bool {
        let (Some(dir), Some(stem)) = (photo.parent(), photo.file_stem()) else {
            return false;
        };
        let stems = self.folders.entry(dir.to_path_buf()).or_insert_with(|| {
            let mut by_stem: std::collections::HashMap<String, Vec<PathBuf>> = Default::default();
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if crate::scan::is_photo_file(&path) {
                    if let Some(s) = path.file_stem() {
                        by_stem
                            .entry(s.to_string_lossy().to_lowercase())
                            .or_default()
                            .push(path);
                    }
                }
            }
            by_stem
                .into_iter()
                .map(|(stem, files)| (stem, crate::scan::without_raw_partners(files)))
                .collect()
        });
        let name = photo
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase());
        stems
            .get(&stem.to_string_lossy().to_lowercase())
            .is_some_and(|photos| {
                photos
                    .iter()
                    .any(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()) != name)
            })
    }
}

/// What goes to the Recycle Bin with a photo: for a raw, its
/// companions; for a JPEG, TIFF or HEIF, only a sidecar named for it in full
/// (`scan.tif.xmp`). Never a file that is a photo of its own — `scan.jpg` is
/// not `scan.tif`'s, whatever its name — and never a `{stem}.xmp` that
/// another photo beside it may own.
pub fn bin_companions(photo: &Path, stems: &mut StemIndex) -> Vec<PathBuf> {
    let name = photo
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let shared = stems.shares_stem(photo);
    let rendered = crate::raster::is_raster(photo);
    companions(photo)
        .into_iter()
        .filter(|c| {
            let file = c
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            let full_sidecar = file == format!("{name}.xmp");
            let stem_sidecar = !full_sidecar && file.ends_with(".xmp");
            if full_sidecar {
                true
            } else if stem_sidecar {
                !shared
            } else {
                // A JPEG or HEIF: a raw's RAW+JPEG partner, never a rendered
                // photo's.
                !rendered && !shared
            }
        })
        .collect()
}

/// Where a photo's sidecar is: Adobe's `{stem}.xmp`, unless another
/// photo beside it shares the stem or a `{name}.xmp` is already there — then
/// the full-name form, so two photos never read and write one sidecar.
pub fn sidecar_path(photo: &Path, stems: &mut StemIndex) -> PathBuf {
    let full = PathBuf::from(format!("{}.xmp", photo.display()));
    if full.exists() || stems.shares_stem(photo) {
        full
    } else {
        photo.with_extension("xmp")
    }
}

/// Moves `paths` to the Recycle Bin in one operation.
pub fn to_recycle_bin(paths: &[PathBuf]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let bin = if cfg!(target_os = "macos") {
        "Trash"
    } else {
        "Recycle Bin"
    };
    trash::delete_all(paths).with_context(|| format!("moving files to the {bin}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> crate::test_support::Scratch {
        crate::test_support::Scratch::new(&format!("shell-{name}"))
    }

    #[test]
    fn a_raw_takes_its_sidecars_and_its_jpeg_along() {
        let dir = scratch("companions");
        let raw = dir.join("DSC00001.ARW");
        for f in [
            "DSC00001.ARW",
            "DSC00001.xmp",
            "DSC00001.ARW.xmp",
            "DSC00001.JPG",
            "DSC00002.JPG",
        ] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let mut found: Vec<String> = companions(&raw)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_lowercase())
            .collect();
        found.sort();
        // Each once, however the disk folds case — and never another frame's.
        assert_eq!(found, ["dsc00001.arw.xmp", "dsc00001.jpg", "dsc00001.xmp"]);
    }

    #[test]
    fn a_raw_alone_has_no_companions() {
        let dir = scratch("alone");
        let raw = dir.join("DSC00003.ARW");
        std::fs::write(&raw, b"x").unwrap();
        assert!(companions(&raw).is_empty());
    }

    #[test]
    fn revealing_a_file_that_is_gone_says_so() {
        let dir = scratch("gone");
        let err = reveal(&dir.join("nope.ARW")).unwrap_err().to_string();
        assert!(err.contains("isn't there"), "{err}");
    }

    /// Two photos sharing a stem are told apart from a raw and its RAW+JPEG
    /// partner.
    #[test]
    fn photos_sharing_a_stem_are_found() {
        let dir = crate::test_support::Scratch::new("shell-stems");
        for f in [
            "scan.tif", "scan.jpg", "DSC1.ARW", "DSC1.JPG", "IMG.HEIC", "img.jpg", "solo.jpg",
        ] {
            std::fs::write(dir.join(f), f).unwrap();
        }
        let mut stems = StemIndex::default();
        assert!(stems.shares_stem(&dir.join("scan.tif")));
        assert!(stems.shares_stem(&dir.join("scan.jpg")));
        assert!(stems.shares_stem(&dir.join("IMG.HEIC")), "case aside");
        assert!(
            !stems.shares_stem(&dir.join("DSC1.ARW")),
            "RAW+JPEG is one photo"
        );
        assert!(!stems.shares_stem(&dir.join("solo.jpg")));
    }

    /// A rejected TIFF takes only its own sidecar to the Recycle Bin, never
    /// the kept JPEG beside it or the stem sidecar they share; a raw still
    /// takes its RAW+JPEG partner and sidecar.
    #[test]
    fn only_a_photos_own_files_are_binned_with_it() {
        let dir = crate::test_support::Scratch::new("shell-bin");
        for f in [
            "scan.tif",
            "scan.jpg",
            "scan.xmp",
            "scan.tif.xmp",
            "DSC1.ARW",
            "DSC1.JPG",
            "DSC1.xmp",
        ] {
            std::fs::write(dir.join(f), f).unwrap();
        }
        let names = |paths: Vec<PathBuf>| -> Vec<String> {
            let mut n: Vec<String> = paths
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().to_lowercase())
                .collect();
            n.sort();
            n
        };
        let mut stems = StemIndex::default();
        assert_eq!(
            names(bin_companions(&dir.join("scan.tif"), &mut stems)),
            ["scan.tif.xmp"]
        );
        assert_eq!(
            names(bin_companions(&dir.join("DSC1.ARW"), &mut stems)),
            ["dsc1.jpg", "dsc1.xmp"]
        );
        // And their sidecars are told apart too.
        assert!(sidecar_path(&dir.join("scan.jpg"), &mut stems).ends_with("scan.jpg.xmp"));
        assert!(sidecar_path(&dir.join("DSC1.ARW"), &mut stems).ends_with("DSC1.xmp"));
    }
}
