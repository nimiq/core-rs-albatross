use prometheus_client::registry::Registry;

use parking_lot::RwLock;

use crate::NumericClosureMetric;
use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use std::sync::Arc;

pub struct BlockMetrics {}

impl BlockMetrics {
    pub fn register(registry: &mut Registry, blockchain: Arc<RwLock<Blockchain>>) {
        BlockMetrics::register_staking(registry, blockchain.clone());
        BlockMetrics::register_accounts_trie(registry, blockchain.clone());
        BlockMetrics::register_chain(registry, blockchain.clone());
        BlockMetrics::register_block_stats(registry, blockchain);
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

    fn register_accounts_trie(registry: &mut Registry, blockchain: Arc<RwLock<Blockchain>>) {
        let sub_registry = registry.sub_registry_with_prefix("accounts_trie");

        let bc = blockchain.clone();
        let closure = Box::new(NumericClosureMetric::new_gauge(Box::new(move || {
            bc.read().state.accounts.size()
        })));
        sub_registry.register("accounts", "Number of accounts", closure);

        let closure = Box::new(NumericClosureMetric::new_gauge(Box::new(move || {
            blockchain.read().state.accounts.num_branches()
        })));
        sub_registry.register("num_branches", "Number of branch nodes", closure);
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

    fn register_block_stats(registry: &mut Registry, blockchain: Arc<RwLock<Blockchain>>) {
        let sub_registry = registry.sub_registry_with_prefix("blocks");

        let bc = blockchain.clone();
        let closure = Box::new(NumericClosureMetric::new_counter(Box::new(move || {
            bc.read().metrics().block_extended_count() as u32
        })));
        sub_registry.register("blocks_extended", "Count of block extends", closure);

        let bc = blockchain.clone();
        let closure = Box::new(NumericClosureMetric::new_counter(Box::new(move || {
            bc.read().metrics().block_known_count() as u32
        })));
        sub_registry.register("known_count", "Count of block knowns", closure);

        let bc = blockchain.clone();
        let closure = Box::new(NumericClosureMetric::new_counter(Box::new(move || {
            bc.read().metrics().block_forked_count() as u32
        })));
        sub_registry.register("forked_count", "Count of block forks", closure);

        let bc = blockchain.clone();
        let closure = Box::new(NumericClosureMetric::new_counter(Box::new(move || {
            bc.read().metrics().block_ignored_count() as u32
        })));
        sub_registry.register("ignored_count", "Count of block ignores", closure);

        let bc = blockchain.clone();
        let closure = Box::new(NumericClosureMetric::new_counter(Box::new(move || {
            bc.read().metrics().block_rebranched_count() as u32
        })));
        sub_registry.register("rebranched_count", "Count of block rebranchs", closure);

        let closure = Box::new(NumericClosureMetric::new_counter(Box::new(move || {
            blockchain.read().metrics().block_orphan_count() as u32
        })));
        sub_registry.register("orphan_count", "Count of block orphans", closure);
    }
}
