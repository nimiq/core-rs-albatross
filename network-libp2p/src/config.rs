use libp2p::{
    gossipsub::{GossipsubConfig, GossipsubConfigBuilder, MessageId},
    identity::Keypair,
    kad::{KademliaBucketInserts, KademliaConfig, KademliaStoreInserts},
    Multiaddr,
};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    time::Duration,
};

use nimiq_hash::Blake2bHash;

use crate::discovery::{behaviour::DiscoveryConfig, peer_contacts::PeerContact};

pub struct Config {
    pub keypair: Keypair,
    pub peer_contact: PeerContact,
    pub seeds: Vec<Multiaddr>,
    pub discovery: DiscoveryConfig,
    pub kademlia: KademliaConfig,
    pub gossipsub: GossipsubConfig,
    pub memory_transport: bool,
}

impl Config {
    pub fn new(
        keypair: Keypair,
        peer_contact: PeerContact,
        seeds: Vec<Multiaddr>,
        genesis_hash: Blake2bHash,
        memory_transport: bool,
    ) -> Self {
        // Hardcoding the minimum number of peers in mesh network before adding more
        // TODO: Maybe change this to a mesh limits configuration argument of this function
        let gossipsub = GossipsubConfigBuilder::default()
            .mesh_n_low(3)
            .validate_messages()
            .max_transmit_size(1_000_000) // TODO find a reasonable value for this parameter
            .validation_mode(libp2p::gossipsub::ValidationMode::Permissive)
            .heartbeat_interval(Duration::from_millis(700))
            // Use the message hash as the message ID instead of the default PeerId + sequence_number
            // to avoid duplicated messages
            .message_id_fn(|message| {
                let mut s = DefaultHasher::new();
                message.data.hash(&mut s);
                MessageId::from(s.finish().to_string())
            })
            .build()
            .expect("Invalid Gossipsub config");

        let mut kademlia = KademliaConfig::default();
        kademlia.set_kbucket_inserts(KademliaBucketInserts::OnConnected);
        kademlia.set_record_ttl(Some(Duration::from_secs(5 * 60)));
        kademlia.set_publication_interval(Some(Duration::from_secs(60)));

        // Since we have a record TTL of 5 minutes, record replication is not needed right now
        kademlia.set_replication_interval(None);
        kademlia.set_record_filtering(KademliaStoreInserts::FilterBoth);

        Self {
            keypair,
            peer_contact: peer_contact.clone(),
            seeds,
            discovery: DiscoveryConfig::new(genesis_hash, peer_contact.services),
            kademlia,
            gossipsub,
            memory_transport,
        }
    }
}
