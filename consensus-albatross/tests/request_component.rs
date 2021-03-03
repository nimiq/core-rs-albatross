use std::{sync::Arc, time::Duration};

use futures::stream::StreamExt;

use beserial::Deserialize;
use nimiq_block_production_albatross::test_utils::produce_macro_blocks;
use nimiq_block_production_albatross::BlockProducer;
use nimiq_blockchain_albatross::Blockchain;
use nimiq_bls::{KeyPair, SecretKey};
use nimiq_consensus_albatross::sync::history::HistorySync;
use nimiq_consensus_albatross::Consensus;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_mempool::{Mempool, MempoolConfig};
use nimiq_network_interface::network::Network;
use nimiq_network_mock::{MockHub, MockNetwork};
use nimiq_primitives::networks::NetworkId;

/// Secret key of validator. Tests run with `network-primitives/src/genesis/unit-albatross.toml`
const SECRET_KEY: &str =
    "196ffdb1a8acc7cbd76a251aeac0600a1d68b3aba1eba823b5e4dc5dbdcdc730afa752c05ab4f6ef8518384ad514f403c5a088a22b17bf1bc14f8ff8decc2a512c0a200f68d7bdf5a319b30356fe8d1d75ef510aed7a8660968c216c328a0000";

struct Node {
    network: Arc<MockNetwork>,
    blockchain: Arc<Blockchain>,
    mempool: Arc<Mempool>,
    consensus: Option<Consensus<MockNetwork>>,
}

impl Node {
    pub async fn new(hub: &mut MockHub) -> Self {
        let env = VolatileEnvironment::new(10).unwrap();

        let blockchain = Arc::new(Blockchain::new(env.clone(), NetworkId::UnitAlbatross).unwrap());

        let network = Arc::new(hub.new_network());

        let history_sync =
            HistorySync::<MockNetwork>::new(Arc::clone(&blockchain), network.subscribe_events());

        let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());

        let consensus = Consensus::from_network(
            env,
            Arc::clone(&blockchain),
            Arc::clone(&mempool),
            Arc::clone(&network),
            history_sync.boxed(),
        )
        .await;

        Node {
            network,
            blockchain,
            mempool,
            consensus: Some(consensus),
        }
    }

    pub fn consume(&mut self) {
        if let Some(c) = self.consensus.take() {
            tokio::spawn(async move {
                c.for_each(|_| async {}).await;
            });
        }
    }
}

#[ignore]
#[tokio::test(core_threads = 4)]
async fn test_request_component() {
    //simple_logger::init_by_env();

    let mut hub = MockHub::default();

    let mut node1 = Node::new(&mut hub).await;
    let mut node2 = Node::new(&mut hub).await;

    let keypair1 =
        KeyPair::from(SecretKey::deserialize_from_vec(&hex::decode(SECRET_KEY).unwrap()).unwrap());
    let producer1 = BlockProducer::new(
        Arc::clone(&node1.blockchain),
        Arc::clone(&node1.mempool),
        keypair1,
    );

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

        log::info!(
            "Node1: at #{} - {}",
            node1.blockchain.block_number(),
            node1.blockchain.head_hash()
        );
        log::info!(
            "Node2: at #{} - {}",
            node2.blockchain.block_number(),
            node2.blockchain.head_hash()
        );

        interval.tick().await;
    }
}
