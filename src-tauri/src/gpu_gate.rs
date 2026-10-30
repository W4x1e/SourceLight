//! One kind of GPU work at a time: the vision model's server, or the app's
//! own models.
//!
//! The vision model runs in `llama-server`, a separate process using Vulkan
//! (Metal on a Mac). The app's own models (Select Subject, Click to Select,
//! AI noise reduction, faces, the search index) run in this process through
//! ONNX Runtime on DirectML (CoreML). Nothing kept the two apart, and on
//! 2026-10-02 a PC froze hard, with no driver reset and no dump, when Select
//! Subject ran while the 8B vision model was tagging on the same card.
//!
//! So the two take turns. The vision model's requests share the gate with
//! each other (its slots are meant to run together). The app's own model runs
//! take it alone, and one that is waiting stops new vision requests from
//! starting, so it goes as soon as the photos in flight finish: under a
//! second for tags. A tagging run simply pauses while a mask is made.
//!
//! A vision request that never ends must not hold a mask up for ever: after
//! [`MAX_WAIT`] the app's model goes ahead anyway, and says so in the session
//! log. When the vision model runs on the CPU there is nothing to keep apart,
//! and its requests do not take the gate at all.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::sync::lock_recover;

/// The longest the app's own model waits for the vision model's requests.
pub const MAX_WAIT: Duration = Duration::from_secs(30);

/// How often a waiting vision request looks again. Small against the 0.3 s
/// or more a request takes.
const VISION_POLL: Duration = Duration::from_millis(25);

#[derive(Default)]
struct State {
    /// Vision requests running now.
    vision: usize,
    /// The app's own model runs, running and waiting.
    local: usize,
    local_waiting: usize,
}

pub struct Gate {
    state: Mutex<State>,
    turned: Condvar,
    /// Whether the vision model is on a GPU; off, its requests pass freely.
    vision_on_gpu: AtomicBool,
}

/// The app's one gate.
pub static GATE: Gate = Gate::new();

impl Gate {
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(State {
                vision: 0,
                local: 0,
                local_waiting: 0,
            }),
            turned: Condvar::new(),
            vision_on_gpu: AtomicBool::new(true),
        }
    }

    /// Says where the vision model's server runs, each time it starts.
    pub fn set_vision_on_gpu(&self, on_gpu: bool) {
        self.vision_on_gpu.store(on_gpu, Ordering::Relaxed);
    }

    /// Waits until no model of the app's own is running or waiting, then
    /// holds the gate for one vision request (or the model's load).
    pub async fn vision(&self) -> VisionTurn<'_> {
        if !self.vision_on_gpu.load(Ordering::Relaxed) {
            return VisionTurn { gate: None };
        }
        loop {
            {
                let mut s = lock_recover(&self.state);
                if s.local == 0 && s.local_waiting == 0 {
                    s.vision += 1;
                    drop(s);
                    // A kept model holds its memory between runs, and the two
                    // filling one card is what froze the PC, not just running
                    // at once. None of the app's runs is under way here.
                    #[cfg(feature = "ai")]
                    crate::develop::ai::forget_subject_model();
                    return VisionTurn { gate: Some(self) };
                }
            }
            tokio::time::sleep(VISION_POLL).await;
        }
    }

    /// Whether a run of the app's own model would wait for a vision request
    /// now, so whoever asked for it can be told why nothing is happening.
    pub fn vision_busy(&self) -> bool {
        lock_recover(&self.state).vision > 0
    }

    /// Waits until no vision request is running (at most `MAX_WAIT`), then
    /// holds the gate for one run of the app's own model. Blocks the thread:
    /// call it where the model itself is about to block it.
    pub fn local(&self, what: &str) -> LocalTurn<'_> {
        self.local_within(what, MAX_WAIT)
    }

    fn local_within(&self, what: &str, max: Duration) -> LocalTurn<'_> {
        let started = Instant::now();
        let mut s = lock_recover(&self.state);
        s.local_waiting += 1;
        while s.vision > 0 {
            let Some(left) = max.checked_sub(started.elapsed()).filter(|d| !d.is_zero()) else {
                eprintln!(
                    "gpu: {what} went ahead after {:.0} s with the vision model still busy",
                    max.as_secs_f32()
                );
                break;
            };
            s = self
                .turned
                .wait_timeout(s, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        s.local_waiting -= 1;
        s.local += 1;
        drop(s);
        let waited = started.elapsed();
        if waited >= Duration::from_millis(100) {
            eprintln!(
                "gpu: {what} waited {:.1} s for the vision model",
                waited.as_secs_f32()
            );
        }
        LocalTurn { gate: self }
    }
}

impl Default for Gate {
    fn default() -> Self {
        Self::new()
    }
}

/// A vision request's hold on the gate, released on drop.
pub struct VisionTurn<'a> {
    gate: Option<&'a Gate>,
}

impl Drop for VisionTurn<'_> {
    fn drop(&mut self) {
        if let Some(gate) = self.gate {
            lock_recover(&gate.state).vision -= 1;
            gate.turned.notify_all();
        }
    }
}

/// A run of the app's own model holding the gate, released on drop.
pub struct LocalTurn<'a> {
    gate: &'a Gate,
}

impl Drop for LocalTurn<'_> {
    fn drop(&mut self) {
        lock_recover(&self.gate.state).local -= 1;
        self.gate.turned.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
    }

    fn leak(gate: Gate) -> &'static Gate {
        Box::leak(Box::new(gate))
    }

    #[test]
    fn a_local_model_waits_for_the_vision_request_in_flight() {
        let gate = leak(Gate::new());
        let rt = runtime();
        let turn = rt.block_on(gate.vision());
        let started = Instant::now();
        let waiter = std::thread::spawn(move || {
            let _t = gate.local("test");
            started.elapsed()
        });
        std::thread::sleep(Duration::from_millis(150));
        drop(turn);
        let waited = waiter.join().unwrap();
        assert!(waited >= Duration::from_millis(140), "{waited:?}");
        assert!(waited < Duration::from_secs(5), "{waited:?}");
    }

    #[test]
    fn a_waiting_local_model_holds_back_new_vision_requests() {
        let gate = leak(Gate::new());
        let rt = runtime();
        let first = rt.block_on(gate.vision());
        let local_done = Arc::new(AtomicBool::new(false));
        let done = Arc::clone(&local_done);
        let waiter = std::thread::spawn(move || {
            let _t = gate.local("test");
            std::thread::sleep(Duration::from_millis(100));
            done.store(true, Ordering::SeqCst);
        });
        // Let the local run queue up behind the first request.
        std::thread::sleep(Duration::from_millis(50));
        let second = std::thread::spawn(move || {
            let rt = runtime();
            let _t = rt.block_on(gate.vision());
            // The second request only starts once the local run is over.
            assert!(local_done.load(Ordering::SeqCst));
        });
        std::thread::sleep(Duration::from_millis(50));
        drop(first);
        waiter.join().unwrap();
        second.join().unwrap();
    }

    #[test]
    fn vision_requests_run_together() {
        let gate = Gate::new();
        let rt = runtime();
        rt.block_on(async {
            let a = gate.vision().await;
            let b = tokio::time::timeout(Duration::from_secs(1), gate.vision())
                .await
                .expect("a second slot is not held back");
            drop((a, b));
        });
    }

    #[test]
    fn a_stuck_vision_request_holds_a_local_model_only_so_long() {
        let gate = Gate::new();
        let rt = runtime();
        let _stuck = rt.block_on(gate.vision());
        let started = Instant::now();
        let _t = gate.local_within("test", Duration::from_millis(100));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_vision_model_on_the_cpu_does_not_take_the_gate() {
        let gate = Gate::new();
        gate.set_vision_on_gpu(false);
        let rt = runtime();
        let _vision = rt.block_on(gate.vision());
        let started = Instant::now();
        let _t = gate.local_within("test", Duration::from_secs(5));
        assert!(started.elapsed() < Duration::from_millis(500));
    }
}
