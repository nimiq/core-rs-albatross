use std::collections::HashMap;
use std::fmt;
use std::fmt::Debug;
use std::hash::Hash;
use std::time::Duration;

use futures::{select, FutureExt};
use futures::channel::oneshot;
use parking_lot::{Mutex, MutexGuard};
use tokio::time::{delay_for, interval_at, Instant};

#[derive(Default)]
pub struct Timers<K: Eq + Hash + Debug> {
    delays: Mutex<HashMap<K, oneshot::Sender<()>>>,
    intervals: Mutex<HashMap<K, oneshot::Sender<()>>>,
}

impl<K: Eq + Hash + Debug> Timers<K> {
    /// Creates a new Timers instance.
    pub fn new() -> Self {
        Self {
            delays: Mutex::new(HashMap::new()),
            intervals: Mutex::new(HashMap::new()),
        }
    }

    /// Clears all internal handles and aborts existing delays and intervals.
    pub fn clear_all(&self) {
        let mut delays = self.delays.lock();
        let mut intervals = self.intervals.lock();

        for (_, sender) in delays.drain() {
            sender.send(()).unwrap_or(());
        }

        for (_, sender) in intervals.drain() {
            sender.send(()).unwrap_or(());
        }
    }

    // Public delay functions
    /// Schedules a closure to be executed with a delay (if not aborted until then).
    /// The key must be unique to all delays and the caller is expected to clear the timeout
    /// (even after successful completion) if the key is going to be used multiple times
    /// or to prevent memory leaks.
    pub fn set_delay<F: Send + 'static>(&self, key: K, func: F, delay: Duration)
        where F: FnOnce() {
        let mut delays = self.delays.lock();
        self.set_delay_guarded(key, func, delay, &mut delays);
    }

    /// Aborts the delayed closure if still being scheduled, and cleans up the internal handle.
    pub fn clear_delay(&self, key: &K) {
        let mut delays = self.delays.lock();
        self.clear_timer_guarded(key, &mut delays);
    }

    /// Aborts the delayed closure if present and schedules a new one.
    pub fn reset_delay<F: Send + 'static>(&self, key: K, func: F, delay: Duration)
        where F: FnOnce() {
        let mut delays = self.delays.lock();
        self.clear_timer_guarded(&key, &mut delays);
        self.set_delay_guarded(key, func, delay, &mut delays);
    }

    /// Checks whether a delayed closure exists under this key.
    pub fn delay_exists(&self, key: &K) -> bool {
        self.delays.lock().contains_key(key)
    }

    // Public interval functions
    /// Schedules a recurring closure to be executed in a specific interval (until abortion).
    /// The key must be unique to all intervals and the caller is expected to clear the timeout
    /// (even after successful completion) if the key is going to be used multiple times
    /// or to prevent memory leaks.
    /// The first interval tick does not happen immediately.
    pub fn set_interval<F: Send + Sync + 'static>(&self, key: K, func: F, duration: Duration)
        where F: Fn() {
        let mut intervals = self.intervals.lock();
        self.set_interval_guarded(key, func, duration, &mut intervals);
    }

    /// Aborts the interval and prevents any new execution. Also cleans up the internal handle.
    pub fn clear_interval(&self, key: &K) {
        let mut intervals = self.intervals.lock();
        self.clear_timer_guarded(key, &mut intervals);
    }

    /// Aborts the interval and schedules a new recurring closure.
    pub fn reset_interval<F: Send + Sync + 'static>(&self, key: K, func: F, duration: Duration)
        where F: Fn() {
        let mut intervals = self.intervals.lock();
        self.clear_timer_guarded(&key, &mut intervals);
        self.set_interval_guarded(key, func, duration, &mut intervals);
    }

    /// Checks whether a recurring closure exists under this key.
    pub fn interval_exists(&self, key: &K) -> bool {
        self.intervals.lock().contains_key(key)
    }

    // Internal functions
    fn set_delay_guarded<F: Send + 'static>(&self, key: K, func: F, delay: Duration, delays: &mut MutexGuard<HashMap<K, oneshot::Sender<()>>>)
        where F: FnOnce() {
        if delays.contains_key(&key) {
            error!("Duplicate delay for key {:?}", &key);
            return;
        }

        let (tx, rx) = oneshot::channel();
        delays.insert(key, tx);
        tokio::spawn(async move {
            let task = async move {
                delay_for(delay).await;
                func();
            };
            select! {
                _ = task.fuse() => (),
                _ = rx.fuse() => (),
            }
        });
    }

    pub fn set_interval_guarded<F: Send + Sync + 'static>(&self, key: K, func: F, duration: Duration, intervals: &mut MutexGuard<HashMap<K, oneshot::Sender<()>>>)
        where F: Fn() {
        if intervals.contains_key(&key) {
            error!("Duplicate delay for key {:?}", &key);
            return;
        }

        let (tx, rx) = oneshot::channel();
        intervals.insert(key, tx);
        tokio::spawn(async move {
            let interval_fut = async move {
                let mut stream = interval_at(Instant::now() + duration, duration);
                loop {
                    stream.tick().await;
                    func();
                }
            };
            select! {
                _ = interval_fut.fuse() => (),
                _ = rx.fuse() => (),
            }
        });
    }

    fn clear_timer_guarded(&self, key: &K, guard: &mut MutexGuard<HashMap<K, oneshot::Sender<()>>>) {
        let handle = guard.remove(key);
        if let Some(handle) = handle {
            handle.send(()).unwrap_or(());
        }
    }
}

impl<K: Eq + Hash + Debug> Drop for Timers<K> {
    fn drop(&mut self) {
        self.clear_all()
    }
}

impl<K: Eq + Hash + Debug> fmt::Debug for Timers<K> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Timers {{ num_delays: {}, num_intervals: {} }}", self.delays.lock().len(), self.intervals.lock().len())
    }
}
