use std::sync::Arc;

use nimiq_blockchain::Blockchain;
use nimiq_blockchain_proxy::BlockchainProxy;
use nimiq_bls::cache::PublicKeyCache;
use nimiq_consensus::{sync::syncer_proxy::SyncerProxy, Consensus};
use nimiq_network_interface::network::Network;
use nimiq_zkp_component::ZKPComponent;
use parking_lot::{Mutex, RwLock};

use crate::test_network::TestNetwork;

/// Given a blockchain and a network creates an instance of Consensus.
pub async fn consensus<N: Network + TestNetwork>(
    blockchain: Arc<RwLock<Blockchain>>,
    net: Arc<N>,
) -> Consensus<N> {
    let blockchain_proxy = BlockchainProxy::from(&blockchain);

    let zkp_proxy = ZKPComponent::new(blockchain_proxy.clone(), Arc::clone(&net), None)
        .await
        .proxy();

    let syncer = SyncerProxy::new_history(
        blockchain_proxy.clone(),
        Arc::clone(&net),
        Arc::new(Mutex::new(PublicKeyCache::new(10))),
        net.subscribe_events(),
    )
    .await;

    Consensus::new(blockchain_proxy, net, syncer, 0, zkp_proxy)
}
