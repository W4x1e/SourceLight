//! Writing files the user owns without ever leaving one half-written, or
//! replacing one by accident.
//!
//! `std::fs::write` opens with create-and-truncate, then writes: the old bytes
//! are gone before the write can fail. On a full disk, an unplugged drive or a
//! crash that left another editor's sidecar empty, its develop settings with it.
//! And "never overwrite" checked for a file and then wrote, so a file that
//! appeared in between — a sync client delivering a same-named frame — was
//! replaced. These three are the ways to write instead.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// `path` with `suffix` appended to its file name, in the same folder: a temp
/// file there renames over the real one in one step, which a temp file on
/// another drive could not.
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Held while [`write_atomic`] replaces a file. The temp name is fixed, so two
/// writers of one file at once — a sidecar written for one photo while a
/// batch reaches the same photo — would share it, one truncating it under the
/// other, and the rename could publish the mix.
static REPLACING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Replaces `path` with `bytes` all at once, or not at all: written to a temp
/// file beside it, flushed to the drive, then renamed over it. If anything
/// fails, the temp file goes and `path` is exactly as it was.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let _one_at_a_time = crate::sync::lock_recover(&REPLACING);
    let tmp = sibling(path, ".sourcelight-tmp");
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Writes `bytes` to `path`, which must not exist yet — refused when the file
/// is opened, not checked beforehand, so nothing that appears in between is
/// ever replaced. `ErrorKind::AlreadyExists` when something is there. A write
/// that fails part-way removes what it wrote.
pub fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    let result = file.write_all(bytes).and_then(|_| file.sync_all());
    if result.is_err() {
        drop(file);
        let _ = std::fs::remove_file(path);
    }
    result
}

/// Moves `from` to `to` only if nothing is at `to`, in one step: a file that
/// appeared at `to` since the name was chosen is never replaced, and the move
/// fails with `ErrorKind::AlreadyExists` instead.
pub fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};
        let wide = |p: &Path| -> Vec<u16> { p.as_os_str().encode_wide().chain([0]).collect() };
        let (from_w, to_w) = (wide(from), wide(to));
        // Without MOVEFILE_REPLACE_EXISTING, which std's rename passes, the
        // move fails if `to` exists. Write-through so it is on the drive when
        // this returns.
        // SAFETY: both are NUL-terminated wide strings that outlive the call.
        let ok = unsafe { MoveFileExW(from_w.as_ptr(), to_w.as_ptr(), MOVEFILE_WRITE_THROUGH) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = |p: &Path| std::ffi::CString::new(p.as_os_str().as_bytes());
        let (from_c, to_c) = (c(from)?, c(to)?);
        // SAFETY: both are NUL-terminated C strings that outlive the call.
        let r = unsafe { libc::renamex_np(from_c.as_ptr(), to_c.as_ptr(), libc::RENAME_EXCL) };
        if r != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        // A hard link fails if `to` exists; then the temp name goes. Where the
        // file system has no links (FAT), fall back to look-then-rename.
        match std::fs::hard_link(from, to) {
            Ok(()) => std::fs::remove_file(from),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(e),
            Err(_) if to.exists() => Err(io::ErrorKind::AlreadyExists.into()),
            Err(_) => std::fs::rename(from, to),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> crate::test_support::Scratch {
        crate::test_support::Scratch::new(&format!("fsutil-{name}"))
    }

    /// The point of the module: when the write cannot finish — here
    /// the temp file cannot even be made, as on a full disk — the file it
    /// was replacing is untouched, not emptied.
    #[test]
    fn a_failed_replace_leaves_the_original_intact() {
        let dir = scratch("atomic");
        let sidecar = dir.join("DSC0001.xmp");
        std::fs::write(&sidecar, "Lightroom's develop settings").unwrap();
        // A folder where the temp file has to go: creating it fails.
        std::fs::create_dir_all(dir.join("DSC0001.xmp.sourcelight-tmp")).unwrap();
        assert!(write_atomic(&sidecar, b"merged packet").is_err());
        assert_eq!(
            std::fs::read_to_string(&sidecar).unwrap(),
            "Lightroom's develop settings"
        );

        std::fs::remove_dir(dir.join("DSC0001.xmp.sourcelight-tmp")).unwrap();
        write_atomic(&sidecar, b"merged packet").unwrap();
        assert_eq!(std::fs::read(&sidecar).unwrap(), b"merged packet");
        assert!(!dir.join("DSC0001.xmp.sourcelight-tmp").exists());
    }

    #[test]
    fn a_new_file_never_replaces_one_that_is_there() {
        let dir = scratch("new");
        let out = dir.join("DSC0001.jpg");
        write_new(&out, b"first").unwrap();
        let e = write_new(&out, b"second").unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&out).unwrap(), b"first");
    }

    #[test]
    fn a_rename_never_replaces_what_is_there() {
        let dir = scratch("rename");
        let (tmp, dst) = (dir.join("a.part"), dir.join("a.ARW"));
        std::fs::write(&tmp, "copy").unwrap();
        std::fs::write(&dst, "a file that appeared meanwhile").unwrap();
        let e = rename_new(&tmp, &dst).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read_to_string(&dst).unwrap(),
            "a file that appeared meanwhile"
        );

        std::fs::remove_file(&dst).unwrap();
        rename_new(&tmp, &dst).unwrap();
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "copy");
        assert!(!tmp.exists());
    }
}
