use libp2p::{
    gossipsub::{GossipsubConfig, GossipsubConfigBuilder},
    identity::Keypair,
    kad::KademliaConfig,
    Multiaddr,
};

use nimiq_hash::Blake2bHash;

use crate::{
    discovery::{behaviour::DiscoveryConfig, peer_contacts::PeerContact},
    message::behaviour::MessageConfig,
};

pub struct Config {
    pub keypair: Keypair,
    pub peer_contact: PeerContact,
    pub min_peers: usize,
    pub seeds: Vec<Multiaddr>,
    pub discovery: DiscoveryConfig,
    pub message: MessageConfig,
    pub kademlia: KademliaConfig,
    pub gossipsub: GossipsubConfig,
}

impl Config {
    pub fn new(
        keypair: Keypair,
        peer_contact: PeerContact,
        seeds: Vec<Multiaddr>,
        genesis_hash: Blake2bHash,
    ) -> Self {
        // Hardcoding the minimum number of peers in mesh network before adding more
        // TODO: Maybe change this to a mesh limits configuration argument of this function
        let gossipsub_config = GossipsubConfigBuilder::default()
            .mesh_n_low(3)
            .validate_messages()
            .build()
            .expect("Invalid Gossipsub config");

        Self {
            keypair,
            peer_contact,
            min_peers: 5,
            seeds,
            discovery: DiscoveryConfig::new(genesis_hash),
            message: MessageConfig::default(),
            kademlia: KademliaConfig::default(),
            gossipsub: gossipsub_config,
        }
    }
}
