use std::{future::Future, time::Duration};

use instant::Instant;
use tokio::time::{
    interval_at, sleep as tokio_sleep, sleep_until as tokio_sleep_until, timeout as tokio_timeout,
    Instant as TokioInstant, Sleep, Timeout,
};
pub use tokio_stream::wrappers::IntervalStream as Interval;

pub fn interval(period: Duration) -> Interval {
    // Limit the period to the maximum allowed by gloo-timers to get consistent behaviour
    // across both implementations.
    assert!(
        period.as_millis() <= u32::MAX as u128,
        "Period as millis must fit in u32"
    );
    Interval::new(interval_at(TokioInstant::now() + period, period))
}

pub fn timeout<F: Future>(duration: Duration, future: F) -> Timeout<F> {
    tokio_timeout(duration, future)
}

pub fn sleep(duration: Duration) -> Sleep {
    tokio_sleep(duration)
}

pub fn sleep_until(deadline: Instant) -> Sleep {
    tokio_sleep_until(TokioInstant::from_std(deadline))
}
