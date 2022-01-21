use futures::{future, StreamExt};
use rand::prelude::StdRng;
use rand::SeedableRng;
use std::fmt::Display;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;

use crate::consensus::consensus;
use crate::test_network::TestNetwork;

use beserial::{Deserialize, Serialize};
use nimiq_blockchain::AbstractBlockchain;
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_build_tools::genesis::{GenesisBuilder, GenesisInfo};
use nimiq_consensus::{Consensus as AbstractConsensus, ConsensusEvent};
use nimiq_database::Environment;
use nimiq_keys::{Address, KeyPair as SchnorrKeyPair, SecureGenerate};
use nimiq_mempool::config::MempoolConfig;
use nimiq_network_interface::{network::Network as NetworkInterface, peer::Peer as PeerInterface};
use nimiq_network_mock::MockHub;
use nimiq_validator::validator::Validator as AbstractValidator;
use nimiq_validator_network::network_impl::ValidatorNetworkImpl;

pub fn seeded_rng(seed: u64) -> StdRng {
    StdRng::seed_from_u64(seed)
}

pub async fn build_validator<N: TestNetwork + NetworkInterface>(
    peer_id: u64,
    validator_address: Address,
    signing_key: SchnorrKeyPair,
    voting_key: BlsKeyPair,
    fee_key: SchnorrKeyPair,
    genesis_info: GenesisInfo,
    hub: &mut Option<MockHub>,
) -> (
    AbstractValidator<N, ValidatorNetworkImpl<N>>,
    AbstractConsensus<N>,
)
where
    N::Error: Send,
    <N::PeerType as PeerInterface>::Id: Serialize + Deserialize + Clone,
{
    let consensus = consensus(peer_id, genesis_info, hub).await;
    let validator_network = Arc::new(ValidatorNetworkImpl::new(Arc::clone(&consensus.network)));
    (
        AbstractValidator::<N, ValidatorNetworkImpl<N>>::new(
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

pub async fn build_validators<N: TestNetwork + NetworkInterface>(
    env: Environment,
    num_validators: usize,
    hub: &mut Option<MockHub>,
) -> Vec<AbstractValidator<N, ValidatorNetworkImpl<N>>>
where
    N::Error: Send,
    <N::PeerType as PeerInterface>::Id: Serialize + Deserialize + Clone + Display,
{
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
    let genesis = genesis_builder.generate(env).unwrap();

    // Instantiate validators.
    let mut validators = vec![];
    let mut consensus = vec![];
    let mut networks = vec![];
    for id in 0..num_validators {
        let (v, c) = build_validator(
            (id + 1) as u64,
            Address::from(&validator_keys[id]),
            signing_keys[id].clone(),
            voting_keys[id].clone(),
            fee_keys[id].clone(),
            genesis.clone(),
            hub,
        )
        .await;
        let network: Arc<N> = Arc::clone(&c.network);
        log::info!(
            "Validator #{}: {}",
            v.validator_slot_band(),
            network.get_local_peer_id()
        );
        validators.push(v);
        consensus.push(c);
        networks.push(network);
    }

    // Connect network
    N::connect_network(&networks).await;

    // Wait until validators are connected.
    let mut events: Vec<BroadcastStream<ConsensusEvent>> =
        consensus.iter().map(|v| v.subscribe_events()).collect();

    // Start consensus
    for consensus in consensus {
        tokio::spawn(consensus);
    }

    future::join_all(events.iter_mut().map(|e| e.next())).await;

    validators
}

pub fn validator_for_slot<N: TestNetwork + NetworkInterface>(
    validators: &[AbstractValidator<N, ValidatorNetworkImpl<N>>],
    block_number: u32,
    view_number: u32,
) -> &AbstractValidator<N, ValidatorNetworkImpl<N>>
where
    N::Error: Send,
    <N::PeerType as PeerInterface>::Id: Serialize + Deserialize + Clone + Send,
{
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
