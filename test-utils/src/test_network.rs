use std::{num::NonZeroU8, sync::Arc};

use async_trait::async_trait;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{network::Network as NetworkInterface, peer_info::Services};
use nimiq_network_libp2p::{
    discovery::peer_contacts::PeerContact, libp2p::core::multiaddr::multiaddr, Config, Keypair,
    Network,
};
use nimiq_network_mock::{MockHub, MockNetwork};

#[async_trait]
pub trait TestNetwork<N = Self>
where
    N: NetworkInterface,
{
    async fn build_network(
        peer_id: u64,
        genesis_hash: Blake2bHash,
        hub: &mut Option<MockHub>,
    ) -> Arc<Self>;
    async fn connect_networks(networks: &[Arc<N>], seed_peer_id: u64);
}

#[async_trait]
impl TestNetwork for MockNetwork {
    async fn build_network(
        peer_id: u64,
        _genesis_hash: Blake2bHash,
        hub: &mut Option<MockHub>,
    ) -> Arc<MockNetwork> {
        let hub = hub
            .as_mut()
            .expect("Can't build a Mock Network without a MockHub");
        Arc::new(hub.new_network_with_address(peer_id))
    }

    async fn connect_networks(networks: &[Arc<MockNetwork>], _seed_peer_id: u64) {
        // Connect validators to each other.
        for (id, network) in networks.iter().enumerate() {
            for other_id in (id + 1)..networks.len() {
                let other_network = networks.get(other_id).unwrap();
                network.dial_mock(other_network);
            }
        }
    }
}

#[async_trait]
impl TestNetwork for Network {
    async fn build_network(
        peer_id: u64,
        genesis_hash: Blake2bHash,
        _hub: &mut Option<MockHub>,
    ) -> Arc<Network> {
        let peer_key = Keypair::generate_ed25519();
        let peer_address = multiaddr![Memory(peer_id)];
        let mut peer_contact = PeerContact::new(
            vec![peer_address.clone()],
            peer_key.public(),
            Services::all(),
            None,
        )
        .expect("Could not create peer contact");
        peer_contact.set_current_time();
        let config = Config::new(
            peer_key,
            peer_contact,
            Vec::new(),
            genesis_hash.clone(),
            true,
            Services::all(),
            None,
            3,
            4000,
            20,
            20,
            false,
            true,
            NonZeroU8::new(1).unwrap(),
        );
        let network = Arc::new(Network::new(config).await);
        network.listen_on(vec![peer_address]).await;
        network
    }

    async fn connect_networks(networks: &[Arc<Network>], seed_peer_id: u64) {
        let seed = multiaddr![Memory(seed_peer_id)];
        // Skip the last network assuming the last one is the seed and doesn't make
        // sense for the seed to connect to itself.
        for network in &networks[0..networks.len() - 1] {
            // Tell the network to connect to seed nodes
            network
                .dial_address(seed.clone())
                .await
                .expect("Failed to dial seed");
        }
    }
}
