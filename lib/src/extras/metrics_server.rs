use std::{net::SocketAddr, sync::Arc};

use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_consensus::ConsensusProxy;
#[cfg(feature = "nimiq-mempool")]
use nimiq_mempool::mempool::Mempool;
pub use nimiq_metrics_server::{install_metrics, MetricsCollector, NimiqTaskMonitor};
use nimiq_network_interface::network::Network;

pub fn start_metrics_server<TNetwork: Network>(
    addr: SocketAddr,
    blockchain_proxy: BlockchainProxy,
    #[cfg(feature = "nimiq-mempool")] mempool: Option<Arc<Mempool>>,
    consensus_proxy: ConsensusProxy<TNetwork>,
    network: Arc<nimiq_network_libp2p::Network>,
    task_monitors: &[NimiqTaskMonitor],
    collector: MetricsCollector,
) {
    #[cfg(not(feature = "nimiq-mempool"))]
    let mempool = None;
    nimiq_metrics_server::start_metrics_server(
        addr,
        blockchain_proxy,
        mempool,
        consensus_proxy,
        network,
        task_monitors,
        collector,
    );
}
