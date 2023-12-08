use std::time::Duration;

use libp2p::{gossipsub, identity::Keypair, kad, Multiaddr};
use nimiq_hash::Blake2bHash;
use nimiq_network_interface::peer_info::Services;
use sha2::{Digest, Sha256};

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
    pub desired_peer_count: usize,
    pub autonat_allow_non_global_ips: bool,
    pub only_secure_ws_connections: bool,
    pub allow_loopback_addresses: bool,
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
        desired_peer_count: usize,
        autonat_allow_non_global_ips: bool,
        only_secure_ws_connections: bool,
        allow_loopback_addresses: bool,
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
                let mut s = Sha256::new();
                s.update(message.topic.as_str());
                s.update(&message.data);
                gossipsub::MessageId::from(s.finalize().to_vec())
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
            discovery: discovery::Config::new(
                genesis_hash,
                required_services,
                only_secure_ws_connections,
            ),
            kademlia,
            gossipsub,
            memory_transport,
            required_services,
            tls: tls_settings,
            desired_peer_count,
            autonat_allow_non_global_ips,
            only_secure_ws_connections,
            allow_loopback_addresses,
        }
    }
}
