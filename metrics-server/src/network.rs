use std::sync::Arc;

use nimiq_network_libp2p::Network;
use prometheus_client::registry::Registry;

use crate::NumericClosureMetric;

pub struct NetworkMetrics {}

impl NetworkMetrics {
    pub fn register(registry: &mut Registry, network: Arc<Network>) {
        let sub_registry = registry.sub_registry_with_prefix("network");

        network.metrics().register(sub_registry);

        let closure =
            NumericClosureMetric::new_gauge(Box::new(move || network.peer_count() as i64));
        sub_registry.register("peer_count", "Number of peers", closure);
    }
}
