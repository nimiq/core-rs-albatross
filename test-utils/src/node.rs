use std::sync::Arc;

use parking_lot::RwLock;

use nimiq_blockchain::Blockchain;
use nimiq_consensus::sync::history::HistorySync;
use nimiq_consensus::Consensus;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_network_interface::network::Network;
use nimiq_network_mock::{MockHub, MockNetwork};
use nimiq_primitives::networks::NetworkId;
use nimiq_utils::time::OffsetTime;

pub struct Node {
    pub network: Arc<MockNetwork>,
    pub blockchain: Arc<RwLock<Blockchain>>,
    pub consensus: Option<Consensus<MockNetwork>>,
}

impl Node {
    pub async fn new(hub: &mut MockHub) -> Self {
        let time = Arc::new(OffsetTime::new());
        let env = VolatileEnvironment::new(10).unwrap();

        let blockchain = Arc::new(RwLock::new(
            Blockchain::new(env.clone(), NetworkId::UnitAlbatross, time).unwrap(),
        ));

        let network = Arc::new(hub.new_network());

        let history_sync =
            HistorySync::<MockNetwork>::new(Arc::clone(&blockchain), network.subscribe_events());

        let consensus = Consensus::from_network(
            env,
            Arc::clone(&blockchain),
            Arc::clone(&network),
            Box::pin(history_sync),
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
