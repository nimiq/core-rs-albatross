use std::time::Duration;
use std::time::Instant;

/// A `RateLimit` can be used to limit the number of occurrences
/// of certain actions within a time period.
pub struct RateLimit {
    allowed_occurrences: usize,
    time_period: Duration,
    last_reset: Instant,
    counter: usize,
}

impl RateLimit {
    /// Creates a `RateLimit`.
    ///
    /// * `allowed_occurrences` - The limit on occurrences of an action within the defined `time_period`.
    /// * `time_period` - The interval at which the number of occurrences will be reset.
    #[inline]
    pub fn new(allowed_occurrences: usize, time_period: Duration) -> Self {
        RateLimit {
            allowed_occurrences,
            time_period,
            last_reset: Instant::now(),
            counter: 0,
        }
    }

    /// Internally reset counter if necessary.
    #[inline]
    fn check_reset(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_reset) > self.time_period {
            self.last_reset = now;
            self.counter = 0;
        }
    }

    /// Determine whether `number` actions are still within the current rate limit.
    pub fn note(&mut self, number: usize) -> bool {
        self.check_reset();
        self.counter += number;
        self.counter <= self.allowed_occurrences
    }

    /// Determine how many actions are still within the current rate limit.
    pub fn num_allowed(&mut self) -> usize {
        self.check_reset();
        self.allowed_occurrences.saturating_sub(self.counter)
    }
}
