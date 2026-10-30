//! A panic hook that leaves a crash log the user can find and send.
//!
//! One `catch_unwind` already guards Develop's frame handler, where a panic
//! once aborted the process (`commands/render.rs`). Everywhere else a panic unwinds to the
//! default hook, prints to a stderr no shipped user ever sees, and is gone — so
//! the first anyone hears of a crash is someone giving up on the app.
//! This writes a timestamped log instead: the panic, where it came from, a
//! backtrace, and the machine facts that turn "it crashed" into something
//! diagnosable.
//!
//! The frontend's errors go to the same folder ([`log_ui_error`]) — a render
//! failure an error boundary caught, or a script error nothing did — and
//! Settings → Advanced offers the folder and a copyable summary, so a user can
//! send what we need without being told where to look.
//!
//! **Nothing here leaves the machine.** These are files on the user's disk,
//! read by the user, sent (or not) by the user.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use crate::sync::lock_recover;

/// The body of a crash log. Pure, so it is tested without having to panic: the
/// hook feeds it the live details, a test feeds it fixed ones.
pub fn report(
    when: &str,
    location: &str,
    message: &str,
    diagnostics: &str,
    backtrace: &str,
) -> String {
    format!(
        "SourceLight crash report\n\
         time:  {when}\n\
         {diagnostics}\n\
         panic: {message}\n\
         at:    {location}\n\
         \n\
         backtrace:\n{backtrace}\n"
    )
}

/// The panic payload as text. Rust panics carry `&str` or `String`; anything else
/// is rare enough that naming its absence is more useful than trying to render it.
fn payload_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "(non-string panic payload)".to_string()
    }
}

/// The diagnostics a crash log carries, minus the GPU. Instant to gather —
/// version and OS are compile-time and cheap — so it is ready before the window
/// paints; the GPU line is appended by [`install`] once a background probe
/// finishes, because enumerating adapters is not something to make startup wait
/// on, nor to do while unwinding.
fn base_diagnostics() -> String {
    format!(
        "version: {}\nos:      {} {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}

/// Installs the panic hook. `log_dir` is where crash logs are written (created on
/// demand); the returned handle is the diagnostics string the hook reads, so a
/// caller can append the GPU line to it once known — see [`run`](crate::run).
///
/// Chains to the previous hook, so the panic still reaches stderr in a dev build.
/// Everything the hook does is best-effort: a panic *inside* the panic hook would
/// abort, so a failed write is swallowed, not unwrapped.
pub fn install(log_dir: PathBuf) -> Arc<Mutex<String>> {
    let diagnostics = Arc::new(Mutex::new(base_diagnostics()));
    let for_hook = Arc::clone(&diagnostics);
    let previous = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |info| {
        let now = chrono::Local::now();
        let when = now.format("%Y-%m-%d %H:%M:%S").to_string();
        let location = info
            .location()
            .map(|l| l.to_string())
            .unwrap_or_else(|| "unknown location".to_string());
        let message = payload_text(info.payload());
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        let diagnostics = lock_recover(&for_hook).clone();

        let text = report(&when, &location, &message, &diagnostics, &backtrace);
        let _ = std::fs::create_dir_all(&log_dir);
        let stamp = now.format("%Y-%m-%dT%H-%M-%S").to_string();
        let _ = std::fs::write(log_dir.join(format!("crash-{stamp}.log")), &text);

        previous(info);
    }));

    diagnostics
}

/// What the app knows about itself for a bug report, and where its logs go.
/// Managed as app state, so the commands and the panic hook read one copy.
pub struct Diagnostics {
    /// Version, OS and — once the background probe finishes — the GPU. The
    /// same string the panic hook writes, from [`install`].
    pub text: Arc<Mutex<String>>,
    pub log_dir: PathBuf,
    ui_logs_written: AtomicU32,
    /// This session's log (`session_log`), when it could be started.
    pub session_log: Option<PathBuf>,
    /// The log of a previous session that never exited, until the interface
    /// has said so: taken once, like the catalogue's recovery.
    pub unclean_exit: Mutex<Option<PathBuf>>,
}

impl Diagnostics {
    pub fn new(text: Arc<Mutex<String>>, log_dir: PathBuf) -> Self {
        Self {
            text,
            log_dir,
            ui_logs_written: AtomicU32::new(0),
            session_log: None,
            unclean_exit: Mutex::new(None),
        }
    }

    pub fn with_session(mut self, session: crate::session_log::Session) -> Self {
        self.session_log = Some(session.log);
        self.unclean_exit = Mutex::new(session.unclean);
        self
    }
}

/// The most frontend error logs one session writes. A render loop that throws
/// on every frame must not fill the disk with identical files; the first few
/// say everything the rest would.
const MAX_UI_LOGS: u32 = 20;

/// The body of a frontend error log. Pure, like [`report`].
pub fn ui_report(
    when: &str,
    surface: &str,
    message: &str,
    stack: &str,
    diagnostics: &str,
) -> String {
    format!(
        "SourceLight interface error\n\
         time:    {when}\n\
         {diagnostics}\n\
         where:   {surface}\n\
         error:   {message}\n\
         \n\
         stack:\n{stack}\n"
    )
}

/// Writes a frontend error beside the crash logs. Returns the file written, or
/// `None` once this session has written [`MAX_UI_LOGS`] of them.
pub fn log_ui_error(
    d: &Diagnostics,
    surface: &str,
    message: &str,
    stack: &str,
) -> std::io::Result<Option<PathBuf>> {
    let seq = d.ui_logs_written.fetch_add(1, Ordering::Relaxed);
    if seq >= MAX_UI_LOGS {
        return Ok(None);
    }
    let now = chrono::Local::now();
    let text = ui_report(
        &now.format("%Y-%m-%d %H:%M:%S").to_string(),
        surface,
        message,
        stack,
        &lock_recover(&d.text),
    );
    std::fs::create_dir_all(&d.log_dir)?;
    // The session's sequence number in the name, not just the time: two
    // boundaries catching the same error land in the same millisecond, and the
    // second must not overwrite the first (the cap test found exactly that).
    let path = d.log_dir.join(format!(
        "ui-error-{}-{seq:02}.log",
        now.format("%Y-%m-%dT%H-%M-%S")
    ));
    std::fs::write(&path, text)?;
    Ok(Some(path))
}

/// The text "Copy diagnostics" puts on the clipboard: the machine facts, where
/// the catalogue and the logs are, and which logs exist — enough for a first
/// reply to a bug report without a round of "can you tell me…".
pub fn summary(diagnostics: &str, db_path: &Path, log_dir: &Path, logs: &[String]) -> String {
    let logs = if logs.is_empty() {
        "  (none)".to_string()
    } else {
        logs.iter()
            .map(|l| format!("  {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "SourceLight diagnostics\n\
         {diagnostics}\n\
         catalogue: {}\n\
         logs:      {}\n\
         recent logs, newest first:\n{logs}\n",
        db_path.display(),
        log_dir.display()
    )
}

/// The newest log files in `log_dir`, newest first, at most `limit`.
pub fn recent_logs(log_dir: &Path, limit: usize) -> Vec<String> {
    let mut logs: Vec<(std::time::SystemTime, String)> = std::fs::read_dir(log_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "log"))
                .filter_map(|e| {
                    let modified = e.metadata().and_then(|m| m.modified()).ok()?;
                    Some((modified, e.file_name().to_string_lossy().into_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    logs.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    logs.into_iter().take(limit).map(|(_, name)| name).collect()
}

/// Opens `dir` in the system file manager.
///
/// `cfg!` in a single function rather than `#[cfg]` variants: every branch
/// compiles everywhere, and there is no platform code path to go stale unseen.
/// Explorer reports a non-zero exit status even when it succeeds, so only a
/// failure to launch it counts as an error.
pub fn open_folder(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let opener = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener).arg(dir).spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_interface_error_log_carries_every_field() {
        let text = ui_report(
            "2026-09-23 10:00:00",
            "Develop",
            "Cannot read properties of undefined",
            "at Develop (Develop.tsx:12)",
            "version: 0.1.0",
        );
        for part in [
            "Develop",
            "Cannot read properties",
            "Develop.tsx:12",
            "version: 0.1.0",
        ] {
            assert!(text.contains(part), "{part} missing from:\n{text}");
        }
    }

    #[test]
    fn interface_errors_stop_being_logged_after_the_cap() {
        let scratch = crate::test_support::Scratch::new("ui-logs");
        let dir = scratch.join("logs");
        let d = Diagnostics::new(Arc::new(Mutex::new("version: t".into())), dir.clone());
        let written = (0..MAX_UI_LOGS + 5)
            .filter(|_| log_ui_error(&d, "test", "boom", "").unwrap().is_some())
            .count();
        assert_eq!(written as u32, MAX_UI_LOGS);
        assert_eq!(recent_logs(&dir, 100).len() as u32, MAX_UI_LOGS);
    }

    #[test]
    fn the_summary_says_where_things_are_and_lists_the_logs() {
        let text = summary(
            "version: 0.1.0",
            Path::new("C:/data/library.db"),
            Path::new("C:/data/logs"),
            &["crash-2026.log".into()],
        );
        assert!(
            text.contains("library.db") && text.contains("logs") && text.contains("crash-2026.log")
        );
        assert!(summary("v", Path::new("a"), Path::new("b"), &[]).contains("(none)"));
    }

    #[test]
    fn a_report_carries_every_field() {
        let text = report(
            "2026-09-08 14:30:00",
            "src/develop/mod.rs:481",
            "index out of bounds",
            "version: 0.1.0\nos: windows x86_64",
            "0: foo\n1: bar",
        );
        assert!(text.contains("index out of bounds"));
        assert!(text.contains("src/develop/mod.rs:481"));
        assert!(text.contains("version: 0.1.0"));
        assert!(text.contains("0: foo\n1: bar"));
    }

    #[test]
    fn a_panic_payload_reads_as_text() {
        // The two forms a real panic produces.
        let s: &str = "boom";
        assert_eq!(payload_text(&s), "boom");
        let owned: String = "kaboom".to_string();
        assert_eq!(payload_text(&owned), "kaboom");
    }
}
