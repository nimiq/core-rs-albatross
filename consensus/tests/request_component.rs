use std::{sync::Arc, time::Duration};

use beserial::Deserialize;
use nimiq_block_production::BlockProducer;
use nimiq_blockchain::AbstractBlockchain;
use nimiq_bls::{KeyPair, SecretKey};
use nimiq_network_mock::MockHub;
use nimiq_test_utils::blockchain::{produce_macro_blocks, signing_key, voting_key};
use nimiq_test_utils::node::Node;

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
