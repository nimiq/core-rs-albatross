use std::sync::Arc;

use nimiq_blockchain::Blockchain;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
#[cfg(feature = "extended-staking")]
use nimiq_primitives::coin::Coin;
use parking_lot::RwLock;
use prometheus_client::registry::{Registry, Unit};
#[cfg(feature = "extended-staking")]
use prometheus_client::{collector::Collector, encoding::DescriptorEncoder, metrics::MetricType};

use crate::{NumericClosureMetric, OptionalClosureMetric};

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

        // The staking gauges are unknown, and therefore not emitted, while the accounts trie is
        // incomplete (during state sync).

        // Number of active validators
        let bc = Arc::clone(&blockchain);
        let closure = OptionalClosureMetric::new_gauge(Box::new(move || {
            bc.read()
                .get_staking_contract_if_complete(None)
                .map(|contract| contract.active_validators.len() as i64)
        }));
        sub_registry.register("active_validators", "Number of active validators", closure);

        // Total stake
        let bc = Arc::clone(&blockchain);
        let closure = OptionalClosureMetric::new_gauge(Box::new(move || {
            bc.read()
                .get_staking_contract_if_complete(None)
                .map(|contract| u64::from(contract.balance) as i64)
        }));
        sub_registry.register("total_stake", "Total stake", closure);

        #[cfg(feature = "extended-staking")]
        Self::register_extended_staking(sub_registry, blockchain);
    }

    #[cfg(feature = "extended-staking")]
    fn register_extended_staking(registry: &mut Registry, blockchain: Arc<RwLock<Blockchain>>) {
        registry.register_collector(Box::new(ExtendedStakingCollector { blockchain }));
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

        // Unknown, and therefore not emitted, while the validator set is not known.
        let closure = OptionalClosureMetric::new_gauge(Box::new(move || {
            blockchain
                .read()
                .current_validators()
                .map(|validators| validators.num_validators() as i64)
        }));
        sub_registry.register(
            "elected_validators",
            "Number of elected validators",
            closure,
        );
    }
}

/// The extended staking gauges. They are computed together so that a scrape takes the
/// blockchain lock once and walks the validators once, instead of once per gauge.
#[cfg(feature = "extended-staking")]
struct ExtendedStakingCollector {
    blockchain: Arc<RwLock<Blockchain>>,
}

#[cfg(feature = "extended-staking")]
struct ExtendedStakingTotals {
    active_stake: Coin,
    inactive_validators: u64,
    inactive_stake: Coin,
    jailed_validators: u64,
    jailed_stake: Coin,
}

#[cfg(feature = "extended-staking")]
impl ExtendedStakingCollector {
    /// Returns `None` while the accounts trie is incomplete.
    fn totals(&self) -> Option<ExtendedStakingTotals> {
        let blockchain = self.blockchain.read();
        let contract = blockchain.get_staking_contract_if_complete(None)?;
        let txn = blockchain.read_transaction();
        let data_store = blockchain.get_staking_contract_store();

        let mut totals = ExtendedStakingTotals {
            active_stake: contract.active_validators.values().cloned().sum(),
            inactive_validators: 0,
            inactive_stake: Coin::ZERO,
            jailed_validators: 0,
            jailed_stake: Coin::ZERO,
        };
        for validator in contract.iter_validators(&data_store.read(&txn)) {
            if validator.inactive_from.is_some() {
                totals.inactive_validators += 1;
                totals.inactive_stake += validator.total_stake;
            }
            if validator.jailed_from.is_some() {
                totals.jailed_validators += 1;
                totals.jailed_stake += validator.total_stake;
            }
        }
        Some(totals)
    }
}

#[cfg(feature = "extended-staking")]
impl std::fmt::Debug for ExtendedStakingCollector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtendedStakingCollector")
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "extended-staking")]
impl Collector for ExtendedStakingCollector {
    fn encode(&self, mut encoder: DescriptorEncoder) -> Result<(), std::fmt::Error> {
        let totals = self.totals();
        let gauges = [
            (
                "active_stake",
                "Total active stake",
                totals.as_ref().map(|t| u64::from(t.active_stake)),
            ),
            (
                "inactive_validators",
                "Number of inactive validators",
                totals.as_ref().map(|t| t.inactive_validators),
            ),
            (
                "inactive_stake",
                "Total inactive stake",
                totals.as_ref().map(|t| u64::from(t.inactive_stake)),
            ),
            (
                "jailed_validators",
                "Number of jailed validators",
                totals.as_ref().map(|t| t.jailed_validators),
            ),
            (
                "jailed_stake",
                "Total jailed stake",
                totals.as_ref().map(|t| u64::from(t.jailed_stake)),
            ),
        ];
        for (name, help, value) in gauges {
            // Like `OptionalClosureMetric`: describe the gauge, but emit no sample while the
            // value is unknown.
            let mut metric_encoder =
                encoder.encode_descriptor(name, help, None, MetricType::Gauge)?;
            if let Some(value) = value {
                metric_encoder.encode_gauge(&(value as i64))?;
            }
        }
        Ok(())
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

    /// The staking gauges, all of which depend on the accounts trie.
    fn staking_gauges() -> Vec<&'static str> {
        #[allow(unused_mut)]
        let mut gauges = vec![
            "nimiq_staking_active_validators",
            "nimiq_staking_total_stake",
        ];
        #[cfg(feature = "extended-staking")]
        gauges.extend([
            "nimiq_staking_active_stake",
            "nimiq_staking_inactive_validators",
            "nimiq_staking_inactive_stake",
            "nimiq_staking_jailed_validators",
            "nimiq_staking_jailed_stake",
        ]);
        gauges
    }

    #[test]
    fn omits_staking_gauges_while_the_accounts_trie_is_incomplete() {
        let producer = TemporaryBlockProducer::new_incomplete();

        let encoded = encode_metrics(&producer.blockchain);

        for gauge in staking_gauges() {
            assert!(
                encoded.contains(&format!("# TYPE {gauge} gauge")),
                "{gauge}"
            );
            assert_eq!(sample(&encoded, gauge), None, "{gauge}");
        }
    }

    #[test]
    fn exports_staking_gauges_once_the_accounts_trie_is_complete() {
        let producer = TemporaryBlockProducer::new();
        let num_active_validators = producer
            .blockchain
            .read()
            .get_staking_contract()
            .active_validators
            .len();

        let encoded = encode_metrics(&producer.blockchain);

        assert_eq!(
            sample(&encoded, "nimiq_staking_active_validators"),
            Some(num_active_validators as f64)
        );
        for gauge in staking_gauges() {
            assert!(sample(&encoded, gauge).is_some(), "{gauge}");
        }
    }

    #[cfg(feature = "extended-staking")]
    #[test]
    fn computes_the_extended_staking_gauges() {
        let producer = TemporaryBlockProducer::new();
        let (active_stake, num_validators) = {
            let blockchain = producer.blockchain.read();
            let contract = blockchain.get_staking_contract();
            let active_stake: nimiq_primitives::coin::Coin =
                contract.active_validators.values().cloned().sum();
            (u64::from(active_stake), contract.active_validators.len())
        };
        assert!(num_validators > 0);

        let encoded = encode_metrics(&producer.blockchain);

        assert_eq!(
            sample(&encoded, "nimiq_staking_active_stake"),
            Some(active_stake as f64)
        );
        // The unit test genesis has no inactive or jailed validators.
        for gauge in [
            "nimiq_staking_inactive_validators",
            "nimiq_staking_inactive_stake",
            "nimiq_staking_jailed_validators",
            "nimiq_staking_jailed_stake",
        ] {
            assert_eq!(sample(&encoded, gauge), Some(0.0), "{gauge}");
        }
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
