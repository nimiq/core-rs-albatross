use std::sync::Arc;

use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
#[cfg(feature = "extended-staking")]
use nimiq_primitives::coin::Coin;
use parking_lot::RwLock;
use prometheus_client::registry::{Registry, Unit};

use crate::NumericClosureMetric;

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

        // Number of active validators
        let bc = Arc::clone(&blockchain);
        let closure = NumericClosureMetric::new_gauge(Box::new(move || {
            bc.read()
                .get_staking_contract_if_complete(None)
                .map(|contract| contract.active_validators.len() as i64)
                .unwrap_or(0)
        }));
        sub_registry.register("active_validators", "Number of active validators", closure);

        // Total stake
        let bc = Arc::clone(&blockchain);
        let closure = NumericClosureMetric::new_gauge(Box::new(move || {
            bc.read()
                .get_staking_contract_if_complete(None)
                .map(|contract| u64::from(contract.balance) as i64)
                .unwrap_or(0)
        }));
        sub_registry.register("total_stake", "Total stake", closure);

        #[cfg(feature = "extended-staking")]
        Self::register_extended_staking(sub_registry, blockchain);
    }

    #[cfg(feature = "extended-staking")]
    fn register_extended_staking(registry: &mut Registry, blockchain: Arc<RwLock<Blockchain>>) {
        // Total active stake
        let bc = Arc::clone(&blockchain);
        let closure = NumericClosureMetric::new_gauge(Box::new(move || {
            let Some(contract) = bc.read().get_staking_contract_if_complete(None) else {
                return 0;
            };
            let sum = contract.active_validators.values().cloned().sum::<Coin>();
            u64::from(sum) as i64
        }));
        registry.register("active_stake", "Total active stake", closure);

        // Number of inactive validators
        let bc = Arc::clone(&blockchain);
        let closure = NumericClosureMetric::new_gauge(Box::new(move || {
            let blockchain = bc.read();
            let Some(contract) = blockchain.get_staking_contract_if_complete(None) else {
                return 0;
            };

            let txn = blockchain.read_transaction();
            let data_store = blockchain.get_staking_contract_store();
            contract
                .iter_validators(&data_store.read(&txn))
                .filter(|validator| validator.inactive_from.is_some())
                .count() as i64
        }));
        registry.register(
            "inactive_validators",
            "Number of inactive validators",
            closure,
        );

        // Total inactive stake
        let bc = Arc::clone(&blockchain);
        let closure = NumericClosureMetric::new_gauge(Box::new(move || {
            let blockchain = bc.read();
            let Some(contract) = blockchain.get_staking_contract_if_complete(None) else {
                return 0;
            };

            let txn = blockchain.read_transaction();
            let data_store = blockchain.get_staking_contract_store();
            let stake = contract
                .iter_validators(&data_store.read(&txn))
                .filter(|validator| validator.inactive_from.is_some())
                .map(|validator| validator.total_stake)
                .sum::<Coin>();
            u64::from(stake) as i64
        }));
        registry.register("inactive_stake", "Total inactive stake", closure);

        // Number of jailed validators
        let bc = Arc::clone(&blockchain);
        let closure = NumericClosureMetric::new_gauge(Box::new(move || {
            let blockchain = bc.read();
            let Some(contract) = blockchain.get_staking_contract_if_complete(None) else {
                return 0;
            };

            let txn = blockchain.read_transaction();
            let data_store = blockchain.get_staking_contract_store();
            contract
                .iter_validators(&data_store.read(&txn))
                .filter(|validator| validator.jailed_from.is_some())
                .count() as i64
        }));
        registry.register("jailed_validators", "Number of jailed validators", closure);

        // Total jailed stake
        let bc = Arc::clone(&blockchain);
        let closure = NumericClosureMetric::new_gauge(Box::new(move || {
            let blockchain = bc.read();
            let Some(contract) = blockchain.get_staking_contract_if_complete(None) else {
                return 0;
            };

            let txn = blockchain.read_transaction();
            let data_store = blockchain.get_staking_contract_store();
            let stake = contract
                .iter_validators(&data_store.read(&txn))
                .filter(|validator| validator.jailed_from.is_some())
                .map(|validator| validator.total_stake)
                .sum::<Coin>();
            u64::from(stake) as i64
        }));
        registry.register("jailed_stake", "Total jailed stake", closure);
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

        let bc = blockchain.clone();
        let closure =
            NumericClosureMetric::new_gauge(Box::new(move || bc.read().block_number() as i64));
        sub_registry.register("block_number", "Number of latest block", closure);

        let bc = blockchain.clone();
        let closure =
            NumericClosureMetric::new_gauge(Box::new(move || bc.read().batch_number() as i64));
        sub_registry.register("batch_number", "Number of latest batch", closure);

        let bc = blockchain.clone();
        let closure =
            NumericClosureMetric::new_gauge(Box::new(move || bc.read().epoch_number() as i64));
        sub_registry.register("epoch_number", "Number of latest epoch", closure);

        // Block timestamps are in milliseconds, Prometheus expects seconds (e.g. for `time() - x`).
        let bc = blockchain.clone();
        let closure = NumericClosureMetric::new_gauge(Box::new(move || {
            bc.read().timestamp() as f64 / 1000.0
        }));
        sub_registry.register_with_unit(
            "head_timestamp",
            "Timestamp of the latest block, in seconds since the Unix epoch",
            Unit::Seconds,
            closure,
        );

        let closure = NumericClosureMetric::new_gauge(Box::new(move || {
            blockchain
                .read()
                .current_validators()
                .map(|validators| validators.num_validators())
                .unwrap_or(0) as i64
        }));
        sub_registry.register(
            "elected_validators",
            "Number of elected validators",
            closure,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nimiq_blockchain::Blockchain;
    use nimiq_blockchain_interface::AbstractBlockchain;
    use nimiq_test_log::test;
    use nimiq_test_utils::block_production::TemporaryBlockProducer;
    use parking_lot::RwLock;
    use prometheus_client::{encoding::text::encode, registry::Registry};

    use super::BlockMetrics;

    /// Registers the chain metrics like the metrics server does and encodes them.
    fn encode_metrics(blockchain: &Arc<RwLock<Blockchain>>) -> String {
        let mut registry = Registry::default();
        BlockMetrics::register(
            registry.sub_registry_with_prefix("nimiq"),
            Arc::clone(blockchain).into(),
        );
        let mut encoded = String::new();
        encode(&mut encoded, &registry).unwrap();
        encoded
    }

    /// Returns the value of the sample `name` (without labels), if there is one.
    fn sample(encoded: &str, name: &str) -> Option<f64> {
        encoded.lines().find_map(|line| {
            let value = line.strip_prefix(name)?.strip_prefix(' ')?;
            Some(value.parse().unwrap())
        })
    }

    #[test]
    fn exports_the_head_timestamp_in_seconds() {
        let producer = TemporaryBlockProducer::new();
        producer.next_block(vec![], false);
        let timestamp_ms = producer.blockchain.read().timestamp();

        let encoded = encode_metrics(&producer.blockchain);

        assert!(encoded.contains("# UNIT nimiq_blockchain_head_timestamp_seconds seconds"));
        assert_eq!(
            sample(&encoded, "nimiq_blockchain_head_timestamp_seconds"),
            Some(timestamp_ms as f64 / 1000.0)
        );
        assert_eq!(sample(&encoded, "nimiq_blockchain_timestamp"), None);
    }
}
