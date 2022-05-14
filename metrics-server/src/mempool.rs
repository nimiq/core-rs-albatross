use prometheus_client::registry::Registry;

use crate::NumericClosureMetric;
use nimiq_mempool::mempool::Mempool;
use std::sync::Arc;

pub struct MempoolMetrics {}

impl MempoolMetrics {
    pub fn register(registry: &mut Registry, mempool: Arc<Mempool>) {
        let sub_registry = registry.sub_registry_with_prefix("mempool");

        let mempool_clone = mempool.clone();
        let closure = Box::new(NumericClosureMetric::new_gauge(Box::new(move || {
            mempool_clone.num_transactions() as u32
        })));
        sub_registry.register("tx_count", "Txs currently in mempool", closure);

        let closure = Box::new(NumericClosureMetric::new_counter(Box::new(move || {
            mempool.metrics().expired_tx_count() as u32
        })));
        sub_registry.register(
            "expired_tx_count",
            "Number of tx removed from mempool because they became invalid",
            closure,
        );
    }
}
