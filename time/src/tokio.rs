use tokio::time::{interval as tokio_interval, interval_at as tokio_interval_at, Instant};
pub use tokio_stream::wrappers::IntervalStream as Interval;

pub fn interval(duration: std::time::Duration) -> Interval {
    Interval::new(tokio_interval(duration))
}

pub fn interval_at(duration: std::time::Duration) -> Interval {
    Interval::new(tokio_interval_at(Instant::now() + duration, duration))
}
