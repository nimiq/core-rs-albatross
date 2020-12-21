use std::{
    sync::Arc,
    time::Duration,
};

use futures::stream::StreamExt;
use async_trait::async_trait;

use beserial::Deserialize;
use nimiq_bls::{KeyPair, SecretKey};
use nimiq_blockchain_albatross::Blockchain;
use nimiq_block_production_albatross::BlockProducer;
use nimiq_mempool::{Mempool, MempoolConfig};
use nimiq_primitives::networks::NetworkId;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_block_production_albatross::test_utils::produce_macro_blocks;
use nimiq_consensus_albatross::sync::{
    request_component::BlockRequestComponent,
    history::HistorySync,
    block_queue::{BlockQueue, BlockTopic},
};
use nimiq_network_mock::{MockHub, MockNetwork, MockPeer};
use nimiq_network_interface::network::Network;
use nimiq_primitives::policy;
use nimiq_consensus_albatross::{Consensus, SyncProtocol};
use nimiq_consensus_albatross::error::SyncError;


/// Secret key of validator. Tests run with `network-primitives/src/genesis/unit-albatross.toml`
const SECRET_KEY: &str =
    "196ffdb1a8acc7cbd76a251aeac0600a1d68b3aba1eba823b5e4dc5dbdcdc730afa752c05ab4f6ef8518384ad514f403c5a088a22b17bf1bc14f8ff8decc2a512c0a200f68d7bdf5a319b30356fe8d1d75ef510aed7a8660968c216c328a0000";


struct DummySync;

#[async_trait]
impl SyncProtocol<MockNetwork> for DummySync {
    async fn perform_sync(&self, _consensus: Arc<Consensus<MockNetwork>>) -> Result<(), SyncError> {
        Ok(())
    }

    fn is_established(&self) -> bool {
        false
    }
}


struct Node {
    network: Arc<MockNetwork>,
    blockchain: Arc<Blockchain>,
    block_queue: BlockQueue<BlockRequestComponent<MockPeer>>,
    mempool: Arc<Mempool>,
    consensus: Arc<Consensus<MockNetwork>>,
}

impl Node {
    pub async fn new(hub: &mut MockHub) -> Self {
        let env = VolatileEnvironment::new(10).unwrap();

        let blockchain = Arc::new(Blockchain::new(env.clone(), NetworkId::UnitAlbatross).unwrap());

        let network = Arc::new(hub.new_network());

        let history_sync = HistorySync::<MockNetwork>::new(Arc::clone(&blockchain), network.subscribe_events());

        let request_component = BlockRequestComponent::new(history_sync.boxed());

        let block_stream = network.subscribe::<BlockTopic>(&BlockTopic::default())
            .await
            .unwrap()
            .map(|(block, _peer_id)| block)
            .boxed();

        let block_queue = BlockQueue::new(
            Default::default(),
            Arc::clone(&blockchain),
            request_component,
            block_stream,
        );

        let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());

        let consensus = Consensus::new(env, Arc::clone(&blockchain), Arc::clone(&mempool), Arc::clone(&network), DummySync).unwrap();

        Node {
            network,
            blockchain,
            block_queue,
            mempool,
            consensus,
        }
    }
}


fn consume_block_queue(block_queue: BlockQueue<BlockRequestComponent<MockPeer>>) {
    tokio::spawn(async move {
        block_queue.for_each(|_| async {}).await;
    });
}


#[tokio::test(core_threads = 4)]
#[ignore]
async fn test_request_component() {
    simple_logger::init_by_env();

    let mut hub = MockHub::default();

    let node1 = Node::new(&mut hub).await;
    let node2 = Node::new(&mut hub).await;

    let keypair1 = KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());
    let producer1 = BlockProducer::new(Arc::clone(&node1.blockchain), Arc::clone(&node1.mempool), keypair1);

    let num_macro_blocks = (policy::BATCHES_PER_EPOCH + 1) as usize;
    //produce_macro_blocks(num_macro_blocks, &producer1, &node1.blockchain);

    consume_block_queue(node1.block_queue);
    consume_block_queue(node2.block_queue);

    // let node1 produce blocks again
    {
        let prod_blockchain = Arc::clone(&node1.blockchain);
        tokio::spawn(async move {
            loop {
                produce_macro_blocks(1, &producer1, &prod_blockchain);
                tokio::time::delay_for(Duration::from_secs(5)).await;
            }
        });
    }

    let mut connected = false;
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        if node1.blockchain.block_number() > 200 && !connected {
            log::info!("Connecting node2 to node 1");
            node2.network.dial_mock(&node1.network);
            connected = true;
        }

        log::info!("Node1: at #{} - {}", node1.blockchain.block_number(), node1.blockchain.head_hash());
        log::info!("Node2: at #{} - {}", node2.blockchain.block_number(), node2.blockchain.head_hash());

        interval.tick().await;
    }
}
