use std::thread;
use std::time::Duration;
use parking_lot::deadlock;
use std::sync::{Arc, Condvar, Mutex};
use failure::_core::sync::atomic::{AtomicBool, Ordering};

/// A background service detecting deadlocks with parking_lot synchronization utilities.
/// Debug information describing locks and dead threads is then printed to the error log.
/// Deadlock detection is active as long as the instance is in scope.
///
/// # Example
///
/// ```
/// use nimiq_lib::extras::deadlock::DeadlockDetector;
///
/// fn main() {
///     let dd = DeadlockDetector::new();
///     // Any deadlocks during this section will be logged.
///     // some_code_using_locks();
///     drop(dd);
///
///     // No more deadlock detection here.
/// }
/// ```
pub struct DeadlockDetector {
    inner: Arc<DeadlockDetectorState>,
}

struct DeadlockDetectorState {
    lock: Mutex<bool>,
    cond: Condvar,
    alive: AtomicBool,
}

impl DeadlockDetector {
    /// Creates a background thread which checks for deadlocks every 10s.
    #[must_use]
    pub fn new() -> Self {
        let state = Arc::new(DeadlockDetectorState {
            lock: Mutex::new(false),
            cond: Condvar::new(),
            alive: AtomicBool::new(true),
        });
        let state2 = Arc::clone(&state);
        thread::Builder::new()
            .name("Deadlock Detection".to_owned())
            .spawn(move || Self::run(state2))
            .unwrap();
        Self { inner: state }
    }

    // Run until the shutdown condition is triggered
    fn run(this: Arc<DeadlockDetectorState>) {
        let mut guard = this.lock.lock().unwrap();
        *guard = true; // mark thread as started
        trace!("Starting Deadlock Detection");
        loop {
            let lock_result = this.cond.wait_timeout(guard, Duration::from_secs(10));
            match lock_result {
                Ok((next_guard, timeout)) => {
                    guard = next_guard;
                    if !timeout.timed_out() {
                        // Got shutdown condition, break loop.
                        break;
                    }
                    // Timed out waiting for shutdown condition,
                    // continue scanning for deadlocks.
                },
                Err(_) => {
                    warn!("Deadlock detection shutdown lock poisoned");
                    break;
                },
            }

            let deadlocks = deadlock::check_deadlock();
            if deadlocks.is_empty() {
                continue;
            }

            error!("{} deadlocks detected", deadlocks.len());
            for (i, threads) in deadlocks.iter().enumerate() {
                error!("Deadlock #{}", i);
                for t in threads {
                    error!("Thread ID {:#?}", t.thread_id());
                    error!("{:#?}", t.backtrace());
                }
            }
        }
        trace!("Stopping Deadlock Detection");
        this.alive.store(false, Ordering::Relaxed);
    }

    fn stop(&mut self) {
        // Spin until thread starts
        while !*self.inner.lock.lock().unwrap() {}
        self.inner.cond.notify_one();
    }
}

impl Drop for DeadlockDetector {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instant_drop() {
        let mut dd = DeadlockDetector::new();
        dd.stop();
        check_whether_stopped(&dd);
    }

    #[test]
    fn test_delay_drop() {
        let mut dd = DeadlockDetector::new();
        // Try to run at least one deadlock detection iteration
        thread::sleep(Duration::from_millis(10));
        thread::yield_now();
        dd.stop();
        check_whether_stopped(&dd);
    }

    fn check_whether_stopped(dd: &DeadlockDetector) {
        let mut i = 0usize;
        while dd.inner.alive.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(10));
            i += 1;
            if i > 500 {
                panic!("Deadlock Detector background thread leaked");
            }
        }
    }
}
