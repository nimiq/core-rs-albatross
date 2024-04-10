use std::{future::Future, time::Duration};

use tokio::time::{
    interval_at, sleep as tokio_sleep, timeout as tokio_timeout, Instant, Sleep, Timeout,
};
pub use tokio_stream::wrappers::IntervalStream as Interval;

pub fn interval(period: Duration) -> Interval {
    assert!(
        period.as_millis() <= u32::MAX as u128,
        "Period as millis must fit in u32"
    );
    Interval::new(interval_at(Instant::now() + period, period))
}

pub fn timeout<F: Future>(duration: Duration, future: F) -> Timeout<F> {
    tokio_timeout(duration, future)
}

pub fn sleep(duration: Duration) -> Sleep {
    tokio_sleep(duration)
}
