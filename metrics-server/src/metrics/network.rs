use std::io;
use std::sync::Arc;

use network::network::Network;

use crate::server;
use crate::server::SerializationType;

pub struct NetworkMetrics<N>
where
    N: Network + 'static,
{
    network: Arc<N>,
}

impl<N> NetworkMetrics<N>
where
    N: Network + 'static,
{
    pub fn new(network: Arc<N>) -> Self {
        NetworkMetrics { network }
    }
}

impl<N> server::Metrics for NetworkMetrics<N>
where
    N: Network + 'static,
{
    fn metrics(
        &self,
        serializer: &mut server::MetricsSerializer<SerializationType>,
    ) -> Result<(), io::Error> {
        unimplemented!();
        // let (message_metrics, network_metrics, peer_metrics) = self.network.connections.metrics();

        // for ((protocol, state), count) in peer_metrics.peer_metrics() {
        //     let str_state = match state {
        //         ConnectionState::Established => "established",
        //         ConnectionState::Connecting => "connecting",
        //         ConnectionState::Closed => "closed",
        //         ConnectionState::Connected => "connected",
        //         ConnectionState::Negotiating => "negotiating",
        //         ConnectionState::New => "new",
        //     };
        //     serializer.metric_with_attributes(
        //         "network_peers",
        //         count,
        //         attributes! {"type" => protocol, "state" => str_state},
        //     )?;
        // }

        // let num_addresses = self.network.addresses.known_addresses_count();
        // let num_ws_addresses = self.network.addresses.known_ws_addresses_count();
        // let num_wss_addresses = self.network.addresses.known_wss_addresses_count();
        // let num_rtc_addresses = self.network.addresses.known_rtc_addresses_count();
        // let num_dumb_addresses =
        //     num_addresses - num_wss_addresses - num_ws_addresses - num_rtc_addresses;
        // serializer.metric_with_attributes(
        //     "network_known_addresses",
        //     num_dumb_addresses,
        //     attributes! {"type" => "dumb"},
        // )?;
        // serializer.metric_with_attributes(
        //     "network_known_addresses",
        //     num_ws_addresses,
        //     attributes! {"type" => "websocket"},
        // )?;
        // serializer.metric_with_attributes(
        //     "network_known_addresses",
        //     num_wss_addresses,
        //     attributes! {"type" => "websocket-secure"},
        // )?;
        // serializer.metric_with_attributes(
        //     "network_known_addresses",
        //     num_rtc_addresses,
        //     attributes! {"type" => "webrtc"},
        // )?;

        // serializer.metric("network_time_now", self.network.time.now())?;
        // serializer.metric_with_attributes(
        //     "network_bytes",
        //     network_metrics.bytes_sent(),
        //     attributes! {"direction" => "sent"},
        // )?;
        // serializer.metric_with_attributes(
        //     "network_bytes",
        //     network_metrics.bytes_received(),
        //     attributes! {"direction" => "received"},
        // )?;

        // for &ty in message_metrics.message_types() {
        //     serializer.metric_with_attributes(
        //         "message_rx_count",
        //         message_metrics.message_occurrences(ty).unwrap_or(0),
        //         attributes! {"type" => format!("{}", ty)},
        //     )?;
        //     serializer.metric_with_attributes(
        //         "message_rx_processing_time",
        //         message_metrics.message_processing_time(ty).unwrap_or(0) / 1000, // Processing time is recorded in microseconds but should be reported in milliseconds
        //         attributes! {"type" => format!("{}", ty)},
        //     )?;
        // }

        Ok(())
    }
}
