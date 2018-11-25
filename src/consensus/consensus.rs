use crate::consensus::base::blockchain::Blockchain;
use crate::consensus::base::mempool::Mempool;
use crate::consensus::networks::NetworkId;
use crate::network::{Network, NetworkConfig, NetworkTime};
use crate::utils::db::Environment;
use std::sync::Arc;

pub struct Consensus {
    pub blockchain: Arc<Blockchain<'static>>,
    pub mempool: Arc<Mempool<'static>>,
    pub network: Arc<Network>
}

impl Consensus {
    pub fn new(env: &'static Environment, network_id: NetworkId, network_config: NetworkConfig) -> Self {
        let network_time = Arc::new(NetworkTime::new());
        let blockchain = Arc::new(Blockchain::new(env, network_id, network_time.clone()));
        let mempool = Mempool::new(blockchain.clone());
        let network = Network::new(blockchain.clone(), network_config, network_time);

        Consensus {
            blockchain,
            mempool,
            network
        }
    }


}
