use std::{sync::Arc, time::Duration};

use nimiq_block_production::BlockProducer;
use nimiq_blockchain::AbstractBlockchain;
use nimiq_bls::KeyPair as BLSKeyPair;
use nimiq_build_tools::genesis::GenesisBuilder;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_keys::{Address, KeyPair, SecureGenerate};
use nimiq_network_mock::{MockHub, MockNetwork};
use nimiq_test_utils::blockchain::{produce_macro_blocks, signing_key, voting_key};
use nimiq_test_utils::node::Node;
use nimiq_test_utils::validator::seeded_rng;

#[ignore]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_request_component() {
    //simple_logger::init_by_env();

    let mut hub = Some(MockHub::default());
    let env = VolatileEnvironment::new(10).expect("Could not open a volatile database");

    // Generate genesis block.
    let key = KeyPair::generate(&mut seeded_rng(0));
    let sgn_key = KeyPair::generate(&mut seeded_rng(0));
    let vtn_key = BLSKeyPair::generate(&mut seeded_rng(0));

    let genesis = GenesisBuilder::default()
        .with_genesis_validator(
            Address::from(&key),
            sgn_key.public,
            vtn_key.public_key,
            Address::default(),
        )
        .generate(env)
        .unwrap();

    let mut node1 = Node::<MockNetwork>::new(1, genesis.clone(), &mut hub).await;
    let mut node2 = Node::<MockNetwork>::new(2, genesis.clone(), &mut hub).await;

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
