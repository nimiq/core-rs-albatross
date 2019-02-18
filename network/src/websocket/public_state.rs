#[cfg(feature = "metrics")]
use std::sync::Arc;

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

            #[cfg(feature = "metrics")]
            network_metrics: Arc::new(NetworkMetrics::default()),
        }
    }
}
