use async_trait::async_trait;
use std::sync::Arc;

use nimiq_hash::Blake2bHash;
use nimiq_network_interface::network::Network as NetworkInterface;
use nimiq_network_libp2p::discovery::peer_contacts::{PeerContact, Services};
use nimiq_network_libp2p::libp2p::core::multiaddr::multiaddr;
use nimiq_network_libp2p::{Config, Keypair, Network};
use nimiq_network_mock::{MockHub, MockNetwork};
use nimiq_utils::time::OffsetTime;

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
    async fn connect_network(networks: &[Arc<N>]);
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
            .expect("Can't build a Mock Network without a MuckHub");
        Arc::new(hub.new_network_with_address(peer_id))
    }

    async fn connect_network(networks: &[Arc<MockNetwork>]) {
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
        let clock = Arc::new(OffsetTime::new());
        let peer_key = Keypair::generate_ed25519();
        let peer_address = multiaddr![Memory(peer_id)];
        let mut peer_contact = PeerContact::new(
            vec![peer_address.clone()],
            peer_key.public(),
            Services::all(),
            None,
        );
        peer_contact.set_current_time();
        let config = Config::new(peer_key, peer_contact, Vec::new(), genesis_hash.clone());
        let network = Arc::new(Network::new(clock, config).await);
        network.listen_on(vec![peer_address]).await;
        network
    }

    async fn connect_network(networks: &[Arc<Network>]) {
        for network in networks {
            // Tell the network to connect to seed nodes
            let seed = multiaddr![Memory(1u64)];
            log::debug!("Dialing seed: {:?}", seed);
            network
                .dial_address(seed)
                .await
                .expect("Failed to dial seed");
        }
    }
}
