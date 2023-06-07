use std::sync::Arc;

use nimiq_mempool::mempool::Mempool;
use prometheus_client::registry::Registry;

use crate::NumericClosureMetric;

pub struct MempoolMetrics {}

impl MempoolMetrics {
    pub fn register(registry: &mut Registry, mempool: Arc<Mempool>) {
        let sub_registry = registry.sub_registry_with_prefix("mempool");

        mempool.metrics().register(sub_registry);

        let closure =
            NumericClosureMetric::new_gauge(Box::new(move || mempool.num_transactions() as i64));
        sub_registry.register("tx_count", "Txs currently in mempool", closure);
    }
}
