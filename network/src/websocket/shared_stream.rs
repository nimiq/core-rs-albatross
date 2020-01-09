#[cfg(feature = "metrics")]
use std::sync::Arc;

use utils::locking::MultiLock;
use network_primitives::address::net_address::NetAddress;

#[cfg(feature = "metrics")]
use crate::network_metrics::NetworkMetrics;
use crate::websocket::error::Error;
use crate::websocket::Message;
use crate::websocket::NimiqMessageStream;
use crate::websocket::public_state::PublicStreamInfo;
use futures::{Stream, Sink};
use std::task::{Context, Poll};
use std::pin::Pin;

#[derive(Clone, Debug)]
pub struct SharedNimiqMessageStream {
    inner: MultiLock<NimiqMessageStream>,
    buffer: Option<Message>,
    state: PublicStreamInfo,
}

impl SharedNimiqMessageStream {
    pub fn net_address(&self) -> NetAddress {
        self.state.net_address
    }

    pub fn outbound(&self) -> bool {
        self.state.outbound
    }

    #[cfg(feature = "metrics")]
    pub fn network_metrics(&self) -> &Arc<NetworkMetrics> {
        &self.state.network_metrics
    }
}

impl From<NimiqMessageStream> for SharedNimiqMessageStream {
    fn from(stream: NimiqMessageStream) -> Self {
        let state = PublicStreamInfo::from(&stream);
        SharedNimiqMessageStream {
            inner: MultiLock::new(stream),
            buffer: None,
            state,
        }
    }
}

impl Stream for SharedNimiqMessageStream {
    type Item = Result<Message, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.inner.poll_lock() {
            Poll::Pending => Poll::Pending,
            Poll::Ready(mut v) => Stream::poll_next(Pin::new(&mut *v), cx),
        }
    }
}

impl SharedNimiqMessageStream {
    fn force_flush(buffer: &mut Option<Message>, v: &mut NimiqMessageStream) -> Result<(), Error> {
        if let Some(msg) = buffer.take() {
            if let Err(err) = Sink::start_send(Pin::new(v), msg) {
                return Err(err);
            }
        }
        Ok(())
    }
}

impl Sink<Message> for SharedNimiqMessageStream {
    type Error = Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this =  &mut *self;
        match this.inner.poll_lock() {
            Poll::Pending => Poll::Pending,
            Poll::Ready(mut v ) => loop {
                // Loop does exactly one or two iterations.
                match Sink::poll_ready(Pin::new(&mut *v), cx) {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                    Poll::Ready(Ok(())) => (),
                };
                if let Some(msg) = this.buffer.take() {
                    if let Err(err) = Sink::start_send(Pin::new(&mut *v), msg) {
                        return Poll::Ready(Err(err));
                    }
                } else {
                    return Poll::Ready(Ok(()));
                }
            },
        }
    }

    fn start_send(mut self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
        if self.buffer.is_some() {
            panic!("Trying to replace an unsent message (did you check poll_ready?)");
        }
        // Place item in buffer and try to send it at the next poll_ready.
        self.buffer = Some(item);
        Ok(())
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = &mut *self;
        match this.inner.poll_lock() {
            Poll::Pending => Poll::Pending,
            Poll::Ready(mut v ) => loop {
                if let Err(err) = Self::force_flush(&mut this.buffer, &mut v) {
                    return Poll::Ready(Err(err));
                }
                return Sink::poll_flush(Pin::new(&mut *v), cx);
            },
        }
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = &mut *self;
        match this.inner.poll_lock() {
            Poll::Pending => Poll::Pending,
            Poll::Ready(mut v ) => loop {
                if let Err(err) = Self::force_flush(&mut this.buffer, &mut v) {
                    return Poll::Ready(Err(err));
                }
                return Sink::poll_close(Pin::new(&mut *v), cx);
            },
        }
    }
}
