//! What a test does when the thing it needs is not in the repository.
//!
//! Real raw files, a copy of a real catalogue, downloaded AI models and a
//! hardware GPU are all things a checkout does not contain, so the tests that
//! need them skip. Before this module each one did so with a bare `eprintln!`
//! and `return` — which reports **green**. A lost fixture, a mistyped path or a
//! CI job that stopped setting a variable all looked exactly like a passing
//! test, and 31 of 53 integration tests were quietly doing it.
//!
//! Three rules, all here so there is one copy of them:
//!
//! - **A run can declare what it has.** `SOURCELIGHT_REQUIRE=raw,gpu` (or
//!   `all`) turns a skip of that kind into a failure. The CI job that fetches
//!   the raw fixtures and `scripts/check.sh --raw` both declare `raw`, so if
//!   the fixtures go missing there, the run goes red instead of quietly
//!   shrinking. `SOURCELIGHT_REQUIRE_GPU` is kept as the older spelling of
//!   `gpu`.
//! - **A misconfiguration is never a skip.** A fixture variable that is set to a
//!   path that does not exist fails immediately, required or not — that is a
//!   typo, not a machine that lacks the data.
//! - **Every skip is counted.** With `SOURCELIGHT_SKIP_LOG` set, each skip
//!   appends a line to that file, which is how `check.sh` and CI report how many
//!   tests actually ran rather than trusting the green total.
//!
//! **Public and not `#[cfg(test)]`, on purpose** — the integration tests in
//! `tests/` link this crate from outside and cannot see anything gated on
//! `cfg(test)`, and the unit tests need the same rules as they do.

use std::io::Write;
use std::path::PathBuf;

/// The kinds of thing a test can need that a checkout does not contain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Needs {
    /// Real raw files — `SOURCELIGHT_TEST_RAW` and friends, fetched by
    /// `scripts/fetch-raw-fixtures.sh`.
    Raw,
    /// A copy of a real user catalogue — `SOURCELIGHT_TEST_DB`.
    Db,
    /// Downloaded AI models, in the app's model directory.
    Model,
    /// A hardware GPU. A software rasteriser counts as none (see
    /// `gpu::test_support`).
    Gpu,
}

impl Needs {
    /// The name used in `SOURCELIGHT_REQUIRE` and the skip log.
    pub fn name(self) -> &'static str {
        match self {
            Needs::Raw => "raw",
            Needs::Db => "db",
            Needs::Model => "model",
            Needs::Gpu => "gpu",
        }
    }

    /// Whether this run has declared it has this, so that a skip is a failure.
    pub fn required(self) -> bool {
        self.required_by(
            &std::env::var("SOURCELIGHT_REQUIRE").unwrap_or_default(),
            std::env::var_os("SOURCELIGHT_REQUIRE_GPU").is_some(),
        )
    }

    /// The rule behind [`Needs::required`], over its inputs rather than the
    /// environment — so it can be tested without mutating process-wide state
    /// that tests on other threads are reading.
    fn required_by(self, declared: &str, legacy_gpu: bool) -> bool {
        let listed = declared
            .split(',')
            .map(str::trim)
            .any(|d| d.eq_ignore_ascii_case("all") || d.eq_ignore_ascii_case(self.name()));
        listed || (self == Needs::Gpu && legacy_gpu)
    }
}

/// Records that the current test cannot run for want of `needs`, and says why.
///
/// **Panics instead** when the run has declared it has `needs` — see
/// [`Needs::required`]. Call it, then `return`.
pub fn skip(needs: Needs, reason: &str) {
    let log = std::env::var_os("SOURCELIGHT_SKIP_LOG");
    skip_as(needs.required(), log.as_deref(), needs, reason);
}

/// [`skip`] over its inputs, so this module's own tests can exercise it without
/// writing phantom skips into the log a real run is counting.
fn skip_as(required: bool, log: Option<&std::ffi::OsStr>, needs: Needs, reason: &str) {
    let test = std::thread::current()
        .name()
        .unwrap_or("<unnamed>")
        .to_string();
    if required {
        panic!(
            "{test} could not run: {reason}. This run declared it has `{}` \
             (SOURCELIGHT_REQUIRE), so a skip is a failure rather than a pass.",
            needs.name()
        );
    }
    eprintln!("skipped ({}): {reason}", needs.name());
    if let Some(log) = log {
        // One `write_all` per line, in append mode, so tests skipping on
        // parallel threads do not interleave within a line.
        let line = format!("{}\t{test}\t{reason}\n", needs.name());
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .and_then(|mut f| f.write_all(line.as_bytes()));
    }
}

/// The path in a fixture environment variable, if one is configured.
///
/// Unset or empty is `None` — the machine does not have this fixture, which the
/// caller reports with [`skip`]. Set to something that does not exist **panics**,
/// required or not: that is a mistyped path or a deleted folder, and treating it
/// as a skip is exactly how these tests used to vanish without anyone noticing.
pub fn fixture_path(var: &str) -> Option<PathBuf> {
    let value = std::env::var_os(var)?;
    if value.is_empty() {
        return None;
    }
    let path = PathBuf::from(value);
    assert!(
        path.exists(),
        "{var} is set to {}, which does not exist — fix or unset it; \
         a misconfigured fixture is not a reason to skip",
        path.display()
    );
    Some(path)
}

/// The raw-fixture folder (`SOURCELIGHT_TEST_RAW`), if configured.
pub fn raw_dir() -> Option<PathBuf> {
    fixture_path("SOURCELIGHT_TEST_RAW")
}

/// Raw files in `dir` whose extension satisfies `wanted` (given lower-cased),
/// sorted so the choice of sample is stable from run to run.
pub fn files_in(dir: &std::path::Path, wanted: impl Fn(&str) -> bool) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .filter(|p| {
                    p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| wanted(&e.to_ascii_lowercase()))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// A scratch directory under the system temp that deletes itself when dropped.
///
/// Before this, every test module rolled its own `temp_dir().join(..)` and
/// never removed it; keyed on the pid, each `cargo test` left a fresh set
/// behind, and one machine had gathered 31 GB of them. `std` rather than the
/// `tempfile` crate: this is all the tests need, and every dependency is
/// another licence to clear for everyone who redistributes the app.
///
/// The name carries the pid, the thread and a per-process counter, so tests
/// on parallel threads — or two scratches in one test — never share a folder.
/// It dereferences to [`Path`](std::path::Path), so `dir.join(..)` and
/// `&dir` work as they did on a `PathBuf`. Keep it bound for as long as the
/// folder is used, and drop anything holding a file open inside it (a SQLite
/// connection, say) first — Windows will not delete an open file.
#[derive(Debug)]
pub struct Scratch(PathBuf);

impl Scratch {
    /// A new, empty folder named `sourcelight-{tag}-…`.
    pub fn new(tag: &str) -> Scratch {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "sourcelight-{tag}-{}-{:?}-{n}",
            std::process::id(),
            std::thread::current().id()
        ));
        // Emptied first: a pid can be reused, and a run killed outright never
        // got to drop its scratch.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    pub fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl std::ops::Deref for Scratch {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl AsRef<std::path::Path> for Scratch {
    fn as_ref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_requires_only_what_it_declares() {
        assert!(!Needs::Raw.required_by("", false), "nothing declared");
        assert!(Needs::Raw.required_by("gpu, raw", false));
        assert!(Needs::Gpu.required_by("gpu, raw", false));
        assert!(!Needs::Model.required_by("gpu, raw", false));
        assert!(Needs::Db.required_by("ALL", false), "`all` is every kind");
        assert!(Needs::Gpu.required_by("", true), "the older GPU spelling");
        assert!(
            !Needs::Raw.required_by("", true),
            "…which means only the GPU"
        );
    }

    #[test]
    fn a_declared_skip_fails_and_an_undeclared_one_does_not() {
        // A private log, never the real run's: an entry there would be a
        // phantom skip in the count this module exists to keep honest.
        let dir = Scratch::new("skip-log");
        let log = dir.join("skips.tsv");
        skip_as(
            false,
            Some(log.as_os_str()),
            Needs::Raw,
            "an undeclared skip",
        );
        let written = std::fs::read_to_string(&log).unwrap();
        assert_eq!(written.lines().count(), 1, "one line per skip: {written:?}");
        assert!(
            written.starts_with("raw\t") && written.trim_end().ends_with("\tan undeclared skip"),
            "kind, test and reason, tab-separated: {written:?}"
        );

        let declared =
            std::panic::catch_unwind(|| skip_as(true, None, Needs::Raw, "fixtures missing"));
        assert!(declared.is_err(), "a declared skip must fail the test");
    }

    #[test]
    fn a_scratch_folder_is_its_own_and_is_gone_once_dropped() {
        let a = Scratch::new("probe");
        let b = Scratch::new("probe");
        assert_ne!(
            a.path(),
            b.path(),
            "same tag, same thread, still two folders"
        );
        std::fs::create_dir_all(a.join("nested")).unwrap();
        std::fs::write(a.join("nested/file"), b"x").unwrap();
        let path = a.to_path_buf();
        drop(a);
        assert!(
            !path.exists(),
            "dropping it removes the folder and its contents"
        );
        assert!(b.exists(), "…and only its own");
    }

    #[test]
    fn a_fixture_path_that_does_not_exist_is_a_misconfiguration() {
        // A variable nothing else reads, so setting it cannot disturb a test
        // running alongside this one.
        let var = "SOURCELIGHT_TEST_SUPPORT_PROBE";
        std::env::set_var(var, "");
        assert_eq!(fixture_path(var), None, "empty is the same as unset");
        std::env::set_var(var, "Z:/definitely/not/here");
        assert!(
            std::panic::catch_unwind(|| fixture_path(var)).is_err(),
            "a path that does not exist must fail, not skip"
        );
        std::env::remove_var(var);
    }
}
