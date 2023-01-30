use std::thread;
use std::time::Duration;

use parking_lot::deadlock;

pub fn initialize_deadlock_detection() {
    // Create a background thread which checks for deadlocks every 10s
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(10));
        let deadlocks = deadlock::check_deadlock();
        if deadlocks.is_empty() {
            continue;
        }

        log::error!("{} deadlocks detected", deadlocks.len());
        for (i, _deadlock) in deadlocks.iter().enumerate() {
            log::error!("Deadlock #{}", i);
        }
    });
}
