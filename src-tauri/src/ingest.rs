//! Importing from a memory card: copy a card's raws into dated folders
//! of a library folder, verified, skipping what is already imported, with an
//! optional second copy for backup.
//!
//! The first step used to happen outside the app — copy the card in Explorer,
//! then point SourceLight at the folder — which is the step event and wedding
//! shooters do after every job, and the one dedicated ingest tools are bought for.
//!
//! Three rules shape it:
//!
//! - **The card is only ever read.** Nothing here writes to, or deletes from,
//!   the source.
//! - **Every copy is verified.** A file is streamed to a `.part` temp while it
//!   is hashed and flushed to the drive. Then the card's file is read a second
//!   time and the temp is read back, both past the OS cache, and only when all
//!   three hashes match is the temp renamed into place. A card that
//!   returns different bytes twice, a flaky USB reader, or a drive that drops
//!   the write fails that file loudly instead of leaving a quietly corrupt raw
//!   in the library. Reading the card twice is what makes it a check on the
//!   card and not only on the copy; the two reads run side by side, so on
//!   separate drives it costs little more than the slower of them.
//! - **A second import copies nothing.** A file is "already imported" when the
//!   catalogue has a photo with its quick hash (size, modified time and first
//!   64 KB, see `scan.rs`). The copy keeps the card's modified time, so the
//!   imported file hashes the same as the card's — which is what makes this
//!   work, and why the time is set explicitly rather than left to the OS.
//!
//! Folder names come from each file's modified time, which cameras set to the
//! capture time. Reading the capture date from EXIF would mean reading every
//! file in full just to plan the import; this plans a 2,000-frame card in the
//! time it takes to list it.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Somewhere to import from that looks like a camera card: a volume with a
/// `DCIM` folder.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportSource {
    /// The folder to import from — the volume's `DCIM`.
    pub path: String,
    /// How to name it in the UI: "E:" or "Untitled".
    pub label: String,
}

/// Cards plugged in now. Windows drive letters and macOS `/Volumes/*` are both
/// just probed for `DCIM` — no platform code needed, since each simply does
/// not exist on the other.
pub fn sources() -> Vec<ImportSource> {
    let mut out = Vec::new();
    for letter in b'D'..=b'Z' {
        let root = format!("{}:\\", letter as char);
        let dcim = Path::new(&root).join("DCIM");
        if dcim.is_dir() {
            out.push(ImportSource {
                path: dcim.to_string_lossy().to_string(),
                label: format!("{}:", letter as char),
            });
        }
    }
    if let Ok(vols) = std::fs::read_dir("/Volumes") {
        for v in vols.flatten() {
            let dcim = v.path().join("DCIM");
            if dcim.is_dir() {
                out.push(ImportSource {
                    path: dcim.to_string_lossy().to_string(),
                    label: v.file_name().to_string_lossy().to_string(),
                });
            }
        }
    }
    out
}

/// Whether `path` is on a camera card: inside a `DCIM` folder, holding
/// one, or on a volume whose top level does. Photos catalogued from there
/// live only on the card, and go — with every rating and edit made to them —
/// the next time the card is formatted in the camera. The same `DCIM` test
/// [`sources`] finds cards by.
pub fn is_on_card(path: &Path) -> bool {
    if path
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case("DCIM"))
        || path.join("DCIM").is_dir()
    {
        return true;
    }
    // The volume: `E:\` on Windows, `/Volumes/NAME` on macOS.
    path.ancestors()
        .find(|a| {
            a.parent()
                .is_none_or(|p| p.parent().is_none() && p.ends_with("Volumes"))
        })
        .is_some_and(|volume| volume.join("DCIM").is_dir())
}

/// One file the import would copy.
#[derive(Debug, Clone)]
pub struct PlannedFile {
    pub src: PathBuf,
    pub size: u64,
    /// Modified time as seconds since the epoch, local-date-bucketed for the
    /// folder name.
    pub modified: i64,
    /// The quick hash the catalogue would give the imported copy.
    pub hash: String,
    /// Sidecars and the RAW+JPEG JPEG that travel with it.
    pub companions: Vec<PathBuf>,
}

/// What a source holds, grouped by day for the UI.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub days: Vec<ImportDay>,
    /// Raw files found.
    pub files: u32,
    /// Of those, how many the catalogue already has.
    pub already: u32,
    /// Bytes the new files (and their companions) would copy.
    pub bytes: f64,
    /// Files and folders on the card that could not be read.
    pub unreadable: Vec<ImportProblem>,
    /// What is on the card that the import does not copy.
    pub left_behind: LeftBehind,
}

/// A file or folder that did not make it, and why.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportProblem {
    pub path: String,
    pub reason: String,
}

impl ImportProblem {
    fn new(path: &Path, why: impl std::fmt::Display) -> Self {
        ImportProblem {
            path: path.to_string_lossy().to_string(),
            reason: why.to_string(),
        }
    }
}

/// What is on the card that the import does not copy. Photographers
/// format on the strength of an import's result, so a clip or voice note left
/// behind is said, not left out.
#[derive(Debug, Clone, Default, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LeftBehind {
    pub videos: u32,
    pub audio: u32,
    pub other: u32,
    /// A few of their names, to recognise them by.
    pub examples: Vec<String>,
}

impl LeftBehind {
    const EXAMPLES: usize = 6;

    fn add(&mut self, path: &Path) {
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            e if VIDEO.contains(&e) => self.videos += 1,
            e if AUDIO.contains(&e) => self.audio += 1,
            _ => self.other += 1,
        }
        if self.examples.len() < Self::EXAMPLES {
            if let Some(name) = path.file_name() {
                self.examples.push(name.to_string_lossy().to_string());
            }
        }
    }

    pub fn total(&self) -> u32 {
        self.videos + self.audio + self.other
    }
}

/// Extensions cameras record video under, in DCIM or beside it.
const VIDEO: &[&str] = &[
    "mp4", "mov", "m4v", "mts", "m2ts", "avi", "mxf", "3gp", "lrv", "insv", "braw", "r3d", "crm",
];
/// Voice memos and recorded sound.
const AUDIO: &[&str] = &["wav", "bwf", "mp3", "m4a", "aac"];

/// Whether an entry is the OS's, not the photographer's: indexing and trash
/// folders, and hidden litter such as `.DS_Store` and `._` resource forks.
fn is_system(entry: &walkdir::DirEntry) -> bool {
    let name = entry.file_name().to_string_lossy();
    entry.depth() > 0
        && (name.starts_with('.')
            || [
                "System Volume Information",
                "$RECYCLE.BIN",
                "Thumbs.db",
                "desktop.ini",
            ]
            .iter()
            .any(|s| name.eq_ignore_ascii_case(s)))
}

/// A path as a key that matches however its case was spelled: Windows and
/// macOS volumes compare names without case, and a companion is found by a
/// spelling of its name that need not be the one on disk.
fn path_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// A source, read: what to copy, and what will not be.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    pub files: Vec<PlannedFile>,
    pub unreadable: Vec<ImportProblem>,
    pub left_behind: LeftBehind,
}

#[derive(Debug, Clone, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportDay {
    /// `YYYY-MM-DD`.
    pub date: String,
    pub files: u32,
    pub new_files: u32,
}

/// Lists a source's photos — raws, and JPEGs without a raw beside them
/// — with what the import needs to know about each, and what it will not
/// copy.
///
/// One unreadable file fails only itself: a card that is starting to fail is
/// exactly when the rest of it most needs copying off. It, and any folder
/// that could not be listed, is named in `unreadable` instead. Videos, sound
/// and other files are counted in `left_behind`, including a card's video
/// folders beside `DCIM` (Sony's `PRIVATE\M4ROOT`, `AVCHD`).
pub fn plan(source: &Path) -> Result<Plan> {
    anyhow::ensure!(
        source.is_dir(),
        "{} can't be read as a folder",
        source.display()
    );
    let mut plan = Plan::default();
    let (mut photos, mut others) = (Vec::new(), Vec::new());
    let walk = walkdir::WalkDir::new(source)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !is_system(e));
    for entry in walk {
        match entry {
            Err(e) => {
                let path = e.path().unwrap_or(source).to_path_buf();
                plan.unreadable.push(ImportProblem::new(&path, e));
            }
            Ok(e) if e.file_type().is_file() => {
                if crate::scan::is_photo_file(e.path()) {
                    photos.push(e.into_path());
                } else {
                    others.push(e.into_path());
                }
            }
            Ok(_) => {}
        }
    }
    for src in crate::scan::without_raw_partners(photos) {
        match plan_file(&src) {
            Ok(f) => plan.files.push(f),
            Err(e) => plan
                .unreadable
                .push(ImportProblem::new(&src, format!("{e:#}"))),
        }
    }
    let travelling: std::collections::HashSet<String> = plan
        .files
        .iter()
        .flat_map(|f| f.companions.iter().map(|c| path_key(c)))
        .collect();
    for other in others {
        if !travelling.contains(&path_key(&other)) {
            plan.left_behind.add(&other);
        }
    }
    // Cameras that keep video outside DCIM: only clips and sound are counted
    // there, since the rest is the card's own bookkeeping.
    if source
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("DCIM"))
    {
        if let Some(volume) = source.parent() {
            let beside = walkdir::WalkDir::new(volume)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| !is_system(e) && e.path() != source)
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file());
            for e in beside {
                let ext = e
                    .path()
                    .extension()
                    .map(|x| x.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                if VIDEO.contains(&ext.as_str()) || AUDIO.contains(&ext.as_str()) {
                    plan.left_behind.add(e.path());
                }
            }
        }
    }
    plan.files
        .sort_by(|a, b| a.modified.cmp(&b.modified).then_with(|| a.src.cmp(&b.src)));
    Ok(plan)
}

/// One photo's entry in a plan: its size, time, quick hash and companions.
fn plan_file(src: &Path) -> Result<PlannedFile> {
    let md = std::fs::metadata(src).with_context(|| format!("reading {}", src.display()))?;
    let modified = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Ok(PlannedFile {
        hash: crate::scan::quick_hash(src).with_context(|| format!("reading {}", src.display()))?,
        companions: crate::shell::companions(src),
        size: md.len(),
        modified,
        src: src.to_path_buf(),
    })
}

/// Bytes `files` and their companions take: what copying them needs free.
pub fn bytes_of<'a>(files: impl IntoIterator<Item = &'a PlannedFile>) -> u64 {
    files
        .into_iter()
        .map(|f| {
            f.size
                + f.companions
                    .iter()
                    .map(|c| std::fs::metadata(c).map(|m| m.len()).unwrap_or(0))
                    .sum::<u64>()
        })
        .sum()
}

/// The local calendar date and time of a modified time.
fn local(modified: i64) -> chrono::DateTime<chrono::Local> {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(modified, 0)
        .single()
        .unwrap_or_else(chrono::Local::now)
}

/// Whole hours a card file's time may have moved since it was imported: a
/// daylight-saving change, or a laptop taken across time zones.
const CLOCK_SHIFTS: std::ops::RangeInclusive<i64> = -14..=14;

/// Whether the catalogue already has `f`: by its quick hash, or — only for a
/// file that looks new — by the hash it would have had with its time a whole
/// number of hours off. The size and first 64 KB still have to match,
/// so only the clock is forgiven.
pub fn already_have(f: &PlannedFile, known: &dyn Fn(&str) -> bool) -> bool {
    if known(&f.hash) {
        return true;
    }
    let shifts: Vec<i64> = CLOCK_SHIFTS.filter(|h| *h != 0).map(|h| h * 3600).collect();
    crate::scan::quick_hashes(&f.src, &shifts)
        .map(|hs| hs.iter().any(|h| known(h)))
        .unwrap_or(false)
}

/// The next number for a renamed file, per folder: one past the
/// highest the folder already holds under this pattern. Every import used to
/// start at 0001, so a second card of the same day landed as `…_0001-1`.
#[derive(Default)]
struct Numbering {
    next: std::collections::HashMap<PathBuf, usize>,
}

impl Numbering {
    fn take(&mut self, opts: &ImportOptions, f: &PlannedFile, root: &Path) -> usize {
        let dir = target_for(opts, f, 0, root)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| root.to_path_buf());
        let next = self
            .next
            .entry(dir.clone())
            .or_insert_with(|| highest_number(&dir, opts, f) + 1);
        let n = *next;
        *next += 1;
        n
    }
}

/// The highest `{seq}` among the files in `dir` named by `opts.rename`, or 0.
fn highest_number(dir: &Path, opts: &ImportOptions, f: &PlannedFile) -> usize {
    let Some((before, after)) = opts.rename.split_once("{seq}") else {
        return 0;
    };
    let prefix = if before.is_empty() {
        String::new()
    } else {
        fill(before, f, &opts.job, 0)
    };
    let suffix = if after.is_empty() {
        String::new()
    } else {
        fill(after, f, &opts.job, 0)
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let stem = e.path().file_stem()?.to_string_lossy().to_string();
            let middle = stem
                .strip_prefix(&prefix)?
                .strip_suffix(suffix.as_str())?
                .to_string();
            (!middle.is_empty() && middle.bytes().all(|b| b.is_ascii_digit()))
                .then(|| middle.parse::<usize>().ok())
                .flatten()
        })
        .max()
        .unwrap_or(0)
}

/// Summarises a plan by day, given which hashes the catalogue already has.
pub fn preview(plan: &Plan, known: &dyn Fn(&str) -> bool) -> ImportPreview {
    let files = &plan.files;
    let mut days: Vec<ImportDay> = Vec::new();
    let mut already = 0;
    let mut bytes = 0.0;
    for f in files {
        let date = local(f.modified).format("%Y-%m-%d").to_string();
        let is_new = !already_have(f, known);
        if is_new {
            bytes += bytes_of([f]) as f64;
        } else {
            already += 1;
        }
        match days.last_mut() {
            Some(d) if d.date == date => {
                d.files += 1;
                d.new_files += is_new as u32;
            }
            _ => days.push(ImportDay {
                date,
                files: 1,
                new_files: is_new as u32,
            }),
        }
    }
    ImportPreview {
        days,
        files: files.len() as u32,
        already,
        bytes,
        unreadable: plan.unreadable.clone(),
        left_behind: plan.left_behind.clone(),
    }
}

/// How the import names things.
#[derive(Debug, Clone, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportOptions {
    pub source: String,
    /// The library folder to import into.
    pub destination: String,
    /// Subfolder template, `/`-separated: `{yyyy}`, `{mm}`, `{dd}`,
    /// `{yyyy-mm-dd}`, `{job}`. Empty for none.
    pub folders: String,
    /// File name template without extension — `{name}` (the original),
    /// `{yyyymmdd}`, `{hhmmss}`, `{seq}`, `{job}` — or empty to keep names.
    pub rename: String,
    /// What `{job}` means: "Smith wedding".
    pub job: String,
    /// A second folder to copy everything to as well, for backup.
    pub backup: Option<String>,
    /// Only these card files, by path: "Try these again" after a failure.
    /// Every new file when absent.
    #[serde(default)]
    pub only: Option<Vec<String>>,
    /// The shoot a retry adds to, rather than starting another.
    #[serde(default)]
    pub shoot_id: Option<i32>,
}

/// Replaces the tokens in `template` for one file. Characters Windows forbids
/// in names are replaced, and a token-free empty result falls back to "_".
fn fill(template: &str, f: &PlannedFile, job: &str, seq: usize) -> String {
    let t = local(f.modified);
    let name = f
        .src
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let out = template
        .replace("{yyyy-mm-dd}", &t.format("%Y-%m-%d").to_string())
        .replace("{yyyymmdd}", &t.format("%Y%m%d").to_string())
        .replace("{hhmmss}", &t.format("%H%M%S").to_string())
        .replace("{yyyy}", &t.format("%Y").to_string())
        .replace("{mm}", &t.format("%m").to_string())
        .replace("{dd}", &t.format("%d").to_string())
        .replace("{seq}", &format!("{seq:04}"))
        .replace("{name}", &name)
        .replace("{job}", job.trim());
    let clean: String = out
        .chars()
        .map(|c| {
            // `/` too: a template is split into folders before it is filled,
            // so a job named "AC/DC" must stay one folder, and "../x" must not
            // climb out of the library.
            if "<>:\"\\|?*/".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let clean = clean.trim().trim_matches('.').to_string();
    if clean.is_empty() {
        "_".into()
    } else {
        clean
    }
}

/// Where one file goes, before collision handling: destination, then each
/// template folder, then the (possibly renamed) file with its extension.
pub fn target_for(opts: &ImportOptions, f: &PlannedFile, seq: usize, root: &Path) -> PathBuf {
    let mut dir = root.to_path_buf();
    for part in opts.folders.split('/').filter(|p| !p.trim().is_empty()) {
        dir.push(fill(part, f, &opts.job, seq));
    }
    let ext = f
        .src
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    let stem = if opts.rename.trim().is_empty() {
        f.src
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "_".into())
    } else {
        fill(&opts.rename, f, &opts.job, seq)
    };
    dir.join(if ext.is_empty() {
        stem
    } else {
        format!("{stem}.{ext}")
    })
}

/// A companion's name beside its renamed raw: the raw's new stem, the
/// companion's own tail (`.xmp`, `.ARW.xmp` becomes `.<new ext>.xmp`, `.JPG`).
fn companion_target(raw_src: &Path, raw_dst: &Path, companion: &Path) -> PathBuf {
    let src_name = raw_src
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let src_stem = raw_src
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let dst_name = raw_dst
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let dst_stem = raw_dst
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let c = companion
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let renamed = if let Some(tail) = c.strip_prefix(&src_name) {
        format!("{dst_name}{tail}")
    } else if let Some(tail) = c.strip_prefix(&src_stem) {
        format!("{dst_stem}{tail}")
    } else {
        c
    };
    raw_dst.with_file_name(renamed)
}

/// `path`, or `path` with `-1`, `-2`… before the extension, whichever does not
/// exist yet — an import never overwrites anything.
fn free_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (1..)
        .map(|n| path.with_file_name(format!("{stem}-{n}{ext}")))
        .find(|p| !p.exists())
        .expect("an unbounded search finds a free name")
}

/// Reads a file through `buf` in full, hashing it.
fn sha256_with(file: &mut std::fs::File, path: &Path, buf: &mut [u8]) -> Result<Vec<u8>> {
    let mut hasher = Sha256::new();
    loop {
        let n = file
            .read(buf)
            .with_context(|| format!("reading {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_vec())
}

/// Block size for uncached reads: a multiple of every sector size in use
/// (512 and 4,096 bytes), which Windows requires of both the length and the
/// buffer's address.
const SECTOR: usize = 4096;

/// A 1 MiB buffer aligned to [`SECTOR`], as an uncached read on Windows needs.
fn aligned_buffer(storage: &mut Vec<u8>) -> &mut [u8] {
    const LEN: usize = 1 << 20;
    storage.resize(LEN + SECTOR, 0);
    let offset = storage.as_ptr().align_offset(SECTOR);
    &mut storage[offset..offset + LEN]
}

/// Opens `path` for reading past the OS file cache where the platform allows
/// it, so what comes back is what the drive holds, not what was just written
/// or read through memory.
///
/// - Windows: `FILE_FLAG_NO_BUFFERING`. A non-cached read of a range first
///   writes back any dirty cached pages for it, then reads the device.
/// - macOS: `F_NOCACHE` on the descriptor.
/// - Linux: the file's clean cached pages are dropped first.
///
/// Falls back to an ordinary open where the flag is refused (some network
/// filesystems): the check then still compares the bytes, only less strictly.
fn open_uncached(path: &Path) -> Result<std::fs::File> {
    let open = || std::fs::File::open(path).with_context(|| format!("reading {}", path.display()));
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_NO_BUFFERING: u32 = 0x2000_0000;
        if let Ok(file) = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_NO_BUFFERING)
            .open(path)
        {
            return Ok(file);
        }
        open()
    }
    #[cfg(not(windows))]
    {
        let file = open()?;
        uncache(&file);
        Ok(file)
    }
}

/// Keeps `file`'s pages out of the OS cache from here on (macOS), or drops the
/// ones already there (Linux). Best effort: a refusal only weakens the check.
#[cfg(not(windows))]
fn uncache(file: &std::fs::File) {
    use std::os::fd::AsRawFd;
    let fd = file.as_raw_fd();
    // SAFETY: plain fcntl/fadvise calls on a descriptor `file` owns and keeps
    // open for the length of the call.
    #[cfg(target_os = "macos")]
    unsafe {
        libc::fcntl(fd, libc::F_NOCACHE, 1);
    }
    // SAFETY: as above.
    #[cfg(target_os = "linux")]
    unsafe {
        libc::posix_fadvise(fd, 0, 0, libc::POSIX_FADV_DONTNEED);
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = fd;
}

/// The SHA-256 of what the drive holds for `path`, read past the cache.
fn sha256_uncached(path: &Path) -> Result<Vec<u8>> {
    let mut file = open_uncached(path)?;
    let mut storage = Vec::new();
    sha256_with(&mut file, path, aligned_buffer(&mut storage))
}

/// Whether a copy can be trusted: the bytes as first read, the card's file
/// read a second time, and the copy as it landed must all hash the same.
/// Each mismatch names the side that failed, since "the card is failing" and
/// "the backup drive is failing" call for different things.
fn check_copy(read: &[u8], source_again: &[u8], landed: &[u8], src: &Path) -> Result<()> {
    anyhow::ensure!(
        read == source_again,
        "{} read differently twice — the card or its reader may be failing",
        src.display()
    );
    anyhow::ensure!(
        read == landed,
        "{} did not copy intact — the drive it was copied to may be failing",
        src.display()
    );
    Ok(())
}

/// Copies `src` to `dst` (which must not exist), verified, keeping the
/// modified time. Returns the bytes copied.
pub fn copy_verified(src: &Path, dst: &Path) -> Result<u64> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp = {
        let mut s = dst.as_os_str().to_owned();
        s.push(".part");
        PathBuf::from(s)
    };
    let result = (|| -> Result<u64> {
        let mut input =
            std::fs::File::open(src).with_context(|| format!("reading {}", src.display()))?;
        let mut output =
            std::fs::File::create(&tmp).with_context(|| format!("writing {}", tmp.display()))?;
        // On macOS the cache is bypassed by descriptor, so neither side's
        // pages should be kept for the reads below to find.
        #[cfg(target_os = "macos")]
        {
            uncache(&input);
            uncache(&output);
        }
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        let mut total = 0u64;
        loop {
            let n = input
                .read(&mut buf)
                .with_context(|| format!("reading {}", src.display()))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            output
                .write_all(&buf[..n])
                .with_context(|| format!("writing {}", tmp.display()))?;
            total += n as u64;
        }
        // A flush that fails is a copy that may not be on the drive: a USB
        // drive dropping out, or a NAS's delayed write failing.
        output
            .sync_all()
            .with_context(|| format!("writing {} to the drive", tmp.display()))?;
        drop(output);
        drop(input);
        // Read the card again and what actually landed, both from the drives
        // rather than memory, side by side: they are usually different drives.
        let (again, landed) = std::thread::scope(|s| {
            let again = s.spawn(|| sha256_uncached(src));
            let landed = sha256_uncached(&tmp);
            let again = again
                .join()
                .unwrap_or_else(|_| Err(anyhow::anyhow!("re-reading {}", src.display())));
            (again, landed)
        });
        check_copy(&hasher.finalize(), &again?, &landed?, src)?;
        if let Ok(modified) = std::fs::metadata(src).and_then(|m| m.modified()) {
            let f = std::fs::OpenOptions::new().write(true).open(&tmp)?;
            f.set_modified(modified)?;
        }
        // Never over a file that appeared at `dst` since its name was chosen:
        // the copy fails instead, and the file there is kept.
        crate::fsutil::rename_new(&tmp, dst).with_context(|| {
            format!(
                "finishing {} (if a file of that name appeared meanwhile, it was kept)",
                dst.display()
            )
        })?;
        Ok(total)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// What an import did.
///
/// Each way a photo can fail to be safe is its own list, because each
/// means something different for the card: `failed` and `unreadable` photos
/// are not in the library at all, `not_backed_up` ones are but only once, and
/// `not_reached` ones were never tried because the import was stopped.
#[derive(Debug, Clone, Default, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportOutcome {
    /// Raw files copied into the library (companions not counted).
    pub copied: u32,
    /// Raw files skipped because the library already has them.
    pub skipped: u32,
    /// Photos not copied into the library: the card could not be read, or the
    /// copy failed its check.
    pub failed: Vec<ImportProblem>,
    /// Photos in the library whose backup copy failed.
    pub not_backed_up: Vec<ImportProblem>,
    /// Files and folders the plan could not read.
    pub unreadable: Vec<ImportProblem>,
    /// What is on the card and not copied: videos, sound, other files.
    pub left_behind: LeftBehind,
    /// Whether the import was stopped before the end.
    pub stopped: bool,
    /// The card files a stop left untried.
    pub not_reached: Vec<String>,
    /// Folders the files went into, for the caller to scan.
    pub folders: Vec<String>,
    /// Where each raw was copied to in the library, for the shoot.
    /// Not sent to the page.
    #[serde(skip)]
    pub copied_paths: Vec<String>,
    /// The shoot the import was remembered as, once it has been — for
    /// the page to turn its folder names into keywords.
    pub shoot_id: Option<i32>,
}

/// Runs an import. `known` says whether the catalogue already has a hash;
/// `on_progress(done, total)` counts raw files. Stops between files when
/// `cancel` is set, and returns what it did so far rather than an error: a
/// stopped import's copies are kept, so they must join the shoot and the
/// catalogue like any others, or the next import copies them again.
pub fn run(
    opts: &ImportOptions,
    plan: &Plan,
    known: &dyn Fn(&str) -> bool,
    cancel: &crate::jobs::CancelToken,
    mut on_progress: impl FnMut(usize, usize),
) -> Result<ImportOutcome> {
    let files = &plan.files;
    let mut out = ImportOutcome {
        unreadable: plan.unreadable.clone(),
        left_behind: plan.left_behind.clone(),
        ..Default::default()
    };
    let roots: Vec<PathBuf> = std::iter::once(PathBuf::from(&opts.destination))
        .chain(
            opts.backup
                .iter()
                .filter(|b| !b.trim().is_empty())
                .map(PathBuf::from),
        )
        .collect();
    let only: Option<std::collections::HashSet<String>> = opts
        .only
        .as_ref()
        .map(|paths| paths.iter().map(|p| path_key(Path::new(p))).collect());
    let wanted = |f: &&PlannedFile| only.as_ref().is_none_or(|o| o.contains(&path_key(&f.src)));
    let candidates: Vec<&PlannedFile> = files.iter().filter(wanted).collect();
    let (todo, have): (Vec<&PlannedFile>, Vec<&PlannedFile>) = candidates
        .iter()
        .copied()
        .partition(|f| !already_have(f, known));
    out.skipped = have.len() as u32;
    let mut folders = std::collections::BTreeSet::new();
    let mut numbering = Numbering::default();
    for (i, f) in todo.iter().enumerate() {
        if cancel.is_cancelled() {
            out.stopped = true;
            out.not_reached = todo[i..]
                .iter()
                .map(|f| f.src.to_string_lossy().to_string())
                .collect();
            break;
        }
        // Numbered in the library's folder, and the same number on the backup,
        // so the two copies keep one name.
        let seq = if opts.rename.contains("{seq}") {
            numbering.take(opts, f, &roots[0])
        } else {
            i + 1
        };
        match place(opts, f, seq, &roots[0]) {
            Ok(dst) => {
                if let Some(parent) = dst.parent() {
                    folders.insert(parent.to_string_lossy().to_string());
                }
                out.copied_paths.push(dst.to_string_lossy().to_string());
                out.copied += 1;
                for root in &roots[1..] {
                    if let Err(e) = place(opts, f, seq, root) {
                        out.not_backed_up
                            .push(ImportProblem::new(&f.src, format!("{e:#}")));
                    }
                }
            }
            Err(e) => out
                .failed
                .push(ImportProblem::new(&f.src, format!("{e:#}"))),
        }
        on_progress(i + 1, todo.len());
    }
    // What the library already has goes to the backup too: the report calls
    // a photo safe to format once it is in both, and these were never copied
    // there. One its backup folder already holds — an earlier run's copy,
    // under whatever number it got — is not copied again.
    if roots.len() > 1 {
        let mut backup_numbering = Numbering::default();
        let mut held: std::collections::HashMap<PathBuf, std::collections::HashSet<String>> =
            std::collections::HashMap::new();
        for (i, f) in have.iter().enumerate() {
            if out.stopped || cancel.is_cancelled() {
                out.stopped = true;
                for f in &have[i..] {
                    out.not_backed_up.push(ImportProblem::new(
                        &f.src,
                        "the import was stopped before it was copied to the backup",
                    ));
                }
                break;
            }
            for root in &roots[1..] {
                let dir = target_for(opts, f, 0, root)
                    .parent()
                    .map_or_else(|| root.clone(), Path::to_path_buf);
                let there = held
                    .entry(dir.clone())
                    .or_insert_with(|| quick_hashes_in(&dir));
                if there.contains(&f.hash) {
                    continue;
                }
                let seq = if opts.rename.contains("{seq}") {
                    backup_numbering.take(opts, f, root)
                } else {
                    0
                };
                match place(opts, f, seq, root) {
                    Ok(_) => {
                        there.insert(f.hash.clone());
                    }
                    Err(e) => out
                        .not_backed_up
                        .push(ImportProblem::new(&f.src, format!("{e:#}"))),
                }
            }
        }
    }
    out.folders = folders.into_iter().collect();
    Ok(out)
}

/// The quick hashes of the files in `dir`, none if it can't be read.
fn quick_hashes_in(dir: &Path) -> std::collections::HashSet<String> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .filter_map(|e| crate::scan::quick_hash(&e.path()).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Copies one planned file and its companions under `root`, verified, and
/// returns where the file landed.
///
/// A file already at its target with the same quick hash is taken as copied
/// rather than copied again beside itself as `-1`: that is what a
/// stopped or crashed import leaves, and what a backup finished later finds.
fn place(opts: &ImportOptions, f: &PlannedFile, seq: usize, root: &Path) -> Result<PathBuf> {
    let target = target_for(opts, f, seq, root);
    anyhow::ensure!(
        target.starts_with(root),
        "{} would land outside {}",
        target.display(),
        root.display()
    );
    if target.is_file() && crate::scan::quick_hash(&target).ok().as_deref() == Some(&f.hash) {
        for c in &f.companions {
            let to = companion_target(&f.src, &target, c);
            if !to.exists() {
                copy_verified(c, &to)?;
            }
        }
        return Ok(target);
    }
    let dst = free_path(&target);
    copy_verified(&f.src, &dst)?;
    for c in &f.companions {
        copy_verified(c, &free_path(&companion_target(&f.src, &dst, c)))?;
    }
    Ok(dst)
}

/// What finishing a backup did.
#[derive(Debug, Clone, Default, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct BackupOutcome {
    /// Photos now on the backup, whether copied now or found there already.
    pub backed_up: u32,
    pub failed: Vec<ImportProblem>,
}

/// Copies a shoot's photos, from the library, to the backup where they are
/// missing: the second copy an import could not make, because the
/// backup drive filled up or was unplugged. From the library rather than the
/// card, which may already be back in the camera. Each lands at the same
/// place under `backup` as under `destination`, verified, companions too.
pub fn finish_backup(
    destination: &Path,
    backup: &Path,
    paths: &[String],
    cancel: &crate::jobs::CancelToken,
    mut on_progress: impl FnMut(usize, usize),
) -> BackupOutcome {
    let mut out = BackupOutcome::default();
    for (i, path) in paths.iter().enumerate() {
        if cancel.is_cancelled() {
            break;
        }
        let src = Path::new(path);
        let done = (|| -> Result<()> {
            let rel = src.strip_prefix(destination).with_context(|| {
                format!("{} isn't inside {}", src.display(), destination.display())
            })?;
            let target = backup.join(rel);
            let have = crate::scan::quick_hash(src)?;
            let dst = if target.is_file()
                && crate::scan::quick_hash(&target).ok().as_deref() == Some(&have)
            {
                target
            } else {
                let dst = free_path(&target);
                copy_verified(src, &dst)?;
                dst
            };
            for c in crate::shell::companions(src) {
                let to = companion_target(src, &dst, &c);
                if !to.exists() {
                    copy_verified(&c, &to)?;
                }
            }
            Ok(())
        })();
        match done {
            Ok(()) => out.backed_up += 1,
            Err(e) => out.failed.push(ImportProblem::new(src, format!("{e:#}"))),
        }
        on_progress(i + 1, paths.len());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> crate::test_support::Scratch {
        crate::test_support::Scratch::new(&format!("ingest-{name}"))
    }

    fn opts(src: &Path, dst: &Path) -> ImportOptions {
        ImportOptions {
            source: src.to_string_lossy().to_string(),
            destination: dst.to_string_lossy().to_string(),
            folders: "{yyyy}/{yyyy-mm-dd}".into(),
            rename: String::new(),
            job: String::new(),
            backup: None,
            only: None,
            shoot_id: None,
        }
    }

    fn planned(path: &str, modified: i64) -> PlannedFile {
        PlannedFile {
            src: PathBuf::from(path),
            size: 1,
            modified,
            hash: "h".into(),
            companions: vec![],
        }
    }

    #[test]
    fn templates_name_folders_and_files_and_stay_legal() {
        // 2025-06-14 12:00 UTC: the same date in every timezone a test runs in.
        let f = planned("E:/DCIM/100MSDCF/DSC00042.ARW", 1_749_902_400);
        let mut o = opts(Path::new("E:/DCIM"), Path::new("D:/Photos"));
        o.job = "Smith: wedding?".into();
        assert_eq!(
            target_for(&o, &f, 7, Path::new("D:/Photos")),
            Path::new("D:/Photos")
                .join("2025")
                .join("2025-06-14")
                .join("DSC00042.ARW")
        );
        o.folders = "{yyyy}/{job}".into();
        o.rename = "{yyyymmdd}_{seq}".into();
        let t = target_for(&o, &f, 7, Path::new("D:/Photos"));
        // Characters Windows refuses in a name are replaced, not passed on.
        assert_eq!(
            t,
            Path::new("D:/Photos")
                .join("2025")
                .join("Smith_ wedding_")
                .join("20250614_0007.ARW")
        );
    }

    /// A job name is one folder, whatever it holds: `/` cannot nest
    /// it, and `..` cannot climb out of the library.
    #[test]
    fn a_job_name_stays_one_folder_inside_the_library() {
        let f = planned("E:/DCIM/100MSDCF/DSC00042.ARW", 1_749_902_400);
        let mut o = opts(Path::new("E:/DCIM"), Path::new("D:/Photos"));
        o.folders = "{job}".into();
        let root = Path::new("D:/Photos");
        for (job, folder) in [
            ("AC/DC 2025", "AC_DC 2025"),
            ("Smith/Jones wedding", "Smith_Jones wedding"),
            ("../../Windows", "_.._Windows"),
            ("..", "_"),
        ] {
            o.job = job.into();
            let t = target_for(&o, &f, 1, root);
            assert_eq!(t, root.join(folder).join("DSC00042.ARW"), "{job}");
            assert!(t.starts_with(root));
        }
    }

    /// The uncached read sees the whole file, including a tail shorter than
    /// a sector, and hashes as an ordinary read does.
    #[test]
    fn an_uncached_read_hashes_the_whole_file() {
        let dir = scratch("uncached");
        let path = dir.join("odd.ARW");
        let bytes: Vec<u8> = (0..(3 << 20) + 123).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(
            sha256_uncached(&path).unwrap(),
            Sha256::digest(&bytes).to_vec()
        );
        // And on Windows really uncached: the flag is accepted on a local
        // drive and reads through the aligned buffer, rather than the check
        // passing on the fallback's ordinary open.
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(0x2000_0000)
                .open(&path)
                .expect("FILE_FLAG_NO_BUFFERING accepted");
            let mut storage = Vec::new();
            let got = sha256_with(&mut file, &path, aligned_buffer(&mut storage)).unwrap();
            assert_eq!(got, Sha256::digest(&bytes).to_vec());
        }
    }

    /// Each way a copy can fail its check names the side at fault.
    #[test]
    fn a_failed_check_says_which_drive_to_distrust() {
        let src = Path::new("E:/DCIM/DSC00001.ARW");
        assert!(check_copy(b"a", b"a", b"a", src).is_ok());
        let card = check_copy(b"a", b"b", b"a", src).unwrap_err().to_string();
        assert!(card.contains("card"), "{card}");
        let drive = check_copy(b"a", b"a", b"b", src).unwrap_err().to_string();
        assert!(drive.contains("drive it was copied to"), "{drive}");
    }

    /// A card is recognised from anywhere on it; an ordinary folder,
    /// even one with photos in a `DCIM`-less tree, is not.
    #[test]
    fn a_folder_on_a_card_is_recognised() {
        let dir = scratch("on-card");
        let folder = dir.join("DCIM").join("100MSDCF");
        std::fs::create_dir_all(&folder).unwrap();
        assert!(is_on_card(&folder), "inside DCIM");
        assert!(is_on_card(&dir), "holding DCIM");
        let photos = dir.join("Photos").join("2025");
        std::fs::create_dir_all(&photos).unwrap();
        assert!(!is_on_card(&photos), "a library folder beside one is not");
        assert!(is_on_card(Path::new("E:/DCIM/100CANON")));
    }

    #[test]
    fn a_renamed_raw_takes_its_sidecars_names_along() {
        let c = companion_target(
            Path::new("E:/DSC00042.ARW"),
            Path::new("D:/x/20250614_0007.ARW"),
            Path::new("E:/DSC00042.ARW.xmp"),
        );
        assert_eq!(c, Path::new("D:/x/20250614_0007.ARW.xmp"));
        let c = companion_target(
            Path::new("E:/DSC00042.ARW"),
            Path::new("D:/x/20250614_0007.ARW"),
            Path::new("E:/DSC00042.JPG"),
        );
        assert_eq!(c, Path::new("D:/x/20250614_0007.JPG"));
    }

    #[test]
    fn a_copy_is_verified_keeps_its_time_and_never_overwrites() {
        let dir = scratch("copy");
        let src = dir.join("card").join("DSC00001.ARW");
        std::fs::create_dir_all(src.parent().unwrap()).unwrap();
        std::fs::write(&src, vec![7u8; 3 << 20]).unwrap();
        let dst = dir.join("lib").join("DSC00001.ARW");

        copy_verified(&src, &dst).unwrap();
        assert_eq!(std::fs::read(&dst).unwrap(), std::fs::read(&src).unwrap());
        // The same modified time, so the copy hashes as the card file does —
        // which is how a second import knows to skip it.
        assert_eq!(
            crate::scan::quick_hash(&dst).unwrap(),
            crate::scan::quick_hash(&src).unwrap()
        );
        assert!(!dst.with_extension("ARW.part").exists());
        // A second copy to the same name picks a free one instead.
        assert_eq!(free_path(&dst), dir.join("lib").join("DSC00001-1.ARW"));
    }

    #[test]
    fn a_card_imported_twice_copies_nothing_the_second_time() {
        let dir = scratch("twice");
        let card = dir.join("card").join("DCIM").join("100MSDCF");
        std::fs::create_dir_all(&card).unwrap();
        for n in 1..=3 {
            std::fs::write(card.join(format!("DSC0000{n}.ARW")), format!("raw {n}")).unwrap();
        }
        std::fs::write(card.join("DSC00001.JPG"), "jpeg").unwrap();
        let lib = dir.join("lib");
        let backup = dir.join("backup");
        let mut o = opts(&dir.join("card"), &lib);
        o.backup = Some(backup.to_string_lossy().to_string());

        let files = plan(&dir.join("card")).unwrap();
        assert_eq!(
            files.files.len(),
            3,
            "raws only; the JPEG travels as a companion"
        );
        assert_eq!(files.left_behind.total(), 0, "{:?}", files.left_behind);
        let never = |_: &str| false;
        let first = run(
            &o,
            &files,
            &never,
            &crate::jobs::Jobs::default().start("import"),
            |_, _| {},
        )
        .unwrap();
        assert_eq!((first.copied, first.skipped), (3, 0));
        assert!(first.failed.is_empty(), "{:?}", first.failed);
        // Both destinations, companions included.
        let count = |root: &Path| {
            walkdir::WalkDir::new(root)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
                .count()
        };
        assert_eq!((count(&lib), count(&backup)), (4, 4));

        // Everything copied now hashes as known: the second run skips it all.
        let imported: std::collections::HashSet<String> = crate::scan::find_photo_files(&lib)
            .iter()
            .map(|p| crate::scan::quick_hash(p).unwrap())
            .collect();
        let known = |h: &str| imported.contains(h);
        let again = run(
            &o,
            &files,
            &known,
            &crate::jobs::Jobs::default().start("import"),
            |_, _| {},
        )
        .unwrap();
        assert_eq!((again.copied, again.skipped), (0, 3));
        assert_eq!(count(&lib), 4);

        let p = preview(&files, &known);
        assert_eq!((p.files, p.already), (3, 3));
    }

    /// A backup chosen after the photos were imported gets them too: the
    /// report calls the card safe once they are in both, and skipping them
    /// left the backup empty. Once there, they are not copied again.
    #[test]
    fn photos_the_library_has_still_go_to_a_backup() {
        let dir = scratch("backup-later");
        let card = dir.join("card").join("DCIM").join("100MSDCF");
        std::fs::create_dir_all(&card).unwrap();
        for n in 1..=3 {
            std::fs::write(card.join(format!("DSC0000{n}.ARW")), format!("raw {n}")).unwrap();
        }
        std::fs::write(card.join("DSC00001.JPG"), "jpeg").unwrap();
        let (lib, backup) = (dir.join("lib"), dir.join("backup"));
        let files = plan(&dir.join("card")).unwrap();
        let job = || crate::jobs::Jobs::default().start("import");
        let mut o = opts(&dir.join("card"), &lib);
        run(&o, &files, &|_| false, &job(), |_, _| {}).unwrap();
        let imported: std::collections::HashSet<String> = crate::scan::find_photo_files(&lib)
            .iter()
            .map(|p| crate::scan::quick_hash(p).unwrap())
            .collect();
        let known = |h: &str| imported.contains(h);
        let count = |root: &Path| {
            walkdir::WalkDir::new(root)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
                .count()
        };

        o.backup = Some(backup.to_string_lossy().to_string());
        let later = run(&o, &files, &known, &job(), |_, _| {}).unwrap();
        assert_eq!((later.copied, later.skipped), (0, 3));
        assert!(later.not_backed_up.is_empty(), "{:?}", later.not_backed_up);
        assert_eq!(count(&backup), 4, "the raws and the JPEG beside one");

        // Numbered names too, and a second time nothing more.
        o.rename = "{yyyymmdd}_{seq}".into();
        let again = run(&o, &files, &known, &job(), |_, _| {}).unwrap();
        assert!(again.not_backed_up.is_empty(), "{:?}", again.not_backed_up);
        assert_eq!(count(&backup), 4);
    }

    /// A camera shooting JPEG only, or a phone's card: its photos are
    /// the JPEGs, and they import like raws.
    #[test]
    fn a_jpeg_only_card_imports_its_jpegs() {
        let dir = scratch("jpeg-only");
        let card = dir.join("card").join("DCIM");
        std::fs::create_dir_all(&card).unwrap();
        std::fs::write(card.join("IMG_0001.JPG"), "jpeg 1").unwrap();
        std::fs::write(card.join("IMG_0002.JPG"), "jpeg 2").unwrap();
        std::fs::write(card.join("IMG_0002.xmp"), "<x/>").unwrap();

        let files = plan(&dir.join("card")).unwrap();
        assert_eq!(files.files.len(), 2);
        let second = files
            .files
            .iter()
            .find(|f| f.src.ends_with("IMG_0002.JPG"))
            .unwrap();
        assert_eq!(second.companions.len(), 1, "its sidecar, never itself");

        let lib = dir.join("lib");
        let out = run(
            &opts(&dir.join("card"), &lib),
            &files,
            &|_: &str| false,
            &crate::jobs::Jobs::default().start("import"),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(out.copied, 2);
        assert_eq!(out.copied_paths.len(), 2);
        assert_eq!(crate::scan::find_photo_files(&lib).len(), 2);
    }

    /// A card as a camera leaves it: three raws in DCIM, the first shot
    /// RAW+JPEG with a sidecar.
    fn card(dir: &Path) -> PathBuf {
        let dcim = dir.join("card").join("DCIM");
        let folder = dcim.join("100MSDCF");
        std::fs::create_dir_all(&folder).unwrap();
        for n in 1..=3 {
            std::fs::write(folder.join(format!("DSC0000{n}.ARW")), format!("raw {n}")).unwrap();
        }
        std::fs::write(folder.join("DSC00001.JPG"), "jpeg").unwrap();
        std::fs::write(folder.join("DSC00001.ARW.xmp"), "<x/>").unwrap();
        dcim
    }

    fn files_under(root: &Path) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.into_path())
            .collect();
        files.sort();
        files
    }

    fn token() -> crate::jobs::CancelToken {
        crate::jobs::Jobs::default().start("import")
    }

    /// What the import will not copy is counted and named: clips and
    /// a voice note in DCIM, a clip in Sony's folder beside it, and a stray
    /// file — but not the OS's own litter or the photos' companions.
    #[test]
    fn what_stays_on_the_card_is_counted() {
        let dir = scratch("left-behind");
        let dcim = card(&dir);
        let folder = dcim.join("100MSDCF");
        std::fs::write(folder.join("C0001.MP4"), "clip").unwrap();
        std::fs::write(folder.join("DSC00002.WAV"), "voice").unwrap();
        std::fs::write(folder.join("NOTES.TXT"), "notes").unwrap();
        std::fs::write(folder.join(".DS_Store"), "litter").unwrap();
        let m4root = dir.join("card").join("PRIVATE").join("M4ROOT").join("CLIP");
        std::fs::create_dir_all(&m4root).unwrap();
        std::fs::write(m4root.join("C0002.MP4"), "clip").unwrap();
        std::fs::write(m4root.join("C0002M01.XML"), "<meta/>").unwrap();
        let svi = dir.join("card").join("System Volume Information");
        std::fs::create_dir_all(&svi).unwrap();
        std::fs::write(svi.join("IndexerVolumeGuid"), "x").unwrap();

        let plan = plan(&dcim).unwrap();
        assert_eq!(plan.files.len(), 3);
        assert!(plan.unreadable.is_empty(), "{:?}", plan.unreadable);
        let left = &plan.left_behind;
        assert_eq!((left.videos, left.audio, left.other), (2, 1, 1), "{left:?}");
        assert!(left.examples.iter().any(|e| e == "C0002.MP4"), "{left:?}");

        let p = preview(&plan, &|_: &str| false);
        assert_eq!(p.left_behind.total(), 4);
    }

    /// One file that can't be read fails only itself: the rest of the
    /// card is planned, and it is named.
    #[cfg(windows)]
    #[test]
    fn an_unreadable_file_fails_only_itself() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = scratch("unreadable");
        let dcim = card(&dir);
        let locked = dcim.join("100MSDCF").join("DSC00002.ARW");
        // No sharing: nothing else can open it while this is held, as with a
        // file the card can no longer read.
        let _hold = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&locked)
            .unwrap();
        let plan = plan(&dcim).unwrap();
        assert_eq!(plan.files.len(), 2);
        assert_eq!(plan.unreadable.len(), 1, "{:?}", plan.unreadable);
        assert!(plan.unreadable[0].path.ends_with("DSC00002.ARW"));

        let out = run(
            &opts(&dcim, &dir.join("lib")),
            &plan,
            &|_: &str| false,
            &token(),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(out.copied, 2);
        assert_eq!(out.unreadable.len(), 1, "carried into the result");
    }

    /// A stopped import says what it did not reach and keeps what it copied,
    /// and importing again picks up where it stopped, copying no
    /// frame a second time as `-1`.
    #[test]
    fn a_stopped_import_resumes_without_duplicates() {
        let dir = scratch("stopped");
        let dcim = card(&dir);
        let lib = dir.join("lib");
        let o = opts(&dcim, &lib);
        let plan = plan(&dcim).unwrap();
        let jobs = crate::jobs::Jobs::default();
        let cancel = jobs.start("import");
        let first = run(&o, &plan, &|_: &str| false, &cancel, |done, _| {
            if done == 1 {
                jobs.cancel("import");
            }
        })
        .unwrap();
        assert!(first.stopped);
        assert_eq!(first.copied, 1);
        assert_eq!(first.copied_paths.len(), 1);
        assert_eq!(first.not_reached.len(), 2);

        // Nothing catalogued yet — the scan may not have run — and still the
        // copy already in place is recognised, not copied beside itself.
        let again = run(&o, &plan, &|_: &str| false, &token(), |_, _| {}).unwrap();
        assert!(!again.stopped);
        assert_eq!(again.copied, 3);
        let names: Vec<String> = files_under(&lib)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(names.iter().all(|n| !n.contains("-1")), "{names:?}");
        assert_eq!(
            names.len(),
            5,
            "three raws, a JPEG and a sidecar: {names:?}"
        );
    }

    /// A backup copy that fails is its own outcome, not a failed import:
    /// the photos are in the library. Finishing the backup later
    /// copies them there from the library, companions too, and a second
    /// finish copies nothing new.
    #[test]
    fn a_failed_backup_is_reported_apart_and_can_be_finished() {
        let dir = scratch("backup");
        let dcim = card(&dir);
        let lib = dir.join("lib");
        let mut o = opts(&dcim, &lib);
        // A file where the backup folder should be: every copy to it fails,
        // as when its drive is full or gone.
        let blocked = dir.join("blocked");
        std::fs::write(&blocked, "not a folder").unwrap();
        o.backup = Some(blocked.to_string_lossy().to_string());
        let plan = plan(&dcim).unwrap();
        let out = run(&o, &plan, &|_: &str| false, &token(), |_, _| {}).unwrap();
        assert_eq!(out.copied, 3);
        assert!(out.failed.is_empty(), "{:?}", out.failed);
        assert_eq!(out.not_backed_up.len(), 3);

        let backup = dir.join("backup");
        std::fs::create_dir_all(&backup).unwrap();
        let done = finish_backup(&lib, &backup, &out.copied_paths, &token(), |_, _| {});
        assert_eq!(done.backed_up, 3, "{:?}", done.failed);
        let rel = |root: &Path| -> Vec<PathBuf> {
            files_under(root)
                .iter()
                .map(|p| p.strip_prefix(root).unwrap().to_path_buf())
                .collect()
        };
        assert_eq!(rel(&backup), rel(&lib), "the same files in the same places");
        let again = finish_backup(&lib, &backup, &out.copied_paths, &token(), |_, _| {});
        assert_eq!(again.backed_up, 3);
        assert_eq!(rel(&backup).len(), 5, "nothing copied twice");
    }

    /// "Try these again" copies only the files named.
    #[test]
    fn a_retry_copies_only_what_it_names() {
        let dir = scratch("only");
        let dcim = card(&dir);
        let lib = dir.join("lib");
        let plan = plan(&dcim).unwrap();
        let mut o = opts(&dcim, &lib);
        let second = plan.files[1].src.to_string_lossy().to_string();
        o.only = Some(vec![second]);
        let out = run(&o, &plan, &|_: &str| false, &token(), |_, _| {}).unwrap();
        assert_eq!((out.copied, out.skipped), (1, 0));
        assert!(out.copied_paths[0].ends_with("DSC00002.ARW"));
    }

    /// A card whose times moved by an hour — a clock change, a laptop in
    /// another zone — is still recognised as imported.
    #[test]
    fn a_card_read_an_hour_off_is_still_already_imported() {
        let dir = scratch("shifted");
        let card = dir.join("card").join("DCIM").join("100MSDCF");
        std::fs::create_dir_all(&card).unwrap();
        let raw = card.join("DSC00001.ARW");
        std::fs::write(&raw, "raw one").unwrap();
        let before: std::collections::HashSet<String> = [crate::scan::quick_hash(&raw).unwrap()]
            .into_iter()
            .collect();

        // The same file, an hour later by the PC's reckoning.
        let t = std::fs::metadata(&raw).unwrap().modified().unwrap();
        std::fs::File::options()
            .write(true)
            .open(&raw)
            .unwrap()
            .set_modified(t + std::time::Duration::from_secs(3600))
            .unwrap();
        let files = plan(&dir.join("card")).unwrap();
        let known = |h: &str| before.contains(h);
        assert!(
            !known(&files.files[0].hash),
            "its plain hash moved with the clock"
        );
        assert!(already_have(&files.files[0], &known));
        assert_eq!(preview(&files, &known).already, 1);

        // Different bytes are new whatever the time.
        std::fs::write(&raw, "another raw").unwrap();
        let files = plan(&dir.join("card")).unwrap();
        assert!(!already_have(&files.files[0], &known));
    }

    /// A second card of the same day carries on the numbering: it
    /// used to start at 0001 again and land as `…_0001-1`.
    #[test]
    fn a_second_card_carries_on_the_numbering() {
        let dir = scratch("numbering");
        let lib = dir.join("lib");
        let import = |name: &str, n: usize| {
            let card = dir.join(name).join("DCIM").join("100MSDCF");
            std::fs::create_dir_all(&card).unwrap();
            for i in 1..=n {
                std::fs::write(card.join(format!("DSC{i:05}.ARW")), format!("{name} {i}")).unwrap();
            }
            let mut o = opts(&dir.join(name), &lib);
            o.folders = String::new();
            o.rename = "shoot_{seq}".into();
            let files = plan(&dir.join(name)).unwrap();
            let out = run(
                &o,
                &files,
                &|_| false,
                &crate::jobs::Jobs::default().start("import"),
                |_, _| {},
            )
            .unwrap();
            assert!(out.failed.is_empty(), "{:?}", out.failed);
        };
        import("card1", 3);
        import("card2", 2);
        let mut names: Vec<String> = std::fs::read_dir(&lib)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "shoot_0001.ARW",
                "shoot_0002.ARW",
                "shoot_0003.ARW",
                "shoot_0004.ARW",
                "shoot_0005.ARW"
            ]
        );
    }
}
