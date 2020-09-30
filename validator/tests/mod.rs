use std::sync::Arc;
use std::time::Duration;

use futures::{future, StreamExt};
use log::Level;
use rand::thread_rng;
use tokio::time;

use beserial::Deserialize;
use nimiq_blockchain_albatross::Blockchain;
use nimiq_bls::KeyPair;
use nimiq_build_tools::genesis::{GenesisBuilder, GenesisInfo};
use nimiq_consensus_albatross::sync::QuickSync;
use nimiq_consensus_albatross::Consensus;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_keys::{Address, SecureGenerate};
use nimiq_mempool::{Mempool, MempoolConfig};
use nimiq_network_mock::network::MockNetwork;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_utils::time::OffsetTime;
use nimiq_validator::validator::Validator;

fn mock_consensus(peer_id: u32, genesis_info: GenesisInfo) -> Arc<Consensus<MockNetwork>> {
    let env = VolatileEnvironment::new(10).unwrap();
    let time = Arc::new(OffsetTime::new());
    let blockchain = Arc::new(
        Blockchain::with_genesis(
            env.clone(),
            time,
            NetworkId::UnitAlbatross,
            genesis_info.block,
            genesis_info.accounts,
        )
        .unwrap(),
    );
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());
    let network = Arc::new(MockNetwork::new(peer_id));
    let sync_protocol = QuickSync::default();
    Consensus::new(env, blockchain, mempool, network, sync_protocol).unwrap()
}

fn mock_validator(
    peer_id: u32,
    signing_key: KeyPair,
    genesis_info: GenesisInfo,
) -> (
    Validator<MockNetwork, MockNetwork>,
    Arc<Consensus<MockNetwork>>,
) {
    let consensus = mock_consensus(peer_id, genesis_info);
    (
        Validator::new(
            Arc::clone(&consensus),
            Arc::clone(&consensus.network),
            signing_key,
            None,
        ),
        consensus,
    )
}

#[tokio::test]
async fn one_validator_can_create_micro_blocks() {
    simple_logger::init_with_level(Level::Info);

    let key = KeyPair::generate(&mut thread_rng());
    let genesis = GenesisBuilder::default()
        .with_genesis_validator(
            key.public_key,
            Address::default(),
            Coin::from_u64_unchecked(10000),
        )
        .generate()
        .unwrap();

    let (validator, consensus1) = mock_validator(1, key, genesis.clone());

    let consensus2 = mock_consensus(2, genesis);
    consensus2.network.connect(&consensus1.network);

    let mut events1 = consensus1.subscribe_events();
    events1.next().await;

    Consensus::sync_blockchain(Arc::downgrade(&consensus1))
        .await
        .expect("Sync failed");

    assert_eq!(consensus1.established(), true);

    tokio::spawn(validator);

    let events1 = consensus1.blockchain.notifier.write().as_stream();
    events1.take(10).for_each(|_| future::ready(())).await;

    assert!(consensus1.blockchain.block_number() >= 10);
}

#[tokio::test]
#[ignore]
async fn three_validators_can_create_micro_blocks() {
    simple_logger::init_with_level(Level::Trace);

    let key1 = KeyPair::generate(&mut thread_rng());
    let key2 = KeyPair::generate(&mut thread_rng());
    let key3 = KeyPair::generate(&mut thread_rng());
    let genesis = GenesisBuilder::default()
        .with_genesis_validator(
            key1.public_key,
            Address::default(),
            Coin::from_u64_unchecked(10000),
        )
        .with_genesis_validator(
            key2.public_key,
            Address::default(),
            Coin::from_u64_unchecked(10000),
        )
        .with_genesis_validator(
            key3.public_key,
            Address::default(),
            Coin::from_u64_unchecked(10000),
        )
        .generate()
        .unwrap();

    let (validator1, consensus1) = mock_validator(1, key1, genesis.clone());
    let (validator2, consensus2) = mock_validator(2, key2, genesis.clone());
    let (validator3, consensus3) = mock_validator(3, key3, genesis.clone());

    consensus1.network.connect(&consensus2.network);
    consensus1.network.connect(&consensus3.network);
    consensus2.network.connect(&consensus3.network);

    let mut events1 = consensus1.subscribe_events();
    let mut events2 = consensus2.subscribe_events();
    let mut events3 = consensus3.subscribe_events();
    time::timeout(
        Duration::from_secs(5),
        future::join_all(vec![events1.next(), events2.next(), events3.next()]),
    )
    .await;

    Consensus::sync_blockchain(Arc::downgrade(&consensus1))
        .await
        .expect("Sync failed");
    Consensus::sync_blockchain(Arc::downgrade(&consensus2))
        .await
        .expect("Sync failed");
    Consensus::sync_blockchain(Arc::downgrade(&consensus3))
        .await
        .expect("Sync failed");

    assert_eq!(consensus1.established(), true);
    assert_eq!(consensus2.established(), true);
    assert_eq!(consensus3.established(), true);

    tokio::spawn(validator1);
    tokio::spawn(validator2);
    tokio::spawn(validator3);

    let events1 = consensus1.blockchain.notifier.write().as_stream();
    time::timeout(
        Duration::from_secs(3),
        events1.take(30).for_each(|_| future::ready(())),
    )
    .await;

    assert!(consensus1.blockchain.block_number() >= 30);
}
