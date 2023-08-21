use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    time::Duration,
};

use libp2p::{gossipsub, identity::Keypair, kad, Multiaddr};
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::peer_info::Services;

use crate::discovery::{self, peer_contacts::PeerContact};

/// TLS settings for configuring a secure WebSocket
pub struct TlsConfig {
    /// Private key (DER-encoded ASN.1 in either PKCS#8 or PKCS#1 format).
    pub private_key: Vec<u8>,
    /// Certificates (in DER-encoded X.509 format). Each of the entries of the vector is a certificate
    /// represented in a `Vec<u8>`.
    pub certificates: Vec<Vec<u8>>,
}

/// LibP2P network configuration
pub struct Config {
    pub keypair: Keypair,
    pub peer_contact: PeerContact,
    pub seeds: Vec<Multiaddr>,
    pub discovery: discovery::Config,
    pub kademlia: kad::Config,
    pub gossipsub: gossipsub::Config,
    pub memory_transport: bool,
    pub required_services: Services,
    pub tls: Option<TlsConfig>,
}

impl Config {
    pub fn new(
        keypair: Keypair,
        peer_contact: PeerContact,
        seeds: Vec<Multiaddr>,
        genesis_hash: Blake2bHash,
        memory_transport: bool,
        required_services: Services,
        tls_settings: Option<TlsConfig>,
    ) -> Self {
        // Hardcoding the minimum number of peers in mesh network before adding more
        // TODO: Maybe change this to a mesh limits configuration argument of this function
        let gossipsub = gossipsub::ConfigBuilder::default()
            .mesh_n_low(3)
            .validate_messages()
            .max_transmit_size(1_000_000) // TODO find a reasonable value for this parameter
            .validation_mode(libp2p::gossipsub::ValidationMode::Permissive)
            .heartbeat_interval(Duration::from_millis(700))
            // Use the message hash as the message ID instead of the default PeerId + sequence_number
            // to avoid duplicated messages
            .message_id_fn(|message| {
                let mut s = DefaultHasher::new();
                message.topic.hash(&mut s);
                message.data.hash(&mut s);
                gossipsub::MessageId::from(s.finish().to_string())
            })
            .build()
            .expect("Invalid Gossipsub config");

        let mut kademlia = kad::Config::default();
        kademlia.set_kbucket_inserts(kad::BucketInserts::OnConnected);
        kademlia.set_record_ttl(Some(Duration::from_secs(5 * 60)));
        kademlia.set_publication_interval(Some(Duration::from_secs(60)));

        // Since we have a record TTL of 5 minutes, record replication is not needed right now
        kademlia.set_replication_interval(None);
        kademlia.set_record_filtering(kad::StoreInserts::FilterBoth);

        Self {
            keypair,
            peer_contact,
            seeds,
            discovery: discovery::Config::new(genesis_hash, required_services),
            kademlia,
            gossipsub,
            memory_transport,
            required_services,
            tls: tls_settings,
        }
    }
}
