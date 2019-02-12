#[cfg(feature = "metrics")]
use std::sync::Arc;
use std::time::Instant;

use network_primitives::address::net_address::NetAddress;

#[cfg(feature = "metrics")]
use crate::network_metrics::NetworkMetrics;
use crate::websocket::NimiqMessageStream;

/// This struct stores public information about the stream.
#[derive(Clone, Debug)]
pub struct PublicStreamInfo {
    // Constant info.
    pub net_address: NetAddress,
    pub outbound: bool,

    // Mutable info.
    pub last_chunk_received_at: Option<Instant>,

    #[cfg(feature = "metrics")]
    pub network_metrics: Arc<NetworkMetrics>,
}

impl<'a> From<&'a PublicStreamInfo> for PublicStreamInfo {
    fn from(state: &'a PublicStreamInfo) -> Self {
        state.clone()
    }
}

impl<'a> From<&'a NimiqMessageStream> for PublicStreamInfo {
    fn from(stream: &'a NimiqMessageStream) -> Self {
        stream.public_state.clone()
    }
}

impl PublicStreamInfo {
    pub fn new(net_address: NetAddress, outbound: bool) -> Self {
        PublicStreamInfo {
            net_address,
            outbound,
            last_chunk_received_at: None,

            #[cfg(feature = "metrics")]
            network_metrics: Arc::new(NetworkMetrics::default()),
        }
    }
    pub fn update(&mut self, state: &PublicStreamInfo) {
        self.last_chunk_received_at = state.last_chunk_received_at.clone();
    }
}