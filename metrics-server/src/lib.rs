use std::{fmt::Debug, net::SocketAddr, sync::Arc};

use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_consensus::ConsensusProxy;
use nimiq_mempool::mempool::Mempool;
use nimiq_network_interface::network::Network;
use nimiq_utils::{spawn, Credentials};
use parking_lot::RwLock;
use prometheus_client::{
    encoding::{EncodeGaugeValue, EncodeMetric, MetricEncoder},
    metrics::MetricType,
    registry::Registry,
};
#[cfg(tokio_unstable)]
use tokio_metrics::RuntimeMonitor;
use tokio_metrics::TaskMonitor;

#[cfg(tokio_unstable)]
use crate::tokio_runtime::TokioRuntimeMetrics;
use crate::{
    chain::BlockMetrics, consensus::ConsensusMetrics, mempool::MempoolMetrics,
    network::NetworkMetrics, server::metrics_server, tokio_task::TokioTaskMetrics,
};

mod chain;
mod consensus;
mod mempool;
mod network;
mod server;
#[cfg(tokio_unstable)]
mod tokio_runtime;
mod tokio_task;

/// Monitor (metrics) for a Nimiq task
#[derive(Clone)]
pub struct NimiqTaskMonitor {
    name: String,
    monitor: TaskMonitor,
}

impl NimiqTaskMonitor {
    pub fn new(name: String, monitor: TaskMonitor) -> Self {
        // Metrics name **should** be snake case according to OpenMetrics spec.
        assert!(
            name.chars()
                .all(|char| char.is_alphanumeric() || char == '_'),
            "Metrics names should not contain non-alphanumeric characters except for underscores"
        );
        Self { name, monitor }
    }
}

struct NumericClosureMetric<T: EncodeGaugeValue + Sized + Debug> {
    metric_type: MetricType,
    lambda: Box<dyn Fn() -> T + Sync + Send>,
}

impl<T: EncodeGaugeValue + Sized + Debug> NumericClosureMetric<T> {
    fn new(
        metric_type: MetricType,
        lambda: Box<dyn Fn() -> T + Sync + Send>,
    ) -> NumericClosureMetric<T> {
        NumericClosureMetric {
            metric_type,
            lambda,
        }
    }

    pub fn new_gauge(lambda: Box<dyn Fn() -> T + Sync + Send>) -> NumericClosureMetric<T> {
        NumericClosureMetric::new(MetricType::Gauge, lambda)
    }
}

impl<T: EncodeGaugeValue + Sized + Debug> EncodeMetric for NumericClosureMetric<T> {
    fn encode(&self, mut encoder: MetricEncoder) -> Result<(), std::fmt::Error> {
        encoder.encode_gauge(&(self.lambda)())?;

        Ok(())
    }

    fn metric_type(&self) -> MetricType {
        self.metric_type
    }
}

impl<T: EncodeGaugeValue + Sized + Debug> Debug for NumericClosureMetric<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NumericClosureMetric")
            .field("metric_type", &self.metric_type)
            .field("value", &(self.lambda)())
            .finish()
    }
}

pub fn start_metrics_server<TNetwork: Network>(
    addr: SocketAddr,
    credentials: Option<Credentials>,
    blockchain_proxy: BlockchainProxy,
    mempool: Option<Arc<Mempool>>,
    consensus_proxy: ConsensusProxy<TNetwork>,
    network: Arc<nimiq_network_libp2p::Network>,
    task_monitors: &[NimiqTaskMonitor],
) {
    let mut registry = Registry::default();
    let nimiq_registry = registry.sub_registry_with_prefix("nimiq");

    BlockMetrics::register(nimiq_registry, blockchain_proxy);
    ConsensusMetrics::register(nimiq_registry, consensus_proxy);
    NetworkMetrics::register(nimiq_registry, network);

    if let Some(mempool) = mempool {
        MempoolMetrics::register(nimiq_registry, mempool);
    }

    // Setup the task metrics
    let task_metrics = Arc::new(RwLock::new(TokioTaskMetrics::new()));
    task_metrics.write().register(
        nimiq_registry,
        &task_monitors
            .iter()
            .map(|e| e.name.clone())
            .collect::<Vec<String>>()[..],
    );

    #[cfg(tokio_unstable)]
    {
        // Setup the tokio runtime metrics
        let handle = tokio::runtime::Handle::current();
        let tokio_rt_monitor = RuntimeMonitor::new(&handle);
        let mut tokio_rt_metrics = TokioRuntimeMetrics::new();
        tokio_rt_metrics.register(nimiq_registry);
        let tokio_rt_metrics = Arc::new(RwLock::new(tokio_rt_metrics));

        // Spawn Tokio runtime metrics updater
        spawn(TokioRuntimeMetrics::update_metric_values(
            tokio_rt_metrics,
            tokio_rt_monitor,
        ));
    }

    // Spawn the metrics server
    spawn(async move { metrics_server(addr, registry, credentials).await.unwrap() });

    // Spawn Tokio task monitor updaters
    for task_monitor in task_monitors {
        spawn({
            let task_metrics = Arc::clone(&task_metrics);
            let task_monitor = task_monitor.clone();
            async move {
                TokioTaskMetrics::update_metric_values(
                    task_metrics,
                    &task_monitor.name,
                    task_monitor.monitor.clone(),
                )
                .await;
            }
        });
    }
}
