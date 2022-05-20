use prometheus_client::registry::Registry;

use parking_lot::RwLock;

use crate::NumericClosureMetric;
use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use std::sync::Arc;

pub struct BlockMetrics {}

impl BlockMetrics {
    pub fn register(registry: &mut Registry, blockchain: Arc<RwLock<Blockchain>>) {
        BlockMetrics::register_staking(registry, blockchain.clone());
        BlockMetrics::register_chain(registry, blockchain.clone());

        let sub_registry = registry.sub_registry_with_prefix("blockchain");
        blockchain.read().metrics().register(sub_registry);
    }

    fn register_staking(registry: &mut Registry, blockchain: Arc<RwLock<Blockchain>>) {
        let sub_registry = registry.sub_registry_with_prefix("staking");

        let bc = blockchain.clone();
        let closure = Box::new(NumericClosureMetric::new_gauge(Box::new(move || {
            bc.read().get_staking_contract().active_validators.len() as u32
        })));
        sub_registry.register("active_validators", "Number of active validators", closure);

        let closure = Box::new(NumericClosureMetric::new_gauge(Box::new(move || {
            blockchain.read().get_staking_contract().parked_set.len() as u32
        })));
        sub_registry.register("parked_validators", "Number of parked validators", closure);
    }

    fn register_chain(registry: &mut Registry, blockchain: Arc<RwLock<Blockchain>>) {
        let sub_registry = registry.sub_registry_with_prefix("blockchain");

        let bc = blockchain.clone();
        let closure = Box::new(NumericClosureMetric::new_gauge(Box::new(move || {
            bc.read().block_number()
        })));
        sub_registry.register("block_number", "Number of latest block", closure);

        let closure = Box::new(NumericClosureMetric::new_gauge(Box::new(move || {
            blockchain.read().view_number()
        })));
        sub_registry.register("view_number", "View number of latest block", closure);
    }
}
