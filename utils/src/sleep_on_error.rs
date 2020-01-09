// Port of tk_listen::sleep_on_error to futures-0.3.
// https://github.com/tailhook/tk-listen/blob/master/src/sleep_on_error.rs

use std::time::Duration;
use tokio::io;
use tokio::time::Delay;
use futures::Stream;
use std::task::{Context, Poll};
use std::pin::Pin;
use std::future::Future;

/// This function defines errors that are per-connection. Which basically
/// means that if we get this error from `accept()` system call it means
/// next connection might be ready to be accepted.
///
/// All other errors will incur a timeout before next `accept()` is performed.
/// The timeout is useful to handle resource exhaustion errors like ENFILE
/// and EMFILE. Otherwise, could enter into tight loop.
fn connection_error(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::ConnectionRefused ||
        e.kind() == io::ErrorKind::ConnectionAborted ||
        e.kind() == io::ErrorKind::ConnectionReset
}

/// A structure returned by `ListenExt::sleep_on_error`
///
/// This is a stream that filters original stream for errors, ignores some
/// of them and sleeps on severe ones.
pub struct SleepOnError<S> {
    stream: S,
    delay: Duration,
    timeout: Option<Delay>,
}

pub trait SleepOnErrorExt: Stream {
    /// Turns a listening stream that you can get from `TcpListener::incoming`
    /// into a stream that supresses errors and sleeps on resource shortage,
    /// effectively allowing listening stream to resume on error.
    fn sleep_on_error(self, delay: Duration) -> SleepOnError<Self>
        where Self: Sized,
    {
        new(self, delay)
    }
}

impl<T, I, E> SleepOnErrorExt for T where T: Stream<Item=Result<I, E>> {}

pub fn new<S>(stream: S, delay: Duration) -> SleepOnError<S>
{
    SleepOnError {
        stream,
        delay,
        timeout: None,
    }
}

impl<I, S> Stream for SleepOnError<S>
    where S: Stream<Item=Result<I, io::Error>> + Unpin
{
    type Item = Result<I, io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(to) = &mut self.timeout {
            match Future::poll(Pin::new(to), cx) {
                Poll::Ready(_) => (),
                Poll::Pending => return Poll::Pending,
            }
        }
        self.timeout = None;
        loop {
            match Stream::poll_next(Pin::new(&mut self.stream), cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Ready(Some(Ok(v))) => return Poll::Ready(Some(Ok(v))),
                Poll::Ready(Some(Err(ref e))) if connection_error(e) => continue,
                Poll::Ready(Some(Err(e))) => {
                    debug!("Accept error: {}. Sleeping {:?}...",
                           e, self.delay);
                    let mut delay = tokio::time::delay_for(self.delay);
                    match Future::poll(Pin::new(&mut delay), cx) {
                        Poll::Ready(_) => continue,
                        Poll::Pending => {
                            self.timeout = Some(delay);
                            return Poll::Pending;
                        }
                    }
                }
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let (_, upper) = self.stream.size_hint();
        (0, upper)
    }
}
