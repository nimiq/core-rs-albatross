use std::sync::Arc;

use parking_lot::RwLock;

use nimiq_blockchain::Blockchain;
use nimiq_consensus::sync::history::HistorySync;
use nimiq_consensus::Consensus as AbstractConsensus;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_genesis_builder::GenesisInfo;
use nimiq_network_interface::network::Network as NetworkInterface;
use nimiq_network_mock::MockHub;
use nimiq_primitives::networks::NetworkId;
use nimiq_utils::time::OffsetTime;
use nimiq_zkp_prover::ZKPComponent;

use crate::test_network::TestNetwork;
use crate::zkp_test_data::zkp_test_exe;

pub struct Node<N: NetworkInterface + TestNetwork> {
    pub network: Arc<N>,
    pub blockchain: Arc<RwLock<Blockchain>>,
    pub consensus: Option<AbstractConsensus<N>>,
}

impl<N: NetworkInterface + TestNetwork> Node<N> {
    pub async fn new(
        peer_id: u64,
        genesis_info: GenesisInfo,
        hub: &mut Option<MockHub>,
        is_prover_active: bool,
    ) -> Self {
        let env = VolatileEnvironment::new(14).unwrap();
        let clock = Arc::new(OffsetTime::new());
        let blockchain = Arc::new(RwLock::new(
            Blockchain::with_genesis(
                env.clone(),
                Arc::clone(&clock),
                NetworkId::UnitAlbatross,
                genesis_info.block,
                genesis_info.accounts,
            )
            .unwrap(),
        ));

        let network = N::build_network(peer_id, genesis_info.hash, hub).await;
        let zkp_proxy = ZKPComponent::new(
            Arc::clone(&blockchain),
            Arc::clone(&network),
            is_prover_active,
            Some(zkp_test_exe()),
            env.clone(),
        )
        .await;

        let sync_protocol = HistorySync::<N>::new(
            Arc::clone(&blockchain),
            Arc::clone(&network),
            network.subscribe_events(),
        );
        let consensus = AbstractConsensus::<N>::with_min_peers(
            env,
            Arc::clone(&blockchain),
            Arc::clone(&network),
            Box::pin(sync_protocol),
            1,
            zkp_proxy.proxy(),
        )
        .await;

        Node {
            network,
            blockchain,
            consensus: Some(consensus),
        }
    }

    pub fn consume(&mut self) {
        if let Some(consensus) = self.consensus.take() {
            tokio::spawn(consensus);
        }
    }
}
