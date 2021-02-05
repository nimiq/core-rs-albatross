use futures::{future, StreamExt};
use rand::prelude::StdRng;
use rand::SeedableRng;
use tokio::sync::broadcast;
use tokio::time;

use nimiq_blockchain_albatross::Blockchain;
use nimiq_bls::KeyPair;
use nimiq_build_tools::genesis::{GenesisBuilder, GenesisInfo};
use nimiq_consensus_albatross::sync::history::HistorySync;
use nimiq_consensus_albatross::{Consensus as AbstractConsensus, ConsensusEvent};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_keys::{Address, SecureGenerate};
use nimiq_mempool::{Mempool, MempoolConfig};
use nimiq_network_interface::network::Network;
use nimiq_network_mock::{MockHub, MockNetwork};
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_utils::time::OffsetTime;
use nimiq_validator::validator::Validator as AbstractValidator;
use nimiq_validator_network::network_impl::ValidatorNetworkImpl;
use std::sync::Arc;
use std::time::Duration;
use nimiq_primitives::account::ValidatorId;
use nimiq_hash::{Hash, Blake2bHash};

type Consensus = AbstractConsensus<MockNetwork>;
type Validator = AbstractValidator<MockNetwork, ValidatorNetworkImpl<MockNetwork>>;

fn seeded_rng(seed: u64) -> StdRng {
    StdRng::seed_from_u64(seed)
}

async fn mock_consensus(hub: &mut MockHub, peer_id: u64, genesis_info: GenesisInfo) -> Consensus {
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
    let network = Arc::new(hub.new_network_with_address(peer_id));
    let sync_protocol =
        HistorySync::<MockNetwork>::new(Arc::clone(&blockchain), network.subscribe_events());
    Consensus::from_network(env, blockchain, mempool, network, sync_protocol.boxed()).await
}

async fn mock_validator(
    hub: &mut MockHub,
    peer_id: u64,
    signing_key: KeyPair,
    genesis_info: GenesisInfo,
) -> (Validator, Consensus) {
    let consensus = mock_consensus(hub, peer_id, genesis_info).await;
    let validator_network = Arc::new(ValidatorNetworkImpl::new(consensus.network.clone()));
    (
        Validator::new(
            &consensus,
            validator_network,
            signing_key,
            None,
        ),
        consensus,
    )
}

async fn mock_validators(hub: &mut MockHub, num_validators: usize) -> Vec<Validator> {
    // Generate validator key pairs.
    let mut rng = seeded_rng(0);
    let keys: Vec<KeyPair> = (0..num_validators)
        .map(|_| KeyPair::generate(&mut rng))
        .collect();

    // Generate genesis block.
    let mut genesis_builder = GenesisBuilder::default();
    for key in &keys {
        genesis_builder.with_genesis_validator(
            key.public_key.hash::<Blake2bHash>().as_slice()[0..20].into(),
            key.public_key,
            Address::default(),
            Coin::from_u64_unchecked(10000),
        );
    }
    let genesis = genesis_builder.generate().unwrap();

    // Instantiate validators.
    let mut validators = vec![];
    let mut consensus = vec![];
    for (id, key) in keys.into_iter().enumerate() {
        let (v, c) = mock_validator(hub, id as u64, key, genesis.clone()).await;
        validators.push(v);
        consensus.push(c);
    }

    // Connect validators to each other.
    for id in 0..num_validators {
        let validator = validators.get(id).unwrap();
        for other_id in (id + 1)..num_validators {
            let other_validator = validators.get(other_id).unwrap();
            validator
                .consensus
                .network
                .dial_mock(&other_validator.consensus.network);
        }
    }

    // Wait until validators are connected.
    let mut events: Vec<broadcast::Receiver<ConsensusEvent<MockNetwork>>> =
        consensus.iter().map(|v| v.subscribe_events()).collect();

    // Start consensus.
    for consensus in consensus {
        tokio::spawn(consensus.for_each(|_| async {}));
    }

    future::join_all(events.iter_mut().map(|e| e.next())).await;

    validators
}

fn validator_for_slot(
    validators: &Vec<Validator>,
    block_number: u32,
    view_number: u32,
) -> &Validator {
    let consensus = &validators.first().unwrap().consensus;
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
    let mut hub = MockHub::default();

    let key = KeyPair::generate(&mut seeded_rng(0));
    let genesis = GenesisBuilder::default()
        .with_genesis_validator(
            ValidatorId::default(),
            key.public_key,
            Address::default(),
            Coin::from_u64_unchecked(10000),
        )
        .generate()
        .unwrap();

    let (validator, mut consensus1) = mock_validator(&mut hub, 1, key, genesis.clone()).await;

    log::debug!("Establishing consensus...");
    consensus1.force_established();
    assert_eq!(consensus1.is_established(), true);

    log::debug!("Spawning validator...");
    tokio::spawn(validator);

    let events1 = consensus1.blockchain.notifier.write().as_stream();
    events1.take(10).for_each(|_| future::ready(())).await;

    assert!(consensus1.blockchain.block_number() >= 10);
}

#[tokio::test]
async fn four_validators_can_create_micro_blocks() {
    let mut hub = MockHub::default();

    let validators = mock_validators(&mut hub, 4).await;

    let blockchain = Arc::clone(&validators.first().unwrap().consensus.blockchain);

    tokio::spawn(future::join_all(validators));

    let events = blockchain.notifier.write().as_stream();
    time::timeout(
        Duration::from_secs(60),
        events.take(30).for_each(|_| future::ready(())),
    )
    .await
    .unwrap();

    assert!(blockchain.block_number() >= 30);
}

#[tokio::test]
async fn four_validators_can_view_change() {
    let mut hub = MockHub::default();

    let validators = mock_validators(&mut hub, 4).await;

    // Disconnect the next block producer.
    let validator = validator_for_slot(&validators, 1, 0);
    validator.consensus.network.disconnect();

    // Listen for blockchain events from the new block producer (after view change).
    let validator = validator_for_slot(&validators, 1, 1);
    let blockchain = Arc::clone(&validator.consensus.blockchain);
    let mut events = blockchain.notifier.write().as_stream();

    // Freeze time to immediately trigger the view change timeout.
    // time::pause();

    tokio::spawn(future::join_all(validators));

    // Wait for the new block producer to create a block.
    events.next().await;

    assert!(blockchain.block_number() > 1);
    assert_eq!(blockchain.view_number(), 1);
}
