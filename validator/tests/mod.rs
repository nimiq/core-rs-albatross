use std::sync::Arc;
use std::time::Duration;

use futures::{future, StreamExt};
use rand::prelude::StdRng;
use rand::SeedableRng;
use tokio::sync::broadcast;
use tokio::time;

use nimiq_blockchain_albatross::Blockchain;
use nimiq_bls::KeyPair;
use nimiq_build_tools::genesis::{GenesisBuilder, GenesisInfo};
use nimiq_consensus_albatross::sync::QuickSync;
use nimiq_consensus_albatross::{Consensus as AbstractConsensus, ConsensusEvent};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_keys::{Address, SecureGenerate};
use nimiq_mempool::{Mempool, MempoolConfig};
use nimiq_network_mock::network::MockNetwork;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_utils::time::OffsetTime;
use nimiq_validator::validator::Validator as AbstractValidator;


type Consensus = AbstractConsensus<MockNetwork>;
type Validator = AbstractValidator<MockNetwork, MockNetwork>;


fn seeded_rng(seed: u64) -> StdRng {
    StdRng::seed_from_u64(seed)
}

fn mock_consensus(peer_id: usize, genesis_info: GenesisInfo) -> Arc<Consensus> {
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

fn mock_validator(peer_id: usize, signing_key: KeyPair, genesis_info: GenesisInfo) -> Validator {
    let consensus = mock_consensus(peer_id, genesis_info);
    Validator::new(
        Arc::clone(&consensus),
        Arc::clone(&consensus.network),
        signing_key,
        None,
    )
}

async fn mock_validators(num_validators: usize) -> Vec<Validator> {
    // Generate validator key pairs.
    let mut rng = seeded_rng(0);
    let keys: Vec<KeyPair> = (0..num_validators)
        .map(|_| KeyPair::generate(&mut rng))
        .collect();

    // Generate genesis block.
    let mut genesis_builder = GenesisBuilder::default();
    for key in &keys {
        genesis_builder.with_genesis_validator(
            key.public_key,
            Address::default(),
            Coin::from_u64_unchecked(10000),
        );
    }
    let genesis = genesis_builder.generate().unwrap();

    // Instantiate validators.
    let validators: Vec<Validator> = keys
        .into_iter()
        .enumerate()
        .map(|(id, key)| mock_validator(id, key, genesis.clone()))
        .collect();

    // Connect validators to each other.
    for id in 0..num_validators {
        let validator = validators.get(id).unwrap();
        for other_id in (id + 1)..num_validators {
            let other_validator = validators.get(other_id).unwrap();
            validator
                .consensus
                .network
                .connect(&other_validator.consensus.network);
        }
    }

    // Wait until validators are connected.
    let mut events: Vec<broadcast::Receiver<ConsensusEvent<MockNetwork>>> = validators
        .iter()
        .map(|v| v.consensus.subscribe_events())
        .collect();
    future::join_all(events.iter_mut().map(|e| e.next())).await;

    // Sync blockchains.
    for validator in &validators {
        Consensus::sync_blockchain(Arc::downgrade(&validator.consensus))
            .await
            .expect("Sync failed");
        assert_eq!(validator.consensus.established(), true);
    }

    validators
}

fn validator_for_slot(
    validators: &Vec<Validator>,
    block_number: u32,
    view_number: u32,
) -> &Validator {
    let consensus = Arc::clone(&validators.first().unwrap().consensus);
    let (slot, _) = consensus
        .blockchain
        .get_slot_owner_at(block_number, view_number, None);
    validators
        .iter()
        .find(|validator| {
            &validator.signing_key().public_key.compress() == slot.public_key().compressed()
        })
        .unwrap()
}

#[tokio::test]
async fn one_validator_can_create_micro_blocks() {
    let key = KeyPair::generate(&mut seeded_rng(0));
    let genesis = GenesisBuilder::default()
        .with_genesis_validator(
            key.public_key,
            Address::default(),
            Coin::from_u64_unchecked(10000),
        )
        .generate()
        .unwrap();

    let validator = mock_validator(1, key, genesis.clone());
    let consensus1 = Arc::clone(&validator.consensus);

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
    let validators = mock_validators(3).await;

    let consensus = Arc::clone(&validators.first().unwrap().consensus);

    tokio::spawn(future::join_all(validators));

    let events = consensus.blockchain.notifier.write().as_stream();
    time::timeout(
        Duration::from_secs(3),
        events.take(30).for_each(|_| future::ready(())),
    )
    .await.unwrap();

    assert!(consensus.blockchain.block_number() >= 30);
}

#[tokio::test]
async fn four_validators_can_view_change() {
    let validators = mock_validators(4).await;

    // Disconnect the next block producer.
    let validator = validator_for_slot(&validators, 1, 0);
    validator.consensus.network.disconnect();

    // Listen for blockchain events from the new block producer (after view change).
    let validator = validator_for_slot(&validators, 1, 1);
    let consensus = Arc::clone(&validator.consensus);
    let mut events = consensus.blockchain.notifier.write().as_stream();

    // Freeze time to immediately trigger the view change timeout.
    time::pause();

    tokio::spawn(future::join_all(validators));

    // Wait for the new block producer to create a block.
    events.next().await;

    assert_eq!(consensus.blockchain.block_number(), 1);
    assert_eq!(consensus.blockchain.view_number(), 1);
}
