use std::time::Duration;

#[cfg(not(feature = "tokio-time"))]
use instant::Instant;
#[cfg(feature = "tokio-time")]
use tokio::time::Instant;

/// The structure to be used to limit the number of requests to a limit of allowed_occurrences within a block_range.
#[derive(Debug, Eq, Hash, PartialEq)]
pub struct RateLimit {
    /// Max allowed requests.
    allowed_occurrences: u32,
    /// The range/window of time.
    time_window: Duration,
    /// The timestamp of the last reset.
    last_reset: Instant,
    /// The counter of requests submited within the current block range.
    occurrences_counter: u32,
}

impl RateLimit {
    pub fn new(allowed_occurrences: u32, time_window: Duration, last_reset: Instant) -> Self {
        RateLimit {
            allowed_occurrences,
            time_window,
            last_reset,
            occurrences_counter: 0,
        }
    }

    /// Updates the last_reset if needed and then increments the counter of number of requests by
    /// the specified number.
    /// Receives the number to increment the counter and the current time measured in seconds.
    pub fn increment_and_is_allowed(&mut self, request_count: u32) -> bool {
        let current_time = Instant::now();
        if self.next_reset_time() <= current_time {
            self.last_reset = current_time;
            self.occurrences_counter = 0;
        }
        self.occurrences_counter += request_count;
        self.occurrences_counter <= self.allowed_occurrences
    }

    /// Checks if this object can be deleted by undertanding if there are still active counters.
    pub fn can_delete(&self, current_time: Instant) -> bool {
        self.occurrences_counter == 0 || self.next_reset_time() <= current_time
    }

    /// Returns the timestamp for the next reset of the counters.
    pub fn next_reset_time(&self) -> Instant {
        self.last_reset + self.time_window
    }
}
