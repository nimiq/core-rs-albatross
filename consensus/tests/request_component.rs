use std::{sync::Arc, time::Duration};

use parking_lot::RwLock;

use nimiq_block_production::BlockProducer;
use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_consensus::sync::history::HistorySync;
use nimiq_consensus::Consensus;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_network_interface::network::Network;
use nimiq_network_mock::{MockHub, MockNetwork};
use nimiq_primitives::networks::NetworkId;
use nimiq_test_utils::blockchain::{produce_macro_blocks, signing_key, voting_key};
use nimiq_utils::time::OffsetTime;

struct Node {
    network: Arc<MockNetwork>,
    blockchain: Arc<RwLock<Blockchain>>,
    consensus: Option<Consensus<MockNetwork>>,
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

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_request_component() {
    //simple_logger::init_by_env();

    let mut hub = MockHub::default();

    let mut node1 = Node::new(&mut hub).await;
    let mut node2 = Node::new(&mut hub).await;

    let producer1 = BlockProducer::new(signing_key(), voting_key());

    //let num_macro_blocks = (policy::BATCHES_PER_EPOCH + 1) as usize;
    //produce_macro_blocks(num_macro_blocks, &producer1, &node1.blockchain);

    node1.consume();
    node2.consume();

    // let node1 produce blocks again
    {
        let prod_blockchain = Arc::clone(&node1.blockchain);
        tokio::spawn(async move {
            loop {
                produce_macro_blocks(1, &producer1, &prod_blockchain);
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
    }

    let mut connected = false;
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        if node1.blockchain.read().block_number() > 200 && !connected {
            log::info!("Connecting node2 to node 1");
            node2.network.dial_mock(&node1.network);
            connected = true;
        }

        log::info!(
            "Node1: at #{} - {}",
            node1.blockchain.read().block_number(),
            node1.blockchain.read().head_hash()
        );
        log::info!(
            "Node2: at #{} - {}",
            node2.blockchain.read().block_number(),
            node2.blockchain.read().head_hash()
        );

        interval.tick().await;
    }
}
