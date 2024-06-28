use std::{sync::Arc, time::Duration};

use futures::StreamExt;
use nimiq_blockchain::BlockProducer;
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_bls::KeyPair as BLSKeyPair;
use nimiq_database::mdbx::MdbxDatabase;
use nimiq_genesis_builder::GenesisBuilder;
use nimiq_keys::{Address, KeyPair, SecureGenerate};
use nimiq_network_mock::{MockHub, MockNetwork};
use nimiq_primitives::{networks::NetworkId, policy::Policy};
use nimiq_test_log::test;
use nimiq_test_utils::{
    blockchain::{produce_macro_blocks, signing_key, voting_key},
    node::Node,
    validator::seeded_rng,
};
use nimiq_time::{interval, sleep};
use nimiq_utils::spawn::spawn;

#[test(tokio::test(flavor = "multi_thread", worker_threads = 4))]
#[ignore]
async fn test_request_component() {
    let mut hub = Some(MockHub::default());
    let env =
        MdbxDatabase::new_volatile(Default::default()).expect("Could not open a volatile database");

    // Generate genesis block.
    let key = KeyPair::generate(&mut seeded_rng(0));
    let sgn_key = KeyPair::generate(&mut seeded_rng(0));
    let vtn_key = BLSKeyPair::generate(&mut seeded_rng(0));

    let genesis = GenesisBuilder::default()
        .with_network(NetworkId::UnitAlbatross)
        .with_genesis_validator(
            Address::from(&key),
            sgn_key.public,
            vtn_key.public_key,
            Address::default(),
            None,
            None,
            false,
        )
        .generate(env)
        .unwrap();

    let mut node1 =
        Node::<MockNetwork>::history_with_genesis_info(1, genesis.clone(), &mut hub, false).await;
    let mut node2 =
        Node::<MockNetwork>::history_with_genesis_info(2, genesis.clone(), &mut hub, false).await;

    let producer1 = BlockProducer::new(signing_key(), voting_key());

    node1.consume();
    node2.consume();

    // let node1 produce blocks again
    {
        let prod_blockchain = Arc::clone(&node1.blockchain);
        spawn(async move {
            loop {
                produce_macro_blocks(&producer1, &prod_blockchain, 1);
                sleep(Duration::from_secs(5)).await;
            }
        });
    }

    let mut connected = false;
    let mut interval = interval(Duration::from_secs(1));
    loop {
        if node1.blockchain.read().block_number() > 200 + Policy::genesis_block_number()
            && !connected
        {
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

        interval.next().await;
    }
}
