use std::sync::Arc;

use futures::{future, StreamExt};
use nimiq_blockchain_interface::AbstractBlockchain;
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_consensus::{Consensus, ConsensusEvent};
use nimiq_database::DatabaseProxy;
use nimiq_genesis_builder::{GenesisBuilder, GenesisInfo};
use nimiq_keys::{Address, KeyPair as SchnorrKeyPair, SecureGenerate};
use nimiq_mempool::config::MempoolConfig;
use nimiq_network_interface::network::Network as NetworkInterface;
use nimiq_network_mock::MockHub;
use nimiq_primitives::{networks::NetworkId, policy::Policy};
use nimiq_serde::{Deserialize, Serialize};
use nimiq_utils::spawn::spawn;
use nimiq_validator::validator::Validator;
use nimiq_validator_network::network_impl::ValidatorNetworkImpl;
use rand::{rngs::StdRng, SeedableRng};
use tokio_stream::wrappers::BroadcastStream;

use crate::{node::Node, test_network::TestNetwork};

pub fn seeded_rng(seed: u64) -> StdRng {
    StdRng::seed_from_u64(seed)
}

pub async fn build_validator<N: TestNetwork + NetworkInterface>(
    peer_id: u64,
    validator_address: Address,
    automatic_reactivate: bool,
    signing_key: SchnorrKeyPair,
    voting_key: BlsKeyPair,
    fee_key: SchnorrKeyPair,
    genesis_info: GenesisInfo,
    hub: &mut Option<MockHub>,
    is_prover_active: bool,
) -> (Validator<ValidatorNetworkImpl<N>>, Consensus<N>)
where
    N::Error: Send,
    N::PeerId: Deserialize + Serialize,
    N::Error: Sync,
{
    let node =
        Node::<N>::history_with_genesis_info(peer_id, genesis_info, hub, is_prover_active).await;
    let consensus = node.consensus.expect("Could not create consensus");
    let validator_network = Arc::new(ValidatorNetworkImpl::new(Arc::clone(&consensus.network)));
    (
        Validator::<ValidatorNetworkImpl<N>>::new(
            node.environment,
            &consensus,
            node.blockchain,
            validator_network,
            validator_address,
            automatic_reactivate,
            signing_key,
            voting_key,
            fee_key,
            MempoolConfig::default(),
        ),
        consensus,
    )
}

pub async fn build_validators<N: TestNetwork + NetworkInterface>(
    env: DatabaseProxy,
    peer_ids: &[u64],
    hub: &mut Option<MockHub>,
    is_prover_active: bool,
) -> Vec<Validator<ValidatorNetworkImpl<N>>>
where
    N::Error: Send + Sync,
    N::PeerId: Deserialize + Serialize,
{
    let num_validators = peer_ids.len();
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
    genesis_builder.with_network(NetworkId::UnitAlbatross);
    for i in 0..num_validators {
        genesis_builder
            .with_genesis_validator(
                Address::from(&validator_keys[i]),
                signing_keys[i].public,
                voting_keys[i].public_key,
                Address::default(),
                None,
                None,
                false,
            )
            .with_genesis_block_number(Policy::genesis_block_number());
    }
    let genesis = genesis_builder.generate(env).unwrap();

    // Instantiate validators.
    let mut validators = vec![];
    let mut consensus = vec![];
    let mut networks = vec![];
    for i in 0..num_validators {
        let (v, c) = build_validator(
            peer_ids[i],
            Address::from(&validator_keys[i]),
            false,
            signing_keys[i].clone(),
            voting_keys[i].clone(),
            fee_keys[i].clone(),
            genesis.clone(),
            hub,
            is_prover_active,
        )
        .await;
        let network: Arc<N> = Arc::clone(&c.network);
        validators.push(v);
        consensus.push(c);
        networks.push(network);
    }

    // Connect network
    N::connect_networks(&networks, *peer_ids.last().unwrap()).await;

    // Wait until validators are connected.
    let mut events: Vec<BroadcastStream<ConsensusEvent>> =
        consensus.iter().map(|v| v.subscribe_events()).collect();

    // Start consensus
    for consensus in consensus {
        spawn(consensus);
    }

    future::join_all(events.iter_mut().map(|e| e.next())).await;

    validators
}

pub fn validator_for_slot<N: TestNetwork + NetworkInterface>(
    validators: &[Validator<ValidatorNetworkImpl<N>>],
    block_number: u32,
    offset: u32,
) -> &Validator<ValidatorNetworkImpl<N>>
where
    N::Error: Send,
    N::PeerId: Deserialize + Serialize,
    N::Error: Sync,
{
    let consensus = &validators.first().unwrap().consensus;

    let slot = consensus
        .blockchain
        .read()
        .get_proposer_at(block_number, offset)
        .expect("Couldn't find slot owner!");

    validators
        .iter()
        .find(|validator| {
            &validator.voting_key().public_key.compress() == slot.validator.voting_key.compressed()
        })
        .unwrap()
}

pub fn pop_validator_for_slot<N: TestNetwork + NetworkInterface>(
    validators: &mut Vec<Validator<ValidatorNetworkImpl<N>>>,
    block_number: u32,
    offset: u32,
) -> Validator<ValidatorNetworkImpl<N>>
where
    N::Error: Send,
    N::PeerId: Deserialize + Serialize,
    N::Error: Sync,
{
    let consensus = &validators.first().unwrap().consensus;

    let slot = consensus
        .blockchain
        .read()
        .get_proposer_at(block_number, offset)
        .expect("Couldn't find slot owner!");

    let index = validators
        .iter()
        .position(|validator| {
            &validator.voting_key().public_key.compress() == slot.validator.voting_key.compressed()
        })
        .unwrap();
    validators.remove(index)
}
