use std::net::SocketAddr;

use prometheus_client::encoding::text::{Encode, EncodeMetric, Encoder};
use prometheus_client::registry::Registry;

use parking_lot::RwLock;

use crate::chain::BlockMetrics;
use crate::consensus::ConsensusMetrics;
use crate::mempool::MempoolMetrics;
use crate::network::NetworkMetrics;
use crate::server::metrics_server;
use nimiq_blockchain::Blockchain;
use nimiq_consensus::ConsensusProxy;
use nimiq_mempool::mempool::Mempool;
use nimiq_network_interface::network::Network;
use prometheus_client::metrics::MetricType;
use std::sync::Arc;

mod chain;
mod consensus;
mod mempool;
mod network;
mod server;

struct NumericClosureMetric<T: Encode + Sized> {
    metric_type: MetricType,
    lambda: Box<dyn Fn() -> T + Sync + Send>,
}

impl<T: Encode + Sized> NumericClosureMetric<T> {
    fn new(
        metric_type: MetricType,
        lambda: Box<dyn Fn() -> T + Sync + Send>,
    ) -> NumericClosureMetric<T> {
        NumericClosureMetric {
            metric_type,
            lambda,
        }
    }

    pub fn new_counter(lambda: Box<dyn Fn() -> T + Sync + Send>) -> NumericClosureMetric<T> {
        NumericClosureMetric::new(MetricType::Counter, lambda)
    }

    pub fn new_gauge(lambda: Box<dyn Fn() -> T + Sync + Send>) -> NumericClosureMetric<T> {
        NumericClosureMetric::new(MetricType::Gauge, lambda)
    }
}

impl<T: Encode + Sized> EncodeMetric for NumericClosureMetric<T> {
    fn encode(&self, mut encoder: Encoder) -> Result<(), std::io::Error> {
        encoder
            .no_suffix()?
            .no_bucket()?
            .encode_value((self.lambda)())?
            .no_exemplar()?;

        Ok(())
    }

    fn metric_type(&self) -> prometheus_client::metrics::MetricType {
        self.metric_type
    }
}

pub fn start_metrics_server<TNetwork: Network>(
    addr: SocketAddr,
    blockchain: Arc<RwLock<Blockchain>>,
    mempool: Option<Arc<Mempool>>,
    consensus_proxy: ConsensusProxy<TNetwork>,
    network: Arc<nimiq_network_libp2p::Network>,
) {
    let mut registry = Registry::default();
    let nimiq_registry = registry.sub_registry_with_prefix("nimiq");

    BlockMetrics::register(nimiq_registry, blockchain);
    ConsensusMetrics::register(nimiq_registry, consensus_proxy);
    NetworkMetrics::register(nimiq_registry, network);

    if let Some(mempool) = mempool {
        MempoolMetrics::register(nimiq_registry, mempool);
    }

    tokio::spawn(metrics_server(addr, registry));
}
