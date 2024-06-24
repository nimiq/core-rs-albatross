use std::{convert::TryInto, future::Future, pin::pin, time::Duration};

use futures::future::{select, Either};
use gloo_timers::future::{IntervalStream, TimeoutFuture};
use instant::Instant;
use send_wrapper::SendWrapper;

pub type Interval = SendWrapper<IntervalStream>;

pub fn interval(period: Duration) -> Interval {
    assert!(!period.is_zero());
    let millis = period
        .as_millis()
        .try_into()
        .expect("Period as millis must fit in u32");
    SendWrapper::new(IntervalStream::new(millis))
}

pub fn timeout<F: Future>(
    duration: Duration,
    future: F,
) -> impl Future<Output = Result<F::Output, ()>> {
    SendWrapper::new(timeout_inner(duration, future))
}

async fn timeout_inner<F: Future>(duration: Duration, future: F) -> Result<F::Output, ()> {
    let millis = duration
        .as_millis()
        .try_into()
        .expect("Duration as millis must fit in u32");
    let timeout = TimeoutFuture::new(millis);
    match select(pin!(future), timeout).await {
        Either::Left((res, _)) => Ok(res),
        Either::Right(_) => Err(()),
    }
}

pub fn sleep(duration: Duration) -> impl Future<Output = ()> {
    let millis = duration
        .as_millis()
        .try_into()
        .expect("Duration as millis must fit in u32");
    SendWrapper::new(TimeoutFuture::new(millis))
}

pub fn sleep_until(deadline: Instant) -> impl Future<Output = ()> {
    sleep(deadline.saturating_duration_since(Instant::now()))
}
