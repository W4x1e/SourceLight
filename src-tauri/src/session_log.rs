//! The session log: what the app was doing, on disk as it happens.
//!
//! A shipped build has no console, so every `eprintln!` in the app went
//! nowhere: which GPU an AI model ran on, a vision model that failed to load,
//! a photo that would not tag. And the crash log (`crash`) only sees a Rust
//! panic. When the whole PC froze with the vision model and an AI mask on the
//! GPU at once, nothing at all was left behind.
//!
//! So stderr goes to `logs/session-<time>-<pid>.log`. Each line is stamped
//! with the time, and every burst of lines is forced to disk before the next
//! is read, so the last thing the app did survives a hard reset.
//!
//! A marker file names the running session's log, and a normal exit removes
//! it. Found at the next launch, it means that session never got that far:
//! its log is kept as `unclean-exit-<time>-<pid>.log`, and the interface says
//! so. Windows shutting down with the app open is a normal exit (tao runs the
//! exit path on `WM_ENDSESSION`).
//!
//! **Nothing here leaves the machine.**

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// The marker file: the running session's log name, removed on a clean exit.
const MARKER: &str = "running";
const SESSION: &str = "session-";
const UNCLEAN: &str = "unclean-exit-";
/// Session logs kept, this one included; unclean-exit logs are kept apart, as
/// many again, so a run of normal launches never pushes the one that matters out.
const KEEP: usize = 5;
/// A session log stops growing here. A loop printing the same error must not
/// fill the disk; its first 20 MB say everything the rest would.
const MAX_BYTES: u64 = 20 * 1024 * 1024;

/// This session's log, and the previous session's if it ended uncleanly.
#[derive(Debug)]
pub struct Session {
    pub log: PathBuf,
    pub unclean: Option<PathBuf>,
}

/// Starts this session's log in `dir`. `stamp` names it (the local time, with
/// the process id so two launches in one second cannot share a file). If the
/// marker is still there, the session it names never exited: its log is
/// renamed to keep it and returned as `unclean`. Old logs are pruned.
pub fn begin(dir: &Path, stamp: &str) -> io::Result<Session> {
    fs::create_dir_all(dir)?;
    let marker = dir.join(MARKER);
    let unclean = fs::read_to_string(&marker).ok().and_then(|name| {
        let name = name.trim();
        // Only ever a session log in this folder, whatever the file says.
        if !is_log(name, SESSION) {
            return None;
        }
        let kept = dir.join(name.replacen(SESSION, UNCLEAN, 1));
        fs::rename(dir.join(name), &kept).ok().map(|_| kept)
    });
    let name = format!("{SESSION}{stamp}.log");
    let log = dir.join(&name);
    File::create(&log)?;
    fs::write(&marker, &name)?;
    prune(dir, SESSION, KEEP);
    prune(dir, UNCLEAN, KEEP);
    Ok(Session { log, unclean })
}

/// Removes the marker on a clean exit, if it is still this session's: a
/// restarted app may already have written its own.
pub fn finish(dir: &Path, log: &Path) {
    let marker = dir.join(MARKER);
    let ours = log
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| fs::read_to_string(&marker).is_ok_and(|m| m.trim() == n));
    if ours {
        let _ = fs::remove_file(marker);
    }
}

fn is_log(name: &str, prefix: &str) -> bool {
    name.starts_with(prefix) && name.ends_with(".log") && !name.contains(['/', '\\'])
}

/// Deletes all but the newest `keep` logs starting with `prefix`. The names
/// start with the time, so name order is age order.
fn prune(dir: &Path, prefix: &str, keep: usize) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| is_log(n, prefix))
        .collect();
    names.sort_unstable_by(|a, b| b.cmp(a));
    for old in names.iter().skip(keep) {
        let _ = fs::remove_file(dir.join(old));
    }
}

/// Sends this process's stderr to `log`, stamped a line at a time. Where
/// there was a stderr before (a terminal, in development), it still gets
/// every line too.
pub fn capture_stderr(log: &Path) -> io::Result<()> {
    let file = OpenOptions::new().append(true).create(true).open(log)?;
    let console = original_stderr();
    let (reader, writer) = io::pipe()?;
    redirect(writer)?;
    std::thread::Builder::new()
        .name("session-log".into())
        .spawn(move || pump(reader, file, console))?;
    Ok(())
}

/// Copies lines from `from` to `file` with the time in front, forcing them to
/// disk whenever no more are waiting. Never returns while the pipe is open:
/// were it to stop reading, the pipe would fill and every `eprintln!` in the
/// app would block.
fn pump(from: impl io::Read, mut file: File, mut console: Option<File>) {
    let mut lines = BufReader::new(from);
    let mut line = Vec::new();
    let mut written = file.metadata().map(|m| m.len()).unwrap_or(0);
    loop {
        line.clear();
        match lines.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
        if let Some(c) = console.as_mut() {
            let _ = c.write_all(&line);
        }
        if written >= MAX_BYTES {
            continue;
        }
        let stamped = stamp_line(
            &chrono::Local::now().format("%H:%M:%S%.3f").to_string(),
            &line,
        );
        written += stamped.len() as u64;
        let _ = file.write_all(&stamped);
        if written >= MAX_BYTES {
            let _ = file.write_all(b"(log full: nothing more is written this session)\n");
        }
        // The end of a burst: on disk now, not whenever Windows gets to it.
        // A burst of many lines costs one flush, a single line one each.
        if lines.buffer().is_empty() {
            let _ = file.sync_data();
        }
    }
    // Keep the read end open: closing it would turn every later `eprintln!`
    // into a write to a broken pipe, which panics.
    std::mem::forget(lines);
}

/// `line` with `time` in front, ending in exactly one newline.
fn stamp_line(time: &str, line: &[u8]) -> Vec<u8> {
    let body = line.strip_suffix(b"\n").unwrap_or(line);
    let body = body.strip_suffix(b"\r").unwrap_or(body);
    let mut out = Vec::with_capacity(time.len() + body.len() + 2);
    out.extend_from_slice(time.as_bytes());
    out.push(b' ');
    out.extend_from_slice(body);
    out.push(b'\n');
    out
}

/// A copy of the stderr the process started with, if it had one. A Windows
/// app with no console has none, and the copy fails.
fn original_stderr() -> Option<File> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsHandle;
        io::stderr()
            .as_handle()
            .try_clone_to_owned()
            .ok()
            .map(File::from)
    }
    #[cfg(unix)]
    {
        use std::os::fd::AsFd;
        io::stderr()
            .as_fd()
            .try_clone_to_owned()
            .ok()
            .map(File::from)
    }
}

/// Makes `writer` the process's stderr. On Windows Rust looks the handle up on
/// every write, so `eprintln!` follows it; on Unix it takes over fd 2, so
/// native libraries' own output comes along too.
#[cfg(windows)]
fn redirect(writer: io::PipeWriter) -> io::Result<()> {
    use std::os::windows::io::IntoRawHandle;
    use windows_sys::Win32::System::Console::{SetStdHandle, STD_ERROR_HANDLE};
    let handle = writer.into_raw_handle();
    // SAFETY: the handle is a pipe this process owns and never closes.
    if unsafe { SetStdHandle(STD_ERROR_HANDLE, handle) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn redirect(writer: io::PipeWriter) -> io::Result<()> {
    use std::os::fd::IntoRawFd;
    let fd = writer.into_raw_fd();
    // SAFETY: both are open descriptors this process owns.
    if unsafe { libc::dup2(fd, 2) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `fd` is ours, now copied to stderr, and not used again.
    unsafe { libc::close(fd) };
    Ok(())
}

/// Where `llama-server` writes: one file, appended to by each start, moved
/// aside to `.old.log` once past 10 MB. The server's own account of what it
/// loaded where, which the app's log only summarises.
pub fn server_log(dir: &Path, header: &str) -> io::Result<File> {
    fs::create_dir_all(dir)?;
    let path = dir.join("llama-server.log");
    if fs::metadata(&path).is_ok_and(|m| m.len() > 10 * 1024 * 1024) {
        let _ = fs::rename(&path, dir.join("llama-server.old.log"));
    }
    let mut file = OpenOptions::new().append(true).create(true).open(&path)?;
    writeln!(
        file,
        "\n--- {} {header}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    )?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(dir: &Path) -> Vec<String> {
        let mut n: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        n.sort();
        n
    }

    #[test]
    fn a_clean_exit_leaves_nothing_to_report() {
        let scratch = crate::test_support::Scratch::new("session-clean");
        let dir = scratch.join("logs");
        let first = begin(&dir, "2026-10-02T14-00-00-1").unwrap();
        assert!(first.unclean.is_none());
        finish(&dir, &first.log);
        let second = begin(&dir, "2026-10-02T15-00-00-2").unwrap();
        assert!(second.unclean.is_none());
        assert!(dir.join(MARKER).exists(), "the running session is marked");
    }

    #[test]
    fn a_session_that_never_exited_keeps_its_log() {
        let scratch = crate::test_support::Scratch::new("session-unclean");
        let dir = scratch.join("logs");
        let first = begin(&dir, "2026-10-02T14-00-00-1").unwrap();
        fs::write(&first.log, "14:28:21.000 vision: loaded\n").unwrap();
        // No finish: the PC froze.
        let second = begin(&dir, "2026-10-02T14-31-00-2").unwrap();
        let kept = second.unclean.expect("the frozen session is reported");
        assert_eq!(
            kept.file_name().unwrap(),
            "unclean-exit-2026-10-02T14-00-00-1.log"
        );
        assert!(fs::read_to_string(kept).unwrap().contains("vision: loaded"));
        assert!(!first.log.exists());
    }

    #[test]
    fn a_restarted_app_keeps_its_own_marker() {
        let scratch = crate::test_support::Scratch::new("session-restart");
        let dir = scratch.join("logs");
        let old = begin(&dir, "2026-10-02T14-00-00-1").unwrap();
        let new = begin(&dir, "2026-10-02T14-00-00-2").unwrap();
        // The old process exits after the new one started.
        finish(&dir, &old.log);
        assert_eq!(
            fs::read_to_string(dir.join(MARKER)).unwrap(),
            new.log.file_name().unwrap().to_str().unwrap()
        );
    }

    #[test]
    fn a_marker_naming_anything_else_is_ignored() {
        let scratch = crate::test_support::Scratch::new("session-marker");
        let dir = scratch.join("logs");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("library.db"), "x").unwrap();
        fs::write(dir.join(MARKER), "..\\library.db").unwrap();
        assert!(begin(&dir, "t-1").unwrap().unclean.is_none());
        fs::write(dir.join(MARKER), "library.db").unwrap();
        assert!(begin(&dir, "t-2").unwrap().unclean.is_none());
        assert!(dir.join("library.db").exists());
    }

    #[test]
    fn old_logs_are_pruned_and_unclean_ones_kept_apart() {
        let scratch = crate::test_support::Scratch::new("session-prune");
        let dir = scratch.join("logs");
        fs::create_dir_all(&dir).unwrap();
        for i in 0..3 {
            fs::write(dir.join(format!("unclean-exit-2026-01-0{i}.log")), "").unwrap();
        }
        fs::write(dir.join("crash-2026-01-01.log"), "").unwrap();
        for i in 0..9 {
            let s = begin(&dir, &format!("2026-10-0{i}")).unwrap();
            finish(&dir, &s.log);
        }
        let all = names(&dir);
        let sessions: Vec<_> = all.iter().filter(|n| n.starts_with(SESSION)).collect();
        assert_eq!(sessions.len(), KEEP);
        assert_eq!(sessions[0], "session-2026-10-04.log", "the oldest went");
        assert_eq!(all.iter().filter(|n| n.starts_with(UNCLEAN)).count(), 3);
        assert!(all.contains(&"crash-2026-01-01.log".to_string()));
    }

    #[test]
    fn lines_are_stamped_once_whatever_they_end_with() {
        assert_eq!(stamp_line("12:00:00.000", b"a\r\n"), b"12:00:00.000 a\n");
        assert_eq!(stamp_line("t", b"b\n"), b"t b\n");
        assert_eq!(stamp_line("t", b"c"), b"t c\n");
    }

    #[test]
    fn the_pump_stamps_every_line_and_stops_at_the_cap() {
        let scratch = crate::test_support::Scratch::new("session-pump");
        let path = scratch.join("s.log");
        let file = File::create(&path).unwrap();
        pump(&b"one\ntwo\nthree"[..], file, None);
        let text = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].ends_with(" one") && lines[2].ends_with(" three"));

        let big = scratch.join("big.log");
        let line = vec![b'x'; 1023];
        let mut input = Vec::new();
        for _ in 0..(MAX_BYTES / 1024 + 50) {
            input.extend_from_slice(&line);
            input.push(b'\n');
        }
        pump(&input[..], File::create(&big).unwrap(), None);
        let size = fs::metadata(&big).unwrap().len();
        assert!(size < MAX_BYTES + 4096, "{size}");
        assert!(fs::read_to_string(&big)
            .unwrap()
            .ends_with("this session)\n"));
    }

    #[test]
    fn the_server_log_is_appended_to_and_moved_aside_when_large() {
        let scratch = crate::test_support::Scratch::new("session-server");
        let dir = scratch.join("logs");
        drop(server_log(&dir, "first").unwrap());
        drop(server_log(&dir, "second").unwrap());
        let text = fs::read_to_string(dir.join("llama-server.log")).unwrap();
        assert!(text.contains("first") && text.contains("second"));
        fs::write(
            dir.join("llama-server.log"),
            vec![b'x'; 10 * 1024 * 1024 + 1],
        )
        .unwrap();
        drop(server_log(&dir, "third").unwrap());
        assert!(dir.join("llama-server.old.log").exists());
        assert!(fs::read_to_string(dir.join("llama-server.log"))
            .unwrap()
            .contains("third"));
    }
}
