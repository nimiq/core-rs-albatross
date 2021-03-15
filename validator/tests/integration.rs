use std::sync::Arc;
use std::time::Duration;

use futures::{future, StreamExt};
use log::LevelFilter::{Debug, Info, Trace};
use rand::prelude::StdRng;
use rand::SeedableRng;
use tokio::sync::broadcast;
use tokio::time;

use nimiq_block_albatross::{Message, MultiSignature, SignedViewChange, ViewChange};
use nimiq_blockchain_albatross::{AbstractBlockchain, Blockchain, BlockchainEvent};
use nimiq_bls::{AggregatePublicKey, AggregateSignature, KeyPair};
use nimiq_build_tools::genesis::{GenesisBuilder, GenesisInfo};
use nimiq_collections::BitSet;
use nimiq_consensus_albatross::sync::history::HistorySync;
use nimiq_consensus_albatross::{Consensus as AbstractConsensus, ConsensusEvent};
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_handel::update::{LevelUpdate, LevelUpdateMessage};
use nimiq_hash::{Blake2bHash, Hash};
use nimiq_keys::{Address, SecureGenerate};
use nimiq_mempool::{Mempool, MempoolConfig};
use nimiq_network_interface::network::Network as NetworkInterface;
use nimiq_network_libp2p::discovery::peer_contacts::{PeerContact, Services};
use nimiq_network_libp2p::libp2p::core::multiaddr::multiaddr;
use nimiq_network_libp2p::{Config, Keypair, Network};
use nimiq_primitives::account::ValidatorId;
use nimiq_primitives::coin::Coin;
use nimiq_primitives::networks::NetworkId;
use nimiq_utils::time::OffsetTime;
use nimiq_validator::aggregation::view_change::SignedViewChangeMessage;
use nimiq_validator::validator::Validator as AbstractValidator;
use nimiq_validator_network::network_impl::ValidatorNetworkImpl;
use nimiq_vrf::VrfSeed;

type Consensus = AbstractConsensus<Network>;
type Validator = AbstractValidator<Network, ValidatorNetworkImpl<Network>>;

fn seeded_rng(seed: u64) -> StdRng {
    StdRng::seed_from_u64(seed)
}

async fn consensus(peer_id: u64, genesis_info: GenesisInfo) -> Consensus {
    let env = VolatileEnvironment::new(10).unwrap();
    let clock = Arc::new(OffsetTime::new());
    let blockchain = Arc::new(
        Blockchain::with_genesis(
            env.clone(),
            Arc::clone(&clock),
            NetworkId::UnitAlbatross,
            genesis_info.block,
            genesis_info.accounts,
        )
        .unwrap(),
    );
    let mempool = Mempool::new(Arc::clone(&blockchain), MempoolConfig::default());

    let peer_key = Keypair::generate_ed25519();
    let peer_address = multiaddr![Memory(peer_id)];
    let mut peer_contact = PeerContact::new(
        vec![peer_address.clone()],
        peer_key.public(),
        Services::all(),
        None,
    );
    peer_contact.set_current_time();
    let mut config = Config::new(peer_key, peer_contact, genesis_info.hash.clone());
    let network = Arc::new(Network::new(clock, config).await);
    network.listen_on_addresses(vec![peer_address]).await;

    let sync_protocol =
        HistorySync::<Network>::new(Arc::clone(&blockchain), network.subscribe_events());
    Consensus::with_min_peers(env, blockchain, mempool, network, sync_protocol.boxed(), 1).await
}

async fn validator(
    peer_id: u64,
    signing_key: KeyPair,
    genesis_info: GenesisInfo,
) -> (Validator, Consensus) {
    let consensus = consensus(peer_id, genesis_info).await;
    let validator_network = Arc::new(ValidatorNetworkImpl::new(consensus.network.clone()));
    (
        Validator::new(&consensus, validator_network, signing_key, None),
        consensus,
    )
}

async fn validators(num_validators: usize) -> Vec<Validator> {
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
        let (v, c) = validator((id + 1) as u64, key, genesis.clone()).await;
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

        tokio::spawn(consensus.for_each(|_| async {}));
    }

    validators
}

#[tokio::test(threaded_scheduler)]
async fn four_validators_can_create_an_epoch() {
    simple_logger::SimpleLogger::new()
        .with_level(Info)
        .with_module_level("nimiq_validator", Debug)
        .with_module_level("nimiq_network_libp2p", Info)
        .with_module_level("nimiq_handel", Info)
        .with_module_level("nimiq_tendermint", Debug)
        .with_module_level("nimiq_blockchain_albatross", Debug)
        .with_module_level("nimiq_block_albatross", Debug)
        .init();

    let validators = validators(4).await;

    let blockchain = Arc::clone(&validators.first().unwrap().consensus.blockchain);

    tokio::spawn(future::join_all(validators));

    let events = blockchain.notifier.write().as_stream();
    time::timeout(
        Duration::from_secs(120),
        events.take(130).for_each(|_| future::ready(())),
    )
    .await
    .unwrap();

    assert!(blockchain.block_number() >= 130);
    assert_eq!(blockchain.view_number(), 0);
}
