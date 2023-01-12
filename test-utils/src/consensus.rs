use crate::node::Node;
use crate::test_network::TestNetwork;

use parking_lot::RwLock;
use std::sync::Arc;

use nimiq_blockchain::Blockchain;
use nimiq_consensus::Consensus as AbstractConsensus;
use nimiq_genesis_builder::GenesisInfo;
use nimiq_network_interface::network::Network as NetworkInterface;
use nimiq_network_mock::MockHub;

pub async fn consensus<N: TestNetwork + NetworkInterface>(
    peer_id: u64,
    genesis_info: GenesisInfo,
    hub: &mut Option<MockHub>,
    is_prover_active: bool,
) -> (AbstractConsensus<N>, Arc<RwLock<Blockchain>>) {
    let node =
        Node::<N>::history_with_genesis_info(peer_id, genesis_info, hub, is_prover_active).await;
    (
        node.consensus.expect("Could not create consensus"),
        node.blockchain,
    )
}
