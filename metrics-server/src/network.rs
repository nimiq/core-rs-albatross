use prometheus_client::registry::Registry;

use crate::NumericClosureMetric;
use nimiq_network_libp2p::Network;
use std::sync::Arc;

pub struct NetworkMetrics {}

impl NetworkMetrics {
    pub fn register(registry: &mut Registry, network: Arc<Network>) {
        let sub_registry = registry.sub_registry_with_prefix("network");

        let closure = Box::new(NumericClosureMetric::new_gauge(Box::new(move || {
            network.peer_count() as u32
        })));
        sub_registry.register("peer_count", "Number of peers", closure);
    }
}
