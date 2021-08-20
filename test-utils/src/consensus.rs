use parking_lot::RwLock;
use std::sync::Arc;

use nimiq_blockchain::Blockchain;
use nimiq_build_tools::genesis::GenesisInfo;
use nimiq_consensus::sync::history::HistorySync;
use nimiq_consensus::Consensus as AbstractConsensus;
use nimiq_database::volatile::VolatileEnvironment;
use nimiq_network_interface::network::Network as NetworkInterface;
use nimiq_network_libp2p::discovery::peer_contacts::{PeerContact, Services};
use nimiq_network_libp2p::libp2p::core::multiaddr::multiaddr;
use nimiq_network_libp2p::{Config, Keypair, Network};
use nimiq_primitives::networks::NetworkId;
use nimiq_utils::time::OffsetTime;

type Consensus = AbstractConsensus<Network>;

pub async fn consensus(peer_id: u64, genesis_info: GenesisInfo) -> Consensus {
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

    let peer_key = Keypair::generate_ed25519();
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
