use std::sync::Arc;

use futures::{future, StreamExt};
use log::LevelFilter::{Debug, Info};
use nimiq_mempool::config::MempoolConfig;
use parking_lot::RwLock;
use rand::prelude::StdRng;
use rand::SeedableRng;

use nimiq_blockchain::{AbstractBlockchain, Blockchain};
use nimiq_bls::KeyPair as BlsKeyPair;
use nimiq_build_tools::genesis::{GenesisBuilder, GenesisInfo};
use nimiq_consensus::sync::history::HistorySync;
use nimiq_consensus::Consensus as AbstractConsensus;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_keys::{Address, KeyPair, SecureGenerate};
use nimiq_network_interface::network::Network as NetworkInterface;
use nimiq_network_libp2p::discovery::peer_contacts::{PeerContact, Services};
use nimiq_network_libp2p::libp2p::core::multiaddr::multiaddr;
use nimiq_network_libp2p::{Config, Keypair as P2PKeyPair, Network};

use nimiq_primitives::networks::NetworkId;
use nimiq_utils::time::OffsetTime;
use nimiq_validator::validator::Validator as AbstractValidator;
use nimiq_validator_network::network_impl::ValidatorNetworkImpl;

type Consensus = AbstractConsensus<Network>;
type Validator = AbstractValidator<Network, ValidatorNetworkImpl<Network>>;

fn seeded_rng(seed: u64) -> StdRng {
    StdRng::seed_from_u64(seed)
}

async fn consensus(peer_id: u64, genesis_info: GenesisInfo) -> Consensus {
    let env = VolatileEnvironment::new(12).unwrap();
    let clock = Arc::new(OffsetTime::new());
    let blockchain = Arc::new(RwLock::new(
        Blockchain::with_genesis(
            env.clone(),
            Arc::clone(&clock),
            NetworkId::UnitAlbatross,
            genesis_info.block,
            genesis_info.accounts,
        )
        .unwrap(),
    ));

    let peer_key = P2PKeyPair::generate_ed25519();
    let peer_address = multiaddr![Memory(peer_id)];
    let mut peer_contact = PeerContact::new(
        vec![peer_address.clone()],
        peer_key.public(),
        Services::all(),
        None,
    );
    peer_contact.set_current_time();
    let config = Config::new(
        peer_key,
        peer_contact,
        Vec::new(),
        genesis_info.hash.clone(),
    );
    let network = Arc::new(Network::new(clock, config).await);
    network.listen_on(vec![peer_address]).await;

    let sync_protocol =
        HistorySync::<Network>::new(Arc::clone(&blockchain), network.subscribe_events());
    Consensus::with_min_peers(env, blockchain, network, Box::pin(sync_protocol), 1).await
}

async fn validator(
    peer_id: u64,
    voting_key: BlsKeyPair,
    validator_address: Address,
    fee_key: KeyPair,
    signing_key: KeyPair,
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

async fn validators(num_validators: usize) -> Vec<Validator> {
    // Generate validator key pairs.
    let mut rng = seeded_rng(0);
    let cold_keys: Vec<KeyPair> = (0..num_validators)
        .map(|_| KeyPair::generate(&mut rng))
        .collect();
    let voting_keys: Vec<BlsKeyPair> = (0..num_validators)
        .map(|_| BlsKeyPair::generate(&mut rng))
        .collect();
    let signing_keys: Vec<KeyPair> = (0..num_validators)
        .map(|_| KeyPair::generate(&mut rng))
        .collect();
    let fee_keys: Vec<KeyPair> = (0..num_validators)
        .map(|_| KeyPair::generate(&mut rng))
        .collect();

    // Generate genesis block.
    let mut genesis_builder = GenesisBuilder::default();
    for i in 0..num_validators {
        genesis_builder.with_genesis_validator(
            Address::from(&cold_keys[i]),
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
        let (v, c) = validator(
            (id + 1) as u64,
            voting_keys[id].clone(),
            Address::from(&cold_keys[id]),
            fee_keys[id].clone(),
            signing_keys[id].clone(),
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

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn four_validators_can_create_an_epoch() {
    simple_logger::SimpleLogger::new()
        .with_level(Info)
        .with_module_level("nimiq_validator", Debug)
        .with_module_level("nimiq_network_libp2p", Info)
        .with_module_level("nimiq_handel", Info)
        .with_module_level("nimiq_tendermint", Debug)
        .with_module_level("nimiq_blockchain", Debug)
        .with_module_level("nimiq_block", Debug)
        .init()
        .ok();

    let validators = validators(4).await;

    let blockchain = Arc::clone(&validators.first().unwrap().consensus.blockchain);

    tokio::spawn(future::join_all(validators));

    let events = blockchain.write().notifier.as_stream();

    events.take(130).for_each(|_| future::ready(())).await;

    assert!(blockchain.read().block_number() >= 130);
    assert_eq!(blockchain.read().view_number(), 0);
}
