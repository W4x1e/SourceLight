//! Where ONNX Runtime's CoreML provider keeps the models it compiles.
//!
//! For every model it runs, the provider converts the graph to a CoreML
//! package and compiles that. Left to itself it does both in the per-user temp
//! dir — `onnxruntime-<unique>` and `onnxruntime-<unique>.mlmodelc`, as big as
//! the model — and deletes them only when the session is released. The app
//! never releases its sessions: on a Mac the event loop ends the process with
//! `exit()`, so `AppState` and the face engine in it are never dropped, and
//! `tauri dev` kills the app outright on every rebuild. So every run that used
//! the face models left its folders behind: 214 of them, 4.4 GB, on the
//! development Mac after two days.
//!
//! Given a cache folder instead, the provider writes there, keeps what it
//! compiled and loads that next time, skipping the conversion and the compile.
//! It tells models apart by a hash of their *path*, though, so a model replaced
//! at the same path (an update) would be handed its predecessor's compile.
//! Each model file therefore gets a folder of its own here, named for the
//! file's path, size and modification time and the runtime's build
//! ([`Entry::open`]): a changed model or runtime gets a new folder, and the
//! stale one is removed.
//!
//! [`sweep_leftovers`] clears what earlier versions left in the temp dir.

use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

/// Names the model file a folder was compiled from, so a stale folder can be
/// recognised once that file changes or goes.
const SOURCE: &str = "source";
/// Written once a session has committed from the folder. Without it, what the
/// folder holds is a compile that never finished, or never loaded.
const READY: &str = "ready";
/// Held while a session is built from the folder, so two processes loading the
/// same model take turns instead of compiling into one folder at once.
const LOCK: &str = "lock";

/// One model file's folder in the cache, locked until the session built from
/// it has committed ([`Self::keep`]) or failed ([`Self::discard`]).
#[derive(Debug)]
pub struct Entry {
    dir: PathBuf,
    ready: bool,
    _lock: File,
}

impl Entry {
    /// Opens `model`'s folder under `root` for the ONNX Runtime build
    /// `runtime`, making it if need be. Waits while another process builds a
    /// session from the same folder.
    ///
    /// A folder without [`READY`] is emptied first. ONNX Runtime takes any
    /// package it finds there for a finished one, so a package half-written by
    /// a process that died would otherwise fail every load after it, and leave
    /// that model on the CPU for good. Folders compiled from a file that has
    /// since changed or gone are removed.
    pub fn open(root: &Path, model: &Path, runtime: &str) -> io::Result<Entry> {
        let model = model.canonicalize()?;
        let name = entry_name(&model, runtime)?;
        prune(root, &name, runtime);
        let dir = root.join(&name);
        fs::create_dir_all(&dir)?;
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(LOCK))?;
        take_turn(&lock)?;
        let ready = dir.join(READY).is_file();
        if !ready {
            empty(&dir)?;
            fs::write(dir.join(SOURCE), model.to_string_lossy().as_bytes())?;
        }
        Ok(Entry {
            dir,
            ready,
            _lock: lock,
        })
    }

    /// The folder to give the CoreML provider as its model cache.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Whether the folder holds a compile that has loaded before.
    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// The session committed, so what the provider wrote is whole and the
    /// next load may use it.
    pub fn keep(self) -> io::Result<()> {
        if self.ready {
            return Ok(());
        }
        fs::write(self.dir.join(READY), b"")
    }

    /// The session did not commit on CoreML. Nothing in the folder is worth
    /// keeping: not a compile that failed, and not a cached one that no longer
    /// loads.
    pub fn discard(self) {
        let _ = empty(&self.dir);
    }
}

/// The folder under `root` that [`Entry::open`] uses for `model`.
pub fn entry_dir(root: &Path, model: &Path, runtime: &str) -> io::Result<PathBuf> {
    Ok(root.join(entry_name(&model.canonicalize()?, runtime)?))
}

/// `<file stem>-<hash>`: the model's name, for whoever looks in the folder, and
/// a hash of everything that decides whether a compile still fits it.
fn entry_name(model: &Path, runtime: &str) -> io::Result<String> {
    let meta = fs::metadata(model)?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    let mut hash = Sha256::new();
    hash.update(model.to_string_lossy().as_bytes());
    hash.update([0]);
    hash.update(meta.len().to_le_bytes());
    hash.update(modified.to_le_bytes());
    hash.update(runtime.as_bytes());
    let hex = crate::hashing::hex(&hash.finalize());
    let stem: String = model
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(40)
        .collect();
    Ok(format!("{stem}-{}", &hex[..16]))
}

/// Removes the folders under `root` whose model file has changed or gone since
/// they were compiled, sparing `current`.
fn prune(root: &Path, current: &str, runtime: &str) {
    let Ok(folders) = fs::read_dir(root) else {
        return;
    };
    for folder in folders.flatten() {
        let name = folder.file_name();
        if name == current {
            continue;
        }
        let Ok(source) = fs::read_to_string(folder.path().join(SOURCE)) else {
            continue;
        };
        let stale = entry_name(Path::new(&source), runtime).map_or(true, |fits| name != *fits);
        if stale {
            let _ = fs::remove_dir_all(folder.path());
        }
    }
}

/// Removes the folders under `root` compiled from any of `models`, which no
/// longer run on CoreML, and returns how many it removed. SAM 2's encoder
/// alone left 561 MB.
pub fn forget(root: &Path, models: &[PathBuf]) -> usize {
    let names: Vec<String> = models
        .iter()
        .map(|m| m.canonicalize().unwrap_or_else(|_| m.clone()))
        .map(|m| m.to_string_lossy().into_owned())
        .collect();
    let Ok(folders) = fs::read_dir(root) else {
        return 0;
    };
    let mut removed = 0;
    for folder in folders.flatten() {
        let Ok(source) = fs::read_to_string(folder.path().join(SOURCE)) else {
            continue;
        };
        if names.contains(&source) && fs::remove_dir_all(folder.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Waits for, then takes, an exclusive lock on `file`, held until it closes.
/// `libc::flock` rather than `File::lock`, which is newer than the crate's
/// minimum Rust.
#[cfg(unix)]
fn take_turn(file: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    loop {
        // SAFETY: the descriptor stays open for as long as `file` lives.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// Only CoreML, on a Mac, uses the cache, so elsewhere there are no turns to
/// take.
#[cfg(not(unix))]
fn take_turn(_: &File) -> io::Result<()> {
    Ok(())
}

/// Deletes everything in `dir` but the lock file and the source record.
fn empty(dir: &Path) -> io::Result<()> {
    for item in fs::read_dir(dir)? {
        let item = item?;
        let name = item.file_name();
        if name == LOCK || name == SOURCE {
            continue;
        }
        if item.file_type()?.is_dir() {
            fs::remove_dir_all(item.path())?;
        } else {
            fs::remove_file(item.path())?;
        }
    }
    Ok(())
}

/// Removes the folders the CoreML provider left in `temp` for processes that
/// have since ended, and returns how many it removed. Every earlier version
/// of the app left some there.
///
/// Only what is named as the provider names it, and only for a process that
/// is gone: one still running may be using its folders.
#[cfg(unix)]
pub fn sweep_leftovers(temp: &Path) -> usize {
    let Ok(items) = fs::read_dir(temp) else {
        return 0;
    };
    let mut removed = 0;
    for item in items.flatten() {
        let Some(pid) = item.file_name().to_str().and_then(leftover_pid) else {
            continue;
        };
        if running(pid) {
            continue;
        }
        let path = item.path();
        let gone = if path.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        removed += usize::from(gone.is_ok());
    }
    removed
}

/// The id of the process that made the provider's temp item `name`:
/// `onnxruntime-`, then `NSProcessInfo.globallyUniqueString` (a UUID, the
/// process id and a hex counter), then `.mlmodelc` for the compiled half.
pub fn leftover_pid(name: &str) -> Option<u32> {
    let unique = name.strip_prefix("onnxruntime-")?.split('.').next()?;
    let parts: Vec<&str> = unique.split('-').collect();
    let hex = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit());
    let uuid = parts.len() == 7
        && parts[..5]
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(part, len)| part.len() == len && hex(part));
    if !uuid || !hex(parts[6]) {
        return None;
    }
    parts[5].parse().ok().filter(|&pid| pid > 0)
}

/// Whether process `pid` still exists. Anything short of "no such process"
/// counts as running, so a folder is only ever removed for certain.
#[cfg(unix)]
fn running(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return true;
    };
    // SAFETY: signal 0 is never delivered; `kill` only checks that the
    // process exists. `leftover_pid` never returns 0, which would mean the
    // whole process group.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Scratch;

    const RUNTIME: &str = "ORT Build Info: git-branch=rel-1.24.4";

    fn model(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    /// Stands in for what the provider writes into the folder.
    fn compile_into(entry: &Entry) -> PathBuf {
        let compiled = entry.dir().join("123/0_dynamic_mlprogram/model");
        fs::create_dir_all(&compiled).unwrap();
        fs::write(compiled.join("weight.bin"), b"weights").unwrap();
        compiled
    }

    #[test]
    fn a_kept_compile_is_there_for_the_next_load() {
        let dir = Scratch::new("coreml-keep");
        let yunet = model(&dir, "yunet.onnx", b"graph");
        let root = dir.join("cache");

        let first = Entry::open(&root, &yunet, RUNTIME).unwrap();
        assert!(!first.is_ready());
        let compiled = compile_into(&first);
        first.keep().unwrap();

        let second = Entry::open(&root, &yunet, RUNTIME).unwrap();
        assert!(second.is_ready());
        assert!(compiled.join("weight.bin").is_file());
        assert_eq!(second.dir(), entry_dir(&root, &yunet, RUNTIME).unwrap());
    }

    /// A process that dies mid-compile leaves a package ONNX Runtime would
    /// take for a whole one; the next load must start again instead.
    #[test]
    fn an_unfinished_compile_is_thrown_away() {
        let dir = Scratch::new("coreml-unfinished");
        let yunet = model(&dir, "yunet.onnx", b"graph");
        let root = dir.join("cache");

        let crashed = Entry::open(&root, &yunet, RUNTIME).unwrap();
        let compiled = compile_into(&crashed);
        drop(crashed);

        let next = Entry::open(&root, &yunet, RUNTIME).unwrap();
        assert!(!next.is_ready());
        assert!(!compiled.exists());
    }

    #[test]
    fn a_failed_compile_is_not_kept() {
        let dir = Scratch::new("coreml-failed");
        let mesh = model(&dir, "mesh.onnx", b"graph");
        let root = dir.join("cache");

        let entry = Entry::open(&root, &mesh, RUNTIME).unwrap();
        let compiled = compile_into(&entry);
        let folder = entry.dir().to_path_buf();
        entry.discard();
        assert!(!compiled.exists());
        assert!(!folder.join(READY).exists());
    }

    /// ONNX Runtime keys its cache on the model's path alone, so an updated
    /// model at the same path must not be handed the old one's compile.
    #[test]
    fn a_changed_model_gets_a_new_folder_and_the_old_one_goes() {
        let dir = Scratch::new("coreml-changed");
        let face = model(&dir, "auraface.onnx", b"version one");
        let root = dir.join("cache");

        let old = Entry::open(&root, &face, RUNTIME).unwrap();
        let old_dir = old.dir().to_path_buf();
        old.keep().unwrap();

        fs::write(&face, b"version two, longer").unwrap();
        let new = Entry::open(&root, &face, RUNTIME).unwrap();
        assert_ne!(new.dir(), old_dir);
        assert!(!new.is_ready());
        assert!(!old_dir.exists(), "the stale compile is still there");
    }

    #[test]
    fn a_new_runtime_compiles_again() {
        let dir = Scratch::new("coreml-runtime");
        let yunet = model(&dir, "yunet.onnx", b"graph");
        let root = dir.join("cache");

        Entry::open(&root, &yunet, RUNTIME).unwrap().keep().unwrap();
        let upgraded = Entry::open(&root, &yunet, "ORT Build Info: git-branch=rel-1.25.0").unwrap();
        assert!(!upgraded.is_ready());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    }

    #[test]
    fn a_deleted_models_compile_goes_when_another_loads() {
        let dir = Scratch::new("coreml-deleted");
        let (sam, yunet) = (
            model(&dir, "sam2.onnx", b"a"),
            model(&dir, "yunet.onnx", b"b"),
        );
        let root = dir.join("cache");

        let sam_entry = Entry::open(&root, &sam, RUNTIME).unwrap();
        let sam_dir = sam_entry.dir().to_path_buf();
        sam_entry.keep().unwrap();
        Entry::open(&root, &yunet, RUNTIME).unwrap().keep().unwrap();
        assert!(sam_dir.exists());

        fs::remove_file(&sam).unwrap();
        Entry::open(&root, &yunet, RUNTIME).unwrap();
        assert!(!sam_dir.exists());
    }

    /// A model that no longer runs on CoreML has its compile removed, and the
    /// others keep theirs.
    #[test]
    fn a_model_off_coreml_loses_its_compile() {
        let dir = Scratch::new("coreml-forget");
        let (encoder, yunet) = (
            model(&dir, "encoder.onnx", b"a"),
            model(&dir, "yunet.onnx", b"b"),
        );
        let root = dir.join("cache");
        let kept = Entry::open(&root, &encoder, RUNTIME).unwrap();
        let encoder_dir = kept.dir().to_path_buf();
        kept.keep().unwrap();
        let face = Entry::open(&root, &yunet, RUNTIME).unwrap();
        let yunet_dir = face.dir().to_path_buf();
        face.keep().unwrap();

        assert_eq!(
            forget(&root, &[encoder.clone(), dir.join("absent.onnx")]),
            1
        );
        assert!(!encoder_dir.exists());
        assert!(yunet_dir.exists());
        assert_eq!(forget(&root, &[encoder]), 0);
        assert_eq!(forget(&dir.join("no cache"), &[yunet]), 0);
    }

    #[test]
    fn the_providers_temp_names_carry_their_process() {
        let package = "onnxruntime-0C758B5E-093B-473C-A091-D4D4D42C581A-8158-0000023A24763298";
        assert_eq!(leftover_pid(package), Some(8158));
        assert_eq!(leftover_pid(&format!("{package}.mlmodelc")), Some(8158));
        assert_eq!(
            leftover_pid(&format!("{package}.model.mlmodel")),
            Some(8158)
        );
        for other in [
            "onnxruntime-cache",
            "onnxruntime-0C758B5E-093B-473C-A091-D4D4D42C581A-0-0000023A24763298",
            "onnxruntime-0C758B5E-093B-473C-A091-D4D4D42C581A-8158",
            "sourcelight-coreml",
            "0C758B5E-093B-473C-A091-D4D4D42C581A-8158-0000023A24763298",
        ] {
            assert_eq!(leftover_pid(other), None, "{other}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_sweep_removes_only_what_ended_processes_left() {
        let temp = Scratch::new("coreml-sweep");
        let ended = {
            let mut child = std::process::Command::new("true").spawn().unwrap();
            let pid = child.id();
            child.wait().unwrap();
            pid
        };
        let name = |pid: u32| {
            format!("onnxruntime-0C758B5E-093B-473C-A091-D4D4D42C581A-{pid}-0000023A24763298")
        };
        let left = temp.join(name(ended));
        let compiled = temp.join(format!("{}.mlmodelc", name(ended)));
        let live = temp.join(name(std::process::id()));
        let unrelated = temp.join("onnxruntime-cache");
        for folder in [&left, &compiled, &live, &unrelated] {
            fs::create_dir_all(folder.join("Data")).unwrap();
        }

        assert_eq!(sweep_leftovers(&temp), 2);
        assert!(!left.exists() && !compiled.exists());
        assert!(live.exists(), "removed a running process's folder");
        assert!(unrelated.exists());
    }
}
