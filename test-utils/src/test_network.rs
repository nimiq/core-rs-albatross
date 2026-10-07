use std::{future::Future, num::NonZeroU8, sync::Arc};

use instant::SystemTime;
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::{network::Network as NetworkInterface, peer_info::Services};
use nimiq_network_libp2p::{
    discovery::peer_contacts::PeerContact, libp2p::core::multiaddr::multiaddr, Config, Keypair,
    Network,
};
use nimiq_network_mock::{MockHub, MockNetwork};

pub trait TestNetwork<N = Self>
where
    N: NetworkInterface,
{
    fn build_network(
        peer_id: u64,
        genesis_hash: Blake2bHash,
        hub: &mut Option<MockHub>,
    ) -> impl Future<Output = Arc<Self>> + Send;
    fn connect_networks(networks: &[Arc<N>], seed_peer_id: u64) -> impl Future<Output = ()> + Send;
}

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

impl TestNetwork for Network {
    async fn build_network(
        _peer_id: u64,
        genesis_hash: Blake2bHash,
        _hub: &mut Option<MockHub>,
    ) -> Arc<Network> {
        let peer_key = Keypair::generate_ed25519();
        // libp2p's memory transport only releases a listening port on `remove_listener`, not when
        // the swarm is dropped, so a port stays taken for the rest of the process. Tests sharing a
        // process (`cargo test`) would collide on fixed ports, so pick a random non-zero one.
        let peer_address = multiaddr![Memory(rand::random::<u64>().max(1))];
        let peer_contact = PeerContact::new(
            vec![peer_address.clone()],
            peer_key.public(),
            Services::all(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
        .expect("Could not create peer contact");
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
            1,
            true,
            NonZeroU8::new(1).unwrap(),
            1024,
        );
        let network = Arc::new(Network::new(config, ()).await);
        network.listen_on(vec![peer_address]).await;
        network
    }

    async fn connect_networks(networks: &[Arc<Network>], _seed_peer_id: u64) {
        // The last network is the seed and doesn't make sense for the seed to connect to itself.
        let (seed_network, networks) = networks.split_last().expect("No networks to connect");
        let seed = seed_network
            .get_own_addresses()
            .into_iter()
            .next()
            .expect("Seed has no address");
        for network in networks {
            // Tell the network to connect to seed nodes
            network
                .dial_address(seed.clone())
                .await
                .expect("Failed to dial seed");
        }
    }
}
