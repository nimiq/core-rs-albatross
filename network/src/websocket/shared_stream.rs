#[cfg(feature = "metrics")]
use std::sync::Arc;

use futures::prelude::*;

use peer_address::address::NetAddress;
use utils::locking::MultiLock;

#[cfg(feature = "metrics")]
use crate::network_metrics::NetworkMetrics;
use crate::websocket::error::Error;
use crate::websocket::public_state::PublicStreamInfo;
use crate::websocket::Message;
use crate::websocket::NimiqMessageStream;

#[derive(Clone, Debug)]
pub struct SharedNimiqMessageStream {
    inner: MultiLock<NimiqMessageStream>,
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
            state,
        }
    }
}

impl Stream for SharedNimiqMessageStream {
    type Item = Message;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        match self.inner.poll_lock() {
            Async::Ready(mut inner) => inner.poll(),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

impl Sink for SharedNimiqMessageStream {
    type SinkItem = Message;
    type SinkError = Error;

    fn start_send(&mut self, item: Self::SinkItem) -> StartSend<Self::SinkItem, Self::SinkError> {
        match self.inner.poll_lock() {
            Async::Ready(mut inner) => inner.start_send(item),
            Async::NotReady => Ok(AsyncSink::NotReady(item)),
        }
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        match self.inner.poll_lock() {
            Async::Ready(mut inner) => inner.poll_complete(),
            Async::NotReady => Ok(Async::NotReady),
        }
    }

    fn close(&mut self) -> Poll<(), Self::SinkError> {
        match self.inner.poll_lock() {
            Async::Ready(mut inner) => inner.close(),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}
