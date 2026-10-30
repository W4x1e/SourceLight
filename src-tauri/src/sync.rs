//! Locking that survives a panic instead of cascading it.

use std::sync::{Mutex, MutexGuard};

/// Locks a mutex, recovering from poisoning rather than panicking.
///
/// Every AI session (CLIP, YuNet, the face embedders, the tagger) and the search
/// cache live behind a `Mutex`. If a call panics while holding one — a bad model
/// file, an unexpected tensor shape — the mutex is poisoned, and the usual
/// `.lock().unwrap()` would then panic on *every* subsequent lock: one bad tensor
/// bricking all AI until the app restarts.
///
/// The data behind these mutexes is a model handle or a cache, not an invariant
/// that a half-finished write could leave unsafe to read — an `ort` session is
/// built to be run again after a failed run, and a cache is rebuilt on the next
/// miss. So the right move on poison is to clear it and carry on: this call
/// proceeds, and the next one reloads or recomputes as normal. No restart, and no
/// panic to unwind through the Tauri command boundary.
pub fn lock_recover<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| {
        // Once cleared, later locks succeed normally rather than seeing the stale
        // poison from a failure two calls ago.
        m.clear_poison();
        poisoned.into_inner()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A panic while the lock is held poisons it; the next `lock_recover` still
    /// hands back a usable guard carrying the value the panic left behind, and the
    /// poison is gone for the call after that.
    #[test]
    fn a_poisoned_lock_recovers_instead_of_panicking() {
        let m = Arc::new(Mutex::new(0_u32));

        let m2 = Arc::clone(&m);
        let poisoned = std::thread::spawn(move || {
            let mut g = m2.lock().unwrap();
            *g = 7;
            panic!("inference blew up mid-lock");
        })
        .join();
        assert!(poisoned.is_err(), "the thread was supposed to panic");
        assert!(m.is_poisoned(), "the panic should have poisoned the mutex");

        // The recovering lock does not panic, and sees the value from before the
        // panic — so a caller can reset it rather than being stuck.
        {
            let mut g = lock_recover(&m);
            assert_eq!(*g, 7);
            *g = 0;
        }
        assert!(!m.is_poisoned(), "poison should have been cleared");

        // And a plain lock works again now the poison is gone.
        assert_eq!(*m.lock().unwrap(), 0);
    }
}
