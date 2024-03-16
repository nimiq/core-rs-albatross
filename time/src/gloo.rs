use std::convert::TryInto;

use gloo_timers::future::IntervalStream;
use send_wrapper::SendWrapper;

pub type Interval = SendWrapper<IntervalStream>;

pub fn interval(duration: std::time::Duration) -> Interval {
    // gloo-timers does not have a way to create an interval that fires its first tick immediately
    interval_at(duration)
}

pub fn interval_at(duration: std::time::Duration) -> Interval {
    SendWrapper::new(IntervalStream::new(
        duration.as_millis().try_into().unwrap(),
    ))
}
