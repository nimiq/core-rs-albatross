#[cfg(feature = "metrics")]
use std::sync::Arc;

use futures::prelude::*;

use utils::locking::MultiLock;
use network_primitives::address::net_address::NetAddress;

#[cfg(feature = "metrics")]
use crate::network_metrics::NetworkMetrics;
use crate::websocket::error::Error;
use crate::websocket::Message;
use crate::websocket::NimiqMessageStream;
use crate::websocket::public_state::PublicStreamInfo;

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
            Async::Ready(mut inner) => {
                let result = inner.poll();
                result
            },
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

impl Sink for SharedNimiqMessageStream {
    type SinkItem = Message;
    type SinkError = Error;

    fn start_send(&mut self, item: Self::SinkItem)
                  -> StartSend<Self::SinkItem, Self::SinkError>
    {
        match self.inner.poll_lock() {
            Async::Ready(mut inner) => {
                let result = inner.start_send(item);
                result
            },
            Async::NotReady => Ok(AsyncSink::NotReady(item)),
        }
    }

    fn poll_complete(&mut self) -> Poll<(), Self::SinkError> {
        match self.inner.poll_lock() {
            Async::Ready(mut inner) => {
                let result = inner.poll_complete();
                result
            },
            Async::NotReady => Ok(Async::NotReady),
        }
    }

    fn close(&mut self) -> Poll<(), Self::SinkError> {
        match self.inner.poll_lock() {
            Async::Ready(mut inner) => {
                let result = inner.close();
                result
            },
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}
