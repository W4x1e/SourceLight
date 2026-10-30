//! Cancelling long jobs by name.
//!
//! A 6 GB model pull started on a metered connection, or by a mis-click, used to
//! be unstoppable short of quitting the app. Each long job now registers under
//! the same id the interface already tracks its progress by — `model:clip`,
//! `vision-runtime`, `vlm-model`, `index`, `faces`, `people` — so wherever the
//! Activity panel shows a progress bar it can offer Cancel, through one command.
//!
//! Cancelling is cooperative: the job checks its token between chunks, photos
//! or rows of a clustering pass, cleans up after itself (a download removes its `.part` file), and
//! returns [`Cancelled`] — which the interface reports as a choice the user
//! made, not as a failure.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::sync::lock_recover;

/// The error a cancelled job returns. Its text is what the interface matches
/// to tell a cancel from a failure — see `isCancelled` in `src/lib/jobs.ts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

/// What [`Cancelled`] reads as, and what the interface looks for.
pub const CANCELLED: &str = "Cancelled";

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(CANCELLED)
    }
}

impl std::error::Error for Cancelled {}

type Registry = Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>;

/// The running jobs that can be cancelled, by id. Cheap to clone; every clone
/// is the same registry.
#[derive(Clone, Default)]
pub struct Jobs {
    running: Registry,
}

impl Jobs {
    /// Registers a job and returns its token. The job is deregistered when the
    /// token drops, however the job ends.
    ///
    /// Both ends go in the session log (`session_log`), so the jobs that were
    /// running when the app stopped can be read back.
    pub fn start(&self, id: &str) -> CancelToken {
        let flag = Arc::new(AtomicBool::new(false));
        lock_recover(&self.running).insert(id.to_string(), Arc::clone(&flag));
        eprintln!("job started: {id}");
        CancelToken {
            id: id.to_string(),
            flag,
            running: Arc::clone(&self.running),
            started: std::time::Instant::now(),
        }
    }

    /// Whether a job is running under `id` now.
    pub fn is_running(&self, id: &str) -> bool {
        lock_recover(&self.running).contains_key(id)
    }

    /// Asks the job running under `id` to stop. False if nothing is — it may
    /// have finished a moment ago, which is not an error.
    pub fn cancel(&self, id: &str) -> bool {
        match lock_recover(&self.running).get(id) {
            Some(flag) => {
                flag.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }
}

/// A running job's side of the cancel: check it between units of work.
pub struct CancelToken {
    id: String,
    flag: Arc<AtomicBool>,
    running: Registry,
    started: std::time::Instant,
}

impl CancelToken {
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    /// `Err(Cancelled)` once cancelled — for `?` between units of work.
    pub fn check(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }
}

impl Drop for CancelToken {
    fn drop(&mut self) {
        eprintln!(
            "job ended: {} after {:.1} s{}",
            self.id,
            self.started.elapsed().as_secs_f32(),
            if self.is_cancelled() {
                " (cancelled)"
            } else {
                ""
            }
        );
        let mut running = lock_recover(&self.running);
        // Only if the entry is still this job's: a second job started under the
        // same id after this one was cancelled must keep its own registration.
        if running
            .get(&self.id)
            .is_some_and(|f| Arc::ptr_eq(f, &self.flag))
        {
            running.remove(&self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cancel_reaches_the_running_job_and_only_it() {
        let jobs = Jobs::default();
        let download = jobs.start("model:clip");
        let index = jobs.start("index");

        assert!(jobs.cancel("model:clip"));
        assert!(download.is_cancelled());
        assert_eq!(download.check(), Err(Cancelled));
        assert!(!index.is_cancelled());
        assert!(index.check().is_ok());
    }

    #[test]
    fn a_finished_job_is_gone_and_cancelling_it_is_harmless() {
        let jobs = Jobs::default();
        drop(jobs.start("faces"));
        assert!(!jobs.cancel("faces"));
    }

    /// Cancel a job, start a fresh one under the same id before the first has
    /// wound down: the first job's cleanup must not unregister the second.
    #[test]
    fn a_restarted_job_keeps_its_registration_when_the_old_one_ends() {
        let jobs = Jobs::default();
        let old = jobs.start("vlm-model");
        jobs.cancel("vlm-model");
        let new = jobs.start("vlm-model");
        drop(old);
        assert!(
            !new.is_cancelled(),
            "the new pull must not inherit the cancel"
        );
        assert!(jobs.cancel("vlm-model"), "…and must still be cancellable");
        assert!(new.is_cancelled());
    }

    #[test]
    fn cancelled_reads_as_the_word_the_interface_looks_for() {
        assert_eq!(Cancelled.to_string(), CANCELLED);
    }

    /// A job is running while its token lives: what lets a second
    /// index run see the first and stand aside.
    #[test]
    fn a_job_runs_while_its_token_lives() {
        let jobs = Jobs::default();
        assert!(!jobs.is_running("index"));
        let token = jobs.start("index");
        assert!(jobs.is_running("index"));
        drop(token);
        assert!(!jobs.is_running("index"));
    }
}
