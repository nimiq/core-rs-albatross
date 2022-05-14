use crate::NumericClosureMetric;
use nimiq_consensus::ConsensusProxy;
use nimiq_network_interface::network::Network;
use prometheus_client::registry::Registry;

pub struct ConsensusMetrics {}

impl ConsensusMetrics {
    pub fn register<TNetwork: Network>(
        registry: &mut Registry,
        consensus: ConsensusProxy<TNetwork>,
    ) {
        let sub_registry = registry.sub_registry_with_prefix("consensus");

        let closure = Box::new(NumericClosureMetric::new_gauge(Box::new(move || {
            consensus.is_established() as u32
        })));
        sub_registry.register(
            "is_established",
            "Whether consensus is established",
            closure,
        );
    }
}
