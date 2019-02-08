use std::io;
use std::sync::Arc;

use network::network::Network;

use crate::server;
use crate::server::SerializationType;

pub struct NetworkMetrics {
    network: Arc<Network>,
}

impl NetworkMetrics {
    pub fn new(network: Arc<Network>) -> Self {
        NetworkMetrics {
            network,
        }
    }
}

impl server::Metrics for NetworkMetrics {
    fn metrics(&self, serializer: &mut server::MetricsSerializer<SerializationType>) -> Result<(), io::Error> {
        // TODO: Add useful peer metrics.
        serializer.metric_with_attributes(
            "network_peers",
            self.network.connections.peer_count(),
            attributes!{"type" => "unknown"}
        )?;
        serializer.metric_with_attributes(
            "network_peers",
            self.network.connections.peer_count_outbound(),
            attributes!{"type" => "unknown", "state" => "outbound"}
        )?;
        serializer.metric_with_attributes(
            "network_peers",
            self.network.connections.connecting_count(),
            attributes!{"type" => "unknown", "state" => "connecting"}
        )?;

        let num_addresses = self.network.addresses.known_addresses_count();
        let num_ws_addresses = self.network.addresses.known_ws_addresses_count();
        let num_wss_addresses = self.network.addresses.known_wss_addresses_count();
        let num_rtc_addresses = self.network.addresses.known_rtc_addresses_count();
        let num_dumb_addresses = num_addresses - num_wss_addresses - num_ws_addresses - num_rtc_addresses;
        serializer.metric_with_attributes(
            "network_known_addresses",
            num_dumb_addresses,
            attributes!{"type" => "dumb"}
        )?;
        serializer.metric_with_attributes(
            "network_known_addresses",
            num_ws_addresses,
            attributes!{"type" => "websocket"}
        )?;
        serializer.metric_with_attributes(
            "network_known_addresses",
            num_wss_addresses,
            attributes!{"type" => "websocket-secure"}
        )?;
        serializer.metric_with_attributes(
            "network_known_addresses",
            num_rtc_addresses,
            attributes!{"type" => "webrtc"}
        )?;

        serializer.metric("network_time_now", self.network.network_time.now())?;

        // TODO: Measure bytes.
        // TODO: Measure messages.

        Ok(())
    }
}