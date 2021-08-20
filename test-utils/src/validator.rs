use futures::{future, StreamExt};
use parking_lot::RwLock;
use rand::prelude::StdRng;
use rand::SeedableRng;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;

use crate::consensus::consensus;
use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_build_tools::genesis::{GenesisBuilder, GenesisInfo};
use nimiq_consensus::sync::history::HistorySync;
use nimiq_consensus::{Consensus as AbstractConsensus, ConsensusEvent};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_keys::{Address, KeyPair as SchnorrKeyPair, SecureGenerate};
use nimiq_mempool::config::MempoolConfig;
use nimiq_network_interface::network::Network as NetworkInterface;
use nimiq_network_libp2p::libp2p::core::multiaddr::multiaddr;
use nimiq_network_libp2p::Network;
use nimiq_network_mock::{MockHub, MockNetwork};
use nimiq_primitives::networks::NetworkId;
use nimiq_utils::time::OffsetTime;
use nimiq_validator::validator::Validator as AbstractValidator;
use nimiq_validator_network::network_impl::ValidatorNetworkImpl;

type Consensus = AbstractConsensus<Network>;
type Validator = AbstractValidator<Network, ValidatorNetworkImpl<Network>>;

type MockConsensus = AbstractConsensus<MockNetwork>;
type MockValidator = AbstractValidator<MockNetwork, ValidatorNetworkImpl<MockNetwork>>;

fn seeded_rng(seed: u64) -> StdRng {
    StdRng::seed_from_u64(seed)
}

async fn build_validator(
    peer_id: u64,
    validator_address: Address,
    signing_key: SchnorrKeyPair,
    voting_key: BlsKeyPair,
    fee_key: SchnorrKeyPair,
    genesis_info: GenesisInfo,
) -> (Validator, Consensus) {
    let consensus = consensus(peer_id, genesis_info).await;
    let validator_network = Arc::new(ValidatorNetworkImpl::new(consensus.network.clone()));
    (
        Validator::new(
            &consensus,
            validator_network,
            validator_address,
            signing_key,
            voting_key,
            fee_key,
            MempoolConfig::default(),
        ),
        consensus,
    )
}

pub async fn build_validators(num_validators: usize) -> Vec<Validator> {
    // Generate validator key pairs.
    let mut rng = seeded_rng(0);
    let voting_keys: Vec<BlsKeyPair> = (0..num_validators)
        .map(|_| BlsKeyPair::generate(&mut rng))
        .collect();
    let validator_keys: Vec<SchnorrKeyPair> = (0..num_validators)
        .map(|_| SchnorrKeyPair::generate(&mut rng))
        .collect();
    let signing_keys: Vec<SchnorrKeyPair> = (0..num_validators)
        .map(|_| SchnorrKeyPair::generate(&mut rng))
        .collect();
    let fee_keys: Vec<SchnorrKeyPair> = (0..num_validators)
        .map(|_| SchnorrKeyPair::generate(&mut rng))
        .collect();

    // Generate genesis block.
    let mut genesis_builder = GenesisBuilder::default();
    for i in 0..num_validators {
        genesis_builder.with_genesis_validator(
            Address::from(&validator_keys[i]),
            signing_keys[i].public,
            voting_keys[i].public_key,
            Address::default(),
        );
    }
    let genesis = genesis_builder.generate().unwrap();

    // Instantiate validators.
    let mut validators = vec![];
    let mut consensus = vec![];
    for id in 0..num_validators {
        let (v, c) = build_validator(
            (id + 1) as u64,
            Address::from(&validator_keys[id]),
            signing_keys[id].clone(),
            voting_keys[id].clone(),
            fee_keys[id].clone(),
            genesis.clone(),
        )
        .await;
        log::info!(
            "Validator #{}: {}",
            v.validator_id(),
            c.network.local_peer_id()
        );
        validators.push(v);
        consensus.push(c);
    }

    // Start consensus.
    for consensus in consensus {
        // Tell the network to connect to seed nodes
        let seed = multiaddr![Memory(1u64)];
        log::debug!("Dialing seed: {:?}", seed);
        consensus
            .network
            .dial_address(seed)
            .await
            .expect("Failed to dial seed");

        tokio::spawn(consensus);
    }

    validators
}

async fn mock_consensus(
    hub: &mut MockHub,
    peer_id: u64,
    genesis_info: GenesisInfo,
) -> MockConsensus {
    let env = VolatileEnvironment::new(12).unwrap();
    let time = Arc::new(OffsetTime::new());
    let blockchain = Arc::new(RwLock::new(
        Blockchain::with_genesis(
            env.clone(),
            time,
            NetworkId::UnitAlbatross,
            genesis_info.block,
            genesis_info.accounts,
        )
        .unwrap(),
    ));
    let network = Arc::new(hub.new_network_with_address(peer_id));
    let sync_protocol =
        HistorySync::<MockNetwork>::new(Arc::clone(&blockchain), network.subscribe_events());
    MockConsensus::from_network(env, blockchain, network, Box::pin(sync_protocol)).await
}

pub async fn mock_validator(
    hub: &mut MockHub,
    peer_id: u64,
    validator_address: Address,
    signing_key: SchnorrKeyPair,
    voting_key: BlsKeyPair,
    fee_key: SchnorrKeyPair,
    genesis_info: GenesisInfo,
) -> (MockValidator, MockConsensus) {
    let consensus = mock_consensus(hub, peer_id, genesis_info).await;
    let validator_network = Arc::new(ValidatorNetworkImpl::new(consensus.network.clone()));
    (
        MockValidator::new(
            &consensus,
            validator_network,
            validator_address,
            signing_key,
            voting_key,
            fee_key,
            MempoolConfig::default(),
        ),
        consensus,
    )
}

pub async fn mock_validators(hub: &mut MockHub, num_validators: usize) -> Vec<MockValidator> {
    // Generate validator key pairs.
    let mut rng = seeded_rng(0);
    let voting_keys: Vec<BlsKeyPair> = (0..num_validators)
        .map(|_| BlsKeyPair::generate(&mut rng))
        .collect();
    let validator_keys: Vec<SchnorrKeyPair> = (0..num_validators)
        .map(|_| SchnorrKeyPair::generate(&mut rng))
        .collect();
    let signing_keys: Vec<SchnorrKeyPair> = (0..num_validators)
        .map(|_| SchnorrKeyPair::generate(&mut rng))
        .collect();
    let fee_keys: Vec<SchnorrKeyPair> = (0..num_validators)
        .map(|_| SchnorrKeyPair::generate(&mut rng))
        .collect();

    // Generate genesis block.
    let mut genesis_builder = GenesisBuilder::default();
    for i in 0..num_validators {
        genesis_builder.with_genesis_validator(
            Address::from(&validator_keys[i]),
            signing_keys[i].public,
            voting_keys[i].public_key,
            Address::default(),
        );
    }
    let genesis = genesis_builder.generate().unwrap();

    // Instantiate validators.
    let mut validators = vec![];
    let mut consensus = vec![];
    for id in 0..num_validators {
        let (v, c) = mock_validator(
            hub,
            id as u64,
            Address::from(&validator_keys[id]),
            signing_keys[id].clone(),
            voting_keys[id].clone(),
            fee_keys[id].clone(),
            genesis.clone(),
        )
        .await;
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
    let mut events: Vec<BroadcastStream<ConsensusEvent>> =
        consensus.iter().map(|v| v.subscribe_events()).collect();

    // Start consensus.
    for consensus in consensus {
        tokio::spawn(consensus);
    }

    future::join_all(events.iter_mut().map(|e| e.next())).await;

    validators
}

pub fn validator_for_slot(
    validators: &Vec<MockValidator>,
    block_number: u32,
    view_number: u32,
) -> &MockValidator {
    let consensus = &validators.first().unwrap().consensus;

    let (slot, _) = consensus
        .blockchain
        .read()
        .get_slot_owner_at(block_number, view_number, None)
        .expect("Couldn't find slot owner!");

    validators
        .iter()
        .find(|validator| {
            &validator.voting_key().public_key.compress() == slot.voting_key.compressed()
        })
        .unwrap()
}
