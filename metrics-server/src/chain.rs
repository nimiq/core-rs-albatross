use prometheus_client::registry::Registry;

use parking_lot::RwLock;

use crate::NumericClosureMetric;
use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
use std::sync::Arc;

pub struct BlockMetrics {}

impl BlockMetrics {
    pub fn register(registry: &mut Registry, blockchain_proxy: BlockchainProxy) {
        BlockMetrics::register_chain(registry, blockchain_proxy.clone());
        if let BlockchainProxy::Full(blockchain) = blockchain_proxy {
            BlockMetrics::register_accounts_trie(registry, blockchain.clone());
            BlockMetrics::register_staking(registry, blockchain.clone());
            let sub_registry = registry.sub_registry_with_prefix("blockchain");
            blockchain.read().metrics().register(sub_registry);
        }
    }

    fn register_staking(registry: &mut Registry, blockchain: Arc<RwLock<Blockchain>>) {
        let sub_registry = registry.sub_registry_with_prefix("staking");

        let bc = blockchain.clone();
        let closure = NumericClosureMetric::new_gauge(Box::new(move || {
            bc.read().get_staking_contract().active_validators.len() as i64
        }));
        sub_registry.register("active_validators", "Number of active validators", closure);

        let closure = NumericClosureMetric::new_gauge(Box::new(move || {
            blockchain.read().get_staking_contract().parked_set.len() as i64
        }));
        sub_registry.register("parked_validators", "Number of parked validators", closure);
    }

    fn register_accounts_trie(registry: &mut Registry, blockchain: Arc<RwLock<Blockchain>>) {
        let sub_registry = registry.sub_registry_with_prefix("accounts_trie");

        let bc = blockchain.clone();
        let closure = NumericClosureMetric::new_gauge(Box::new(move || {
            bc.read().state.accounts.size() as i64
        }));
        sub_registry.register("accounts", "Number of accounts", closure);

        let closure = NumericClosureMetric::new_gauge(Box::new(move || {
            blockchain.read().state.accounts.num_branches() as i64
        }));
        sub_registry.register("num_branches", "Number of branch nodes", closure);
    }

    fn register_chain(registry: &mut Registry, blockchain: BlockchainProxy) {
        let sub_registry = registry.sub_registry_with_prefix("blockchain");

        let closure = NumericClosureMetric::new_gauge(Box::new(move || {
            blockchain.read().block_number() as i64
        }));
        sub_registry.register("block_number", "Number of latest block", closure);
    }
}
